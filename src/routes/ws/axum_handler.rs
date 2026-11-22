use axum::{
    extract::{
        ConnectInfo,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;
use tokio::time::{Duration, timeout};

use crate::{
    routes::ws::state::{
        is_current_vendo_connection, notify_customer_session, register_vendo_connection,
        unregister_vendo_connection,
    },
    utils::{
        crypto::{
            decrypt_payload, derive_vendo_session_key, encrypt_payload,
            generate_vendo_challenge, verify_vendo_auth_proof,
        },
        db::{
            VendoPaymentEvent, VendoPaymentResult, convert_amount_to_time,
            credit_vendo_payment_for_session, get_vendo_auth_verifier,
            get_vendo_reservation_for_vendo,
        },
    },
};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HANDSHAKE_MESSAGE: usize = 2048;
const MAX_APPLICATION_MESSAGE: usize = 4096;

#[derive(Deserialize)]
struct EncryptedEnvelope {
    #[serde(rename = "type")]
    msg_type: String,
    payload: String,
}

#[derive(Deserialize)]
struct PaymentMessage {
    #[serde(rename = "type")]
    msg_type: String,
    event_id: String,
    sequence: u64,
    amount: u32,
    pulse_count: u32,
    device_timestamp: u64,
}

async fn send_encrypted_json(
    socket: &mut WebSocket,
    session_key: &[u8; 32],
    value: Value,
) -> Result<(), ()> {
    let plaintext = value.to_string();
    let encrypted = encrypt_payload(&plaintext, session_key).map_err(|_| ())?;
    let envelope = json!({
        "type": "ENCRYPTED",
        "payload": STANDARD.encode(encrypted),
    });

    socket
        .send(Message::Text(envelope.to_string().into()))
        .await
        .map_err(|_| ())
}

async fn send_payment_rejection(
    socket: &mut WebSocket,
    session_key: &[u8; 32],
    reason: &str,
) -> Result<(), ()> {
    send_encrypted_json(
        socket,
        session_key,
        json!({
            "type": "PAYMENT_REJECTED",
            "reason": reason,
        }),
    )
    .await
}

#[derive(Deserialize)]
struct HelloMessage {
    #[serde(rename = "type")]
    msg_type: String,
    vendo_id: String,
}

#[derive(Deserialize)]
struct AuthMessage {
    #[serde(rename = "type")]
    msg_type: String,
    proof: String,
}

pub async fn axum_ws_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
) -> impl IntoResponse {
    let ip = remote.ip().to_string();

    ws.on_upgrade(move |socket| handle_socket(socket, ip))
}

async fn receive_text(socket: &mut WebSocket) -> Result<String, ()> {
    match timeout(HANDSHAKE_TIMEOUT, socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) => {
            let text = text.to_string();

            if text.len() > MAX_HANDSHAKE_MESSAGE {
                return Err(());
            }

            Ok(text)
        }
        _ => Err(()),
    }
}

async fn reject_connection(socket: &mut WebSocket) {
    let _ = socket.send(Message::Close(None)).await;
}

