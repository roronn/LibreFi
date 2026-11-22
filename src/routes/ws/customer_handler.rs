use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::Value;
use tokio::time::{timeout, Duration};

use crate::utils::db::{find_session_by_browser_token, get_session};

const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_MESSAGE_BYTES: usize = 2048;

#[derive(Deserialize)]
struct CustomerMessage {
    #[serde(rename = "type")]
    msg_type: String,
    value: Option<Value>,
}

pub async fn customer_ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_customer_socket)
}

async fn receive_text(socket: &mut WebSocket) -> Result<String, ()> {
    match timeout(AUTH_TIMEOUT, socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) => {
            let text = text.to_string();
            if text.len() > MAX_MESSAGE_BYTES {
                return Err(());
            }
            Ok(text)
        }
        _ => Err(()),
    }
}

async fn close_socket(socket: &mut WebSocket) {
    let _ = socket.send(Message::Close(None)).await;
}

async fn handle_customer_socket(mut socket: WebSocket) {
    // The browser token is a bearer credential issued by /api/init.
    // MAC addresses are deliberately not used as customer identity.
    let message_text = match receive_text(&mut socket).await {
        Ok(text) => text,
        Err(()) => {
            close_socket(&mut socket).await;
            return;
        }
    };

    let message: CustomerMessage = match serde_json::from_str(&message_text) {
        Ok(message) => message,
        Err(_) => {
            close_socket(&mut socket).await;
            return;
        }
    };

    if message.msg_type != "ACK" {
        close_socket(&mut socket).await;
        return;
    }

    let browser_token = match message.value.as_ref().and_then(Value::as_str) {
        Some(token)
            if token.len() == 64
                && token.bytes().all(|byte| byte.is_ascii_hexdigit()) =>
        {
            token
        }
        _ => {
            close_socket(&mut socket).await;
            return;
        }
    };

    let session_id = match find_session_by_browser_token(browser_token) {
        Ok(Some(session_id)) => session_id,
        Ok(None) | Err(_) => {
            close_socket(&mut socket).await;
            return;
        }
    };

    // Confirm the token still points to a readable session. Expired sessions
    // may remain connected so the customer can start a new purchase flow.
    match get_session(&session_id) {
        Ok(Some(_)) => {}
        _ => {
            close_socket(&mut socket).await;
            return;
        }
    }

    let authenticated = serde_json::json!({
        "type": "CUSTOMER_AUTH_OK",
        "status": "ok",
    });

    if socket
        .send(Message::Text(authenticated.to_string().into()))
        .await
        .is_err()
    {
        return;
    }

    loop {
        match socket.recv().await {
            Some(Ok(Message::Ping(payload))) => {
                if socket.send(Message::Pong(payload)).await.is_err() {
                    break;
                }
            }
            Some(Ok(Message::Pong(_))) => {}
            Some(Ok(Message::Text(text))) => {
                let parsed = serde_json::from_str::<CustomerMessage>(text.as_str());
                match parsed {
                    Ok(message) if message.msg_type == "ping" => {
                        let pong = serde_json::json!({ "type": "pong" });
                        if socket
                            .send(Message::Text(pong.to_string().into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(message) if message.msg_type == "DONE" => {
                        let _ = socket.send(Message::Close(None)).await;
                        break;
                    }
                    _ => {
                        close_socket(&mut socket).await;
                        break;
                    }
                }
            }
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
            Some(Ok(_)) => {
                close_socket(&mut socket).await;
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn browser_token_validation_requires_64_hex_characters() {
        let valid = "a".repeat(64);
        assert_eq!(valid.len(), 64);
        assert!(valid.bytes().all(|byte| byte.is_ascii_hexdigit()));

        let short = "a".repeat(63);
        assert!(!(short.len() == 64
            && short.bytes().all(|byte| byte.is_ascii_hexdigit())));

        let invalid = format!("{}z", "a".repeat(63));
        assert!(!(invalid.len() == 64
            && invalid.bytes().all(|byte| byte.is_ascii_hexdigit())));
    }
}
