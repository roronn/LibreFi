use axum::{
    extract::{
        ConnectInfo,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use std::net::SocketAddr;
use tokio::time::{Duration, timeout};

use crate::{
    routes::ws::state::{register_vendo_connection, unregister_vendo_connection},
    utils::{
        crypto::{derive_vendo_session_key, generate_vendo_challenge, verify_vendo_auth_proof},
        db::get_vendo_auth_verifier,
    },
};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HANDSHAKE_MESSAGE: usize = 2048;

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

    // Keep the session key alive for this connection.
    let _session_key = session_key;

    // Stage 2 only: maintain the authenticated connection.
    // Application messages will be added in the subsequent stages.
    loop {
        match socket.recv().await {
            Some(Ok(Message::Ping(payload))) => {
                if socket.send(Message::Pong(payload)).await.is_err() {
                    break;
                }
            }
            Some(Ok(Message::Pong(_))) => {}
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                break;
            }
            Some(Ok(_)) => {
                // Do not accept application messages before their
                // authenticated encrypted protocol is implemented.
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