async fn handle_socket(mut socket: WebSocket, ip: String) {
    // Step 1: Receive the device HELLO message.
    let hello_text = match receive_text(&mut socket).await {
        Ok(text) => text,
        Err(()) => {
            reject_connection(&mut socket).await;
            return;
        }
    };

    let hello: HelloMessage = match serde_json::from_str(&hello_text) {
        Ok(message) => message,
        Err(_) => {
            reject_connection(&mut socket).await;
            return;
        }
    };

    if hello.msg_type != "HELLO"
        || hello.vendo_id.is_empty()
        || hello.vendo_id.len() > 128
        || hello.vendo_id.chars().any(char::is_control)
    {
        reject_connection(&mut socket).await;
        return;
    }

    let vendo_id = hello.vendo_id;

    // Step 2: Retrieve the stored credential verifier.
    // Disabled vendos may authenticate; availability is handled separately.
    let verifier = match get_vendo_auth_verifier(&vendo_id) {
        Ok(Some(verifier)) => verifier,
        Ok(None) => {
            reject_connection(&mut socket).await;
            return;
        }
        Err(error) => {
            eprintln!("Vendo credential lookup failed: {}", error);
            reject_connection(&mut socket).await;
            return;
        }
    };

    // Step 3: Generate and send a fresh challenge.
    let challenge = match generate_vendo_challenge() {
        Ok(challenge) => challenge,
        Err(error) => {
            eprintln!("Vendo challenge generation failed: {}", error);
            reject_connection(&mut socket).await;
            return;
        }
    };

    let challenge_message = serde_json::json!({
        "type": "CHALLENGE",
        "challenge": STANDARD.encode(challenge),
    });

    if socket
        .send(Message::Text(challenge_message.to_string().into()))
        .await
        .is_err()
    {
        return;
    }

    // Step 4: Receive and verify the device's HMAC proof.
    let auth_text = match receive_text(&mut socket).await {
        Ok(text) => text,
        Err(()) => {
            reject_connection(&mut socket).await;
            return;
        }
    };

    let auth: AuthMessage = match serde_json::from_str(&auth_text) {
        Ok(message) => message,
        Err(_) => {
            reject_connection(&mut socket).await;
            return;
        }
    };

    if auth.msg_type != "AUTH" {
        reject_connection(&mut socket).await;
        return;
    }

    let proof = match STANDARD.decode(&auth.proof) {
        Ok(proof) if proof.len() == 32 => proof,
        _ => {
            reject_connection(&mut socket).await;
            return;
        }
    };

    if !verify_vendo_auth_proof(&verifier, &vendo_id, &challenge, &proof) {
        reject_connection(&mut socket).await;
        return;
    }

    // Step 5: Derive the unique AES-256-GCM session key.
    // It is retained for the authenticated connection's future
    // encrypted-message implementation.
    let session_key = match derive_vendo_session_key(&verifier, &vendo_id, &challenge) {
        Ok(key) => key,
        Err(error) => {
            eprintln!("Vendo session key derivation failed: {}", error);
            reject_connection(&mut socket).await;
            return;
        }
    };

    // Step 6: Register only after successful authentication.
    let generation = register_vendo_connection(&vendo_id);

    let success_message = serde_json::json!({
        "type": "AUTH_OK",
        "protocol": "vendo-auth-v1",
    });

    if socket
        .send(Message::Text(success_message.to_string().into()))
        .await
        .is_err()
    {
        unregister_vendo_connection(&vendo_id, generation);
        return;
    }

    crate::debug_println!("[WS] Authenticated vendo {} from {}", vendo_id, ip);

    // Only the current authenticated connection generation may process payments.
    loop {
        match socket.recv().await {
            Some(Ok(Message::Ping(payload))) => {
                if socket.send(Message::Pong(payload)).await.is_err() {
                    break;
                }
            }
            Some(Ok(Message::Pong(_))) => {}
            Some(Ok(Message::Text(text))) => {
                if !is_current_vendo_connection(&vendo_id, generation) {
                    break;
                }

                let text = text.to_string();
                if text.len() > MAX_APPLICATION_MESSAGE {
                    reject_connection(&mut socket).await;
                    break;
                }

                let envelope: EncryptedEnvelope = match serde_json::from_str::<EncryptedEnvelope>(&text) {
                    Ok(message) if message.msg_type == "ENCRYPTED" => message,
                    _ => {
                        reject_connection(&mut socket).await;
                        break;
                    }
                };

                let encrypted = match STANDARD.decode(&envelope.payload) {
                    Ok(bytes) => bytes,
                    Err(_) => {
                        reject_connection(&mut socket).await;
                        break;
                    }
                };

                let plaintext = match decrypt_payload(&encrypted, &session_key) {
                    Ok(plaintext) if plaintext.len() <= MAX_APPLICATION_MESSAGE => plaintext,
                    _ => {
                        reject_connection(&mut socket).await;
                        break;
                    }
                };

                let payment: PaymentMessage = match serde_json::from_str::<PaymentMessage>(&plaintext) {
                    Ok(message) if message.msg_type == "PAYMENT" => message,
                    _ => {
                        reject_connection(&mut socket).await;
                        break;
                    }
                };

                let reservation = match get_vendo_reservation_for_vendo(&vendo_id) {
                    Ok(Some(reservation)) => reservation,
                    Ok(None) => {
                        let _ = send_payment_rejection(
                            &mut socket,
                            &session_key,
                            "No customer session has reserved this vendo",
                        )
                        .await;
                        continue;
                    }
                    Err(error) => {
                        eprintln!("Vendo reservation lookup failed: {}", error);
                        let _ = send_payment_rejection(
                            &mut socket,
                            &session_key,
                            "Unable to verify the active reservation",
                        )
                        .await;
                        continue;
                    }
                };

                let seconds = convert_amount_to_time(payment.amount);
                if seconds == 0 {
                    let _ = send_payment_rejection(
                        &mut socket,
                        &session_key,
                        "Payment amount does not match an active rate",
                    )
                    .await;
                    continue;
                }

                let event = VendoPaymentEvent {
                    vendo_id: vendo_id.clone(),
                    event_id: payment.event_id.clone(),
                    sequence: payment.sequence,
                    amount: payment.amount,
                    pulse_count: payment.pulse_count,
                    device_timestamp: payment.device_timestamp,
                };

                match credit_vendo_payment_for_session(
                    &reservation.session_id,
                    seconds as u64,
                    event,
                ) {
                    Ok(VendoPaymentResult::Credited {
                        session_id: _credited_session_id,
                        session: _credited_session,
                        browser_token,
                    }) => {
                        let mut customer_event = json!({
                            "type": "coin",
                            "value": payment.amount,
                            "time": seconds,
                        });

                        if let Some(token) = browser_token {
                            customer_event["browser_token"] = json!(token);
                        }

                        // The browser socket is registered under the reserved
                        // session ID even when an expired session was replaced.
                        notify_customer_session(
                            &reservation.session_id,
                            customer_event,
                        );

                        if send_encrypted_json(
                            &mut socket,
                            &session_key,
                            json!({
                                "type": "PAYMENT_ACCEPTED",
                                "event_id": payment.event_id,
                                "sequence": payment.sequence,
                                "credited_seconds": seconds,
                            }),
                        )
                        .await
                        .is_err()
                        {
                            break;
                        }
                    }
                    Ok(VendoPaymentResult::Duplicate { original_event_id }) => {
                        if send_encrypted_json(
                            &mut socket,
                            &session_key,
                            json!({
                                "type": "PAYMENT_DUPLICATE",
                                "event_id": original_event_id,
                            }),
                        )
                        .await
                        .is_err()
                        {
                            break;
                        }
                    }
                    Err(error) => {
                        if send_payment_rejection(
                            &mut socket,
                            &session_key,
                            &error,
                        )
                        .await
                        .is_err()
                        {
                            break;
                        }
                    }
                }
            }
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                break;
            }
            Some(Ok(_)) => {
                reject_connection(&mut socket).await;
                break;
            }
        }
    }

    // A stale connection must never unregister a newer connection.
    unregister_vendo_connection(&vendo_id, generation);

    crate::debug_println!("[WS] Vendo {} disconnected", vendo_id);
}

