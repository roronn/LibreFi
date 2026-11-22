use axum::{
    body::{to_bytes, Body},
    extract::{ConnectInfo, State},
    http::{HeaderName, HeaderValue, Request as HttpRequest, StatusCode},
    response::Response as AxumResponse,
    routing::{any, get},
    Router,
};
use redb::ReadableDatabase;
use rust_embed::RustEmbed;
use std::{
    collections::HashMap,
    net::SocketAddr,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

#[derive(RustEmbed)]
#[folder = "dist"]
pub struct Asset;

pub struct Request {
    pub body: Vec<u8>,
    pub ip: String,
    pub query: String,
}

pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    pub content_type: String,
    pub headers: HashMap<String, String>,
}

type Handler = Arc<dyn Fn(&Request, &mut Response) + Send + Sync>;

struct RouteTable {
    get_routes: HashMap<String, Handler>,
    post_routes: HashMap<String, Handler>,
    put_routes: HashMap<String, Handler>,
    delete_routes: HashMap<String, Handler>,
}

pub struct Server {
    pub port: u16,
    get_routes: HashMap<String, Handler>,
    post_routes: HashMap<String, Handler>,
    put_routes: HashMap<String, Handler>,
    delete_routes: HashMap<String, Handler>,
}

impl Server {
    pub fn new(port: u16) -> Self {
        Server {
            port,
            get_routes: HashMap::new(),
            post_routes: HashMap::new(),
            put_routes: HashMap::new(),
            delete_routes: HashMap::new(),
        }
    }

    pub fn get<F>(&mut self, path: &str, handler: F)
    where
        F: Fn(&Request, &mut Response) + Send + Sync + 'static,
    {
        self.get_routes.insert(path.to_string(), Arc::new(handler));
    }

    pub fn post<F>(&mut self, path: &str, handler: F)
    where
        F: Fn(&Request, &mut Response) + Send + Sync + 'static,
    {
        self.post_routes.insert(path.to_string(), Arc::new(handler));
    }

    pub fn put<F>(&mut self, path: &str, handler: F)
    where
        F: Fn(&Request, &mut Response) + Send + Sync + 'static,
    {
        self.put_routes.insert(path.to_string(), Arc::new(handler));
    }

    pub fn delete<F>(&mut self, path: &str, handler: F)
    where
        F: Fn(&Request, &mut Response) + Send + Sync + 'static,
    {
        self.delete_routes.insert(path.to_string(), Arc::new(handler));
    }

    pub fn run(&self) {
        let port = self.port;

        let routes = Arc::new(RouteTable {
            get_routes: self.get_routes.clone(),
            post_routes: self.post_routes.clone(),
            put_routes: self.put_routes.clone(),
            delete_routes: self.delete_routes.clone(),
        });

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create Tokio runtime");

        runtime.block_on(async move {
            let address = SocketAddr::from(([0, 0, 0, 0], port));

            let listener = tokio::net::TcpListener::bind(address)
                .await
                .unwrap_or_else(|error| {
                    panic!("Failed to bind HTTP server on {}: {}", address, error)
                });

            println!("Axum server running on port {}", port);

            let app = Router::new()
                .route("/ws", get(crate::routes::ws::customer_handler::customer_ws_handler))
                .route("/ws/vendo", get(crate::routes::ws::axum_handler::axum_ws_handler))
                .fallback(any(dispatch))
                .with_state(routes);

            if let Err(error) = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            {
                eprintln!("HTTP server error: {}", error);
            }
        });
    }
}

async fn dispatch(
    State(routes): State<Arc<RouteTable>>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    request: HttpRequest<Body>,
) -> AxumResponse {
    let method = request.method().clone();
    let raw_path = request.uri().path().to_string();
    let query = request.uri().query().unwrap_or("").to_string();

    let mut path = raw_path.clone();

    if path.len() > 1 && path.ends_with('/') {
        path.pop();
    }

    let ip = remote.ip().to_string();

    let headers: HashMap<String, String> = request
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_lowercase(), value.to_string()))
        })
        .collect();

    crate::debug_println!(
        "-> Received {} request for: '{}' (mapped to '{}')",
        method,
        raw_path,
        path
    );

    let body = match to_bytes(request.into_body(), 16 * 1024 * 1024).await {
        Ok(body) => body.to_vec(),
        Err(_) => {
            return simple_response(
                StatusCode::BAD_REQUEST,
                b"400 - Invalid request body".to_vec(),
            );
        }
    };

    let result = tokio::task::spawn_blocking(move || {
        handle_request(
            &routes,
            &method.to_string().to_lowercase(),
            &path,
            &query,
            &ip,
            body,
            &headers,
        )
    })
    .await;

    match result {
        Ok(response) => build_response(response),
        Err(error) => {
            eprintln!("HTTP request task failed: {}", error);
            simple_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                b"500 - Internal Server Error".to_vec(),
            )
        }
    }
}

