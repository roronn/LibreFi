use crate::network::server::{Response, Server};
use crate::utils::db::{
    current_timestamp,
    get_db,
    redeem_voucher_for_customer_session,
    CONFIG_TABLE,
    VOUCHERS_TABLE,
    SESSIONS_TABLE,
    Voucher,
    Session,
    WIFI_GRACE_SECONDS,
};
use redb::ReadableDatabase;
use serde::Deserialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Deserialize)]
struct RedeemBody {
    code: String,
    browser_token: Option<String>,
}

fn json_error(status: u16, message: &str, res: &mut Response) {
    res.status = status;
    res.body = serde_json::json!({ "error": message }).to_string().into_bytes();
    res.content_type = "application/json".to_string();
}

fn is_valid_mac(mac: &str) -> bool {
    let parts: Vec<&str> = mac.split(':').collect();

    parts.len() == 6
        && parts
            .iter()
            .all(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_hexdigit()))
}

fn get_fail_count(mac: &str) -> (u32, u64) {
    let db = get_db();

    if let Ok(read_txn) = db.begin_read() {
        if let Ok(table) = read_txn.open_table(CONFIG_TABLE) {
            let key = format!("redeem_fail:{}", mac);

            if let Ok(Some(value)) = table.get(key.as_str()) {
                let parts: Vec<&str> = value.value().splitn(2, ':').collect();

                if parts.len() == 2 {
                    return (
                        parts[0].parse::<u32>().unwrap_or(0),
                        parts[1].parse::<u64>().unwrap_or(0),
                    );
                }
            }
        }
    }

    (0, 0)
}

fn set_fail_count(mac: &str, count: u32) {
    let db = get_db();

    if let Ok(write_txn) = db.begin_write() {
        if let Ok(mut table) = write_txn.open_table(CONFIG_TABLE) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            let key = format!("redeem_fail:{}", mac);
            let value = format!("{}:{}", count, now);

            let _ = table.insert(key.as_str(), value.as_str());
        }

        let _ = write_txn.commit();
    }
}

fn clear_fail_count(mac: &str) {
    let db = get_db();

    if let Ok(write_txn) = db.begin_write() {
        if let Ok(mut table) = write_txn.open_table(CONFIG_TABLE) {
            let key = format!("redeem_fail:{}", mac);
            let _ = table.remove(key.as_str());
        }

        let _ = write_txn.commit();
    }
}

