use std::sync::mpsc;
use std::time::Duration;
use tiny_http::ReadWrite;
use tungstenite::protocol::{Message, WebSocket};

use super::state::{
    RelayRequest, RelayResult, USER_EVENTS, WsMessage,
    get_active_user, get_coinslot_senders,
    refresh_active_user, release_active_user, reserve_active_user,
};

pub fn run_user(
    mut ws: WebSocket<Box<dyn ReadWrite + Send>>,
    subnet: String,
    ip: String,
    owner_id: String,
) {
    crate::debug_println!("[USER:{}] Browser client connected.", ip);

    loop {
        let msg = match ws.read() {
            Ok(m) => m,
            Err(_) => {
                crate::debug_println!("[USER:{}] Browser disconnected.", ip);
                break;
            }
        };

        if msg.is_close() {
            crate::debug_println!("[USER:{}] Browser closed the connection.", ip);
            break;
        }

        if !msg.is_text() {
            continue;
        }

        let txt = msg.to_text().unwrap_or("");
        crate::debug_println!("[USER:{}] Received: {}", ip, txt);

        if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(txt) {
            if ws_msg.msg_type == "ping" {
                if refresh_active_user(&subnet, &owner_id) {
                    if let Some(active) = get_active_user(&subnet) {
                        if active.owner_id == owner_id {
                            let mut events_map = USER_EVENTS.lock().unwrap();

                            if let Some(events) = events_map.get_mut(&owner_id) {
                                for ev in events.drain(..) {
                                    if let Ok(json) = serde_json::to_string(&ev) {
                                        let _ = ws.send(Message::Text(json.into()));
                                    }
                                }
                            }
                        }
                    }
                }
            } else if ws_msg.msg_type == "ACK" {
                let browser_token = ws_msg.value.as_ref()
                    .and_then(|value| value.as_str())
                    .filter(|token| {
                        token.len() == 64
                            && token.bytes().all(|b| b.is_ascii_hexdigit())
                    })
                    .map(str::to_string);

                handle_user_ack(
                    &mut ws,
                    &ip,
                    &subnet,
                    &owner_id,
                    browser_token,
                );
            } else if ws_msg.msg_type == "DONE" {
                if release_active_user(&subnet, &owner_id) {
                    crate::debug_println!(
                        "[USER:{}] Owner completed coin-slot session.",
                        ip
                    );

                    let sender = get_coinslot_senders()
                        .lock()
                        .unwrap()
                        .get(&subnet)
                        .cloned();

                    if let Some(coinslot_tx) = sender {
                        let _ = coinslot_tx.send(RelayRequest {
                            reply_tx: None,
                            command: "close".to_string(),
                        });
                    }
                }
                break;
            } else {
                crate::debug_println!(
                    "[USER:{}] Unhandled message type: {}",
                    ip,
                    ws_msg.msg_type
                );
            }
        } else {
            crate::debug_println!(
                "[USER:{}] Could not parse message: {}",
                ip,
                txt
            );
        }
    }

    // Unexpected browser disconnection does not release the reservation.
    // The 30-second lease expires in the coinslot thread instead.
    crate::debug_println!(
        "[USER:{}] User thread exiting; reservation will expire or be explicitly released.",
        ip
    );
}

pub fn handle_user_ack(
    ws: &mut WebSocket<Box<dyn ReadWrite + Send>>,
    ip: &str,
    subnet: &str,
    owner_id: &str,
    browser_token: Option<String>,
) -> bool {
    let mac = crate::utils::get_mac_from_ip::get_mac_from_ip(ip)
        .unwrap_or_default();

    if !reserve_active_user(subnet, &mac, owner_id, browser_token) {
        crate::debug_println!("[USER:{}] Coinslot is already reserved.", ip);

        let _ = ws.send(Message::Text(
            "{\"error\":\"Coinslot is currently in use by another user\"}"
                .to_string()
                .into(),
        ));
        return false;
    }

    let sender = get_coinslot_senders()
        .lock()
        .unwrap()
        .get(subnet)
        .cloned();

    let coinslot_tx = match sender {
        Some(tx) => tx,
        None => {
            release_active_user(subnet, owner_id);

            let _ = ws.send(Message::Text(
                "{\"error\":\"Coinslot is not available\"}"
                    .to_string()
                    .into(),
            ));
            return false;
        }
    };

    let (reply_tx, reply_rx) = mpsc::sync_channel::<RelayResult>(1);

    if coinslot_tx
        .send(RelayRequest {
            reply_tx: Some(reply_tx),
            command: "ACK".to_string(),
        })
        .is_err()
    {
        release_active_user(subnet, owner_id);

        let _ = ws.send(Message::Text(
            "{\"error\":\"Coinslot disconnected\"}"
                .to_string()
                .into(),
        ));
        return false;
    }

    match reply_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(RelayResult::Available) => {
            crate::debug_println!(
                "[USER:{}] Coinslot confirmed available.",
                ip
            );

            let _ = ws.send(Message::Text(
                "{\"type\":\"ACK_SUCCESS\"}".to_string().into(),
            ));
            true
        }
        Ok(RelayResult::NotAvailable(reason)) => {
            release_active_user(subnet, owner_id);

            let error = serde_json::json!({ "error": reason });
            let _ = ws.send(Message::Text(error.to_string().into()));
            false
        }
        Ok(RelayResult::Timeout) | Err(_) => {
            release_active_user(subnet, owner_id);

            let _ = ws.send(Message::Text(
                "{\"error\":\"Coinslot did not respond in time\"}"
                    .to_string()
                    .into(),
            ));
            false
        }
    }
}
