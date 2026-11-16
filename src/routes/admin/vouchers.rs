use crate::network::server::Server;
use crate::utils::db::{VOUCHERS_TABLE, Voucher, get_db};
use rand::Rng;
use redb::{ReadableDatabase, ReadableTable};
use serde::{Deserialize, Serialize};

use super::auth::parse_admin_payload;

#[derive(Deserialize)]
struct InsertVoucherPayload {
    price: f64,
    time: u32,
    quantity: usize,
}

#[derive(Deserialize)]
struct DeleteVoucherPayload {
    code: String,
}

#[derive(Deserialize)]
struct PdfDownloadPayload {
    codes: Vec<String>,
}

#[derive(Serialize)]
struct PdfDownloadCount {
    code: String,
    count: u32,
}

#[derive(Serialize)]
struct PdfDownloadResponse {
    ok: bool,
    updated: Vec<PdfDownloadCount>,
}
#[derive(Deserialize, Default)]
struct GetVouchersPayload {
    page: Option<usize>,
}

#[derive(Serialize)]
struct VouchersListResponse {
    ok: bool,
    vouchers: Vec<Voucher>,
    total_pages: usize,
    current_page: usize,
}

#[derive(Serialize)]
struct VouchersBatchResponse {
    ok: bool,
    vouchers: Vec<Voucher>,
}

fn generate_random_code() -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();

    (0..6)
        .map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char)
        .collect()
}