pub fn redeem(server: &mut Server) {
    // Read-only check for the last successfully redeemed voucher.
    server.post("/api/voucher/recovery-status", |req, res| {
        if req.body.len() > 65536 {
            json_error(413, "Payload too large", res);
            return;
        }

        let body_str = match String::from_utf8(req.body.clone()) {
            Ok(value) => value,
            Err(_) => {
                json_error(400, "Body is not valid UTF-8", res);
                return;
            }
        };

        let body: RedeemBody = match serde_json::from_str(&body_str) {
            Ok(value) => value,
            Err(_) => {
                json_error(400, "Expected JSON containing a voucher code", res);
                return;
            }
        };

        let code = body.code.trim();

        if code.is_empty() || code.len() > 64 {
            json_error(400, "Invalid voucher code", res);
            return;
        }

        let status = (|| -> Result<bool, String> {
            let db = get_db();
            let read_txn = db.begin_read().map_err(|e| e.to_string())?;

            let voucher = {
                let table = read_txn
                    .open_table(VOUCHERS_TABLE)
                    .map_err(|e| e.to_string())?;

                match table.get(code).map_err(|e| e.to_string())? {
                    Some(value) => {
                        serde_json::from_str::<Voucher>(value.value())
                            .map_err(|e| e.to_string())?
                    }
                    None => return Ok(false),
                }
            };

            if !voucher.used {
                return Ok(false);
            }

            let Some(session_id) = voucher.session_id else {
                return Ok(false);
            };

            let session_json = {
                let table = read_txn
                    .open_table(SESSIONS_TABLE)
                    .map_err(|e| e.to_string())?;

                table
                    .get(session_id.as_str())
                    .map_err(|e| e.to_string())?
                    .map(|value| value.value().to_string())
            };

            let Some(session_json) = session_json else {
                return Ok(false);
            };

            let mut session: Session =
                serde_json::from_str(&session_json).map_err(|e| e.to_string())?;

            session.settle(current_timestamp());

            Ok(
                session.expired_at.is_none()
                    && session.remaining_seconds > 0
                    && !session.wifi_connected
                    && session.disconnected_at.is_some_and(|disconnected_at| {
                        current_timestamp().saturating_sub(disconnected_at) >= WIFI_GRACE_SECONDS
                    })
            )
        })();

        match status {
            Ok(recoverable) => {
                res.status = 200;
                res.body = serde_json::json!({
                    "recoverable": recoverable
                })
                .to_string()
                .into_bytes();
                res.content_type = "application/json".to_string();
            }
            Err(error) => {
                crate::debug_println!("Voucher recovery status check failed: {}", error);
                json_error(500, "Could not check voucher recovery status", res);
            }
        }
    });

    server.post("/api/redeem", |req, res| {
        if req.body.len() > 65536 {
            json_error(413, "Payload too large", res);
            return;
        }

        let body_str = match String::from_utf8(req.body.clone()) {
            Ok(value) => value,
            Err(_) => {
                json_error(400, "Body is not valid UTF-8", res);
                return;
            }
        };

        let body: RedeemBody = match serde_json::from_str(&body_str) {
            Ok(value) => value,
            Err(_) => {
                json_error(400, "Expected JSON containing a voucher code and browser token", res);
                return;
            }
        };

        let code = body.code.trim();
        let browser_token = body.browser_token.as_deref().unwrap_or("").trim();

        if code.is_empty() || code.len() > 64 {
            json_error(400, "Invalid voucher code", res);
            return;
        }

        // Rate-limit by client IP only; MAC is not used as customer identity.
        let client_key = req.ip.clone();
        let (fail_count, last_fail_ts) = get_fail_count(&client_key);
        let now = current_timestamp();

        if fail_count >= 5 {
            if now.saturating_sub(last_fail_ts) < 60 {
                json_error(
                    429,
                    "Too many failed attempts. Try again in 60 seconds.",
                    res,
                );
                return;
            }
            clear_fail_count(&client_key);
        }

        match redeem_voucher_for_customer_session(code, body.browser_token.as_deref()) {
            Ok((session_id, session, credited, new_browser_token)) => {
                clear_fail_count(&client_key);
                let now = current_timestamp();

                crate::debug_println!(
                    "Voucher operation completed for customer session {}, credited={}",
                    session_id,
                    credited
                );

                res.status = 200;
                res.body = serde_json::json!({
                    "ok": true,
                    "session_id": session_id,
                    "browser_token": new_browser_token,
                    "name": session.name,
                    "ip": req.ip,
                    "paused": session.paused,
                    "pause_attempts": session.pause_attempts,
                    "pause_day": session.pause_day,
                    "remaining_seconds": session.remaining_seconds,
                    "status": session.status(now),
                    "now": now,
                    "credited": credited,
                    "recovered": !credited
                })
                .to_string()
                .into_bytes();
                res.content_type = "application/json".to_string();
            }
            Err(error) => {
                if error == "Voucher not found" {
                    set_fail_count(&client_key, fail_count.saturating_add(1));
                    json_error(404, "Voucher not found", res);
                } else if error.contains("seven-minute disconnect grace period")
                    || error == "Voucher recovery is not yet eligible"
                {
                    json_error(
                        409,
                        "This voucher can recover your session after the seven-minute disconnect grace period.",
                        res,
                    );
                } else if error == "Voucher session expired"
                    || error == "Original voucher session not found"
                {
                    json_error(
                        409,
                        "Your previous voucher could not be recovered. Your session has expired. Please purchase a new voucher or insert coins.",
                        res,
                    );
                } else if error == "Browser token is not recognized"
                    || error == "A valid browser token is required"
                {
                    json_error(401, "Your customer session could not be verified. Refresh the page and try again.", res);
                } else if error == "Voucher belongs to a different customer" {
                    json_error(403, "This voucher belongs to a different customer.", res);
                } else {
                    crate::debug_println!("Voucher operation failed: {}", error);
                    json_error(409, "Could not recover or redeem this voucher for the current session.", res);
                }
            }
        }
    });
}
}