// Real WebSocket authentication smoke test.
#[cfg(test)]
mod auth_smoke_tests {
    use super::*;
    use axum::{Router, routing::get};
    use base64::engine::general_purpose::STANDARD;
    use std::net::SocketAddr;
    use tokio::net::TcpListener;
    use tungstenite::{Message, connect};



    #[tokio::test]
    async fn stress_test_ten_vendos_authenticate_and_process_payments_over_websockets() {
        use crate::routes::ws::state::is_vendo_online;
        use crate::utils::crypto::{decrypt_payload, derive_vendo_session_key, encrypt_payload, vendo_auth_proof};
        use crate::utils::db::{
            create_session, get_db, get_session, init_test_db_once, register_vendo,
            reserve_vendo_for_customer_session, Session, RATES_TABLE,
        };
        use std::sync::{Arc, Barrier};
        use tungstenite::{connect, Message};

        const VENDO_COUNT: usize = 10;
        init_test_db_once();

        let test_id = crate::utils::db::generate_secure_id().unwrap();

        // Give this isolated test a known payment rate: 10 units = 60 seconds.
        let rate_key = u32::MAX - (crate::utils::db::current_timestamp() as u32 % 100_000);
        let rate_value = serde_json::json!({ "price": 10.0, "time": 60 }).to_string();
        {
            let db = get_db();
            let write_txn = db.begin_write().unwrap();
            {
                let mut rates = write_txn.open_table(RATES_TABLE).unwrap();
                rates.insert(rate_key, rate_value.as_str()).unwrap();
            }
            write_txn.commit().unwrap();
        }

        // Prepare ten unique vendo credentials and customer sessions.
        let mut devices = Vec::with_capacity(VENDO_COUNT);
        for index in 0..VENDO_COUNT {
            let (vendo, credential) =
                register_vendo(&format!("WS Stress Vendo {index}")).unwrap();
            let session_id = format!("ws-stress-session-{test_id}-{index}");
            let mac = format!("02:20:00:{:02x}:{:02x}:{:02x}", index / 256, index % 256, index + 1);
            let token = format!("ws-stress-token-{test_id}-{index}");
            let mut session = Session::new(
                format!("WS Stress Customer {index}"),
                "127.0.0.1".to_string(),
                crate::utils::db::current_timestamp(),
            );
            session.remaining_seconds = 100;
            session.paused = true;

            create_session(&session_id, &session, &mac, &token).unwrap();
            devices.push((vendo, credential, session_id));
        }

        // Run the production Axum vendo WebSocket route on an ephemeral port.
        let app = Router::new().route("/ws/vendo", get(super::axum_ws_handler));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_task = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        let url = format!("ws://{}/ws/vendo", address);

        let worker_result = tokio::task::spawn_blocking(move || {
            // Authenticate all ten independent WebSocket connections first.
            let mut clients = Vec::with_capacity(VENDO_COUNT);
            for (index, (vendo, _credential, session_id)) in devices.iter().enumerate() {
                let verifier = crate::utils::db::get_vendo_auth_verifier(&vendo.vendo_id)
                    .unwrap()
                    .expect("Registered vendo must have a credential verifier");

                let (mut client, _) = connect(url.as_str()).unwrap();
                client.send(Message::Text(serde_json::json!({
                    "type": "HELLO",
                    "vendo_id": vendo.vendo_id,
                }).to_string().into())).unwrap();

                let challenge_text = match client.read().unwrap() {
                    Message::Text(text) => text.to_string(),
                    other => panic!("Vendo {index}: expected CHALLENGE, received {other:?}"),
                };
                let challenge_json: serde_json::Value =
                    serde_json::from_str(&challenge_text).unwrap();
                assert_eq!(challenge_json["type"], "CHALLENGE");
                let challenge_bytes = STANDARD
                    .decode(challenge_json["challenge"].as_str().unwrap())
                    .unwrap();
                let challenge: [u8; 32] = challenge_bytes.try_into().unwrap();

                let proof = vendo_auth_proof(&verifier, &vendo.vendo_id, &challenge).unwrap();
                client.send(Message::Text(serde_json::json!({
                    "type": "AUTH",
                    "proof": STANDARD.encode(proof),
                }).to_string().into())).unwrap();

                let auth_text = match client.read().unwrap() {
                    Message::Text(text) => text.to_string(),
                    other => panic!("Vendo {index}: expected AUTH_OK, received {other:?}"),
                };
                let auth_json: serde_json::Value = serde_json::from_str(&auth_text).unwrap();
                assert_eq!(auth_json["type"], "AUTH_OK");
                assert!(is_vendo_online(&vendo.vendo_id));

                // Reservation is allowed only after the real WebSocket has
                // completed authentication and registered the vendo online.
                reserve_vendo_for_customer_session(&vendo.vendo_id, session_id)
                    .expect("Authenticated vendo should reserve its customer session");

                let session_key =
                    derive_vendo_session_key(&verifier, &vendo.vendo_id, &challenge).unwrap();
                clients.push((client, session_id.clone(), session_key, index));
            }

            // Release all ten real WebSocket clients to send a payment together.
            let barrier = Arc::new(Barrier::new(VENDO_COUNT + 1));
            let mut senders = Vec::with_capacity(VENDO_COUNT);
            for (mut client, session_id, session_key, index) in clients {
                let barrier_for_worker = Arc::clone(&barrier);
                let test_id_for_worker = test_id.clone();
                senders.push(std::thread::spawn(move || {
                    barrier_for_worker.wait();

                    let event_id = format!("ws-payment-{test_id_for_worker}-{index}");
                    let plaintext = serde_json::json!({
                        "type": "PAYMENT",
                        "event_id": event_id,
                        "sequence": 1,
                        "amount": 10,
                        "pulse_count": 1,
                        "device_timestamp": 1,
                    }).to_string();
                    let encrypted = encrypt_payload(&plaintext, &session_key).unwrap();
                    let envelope = serde_json::json!({
                        "type": "ENCRYPTED",
                        "payload": STANDARD.encode(encrypted),
                    });
                    client.send(Message::Text(envelope.to_string().into())).unwrap();

                    let response_text = match client.read().unwrap() {
                        Message::Text(text) => text.to_string(),
                        other => panic!("Vendo {index}: expected encrypted payment response, received {other:?}"),
                    };
                    let response: serde_json::Value =
                        serde_json::from_str(&response_text).unwrap();
                    assert_eq!(response["type"], "ENCRYPTED");

                    let response_bytes = STANDARD
                        .decode(response["payload"].as_str().unwrap())
                        .unwrap();
                    let response_plaintext =
                        decrypt_payload(&response_bytes, &session_key).unwrap();
                    let payment_result: serde_json::Value =
                        serde_json::from_str(&response_plaintext).unwrap();

                    assert_eq!(payment_result["type"], "PAYMENT_ACCEPTED");
                    assert_eq!(payment_result["event_id"], event_id);
                    assert_eq!(payment_result["credited_seconds"], 60);

                    let session = get_session(&session_id)
                        .unwrap()
                        .expect("Customer session should remain available");
                    assert_eq!(session.remaining_seconds, 160);

                    let _ = client.close(None);
                }));
            }

            barrier.wait();
            for sender in senders {
                sender.join().expect("Vendo payment worker must not panic");
            }
        })
        .await;

        server_task.abort();
        worker_result.expect("Concurrent WebSocket payment test task should complete");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn stress_test_100_customers_with_ten_live_vendo_payments() {
        use crate::routes::ws::customer_handler::customer_ws_handler;
        use crate::utils::crypto::{
            decrypt_payload, derive_vendo_session_key, encrypt_payload, vendo_auth_proof,
        };
        use crate::utils::db::{
            create_session, generate_browser_token, generate_secure_id, get_db, get_session,
            init_test_db_once, register_vendo, reserve_vendo_for_customer_session, Session,
            RATES_TABLE,
        };
        use std::thread;

        const CUSTOMER_COUNT: usize = 100;
        const VENDO_COUNT: usize = 10;
        init_test_db_once();

        let test_id = generate_secure_id().unwrap();
        let rate_key = u32::from_str_radix(&test_id[..8], 16).unwrap();
        let rate_value = serde_json::json!({ "price": 10.0, "time": 60 }).to_string();
        {
            let db = get_db();
            let write_txn = db.begin_write().unwrap();
            {
                let mut rates = write_txn.open_table(RATES_TABLE).unwrap();
                rates.insert(rate_key, rate_value.as_str()).unwrap();
            }
            write_txn.commit().unwrap();
        }

        let mut customers = Vec::with_capacity(CUSTOMER_COUNT);
        for index in 0..CUSTOMER_COUNT {
            let session_id = format!("combined-load-session-{test_id}-{index}");
            let token = generate_browser_token().unwrap();
            let mac = format!("02:30:00:{:02x}:{:02x}:{:02x}", index / 256, index % 256, index + 1);
            let mut session = Session::new(
                format!("Combined Load Customer {index}"),
                "127.0.0.1".to_string(),
                crate::utils::db::current_timestamp(),
            );
            session.remaining_seconds = 100;
            session.paused = true;
            create_session(&session_id, &session, &mac, &token).unwrap();
            customers.push((session_id, token));
        }

        let mut vendos = Vec::with_capacity(VENDO_COUNT);
        for index in 0..VENDO_COUNT {
            let (vendo, _credential) =
                register_vendo(&format!("Combined Load Vendo {test_id}-{index}")).unwrap();
            let verifier = crate::utils::db::get_vendo_auth_verifier(&vendo.vendo_id)
                .unwrap()
                .expect("Registered vendo must have a verifier");
            vendos.push((vendo, verifier));
        }

        let app = Router::new()
            .route("/ws/vendo", get(super::axum_ws_handler))
            .route("/ws/customer", get(customer_ws_handler));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_task = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        let url = format!("ws://{address}");

        let worker_result = tokio::task::spawn_blocking(move || {
            // Authenticate and hold all ten vendo WebSockets open.
            let mut vendo_clients = Vec::with_capacity(VENDO_COUNT);
            for (index, (vendo, verifier)) in vendos.iter().enumerate() {
                let (mut client, _) = connect(format!("{url}/ws/vendo")).unwrap();
                client.send(Message::Text(serde_json::json!({
                    "type": "HELLO",
                    "vendo_id": vendo.vendo_id,
                }).to_string().into())).unwrap();

                let challenge_text = match client.read().unwrap() {
                    Message::Text(text) => text.to_string(),
                    other => panic!("Vendo {index}: expected CHALLENGE, got {other:?}"),
                };
                let challenge_json: serde_json::Value =
                    serde_json::from_str(&challenge_text).unwrap();
                assert_eq!(challenge_json["type"], "CHALLENGE");
                let challenge_bytes = STANDARD
                    .decode(challenge_json["challenge"].as_str().unwrap())
                    .unwrap();
                let challenge: [u8; 32] = challenge_bytes.try_into().unwrap();
                let proof = vendo_auth_proof(verifier, &vendo.vendo_id, &challenge).unwrap();

                client.send(Message::Text(serde_json::json!({
                    "type": "AUTH",
                    "proof": STANDARD.encode(proof),
                }).to_string().into())).unwrap();

                let auth_text = match client.read().unwrap() {
                    Message::Text(text) => text.to_string(),
                    other => panic!("Vendo {index}: expected AUTH_OK, got {other:?}"),
                };
                let auth_json: serde_json::Value =
                    serde_json::from_str(&auth_text).unwrap();
                assert_eq!(auth_json["type"], "AUTH_OK");

                let session_key =
                    derive_vendo_session_key(verifier, &vendo.vendo_id, &challenge).unwrap();
                vendo_clients.push((client, vendo.vendo_id.clone(), session_key, index));
            }

            // Open and authenticate 100 customer WebSockets, keeping every
            // connection alive while the ten vendo payments are processed.
            let mut customer_clients = Vec::with_capacity(CUSTOMER_COUNT);
            let mut customer_session_ids = Vec::with_capacity(CUSTOMER_COUNT);
            for (index, (session_id, token)) in customers.iter().enumerate() {
                let (mut client, _) = connect(format!("{url}/ws/customer")).unwrap();
                client.send(Message::Text(serde_json::json!({
                    "type": "ACK",
                    "value": token,
                }).to_string().into())).unwrap();

                let auth_text = match client.read().unwrap() {
                    Message::Text(text) => text.to_string(),
                    other => panic!("Customer {index}: expected CUSTOMER_AUTH_OK, got {other:?}"),
                };
                let auth_json: serde_json::Value =
                    serde_json::from_str(&auth_text).unwrap();
                assert_eq!(auth_json["type"], "CUSTOMER_AUTH_OK");

                let options_text = match client.read().unwrap() {
                    Message::Text(text) => text.to_string(),
                    other => panic!("Customer {index}: expected VENDO_OPTIONS, got {other:?}"),
                };
                let options_json: serde_json::Value =
                    serde_json::from_str(&options_text).unwrap();
                assert_eq!(options_json["type"], "VENDO_OPTIONS");
                assert!(options_json["available"].as_array().unwrap().len() >= VENDO_COUNT);

                customer_clients.push(client);
                customer_session_ids.push(session_id.clone());
            }

            // Reserve ten distinct customer sessions, one per authenticated vendo.
            for index in 0..VENDO_COUNT {
                reserve_vendo_for_customer_session(
                    &vendo_clients[index].1,
                    &customer_session_ids[index],
                ).expect("Each online vendo should reserve its assigned customer session");
            }

            // Send ten encrypted payment events concurrently over the live sockets.
            let mut payment_workers = Vec::with_capacity(VENDO_COUNT);
            for (mut client, vendo_id, session_key, index) in vendo_clients {
                let session_id = customer_session_ids[index].clone();
                let event_id = format!("combined-load-payment-{test_id}-{index}");
                payment_workers.push(thread::spawn(move || {
                    let plaintext = serde_json::json!({
                        "type": "PAYMENT",
                        "event_id": event_id,
                        "sequence": 1,
                        "amount": 10,
                        "pulse_count": 1,
                        "device_timestamp": 1,
                    }).to_string();
                    let encrypted = encrypt_payload(&plaintext, &session_key).unwrap();
                    let envelope = serde_json::json!({
                        "type": "ENCRYPTED",
                        "payload": STANDARD.encode(encrypted),
                    });
                    client.send(Message::Text(envelope.to_string().into())).unwrap();

                    let response_text = match client.read().unwrap() {
                        Message::Text(text) => text.to_string(),
                        other => panic!("Vendo {index} ({vendo_id}): expected encrypted result, got {other:?}"),
                    };
                    let response: serde_json::Value =
                        serde_json::from_str(&response_text).unwrap();
                    assert_eq!(response["type"], "ENCRYPTED");
                    let response_bytes = STANDARD
                        .decode(response["payload"].as_str().unwrap())
                        .unwrap();
                    let response_plaintext =
                        decrypt_payload(&response_bytes, &session_key).unwrap();
                    let result: serde_json::Value =
                        serde_json::from_str(&response_plaintext).unwrap();
                    assert_eq!(result["type"], "PAYMENT_ACCEPTED");
                    assert_eq!(result["credited_seconds"], 60);
                    let session = get_session(&session_id)
                        .unwrap()
                        .expect("Paid customer session must remain available");
                    assert_eq!(session.remaining_seconds, 160);
                }));
            }
            for worker in payment_workers {
                worker.join().expect("Concurrent payment worker must not panic");
            }

            // Confirm every customer socket is still responsive. The ten paid
            // sessions must also receive their coin notification while all
            // ninety other customer connections remain connected.
            for (index, client) in customer_clients.iter_mut().enumerate() {
                client.send(Message::Text(
                    serde_json::json!({ "type": "ping" }).to_string().into()
                )).unwrap();

                let mut saw_coin_notification = false;
                let mut saw_pong = false;
                for _ in 0..3 {
                    let message = client.read().unwrap();
                    if let Message::Text(text) = message {
                        let value: serde_json::Value =
                            serde_json::from_str(text.as_ref()).unwrap();
                        if value["type"] == "coin" {
                            saw_coin_notification = true;
                        } else if value["type"] == "pong" {
                            saw_pong = true;
                            break;
                        }
                    }
                }
                assert!(saw_pong, "Customer {index} WebSocket stopped responding under combined load");
                if index < VENDO_COUNT {
                    assert!(saw_coin_notification, "Paid customer {index} did not receive its coin notification");
                }
            }

            for mut client in customer_clients {
                let _ = client.close(None);
            }
        }).await;

        server_task.abort();
        worker_result.expect("Combined 100-customer/10-vendo load test task must complete");
    }

    #[tokio::test]
    async fn websocket_authentication_smoke_test() {
        use crate::utils::db::{get_vendo_auth_verifier, init_test_db_once, register_vendo};

        // Reuse the shared database initialized once for this test process.
        init_test_db_once();

        // Register a test vendo and retrieve its verifier.
        let (vendo, _raw_credential) = register_vendo("WebSocket Test Vendo").unwrap();

        let verifier = get_vendo_auth_verifier(&vendo.vendo_id)
            .unwrap()
            .expect("Test vendo must have an authentication verifier");

        // Start the real Axum WebSocket handler on an ephemeral port.
        let app = Router::new().route("/ws", get(super::axum_ws_handler));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();

        let address = listener.local_addr().unwrap();

        let server_task = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });

