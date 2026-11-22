use crate::network::server::Server;
use crate::utils::db::{
    current_timestamp, DEVICE_SESSIONS_TABLE, SESSIONS_TABLE, get_db,
};
use redb::{ReadableDatabase, ReadableTable};
use serde::Serialize;

use super::auth::parse_admin_payload;

#[derive(serde::Deserialize, Default)]
struct GetUsersPayload {
    page: Option<usize>,
}

#[derive(Serialize, Clone)]
struct UserWithMac {
    mac: String,
    session_id: String,
    name: String,
    ip: String,
    paused: bool,
    pause_attempts: u8,
    pause_day: u32,
    remaining_seconds: u64,
}

#[derive(Serialize)]
struct UsersListResponse {
    ok: bool,
    users: Vec<UserWithMac>,
    total_pages: usize,
    current_page: usize,
}

pub fn handle_users(server: &mut Server) {
    server.delete("/api/admin/users", |req, res| {
        #[derive(serde::Deserialize)]
        struct DeleteUserPayload { session_id: String }

        let json = match parse_admin_payload(req) {
            Ok(json) => json,
            Err((status, body)) => {
                res.status = status;
                res.body = body;
                res.content_type = "application/json".to_string();
                return;
            }
        };
        let payload: DeleteUserPayload = match serde_json::from_str::<DeleteUserPayload>(&json) {
            Ok(payload) if !payload.session_id.trim().is_empty() => payload,
            _ => {
                res.status = 400;
                res.body = br#"{\"ok\":false,\"error\":\"Invalid session_id\"}"#.to_vec();
                res.content_type = "application/json".to_string();
                return;
            }
        };

        match crate::utils::db::delete_customer_session(&payload.session_id) {
            Ok(()) => {
                res.status = 200;
                res.body = br#"{\"ok\":true}"#.to_vec();
            }
            Err(error) => {
                res.status = if error == "Customer session not found" { 404 } else { 500 };
                res.body = serde_json::json!({"ok":false,"error":error}).to_string().into_bytes();
            }
        }
        res.content_type = "application/json".to_string();
    });

    server.get("/api/admin/users", |req, res| {
        let json = match parse_admin_payload(req) {
            Ok(j) => j,
            Err((status, body)) => {
                res.status = status;
                res.body = body;
                res.content_type = String::from("application/json");
                return;
            }
        };

        let payload: GetUsersPayload = serde_json::from_str(&json).unwrap_or_default();
        let page = payload.page.unwrap_or(1).max(1);
        let limit = 10;
        let now = current_timestamp();
        let db = get_db();
        let mut all_users = Vec::new();

        if let Ok(read_txn) = db.begin_read() {
            if let (Ok(devices), Ok(sessions)) = (
                read_txn.open_table(DEVICE_SESSIONS_TABLE),
                read_txn.open_table(SESSIONS_TABLE),
            ) {
                if let Ok(iter) = devices.iter() {
                    for item in iter {
                        let Ok((mac, session_id)) = item else { continue };
                        let Ok(Some(value)) = sessions.get(session_id.value()) else { continue };
                        let Ok(session) = serde_json::from_str::<crate::utils::db::Session>(value.value()) else {
                            continue;
                        };

                        let remaining_seconds = session.remaining_at(now);

                        all_users.push(UserWithMac {
                            mac: mac.value().to_string(),
                            session_id: session_id.value().to_string(),
                            name: session.name,
                            ip: session.ip,
                            paused: session.paused,
                            pause_attempts: session.pause_attempts,
                            pause_day: session.pause_day,
                            remaining_seconds,
                        });
                    }
                }
            }
        }

        all_users.sort_by(|a, b| a.mac.cmp(&b.mac));
        let total_users = all_users.len();
        let total_pages = if total_users == 0 { 1 } else { (total_users + limit - 1) / limit };
        let start = (page - 1) * limit;
        let end = (start + limit).min(total_users);
        let users = if start < total_users { all_users[start..end].to_vec() } else { Vec::new() };

        let response = UsersListResponse { ok: true, users, total_pages, current_page: page };
        res.status = 200;
        res.body = serde_json::to_vec(&response).unwrap();
        res.content_type = String::from("application/json");
    });
}