fn handle_request(
    routes: &RouteTable,
    method: &str,
    path: &str,
    query: &str,
    ip: &str,
    body: Vec<u8>,
    headers: &HashMap<String, String>,
) -> Response {
    let req = Request {
        body,
        ip: ip.to_string(),
        query: query.to_string(),
    };

    let mut res = Response {
        status: 404,
        body: b"404 - Your father Not Found".to_vec(),
        content_type: "text/plain".to_string(),
        headers: HashMap::new(),
    };

    let route = match method {
        "get" => routes.get_routes.get(path),
        "post" => routes.post_routes.get(path),
        "put" => routes.put_routes.get(path),
        "delete" => routes.delete_routes.get(path),
        _ => None,
    };

    if let Some(handler) = route {
        if catch_unwind(AssertUnwindSafe(|| handler(&req, &mut res))).is_err() {
            res.status = 500;
            res.body = b"500 - Internal Server Error".to_vec();
            res.content_type = "text/plain".to_string();
        }

        return res;
    }

    if method == "options" {
        res.status = 204;
        res.body.clear();
        return res;
    }

    if method != "get" {
        return res;
    }

    let mut safe_path = path.trim_start_matches('/').replace("..", "");

    if safe_path.is_empty() {
        safe_path = "index.html".to_string();
    }

    let supports_gzip = headers
        .get("accept-encoding")
        .map(|value| value.to_lowercase().contains("gzip"))
        .unwrap_or(false);

    let get_asset = |asset_path: &str| -> Option<(Vec<u8>, bool)> {
        if supports_gzip {
            if let Some(file) = Asset::get(&format!("{}.gz", asset_path)) {
                return Some((file.data.into_owned(), true));
            }
        }

        Asset::get(asset_path).map(|file| (file.data.into_owned(), false))
    };

    let mut contents_res = get_asset(&safe_path);

    let captive_paths = [
        "/hotspot-detect.html",
        "/generate_204",
        "/gen_204",
        "/ncsi.txt",
        "/connecttest.txt",
        "/success.txt",
        "/canonical.html",
        "/library/test/success.html",
    ];

    let is_cpd = captive_paths.iter().any(|cp_path| path.contains(cp_path));

    let mut has_access = false;

    if let Some(mac) = crate::utils::get_mac_from_ip::get_mac_from_ip(&req.ip) {
        let db = crate::utils::db::get_db();

        if let Ok(read_txn) = db.begin_read() {
            if let Ok(table) = read_txn.open_table(crate::utils::db::USERS_TABLE) {
                if let Ok(Some(record)) = table.get(mac.as_str()) {
                    if let Ok(user) =
                        serde_json::from_str::<crate::utils::db::User>(record.value())
                    {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs() as u32;

                        if user.expires_on > now && !user.paused {
                            has_access = true;
                        }
                    }
                }
            }
        }
    }

    let mut handled_cpd = false;

    if is_cpd && has_access {
        handled_cpd = true;

        if path.contains("hotspot-detect.html") || path.contains("success.html") {
            res.status = 200;
            res.body =
                b"<HTML><HEAD><TITLE>Success</TITLE></HEAD><BODY>Success</BODY></HTML>"
                    .to_vec();
            res.content_type = "text/html".to_string();
        } else if path.contains("success.txt") {
            res.status = 200;
            res.body = b"success\n".to_vec();
            res.content_type = "text/plain".to_string();
        } else {
            res.status = 204;
            res.body.clear();
        }
    } else if is_cpd && !has_access {
        handled_cpd = true;
        res.status = 302;
        res.headers.insert(
            "Location".to_string(),
            "http://10.0.0.1/".to_string(),
        );
        res.body = b"Found".to_vec();
    } else if contents_res.is_none() {
        safe_path = "index.html".to_string();
        contents_res = get_asset(&safe_path);
    }

    if !handled_cpd {
        if let Some((contents, is_gz)) = contents_res {
            res.status = 200;
            res.body = contents;

            if safe_path.ends_with(".html") {
                res.content_type = "text/html".to_string();
            } else if safe_path.ends_with(".css") {
                res.content_type = "text/css".to_string();
            } else if safe_path.ends_with(".js") {
                res.content_type = "application/javascript".to_string();
            } else if safe_path.ends_with(".json") {
                res.content_type = "application/json".to_string();
            } else if safe_path.ends_with(".svg") {
                res.content_type = "image/svg+xml".to_string();
            } else if safe_path.ends_with(".woff2") {
                res.content_type = "font/woff2".to_string();
            }

            if is_gz {
                res.headers.insert(
                    "Content-Encoding".to_string(),
                    "gzip".to_string(),
                );
            }
        } else {
            res.status = 404;
            res.body = b"404 - index.html not found!".to_vec();
            res.content_type = "text/plain".to_string();
        }
    }

    res
}

fn build_response(response: Response) -> AxumResponse {
    let status = StatusCode::from_u16(response.status)
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    let mut builder = AxumResponse::builder()
        .status(status)
        .header("Content-Type", response.content_type)
        .header("Access-Control-Allow-Origin", "*")
        .header(
            "Access-Control-Allow-Methods",
            "GET, POST, PUT, DELETE, OPTIONS",
        )
        .header(
            "Access-Control-Allow-Headers",
            "Content-Type, Authorization",
        );

    for (name, value) in response.headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            builder = builder.header(name, value);
        }
    }

    builder
        .body(Body::from(response.body))
        .unwrap_or_else(|_| {
            simple_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                b"500 - Internal Server Error".to_vec(),
            )
        })
}

fn simple_response(status: StatusCode, body: Vec<u8>) -> AxumResponse {
    AxumResponse::builder()
        .status(status)
        .header("Content-Type", "text/plain")
        .body(Body::from(body))
        .unwrap()
}