        let url = format!("ws://{}/ws", address);
        let vendo_id = vendo.vendo_id.clone();

        // Run the synchronous Tungstenite client outside the async runtime.
        tokio::task::spawn_blocking(move || {
            // First connection: valid credential must authenticate.
            let (mut client, _) = connect(url.as_str()).unwrap();

            let hello = serde_json::json!({
                "type": "HELLO",
                "vendo_id": vendo_id,
            });

            client
                .send(Message::Text(hello.to_string().into()))
                .unwrap();

            let challenge_message = client.read().unwrap();

            let challenge_text = match challenge_message {
                Message::Text(text) => text.to_string(),
                other => panic!("Expected CHALLENGE, received: {:?}", other),
            };

            let challenge_json: serde_json::Value = serde_json::from_str(&challenge_text).unwrap();

            assert_eq!(challenge_json["type"], "CHALLENGE");

            let challenge_bytes = STANDARD
                .decode(challenge_json["challenge"].as_str().unwrap())
                .unwrap();

            let challenge: [u8; 32] = challenge_bytes
                .try_into()
                .expect("Challenge must be 32 bytes");

            let proof =
                crate::utils::crypto::vendo_auth_proof(&verifier, &vendo_id, &challenge).unwrap();

            let auth = serde_json::json!({
                "type": "AUTH",
                "proof": STANDARD.encode(proof),
            });

            client.send(Message::Text(auth.to_string().into())).unwrap();

            let response = client.read().unwrap();

            let response_text = match response {
                Message::Text(text) => text.to_string(),
                other => panic!("Expected AUTH_OK, received: {:?}", other),
            };

            let response_json: serde_json::Value = serde_json::from_str(&response_text).unwrap();

            assert_eq!(response_json["type"], "AUTH_OK");
            assert_eq!(response_json["protocol"], "vendo-auth-v1");

            let _ = client.close(None);

            // Second connection: deliberately corrupted proof must fail.
            let (mut client, _) = connect(url.as_str()).unwrap();

            let hello = serde_json::json!({
                "type": "HELLO",
                "vendo_id": vendo_id,
            });

            client
                .send(Message::Text(hello.to_string().into()))
                .unwrap();

            let challenge_message = client.read().unwrap();

            let challenge_text = match challenge_message {
                Message::Text(text) => text.to_string(),
                other => panic!("Expected CHALLENGE, received: {:?}", other),
            };

            let challenge_json: serde_json::Value = serde_json::from_str(&challenge_text).unwrap();

            let challenge_bytes = STANDARD
                .decode(challenge_json["challenge"].as_str().unwrap())
                .unwrap();

            let challenge: [u8; 32] = challenge_bytes
                .try_into()
                .expect("Challenge must be 32 bytes");

            let mut invalid_proof =
                crate::utils::crypto::vendo_auth_proof(&verifier, &vendo_id, &challenge).unwrap();

            invalid_proof[0] ^= 0x01;

            let auth = serde_json::json!({
                "type": "AUTH",
                "proof": STANDARD.encode(invalid_proof),
            });

            client.send(Message::Text(auth.to_string().into())).unwrap();

            let response = client.read();

            assert!(
                !matches!(
                    response,
                    Ok(Message::Text(ref text))
                        if text.contains("AUTH_OK")
                ),
                "Invalid credential must never receive AUTH_OK"
            );
        })
        .await
        .unwrap();

        server_task.abort();
    }
}
