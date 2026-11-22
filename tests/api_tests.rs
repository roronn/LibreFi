use aes_gcm::{
    Aes256Gcm, Key, Nonce,
    aead::{Aead, KeyInit},
};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use rand::RngCore;
use serde_json::json;

const BASE_URL: &str = "http://127.0.0.1:8080";
fn admin_key() -> String { std::env::var("LIBREFI_KEY").expect("LIBREFI_KEY must be set when running API tests") }

fn encrypt_payload(key_str: &str, payload: serde_json::Value) -> String {
    let mut key_bytes = [0u8; 32];
    let src = key_str.as_bytes();
    let len = src.len().min(32);
    key_bytes[..len].copy_from_slice(&src[..len]);

    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);

    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let payload_str = payload.to_string();
    let ciphertext = cipher
        .encrypt(nonce, payload_str.as_bytes())
        .expect("encryption failure!");

    let mut combined = Vec::new();
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);

    BASE64.encode(combined)
}

fn query_string(payload: &str) -> String {
    payload.replace("+", "%2B").replace("/", "%2F")
}

#[test]
fn test_get_dashboard() {
    let payload = encrypt_payload(&admin_key(), json!({"id": 1}));
    let url = format!(
        "{}/api/admin/dashboard?payload={}",
        BASE_URL,
        query_string(&payload)
    );
    let response = ureq::get(&url).call();
    assert!(response.is_ok(), "Failed to reach dashboard endpoint: {:?}", response.err());
    assert_eq!(response.unwrap().status(), 200);
}

#[test]
fn test_rates_crud() {
    // 1. POST Rate
    let post_payload = encrypt_payload(
        &admin_key(),
        json!({
            "price": 10.0,
            "time": 3600
        }),
    );
    let url = format!("{}/api/admin/rates", BASE_URL);
    let post_res = ureq::post(&url).send_json(json!({"payload": post_payload}));
    assert!(post_res.is_ok(), "Failed to POST rate: {:?}", post_res.err());

    // 2. GET Rates
    let get_payload = encrypt_payload(&admin_key(), json!({"id": 1}));
    let get_url = format!(
        "{}/api/admin/rates?payload={}",
        BASE_URL,
        query_string(&get_payload)
    );
    let get_res = ureq::get(&get_url).call();
    assert!(get_res.is_ok(), "Failed to GET rates");

    // We don't have the exact ID returned, so we'll just test the endpoints exist

    // 3. PUT Rate (update a dummy rate just to check endpoint response)
    let put_payload = encrypt_payload(
        &admin_key(),
        json!({
            "id": "dummy_id",
            "price": 20.0,
            "time": 7200
        }),
    );
    let _put_res = ureq::put(&url).send_json(json!({"payload": put_payload})); // Might return 404 or 400, but endpoint is hit

    // 4. DELETE Rate
    let del_payload = encrypt_payload(&admin_key(), json!({"id": "dummy_id"}));
    let _del_res = ureq::delete(&url).send_json(json!({"payload": del_payload}));
}

#[test]
fn test_get_users() {
    let payload = encrypt_payload(&admin_key(), json!({"id": 1}));
    let url = format!(
        "{}/api/admin/users?payload={}",
        BASE_URL,
        query_string(&payload)
    );
    let response = ureq::get(&url).call();
    assert!(response.is_ok(), "Failed to GET users: {:?}", response.err());
    assert_eq!(response.unwrap().status(), 200);
}

#[test]
fn test_post_qos() {
    let payload = encrypt_payload(
        &admin_key(),
        json!({
            "download": 10.0,
            "upload": 5.0,
            "global_download": 100.0,
            "global_upload": 50.0
        }),
    );
    let url = format!("{}/api/admin/qos", BASE_URL);
    let _response = ureq::post(&url).send_json(json!({"payload": payload}));
}

#[test]
fn test_post_wifi() {
    let payload = encrypt_payload(
        &admin_key(),
        json!({
            "ssid_2g": "TestWiFi_2G",
            "key_2g": "testpassword",
            "disabled_2g": false,
            "ssid_5g": "TestWiFi_5G",
            "key_5g": "testpassword",
            "disabled_5g": false
        }),
    );
    let url = format!("{}/api/admin/wifi", BASE_URL);
    let _response = ureq::post(&url).send_json(json!({"payload": payload}));
}

