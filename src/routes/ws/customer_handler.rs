use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::time::{timeout, Duration};

use crate::utils::db::{
    find_session_by_browser_token, get_session, list_available_vendos,
    release_vendo_reservation_for_session, reserve_vendo_for_customer_session,
};

const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_MESSAGE_BYTES: usize = 2048;

#[derive(Deserialize)]
struct CustomerMessage {
    #[serde(rename = "type")]
    msg_type: String,
    value: Option<Value>,
    vendo_id: Option<String>,
}

fn valid_browser_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
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

async fn send_json(socket: &mut WebSocket, value: Value) -> Result<(), ()> {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .map_err(|_| ())
}

async fn close_socket(socket: &mut WebSocket) {
    let _ = socket.send(Message::Close(None)).await;
}

async fn send_vendo_options(socket: &mut WebSocket, session_id: &str) -> Result<(), ()> {
    let available = list_available_vendos().map_err(|error| {
        eprintln!("Failed to list available vendos: {}", error);
    }).map_err(|_| ())?;

    match available.len() {
        0 => send_json(socket, json!({
            "type": "VENDO_OPTIONS",
            "available": [],
        })).await,
        1 => {
            let vendo = &available[0];
            match reserve_vendo_for_customer_session(&vendo.vendo_id, session_id) {
                Ok(reservation) => send_json(socket, json!({
                    "type": "VENDO_ASSIGNED",
                    "vendo_id": reservation.vendo_id,
                    "vendo_name": vendo.name,
                })).await,
                Err(error) => send_json(socket, json!({
                    "type": "VENDO_ASSIGNMENT_FAILED",
                    "reason": error,
                })).await,
            }
        }
        _ => {
            let choices: Vec<Value> = available.iter().map(|vendo| json!({
                "vendo_id": vendo.vendo_id,
                "name": vendo.name,
            })).collect();

            send_json(socket, json!({
                "type": "VENDO_OPTIONS",
                "prompt": "Want to choose another vendo?",
                "available": choices,
            })).await
        }
    }
}

pub async fn customer_ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_customer_socket)
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
        Some(token) if valid_browser_token(token) => token,
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

    match get_session(&session_id) {
        Ok(Some(_)) => {}
        _ => {
            close_socket(&mut socket).await;
            return;
        }
    }

    if send_json(&mut socket, json!({
        "type": "CUSTOMER_AUTH_OK",
        "status": "ok",
    })).await.is_err() {
        return;
    }

    // One eligible vendo is reserved automatically. Multiple eligible vendos
    // are offered for explicit selection. An empty list produces no customer
    // facing "unavailable" message.
    if send_vendo_options(&mut socket, &session_id).await.is_err() {
        return;
    }

    let (customer_tx, mut customer_rx) = tokio::sync::mpsc::unbounded_channel();
    let connection_id = crate::routes::ws::state::register_customer_connection(
        &session_id,
        customer_tx,
    );

    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
            Some(Ok(Message::Ping(payload))) => {
                if socket.send(Message::Pong(payload)).await.is_err() {
                    break;
                }
            }
            Some(Ok(Message::Pong(_))) => {}
            Some(Ok(Message::Text(text))) => {
                if text.len() > MAX_MESSAGE_BYTES {
                    close_socket(&mut socket).await;
                    break;
                }

                match serde_json::from_str::<CustomerMessage>(text.as_str()) {
                    Ok(message) if message.msg_type == "ping" => {
                        if send_json(&mut socket, json!({ "type": "pong" })).await.is_err() {
                            break;
                        }
                    }
                    Ok(message) if message.msg_type == "SELECT_VENDO" => {
                        let vendo_id = match message.vendo_id {
                            Some(id) if !id.trim().is_empty()
                                && id.len() <= 128
                                && !id.chars().any(char::is_control) => id,
                            _ => {
                                if send_json(&mut socket, json!({
                                    "type": "VENDO_ASSIGNMENT_FAILED",
                                    "reason": "Invalid vendo selection",
                                })).await.is_err() {
                                    break;
                                }
                                continue;
                            }
                        };

                        match reserve_vendo_for_customer_session(&vendo_id, &session_id) {
                            Ok(reservation) => {
                                if send_json(&mut socket, json!({
                                    "type": "VENDO_ASSIGNED",
                                    "vendo_id": reservation.vendo_id,
                                })).await.is_err() {
                                    break;
                                }
                            }
                            Err(error) => {
                                if send_json(&mut socket, json!({
                                    "type": "VENDO_ASSIGNMENT_FAILED",
                                    "reason": error,
                                })).await.is_err() {
                                    break;
                                }
                                // Refresh the list if a choice lost a race.
                                if send_vendo_options(&mut socket, &session_id).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Ok(message) if message.msg_type == "RELEASE_VENDO" => {
                        match release_vendo_reservation_for_session(&session_id) {
                            Ok(_) => {
                                if send_json(&mut socket, json!({
                                    "type": "VENDO_RELEASED",
                                })).await.is_err() {
                                    break;
                                }
                            }
                            Err(error) => {
                                if send_json(&mut socket, json!({
                                    "type": "VENDO_ASSIGNMENT_FAILED",
                                    "reason": error,
                                })).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Ok(message) if message.msg_type == "DONE" => {
                        let _ = release_vendo_reservation_for_session(&session_id);
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
            },
            notification = customer_rx.recv() => {
                match notification {
                    Some(value) => {
                        if send_json(&mut socket, value).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }

    crate::routes::ws::state::unregister_customer_connection(
        &session_id,
        connection_id,
    );
}

#[cfg(test)]
mod tests {
    use super::valid_browser_token;

    #[test]
    fn browser_token_validation_requires_64_hex_characters() {
        assert!(valid_browser_token(&"a".repeat(64)));
        assert!(!valid_browser_token(&"a".repeat(63)));
        assert!(!valid_browser_token(&format!("{}z", "a".repeat(63))));
    }
}
