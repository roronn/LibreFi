use crate::network::server::Server;
use crate::utils::db::{
    Session,
    current_timestamp,
    find_session_by_browser_token,
    find_session_by_mac,
    generate_browser_token,
    generate_secure_id,
    settle_and_get_session,
    link_device_and_browser_to_session,
    create_session,
};
use crate::utils::get_mac_from_ip::get_mac_from_ip;

fn json_error(status: u16, message: &str, res: &mut crate::network::server::Response) {
    res.status = status;
    res.body = serde_json::json!({ "error": message }).to_string().into_bytes();
    res.content_type = "application/json".to_string();
}

pub fn init(server: &mut Server) {
    server.get("/api/init", |req, res| {
        let Some(mac) = get_mac_from_ip(&req.ip) else {
            json_error(403, "Device not recognized", res);
            return;
        };

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

        let browser_token = match supplied_token {
            Some(token) => token.to_string(),
            None => match generate_browser_token() {
                Ok(token) => token,
                Err(_) => {
                    json_error(500, "Could not generate recovery token", res);
                    return;
                }
            },
        };

        let session_id = if let Some(token) = supplied_token {
            match find_session_by_browser_token(token) {
                Ok(Some(id)) => Some(id),
                Ok(None) => None,
                Err(_) => {
                    json_error(500, "Could not look up browser session", res);
                    return;
                }
            }
        } else {
            None
        };

        let session_id = match session_id {
            Some(id) => Some(id),
            None => match find_session_by_mac(&mac) {
                Ok(id) => id,
                Err(_) => {
                    json_error(500, "Could not look up device session", res);
                    return;
                }
            },
        };

        let session_id = match session_id {
            Some(id) => {
                if link_device_and_browser_to_session(&id, &mac, &browser_token).is_err() {
                    json_error(500, "Could not link recovery information", res);
                    return;
                }
                id
            }
            None => {
                let id = match generate_secure_id() {
                    Ok(id) => id,
                    Err(_) => {
                        json_error(500, "Could not generate session ID", res);
                        return;
                    }
                };

                let session = Session::new(String::new(), req.ip.clone(), now);

                if create_session(&id, &session, &mac, &browser_token).is_err() {
                    json_error(500, "Could not create session", res);
                    return;
                }

                id
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
            "mac": mac,
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