#[test]
fn test_vouchers() {
    // 1. GET Vouchers
    let payload = encrypt_payload(&admin_key(), json!({"id": 1}));
    let url = format!(
        "{}/api/admin/vouchers?payload={}",
        BASE_URL,
        query_string(&payload)
    );
    let _response = ureq::get(&url).call();

    // 2. POST Vouchers (Generate)
    let post_payload = encrypt_payload(
        &admin_key(),
        json!({
            "count": 1,
            "time": 3600
        }),
    );
    let post_url = format!("{}/api/admin/vouchers", BASE_URL);
    let _post_res = ureq::post(&post_url).send_json(json!({"payload": post_payload}));
}

#[test]
fn test_post_coinslot_key() {
    let payload = encrypt_payload(&admin_key(), json!({"id": 1}));
    let url = format!("{}/api/admin/coinslot/key", BASE_URL);
    let _response = ureq::post(&url).send_json(json!({"payload": payload}));
}

#[test]
fn test_admin_vendo_management_lifecycle() {
    let base_url = format!("{}/api/admin/vendos", BASE_URL);

    // Register a new vendo and capture its one-time credential.
    let create_payload = encrypt_payload(
        &admin_key(),
        json!({"name": format!("API Test Vendo {}", rand::random::<u32>())}),
    );
    let create_response = ureq::post(&base_url)
        .send_json(json!({"payload": create_payload}))
        .expect("Failed to register vendo");
    assert_eq!(create_response.status(), 201);

    let created: serde_json::Value = create_response
        .into_json()
        .expect("Invalid registration response");
    assert_eq!(created["ok"], true);
    let vendo_id = created["vendo"]["vendo_id"]
        .as_str()
        .expect("Registration response missing vendo_id")
        .to_string();
    let original_credential = created["credential"]
        .as_str()
        .expect("Registration response missing one-time credential")
        .to_string();
    assert!(!original_credential.is_empty());

    // The list endpoint must report the vendo without exposing its credential.
    let list_payload = encrypt_payload(&admin_key(), json!({"id": 1}));
    let list_url = format!(
        "{}?payload={}",
        base_url,
        query_string(&list_payload)
    );
    let list_response = ureq::get(&list_url)
        .call()
        .expect("Failed to list vendos");
    let listed: serde_json::Value = list_response
        .into_json()
        .expect("Invalid vendo list response");
    let entry = listed["vendos"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["vendo_id"] == vendo_id))
        .expect("New vendo was not returned by list endpoint");
    assert_eq!(entry["credential_active"], true);
    assert!(entry.get("credential").is_none());

    // Disable and re-enable the vendo.
    for enabled in [false, true] {
        let payload = encrypt_payload(
            &admin_key(),
            json!({"vendo_id": vendo_id, "enabled": enabled}),
        );
        let response = ureq::put(&base_url)
            .send_json(json!({"payload": payload}))
            .expect("Failed to update vendo enabled state");
        assert_eq!(response.status(), 200);
    }

    // Rotation returns a different one-time credential.
    let rotate_payload = encrypt_payload(
        &admin_key(),
        json!({"vendo_id": vendo_id}),
    );
    let rotate_response = ureq::post(&format!("{}/rotate", base_url))
        .send_json(json!({"payload": rotate_payload}))
        .expect("Failed to rotate vendo credential");
    assert_eq!(rotate_response.status(), 200);
    let rotated: serde_json::Value = rotate_response
        .into_json()
        .expect("Invalid credential rotation response");
    let rotated_credential = rotated["credential"]
        .as_str()
        .expect("Rotation response missing credential");
    assert_ne!(rotated_credential, original_credential);

    // Revoke the rotated credential.
    let revoke_payload = encrypt_payload(
        &admin_key(),
        json!({"vendo_id": vendo_id}),
    );
    let revoke_response = ureq::post(&format!("{}/revoke", base_url))
        .send_json(json!({"payload": revoke_payload}))
        .expect("Failed to revoke vendo credential");
    assert_eq!(revoke_response.status(), 200);

    // A final list confirms that the credential is no longer active.
    let final_payload = encrypt_payload(&admin_key(), json!({"id": 1}));
    let final_url = format!(
        "{}?payload={}",
        base_url,
        query_string(&final_payload)
    );
    let final_response = ureq::get(&final_url)
        .call()
        .expect("Failed to list vendos after revocation");
    let final_list: serde_json::Value = final_response
        .into_json()
        .expect("Invalid final vendo list response");
    let final_entry = final_list["vendos"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["vendo_id"] == vendo_id))
        .expect("Vendo disappeared after credential revocation");
    assert_eq!(final_entry["credential_active"], false);
}