pub fn handle_vouchers(server: &mut Server) {
    server.get("/api/admin/vouchers", |req, res| {
        let json = match parse_admin_payload(req) {
            Ok(j) => j,
            Err((status, body)) => {
                res.status = status;
                res.body = body;
                res.content_type = String::from("application/json");
                return;
            }
        };

        let get_payload: GetVouchersPayload = serde_json::from_str(&json).unwrap_or_default();
        let page = get_payload.page.unwrap_or(1).max(1);
        let limit = 10;

        let db = get_db();
        let mut all_vouchers = Vec::new();

        if let Ok(read_txn) = db.begin_read() {
            if let Ok(table) = read_txn.open_table(VOUCHERS_TABLE) {
                if let Ok(iter) = table.iter() {
                    for item in iter {
                        if let Ok((_key, value)) = item {
                            if let Ok(voucher) = serde_json::from_str::<Voucher>(value.value()) {
                                all_vouchers.push(voucher);
                            }
                        }
                    }
                }
            }
        }

        all_vouchers.sort_by(|a, b| b.created_at.unwrap_or(0).cmp(&a.created_at.unwrap_or(0)));

        let total_vouchers = all_vouchers.len();
        let total_pages = if total_vouchers == 0 {
            1
        } else {
            (total_vouchers as f64 / limit as f64).ceil() as usize
        };

        let start = (page - 1) * limit;
        let end = (start + limit).min(total_vouchers);
        let vouchers = if start < total_vouchers {
            all_vouchers[start..end].to_vec()
        } else {
            Vec::new()
        };

        let resp = VouchersListResponse {
            ok: true,
            vouchers,
            total_pages,
            current_page: page,
        };
        res.status = 200;
        res.body = serde_json::to_vec(&resp).unwrap();
        res.content_type = String::from("application/json");
    });

    server.post("/api/admin/vouchers", |req, res| {
        let json = match parse_admin_payload(req) {
            Ok(j) => j,
            Err((status, body)) => {
                res.status = status;
                res.body = body;
                res.content_type = String::from("application/json");
                return;
            }
        };

        let payload: InsertVoucherPayload = match serde_json::from_str(&json) {
            Ok(p) => p,
            Err(e) => {
                res.status = 400;
                res.body = format!("{{\"error\":\"Invalid voucher payload: {}\"}}", e).into_bytes();
                res.content_type = String::from("application/json");
                return;
            }
        };

        if !payload.price.is_finite()
            || payload.price <= 0.0
            || payload.time == 0
            || payload.quantity == 0
            || payload.quantity > 500
        {
            res.status = 400;
            res.body = br#"{"error":"Price and time must be positive, and quantity must be between 1 and 500."}"#.to_vec();
            res.content_type = String::from("application/json");
            return;
        }

        let db = get_db();
        let write_txn = match db.begin_write() {
            Ok(txn) => txn,
            Err(e) => {
                res.status = 500;
                res.body = format!("{{\"error\":\"Database error: {}\"}}", e).into_bytes();
                res.content_type = String::from("application/json");
                return;
            }
        };

        let mut generated = Vec::with_capacity(payload.quantity);
        let mut generation_error = None;

        {
            let mut table = match write_txn.open_table(VOUCHERS_TABLE) {
                Ok(table) => table,
                Err(e) => {
                    res.status = 500;
                    res.body = format!("{{\"error\":\"Database error: {}\"}}", e).into_bytes();
                    res.content_type = String::from("application/json");
                    return;
                }
            };

            for _ in 0..payload.quantity {
                let mut unique_code = None;

                for _ in 0..100 {
                    let candidate = generate_random_code();

                    match table.get(candidate.as_str()) {
                        Ok(existing) => {
                            if existing.is_none() {
                                unique_code = Some(candidate);
                                break;
                            }
                        }
                        Err(e) => {
                            generation_error = Some(format!("Database error: {}", e));
                            break;
                        }
                    }
                }

                if generation_error.is_some() {
                    break;
                }

                let code = match unique_code {
                    Some(code) => code,
                    None => {
                        generation_error = Some(
                            "Unable to generate a unique voucher code. Please try again.".to_string()
                        );
                        break;
                    }
                };

                let created_at = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();

                let voucher = Voucher {
                    code: code.clone(),
                    price: payload.price,
                    time: payload.time,
                    used: false,
                    pdf_download_count: 0,
                    created_at: Some(created_at),
                    session_id: None,
                    redeemed_at: None,
                };

                let json_value = match serde_json::to_string(&voucher) {
                    Ok(value) => value,
                    Err(e) => {
                        generation_error = Some(format!("Serialization error: {}", e));
                        break;
                    }
                };

                if let Err(e) = table.insert(code.as_str(), json_value.as_str()) {
                    generation_error = Some(format!("Database error: {}", e));
                    break;
                }

                generated.push(voucher);
            }
        }

        if let Some(error) = generation_error {
            drop(write_txn);
            res.status = 500;
            res.body = serde_json::json!({ "error": error }).to_string().into_bytes();
            res.content_type = String::from("application/json");
            return;
        }

        if let Err(e) = write_txn.commit() {
            res.status = 500;
            res.body = format!("{{\"error\":\"Database commit failed: {}\"}}", e).into_bytes();
            res.content_type = String::from("application/json");
            return;
        }

        crate::debug_println!("-> Generated batch of {} vouchers", generated.len());

        let response = VouchersBatchResponse {
            ok: true,
            vouchers: generated,
        };

        res.status = 200;
        res.body = serde_json::to_vec(&response).unwrap();
        res.content_type = String::from("application/json");
    });

    server.post("/api/admin/vouchers/pdf-download", |req, res| {
        let json = match parse_admin_payload(req) {
            Ok(j) => j,
            Err((status, body)) => {
                res.status = status;
                res.body = body;
                res.content_type = String::from("application/json");
                return;
            }
        };

        let payload: PdfDownloadPayload = match serde_json::from_str(&json) {
            Ok(p) => p,
            Err(e) => {
                res.status = 400;
                res.body = format!("{{\"error\":\"Invalid PDF download payload: {}\"}}", e).into_bytes();
                res.content_type = String::from("application/json");
                return;
            }
        };

        if payload.codes.is_empty() {
            res.status = 400;
            res.body = b"{\"error\":\"No voucher codes supplied\"}".to_vec();
            res.content_type = String::from("application/json");
            return;
        }

        let db = get_db();
        let write_txn = db.begin_write().unwrap();
        let mut updated: Vec<PdfDownloadCount> = Vec::new();

        {
            let mut table = write_txn.open_table(VOUCHERS_TABLE).unwrap();

            for code in &payload.codes {
                if updated.iter().any(|item| item.code == *code) {
                    continue;
                }

                let existing_json = match table.get(code.as_str()).unwrap() {
                    Some(value) => value.value().to_string(),
                    None => continue,
                };

                let mut voucher: Voucher = match serde_json::from_str(&existing_json) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                if voucher.used {
                    continue;
                }

                voucher.pdf_download_count =
                    voucher.pdf_download_count.saturating_add(1);

                let updated_json = serde_json::to_string(&voucher).unwrap();
                table.insert(code.as_str(), updated_json.as_str()).unwrap();

                updated.push(PdfDownloadCount {
                    code: code.clone(),
                    count: voucher.pdf_download_count,
                });
            }
        }

        write_txn.commit().unwrap();

        let response = PdfDownloadResponse {
            ok: true,
            updated,
        };

        res.status = 200;
        res.body = serde_json::to_vec(&response).unwrap();
        res.content_type = String::from("application/json");
    });
    server.delete("/api/admin/vouchers", |req, res| {
        let json = match parse_admin_payload(req) {
            Ok(j) => j,
            Err((status, body)) => {
                res.status = status;
                res.body = body;
                res.content_type = String::from("application/json");
                return;
            }
        };

        let payload: DeleteVoucherPayload = match serde_json::from_str(&json) {
            Ok(p) => p,
            Err(e) => {
                res.status = 400;
                res.body = format!("{{\"error\":\"Invalid delete payload: {}\"}}", e).into_bytes();
                res.content_type = String::from("application/json");
                return;
            }
        };

        let db = get_db();
        let write_txn = db.begin_write().unwrap();
        let mut deleted = false;
        {
            let mut table = write_txn.open_table(VOUCHERS_TABLE).unwrap();
            if table.remove(payload.code.as_str()).unwrap().is_some() {
                deleted = true;
            }
        }
        write_txn.commit().unwrap();

        crate::debug_println!("-> Voucher [code={}] deleted: {}", payload.code, deleted);

        res.status = 200;
        res.body = format!(
            "{{\"ok\":true,\"code\":\"{}\",\"deleted\":{}}}",
            payload.code, deleted
        )
        .into_bytes();
        res.content_type = String::from("application/json");
    });
}


