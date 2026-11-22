use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::time::{timeout, Duration};

use crate::utils::db::{
    find_session_by_browser_token, get_session, get_vendo,
    get_vendo_reservation_for_session, list_available_vendos,
    release_vendo_reservation_for_session, reserve_vendo_for_customer_session, Vendo,
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

enum VendoDiscoveryPlan {
    NoneAvailable,
    AutoAssign(Vendo),
    Choose(Vec<Vendo>),
}

fn plan_vendo_discovery(mut available: Vec<Vendo>) -> VendoDiscoveryPlan {
    match available.len() {
        0 => VendoDiscoveryPlan::NoneAvailable,
        1 => VendoDiscoveryPlan::AutoAssign(available.remove(0)),
        _ => VendoDiscoveryPlan::Choose(available),
    }
}

fn existing_vendo_assignment(session_id: &str) -> Result<Option<Value>, String> {
    let reservation = match get_vendo_reservation_for_session(session_id)? {
        Some(reservation) => reservation,
        None => return Ok(None),
    };

    // A reservation belongs to this authenticated session. Preserve it across
    // browser reconnects even if the vendo was subsequently disabled or went
    // offline; an existing reservation is allowed to finish.
    let vendo = get_vendo(&reservation.vendo_id)?
        .ok_or_else(|| "Reserved vendo record no longer exists".to_string())?;

    Ok(Some(json!({
        "type": "VENDO_ASSIGNED",
        "vendo_id": reservation.vendo_id,
        "vendo_name": vendo.name,
    })))
}

async fn restore_existing_vendo_reservation(
    socket: &mut WebSocket,
    session_id: &str,
) -> Result<bool, ()> {
    let assignment = existing_vendo_assignment(session_id).map_err(|error| {
        eprintln!("Failed to restore customer vendo reservation: {}", error);
    }).map_err(|_| ())?;

    match assignment {
        Some(message) => {
            send_json(socket, message).await?;
            Ok(true)
        }
        None => Ok(false),
    }
}

async fn send_vendo_options(socket: &mut WebSocket, session_id: &str) -> Result<(), ()> {
    let available = list_available_vendos().map_err(|error| {
        eprintln!("Failed to list available vendos: {}", error);
    }).map_err(|_| ())?;

    match plan_vendo_discovery(available) {
        VendoDiscoveryPlan::NoneAvailable => send_json(socket, json!({
            "type": "VENDO_OPTIONS",
            "available": [],
        })).await,
        VendoDiscoveryPlan::AutoAssign(vendo) => {
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
        VendoDiscoveryPlan::Choose(available) => {
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

    // Restore this session's existing reservation before discovering new
    // vendos. This prevents a browser reconnect from getting locked out by
    // its own still-active reservation.
    match restore_existing_vendo_reservation(&mut socket, &session_id).await {
        Ok(true) => {}
        Ok(false) => {
            // One eligible vendo is reserved automatically. Multiple eligible
            // vendos are offered for explicit selection. An empty list stays silent.
            if send_vendo_options(&mut socket, &session_id).await.is_err() {
                return;
            }
        }
        Err(()) => return,
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
    use super::{plan_vendo_discovery, valid_browser_token, VendoDiscoveryPlan};
    use crate::utils::db::Vendo;

    fn test_vendo(id: &str) -> Vendo {
        Vendo {
            vendo_id: id.to_string(),
            name: format!("Vendo {id}"),
            enabled: true,
            created_at: 1,
        }
    }

    #[test]
    fn customer_reconnect_restores_its_existing_vendo_reservation() {
        use crate::utils::db::{
            create_session, init_test_db_once, register_vendo,
            reserve_vendo_for_customer_session,
        };

        init_test_db_once();

        let (vendo, _credential) = register_vendo("Reconnect Test Vendo").unwrap();
        let _generation =
            crate::routes::ws::state::register_vendo_connection(&vendo.vendo_id);

        let session_id = format!("reconnect-session-{}", vendo.vendo_id);
        let session = crate::utils::db::Session::new(
            "Reconnect Test Customer".to_string(),
            "10.0.0.250".to_string(),
            crate::utils::db::current_timestamp(),
        );
        create_session(
            &session_id,
            &session,
            &format!("mac-{}", vendo.vendo_id),
            &format!("token-{}", vendo.vendo_id),
        )
        .unwrap();

        let reservation =
            reserve_vendo_for_customer_session(&vendo.vendo_id, &session_id).unwrap();

        let assignment = existing_vendo_assignment(&session_id)
            .unwrap()
            .expect("Reconnect should restore the existing reservation");

        assert_eq!(assignment["type"], "VENDO_ASSIGNED");
        assert_eq!(assignment["vendo_id"], reservation.vendo_id);
        assert_eq!(assignment["vendo_name"], vendo.name);

        assert!(
            crate::routes::ws::state::unregister_vendo_connection(
                &vendo.vendo_id,
                _generation,
            )
        );
    }

    #[test]
    fn browser_token_validation_requires_64_hex_characters() {
        assert!(valid_browser_token(&"a".repeat(64)));
        assert!(!valid_browser_token(&"a".repeat(63)));
        assert!(!valid_browser_token(&format!("{}z", "a".repeat(63))));
    }

    #[test]
    fn customer_vendo_discovery_silently_handles_no_available_vendos() {
        assert!(matches!(
            plan_vendo_discovery(Vec::new()),
            VendoDiscoveryPlan::NoneAvailable
        ));
    }

    #[test]
    fn customer_vendo_discovery_auto_assigns_the_only_available_vendo() {
        match plan_vendo_discovery(vec![test_vendo("vendo-one")]) {
            VendoDiscoveryPlan::AutoAssign(vendo) => {
                assert_eq!(vendo.vendo_id, "vendo-one");
                assert_eq!(vendo.name, "Vendo vendo-one");
            }
            _ => panic!("One available vendo must be auto-assigned"),
        }
    }

    #[test]
    fn customer_vendo_discovery_offers_a_choice_when_multiple_are_available() {
        match plan_vendo_discovery(vec![
            test_vendo("vendo-one"),
            test_vendo("vendo-two"),
        ]) {
            VendoDiscoveryPlan::Choose(vendos) => {
                assert_eq!(vendos.len(), 2);
                assert_eq!(vendos[0].vendo_id, "vendo-one");
                assert_eq!(vendos[1].vendo_id, "vendo-two");
            }
            _ => panic!("Multiple available vendos must be offered for selection"),
        }
    }
}
