use crate::network::server::Server;
use crate::utils::db::{
    Session,
    current_timestamp,
    find_session_by_browser_token,
    generate_browser_token,
    generate_secure_id,
    settle_and_get_session,
    create_session_without_device,
};

fn json_error(status: u16, message: &str, res: &mut crate::network::server::Response) {
    res.status = status;
    res.body = serde_json::json!({ "error": message }).to_string().into_bytes();
    res.content_type = "application/json".to_string();
}

pub fn init(server: &mut Server) {
    server.get("/api/init", |req, res| {
        let now = current_timestamp();

        let supplied_token = req
            .query
            .split('&')
            .filter_map(|part| {
                let (key, value) = part.split_once('=')?;
                if key == "browser_token" {
                    Some(value)
                } else {
                    None
                }
            })
            .next()
            .filter(|token| {
                token.len() == 64
                    && token.bytes().all(|b| b.is_ascii_hexdigit())
            });

        // A browser token is the only credential that can restore a customer
        // session. A matching MAC address must never recover or claim a session.
        let existing_session_id = if let Some(token) = supplied_token {
            match find_session_by_browser_token(token) {
                Ok(id) => id,
                Err(_) => {
                    json_error(500, "Could not look up browser session", res);
                    return;
                }
            }
        } else {
            None
        };

        let (session_id, browser_token) = match existing_session_id {
            Some(id) => (id, supplied_token.unwrap().to_string()),
            None => {
                // Unknown, missing, or malformed tokens cannot claim an
                // existing session. Issue a fresh token and create a new one.
                let token = match generate_browser_token() {
                    Ok(token) => token,
                    Err(_) => {
                        json_error(500, "Could not generate recovery token", res);
                        return;
                    }
                };

                let id = match generate_secure_id() {
                    Ok(id) => id,
                    Err(_) => {
                        json_error(500, "Could not generate session ID", res);
                        return;
                    }
                };

                let session = Session::new(String::new(), req.ip.clone(), now);

                if create_session_without_device(&id, &session, &token).is_err() {
                    json_error(500, "Could not create session", res);
                    return;
                }

                (id, token)
            }
        };

        let session = match settle_and_get_session(&session_id, now) {
            Ok(Some(session)) => session,
            Ok(None) => {
                json_error(404, "Session not found", res);
                return;
            }
            Err(_) => {
                json_error(500, "Could not update session", res);
                return;
            }
        };

        let response = serde_json::json!({
            "session_id": session_id,
            "browser_token": browser_token,
            "name": session.name,
            "ip": req.ip,
            "mac": crate::utils::get_mac_from_ip::get_mac_from_ip(&req.ip),
            "paused": session.paused,
            "pause_attempts": session.pause_attempts,
            "pause_day": session.pause_day,
            "remaining_seconds": session.remaining_seconds,
            "status": session.status(now),
            "now": now
        });

        res.status = 200;
        res.body = response.to_string().into_bytes();
        res.content_type = "application/json".to_string();
    });
}
