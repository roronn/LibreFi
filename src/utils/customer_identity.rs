use ed25519_dalek::{Signature, VerifyingKey};
use redb::{ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

use crate::utils::db::{
    BROWSER_TOKENS_TABLE, SESSIONS_TABLE, current_timestamp, generate_browser_token,
    generate_secure_id, get_db, hash_recovery_token,
};

const CHALLENGE_TTL_SECONDS: u64 = 60;

pub const CUSTOMER_DEVICE_KEYS_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("customer_device_keys");
pub const CUSTOMER_AUTH_CHALLENGES_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("customer_auth_challenges");

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CustomerDeviceKey {
    key_hash: String,
    session_id: String,
    public_key: String,
    created_at: u64,
    revoked_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CustomerAuthChallenge {
    challenge_id: String,
    challenge: String,
    purpose: String,
    session_id: String,
    key_hash: String,
    created_at: u64,
    expires_at: u64,
    used_at: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct CustomerAuthResult {
    pub session_id: String,
    pub browser_token: Option<String>,
}

fn decode_hex<const N: usize>(value: &str) -> Result<[u8; N], String> {
    if value.len() != N * 2 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Invalid hexadecimal value".to_string());
    }

    let mut output = [0u8; N];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let pair = std::str::from_utf8(pair).map_err(|error| error.to_string())?;
        output[index] = u8::from_str_radix(pair, 16).map_err(|error| error.to_string())?;
    }
    Ok(output)
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        write!(&mut output, "{byte:02x}").unwrap();
    }
    output
}

fn valid_browser_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn signature_message(purpose: &str, challenge_id: &str, challenge: &str) -> Vec<u8> {
    format!(
        "LibreFi/customer-auth/v1\n{purpose}\n{challenge_id}\n{challenge}"
    )
    .into_bytes()
}

fn verify_customer_signature(
    public_key: &str,
    signature: &str,
    purpose: &str,
    challenge_id: &str,
    challenge: &str,
) -> Result<bool, String> {
    let public_key_bytes = decode_hex::<32>(public_key)?;
    let signature_bytes = decode_hex::<64>(signature)?;
    let verifying_key = VerifyingKey::from_bytes(&public_key_bytes)
        .map_err(|_| "Invalid customer public key".to_string())?;
    let signature = Signature::from_bytes(&signature_bytes);
    let message = signature_message(purpose, challenge_id, challenge);

    Ok(verifying_key.verify_strict(&message, &signature).is_ok())
}

/// Creates a short-lived challenge. Registration is authorized by the current
/// browser token; authentication is authorized by an already-registered key.
pub fn issue_customer_challenge(
    purpose: &str,
    public_key: &str,
    browser_token: Option<&str>,
) -> Result<(String, String, u64), String> {
    if purpose != "register" && purpose != "authenticate" {
        return Err("Unsupported customer authentication purpose".to_string());
    }

    let public_key_bytes = decode_hex::<32>(public_key)?;
    VerifyingKey::from_bytes(&public_key_bytes)
        .map_err(|_| "Invalid customer public key".to_string())?;
    let canonical_public_key = encode_hex(&public_key_bytes);
    let key_hash = hash_recovery_token(&canonical_public_key);
    let challenge_id = generate_secure_id()?;
    let challenge = generate_secure_id()?;
    let now = current_timestamp();
    let expires_at = now.saturating_add(CHALLENGE_TTL_SECONDS);

    let db = get_db();
    let write_txn = db.begin_write().map_err(|error| error.to_string())?;

    let session_id = if purpose == "register" {
        let token = browser_token.ok_or_else(|| "Browser token is required".to_string())?;
        if !valid_browser_token(token) {
            return Err("Invalid browser token".to_string());
        }

        let token_hash = hash_recovery_token(token);
        let tokens = write_txn
            .open_table(BROWSER_TOKENS_TABLE)
            .map_err(|error| error.to_string())?;
        let session_id = tokens
            .get(token_hash.as_str())
            .map_err(|error| error.to_string())?
            .map(|value| value.value().to_string())
            .ok_or_else(|| "Browser token is not recognized".to_string())?;
        drop(tokens);

        let sessions = write_txn
            .open_table(SESSIONS_TABLE)
            .map_err(|error| error.to_string())?;
        if sessions
            .get(session_id.as_str())
            .map_err(|error| error.to_string())?
            .is_none()
        {
            return Err("Customer session not found".to_string());
        }
        session_id
    } else {
        let keys = write_txn
            .open_table(CUSTOMER_DEVICE_KEYS_TABLE)
            .map_err(|error| error.to_string())?;
        let record_json = keys
            .get(key_hash.as_str())
            .map_err(|error| error.to_string())?
            .map(|value| value.value().to_string())
            .ok_or_else(|| "Customer key is not registered".to_string())?;
        drop(keys);

        let record: CustomerDeviceKey =
            serde_json::from_str(&record_json).map_err(|error| error.to_string())?;
        if record.key_hash != key_hash || record.revoked_at.is_some() {
            return Err("Customer key is not active".to_string());
        }

        let sessions = write_txn
            .open_table(SESSIONS_TABLE)
            .map_err(|error| error.to_string())?;
        if sessions
            .get(record.session_id.as_str())
            .map_err(|error| error.to_string())?
            .is_none()
        {
            return Err("Customer session not found".to_string());
        }
        record.session_id
    };

    let record = CustomerAuthChallenge {
        challenge_id: challenge_id.clone(),
        challenge: challenge.clone(),
        purpose: purpose.to_string(),
        session_id,
        key_hash,
        created_at: now,
        expires_at,
        used_at: None,
    };
    let record_json = serde_json::to_string(&record).map_err(|error| error.to_string())?;

    let mut challenges = write_txn
        .open_table(CUSTOMER_AUTH_CHALLENGES_TABLE)
        .map_err(|error| error.to_string())?;

    // Keep the persistent challenge table bounded by pruning expired or used entries.
    let expired_ids = challenges
        .iter()
        .map_err(|error| error.to_string())?
        .map(|entry| {
            let (key, value) = entry.map_err(|error| error.to_string())?;
            let record: CustomerAuthChallenge =
                serde_json::from_str(value.value()).map_err(|error| error.to_string())?;
            Ok(if record.expires_at <= now || record.used_at.is_some() {
                Some(key.value().to_string())
            } else {
                None
            })
        })
        .collect::<Result<Vec<_>, String>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    for expired_id in expired_ids {
        challenges.remove(expired_id.as_str()).map_err(|error| error.to_string())?;
    }

    challenges
        .insert(challenge_id.as_str(), record_json.as_str())
        .map_err(|error| error.to_string())?;
    drop(challenges);

    write_txn.commit().map_err(|error| error.to_string())?;
    Ok((challenge_id, challenge, expires_at))
}

/// Verifies and consumes a challenge. Every valid challenge can be used once.
pub fn complete_customer_challenge(
    purpose: &str,
    challenge_id: &str,
    public_key: &str,
    signature: &str,
    browser_token: Option<&str>,
) -> Result<CustomerAuthResult, String> {
    if purpose != "register" && purpose != "authenticate" {
        return Err("Unsupported customer authentication purpose".to_string());
    }

    let public_key_bytes = decode_hex::<32>(public_key)?;
    VerifyingKey::from_bytes(&public_key_bytes)
        .map_err(|_| "Invalid customer public key".to_string())?;
    let canonical_public_key = encode_hex(&public_key_bytes);
    let key_hash = hash_recovery_token(&canonical_public_key);
    let now = current_timestamp();

    let db = get_db();
    let write_txn = db.begin_write().map_err(|error| error.to_string())?;

    let mut challenges = write_txn
        .open_table(CUSTOMER_AUTH_CHALLENGES_TABLE)
        .map_err(|error| error.to_string())?;
    let challenge_json = challenges
        .get(challenge_id)
        .map_err(|error| error.to_string())?
        .map(|value| value.value().to_string())
        .ok_or_else(|| "Customer challenge not found".to_string())?;
    let mut challenge: CustomerAuthChallenge =
        serde_json::from_str(&challenge_json).map_err(|error| error.to_string())?;

    if challenge.challenge_id != challenge_id
        || challenge.purpose != purpose
        || challenge.key_hash != key_hash
        || challenge.used_at.is_some()
        || challenge.expires_at <= now
    {
        return Err("Customer challenge is invalid or expired".to_string());
    }

    let signature_valid = verify_customer_signature(
        &canonical_public_key,
        signature,
        purpose,
        challenge_id,
        &challenge.challenge,
    )?;

    // Consume the challenge even when its signature is invalid, preventing
    // repeated guesses against the same challenge.
    challenge.used_at = Some(now);
    let updated_challenge =
        serde_json::to_string(&challenge).map_err(|error| error.to_string())?;
    challenges
        .insert(challenge_id, updated_challenge.as_str())
        .map_err(|error| error.to_string())?;
    drop(challenges);

    if !signature_valid {
        write_txn.commit().map_err(|error| error.to_string())?;
        return Err("Customer signature is invalid".to_string());
    }

    if purpose == "register" {
        let token = browser_token.ok_or_else(|| "Browser token is required".to_string())?;
        if !valid_browser_token(token) {
            return Err("Invalid browser token".to_string());
        }
        let token_hash = hash_recovery_token(token);
        let tokens = write_txn
            .open_table(BROWSER_TOKENS_TABLE)
            .map_err(|error| error.to_string())?;
        let token_session_id = tokens
            .get(token_hash.as_str())
            .map_err(|error| error.to_string())?
            .map(|value| value.value().to_string());
        drop(tokens);

        if token_session_id.as_deref() != Some(challenge.session_id.as_str()) {
            write_txn.commit().map_err(|error| error.to_string())?;
            return Err("Browser token does not own this session".to_string());
        }

        let mut keys = write_txn
            .open_table(CUSTOMER_DEVICE_KEYS_TABLE)
            .map_err(|error| error.to_string())?;
        if let Some(existing_json) = keys
            .get(key_hash.as_str())
            .map_err(|error| error.to_string())?
            .map(|value| value.value().to_string())
        {
            let existing: CustomerDeviceKey =
                serde_json::from_str(&existing_json).map_err(|error| error.to_string())?;
            if existing.session_id != challenge.session_id || existing.revoked_at.is_some() {
                drop(keys);
                write_txn.commit().map_err(|error| error.to_string())?;
                return Err("Customer key is already assigned or revoked".to_string());
            }
        } else {
            let record = CustomerDeviceKey {
                key_hash: key_hash.clone(),
                session_id: challenge.session_id.clone(),
                public_key: canonical_public_key,
                created_at: now,
                revoked_at: None,
            };
            let record_json =
                serde_json::to_string(&record).map_err(|error| error.to_string())?;
            keys.insert(key_hash.as_str(), record_json.as_str())
                .map_err(|error| error.to_string())?;
        }
        drop(keys);
        write_txn.commit().map_err(|error| error.to_string())?;

        return Ok(CustomerAuthResult {
            session_id: challenge.session_id,
            browser_token: None,
        });
    }

    let keys = write_txn
        .open_table(CUSTOMER_DEVICE_KEYS_TABLE)
        .map_err(|error| error.to_string())?;
    let record_json = keys
        .get(key_hash.as_str())
        .map_err(|error| error.to_string())?
        .map(|value| value.value().to_string())
        .ok_or_else(|| "Customer key is not registered".to_string())?;
    drop(keys);

    let record: CustomerDeviceKey =
        serde_json::from_str(&record_json).map_err(|error| error.to_string())?;
    if record.session_id != challenge.session_id || record.revoked_at.is_some() {
        write_txn.commit().map_err(|error| error.to_string())?;
        return Err("Customer key is not active".to_string());
    }

    let sessions = write_txn
        .open_table(SESSIONS_TABLE)
        .map_err(|error| error.to_string())?;
    if sessions
        .get(challenge.session_id.as_str())
        .map_err(|error| error.to_string())?
        .is_none()
    {
        write_txn.commit().map_err(|error| error.to_string())?;
        return Err("Customer session not found".to_string());
    }
    drop(sessions);

    let browser_token = generate_browser_token()?;
    let token_hash = hash_recovery_token(&browser_token);
    let mut tokens = write_txn
        .open_table(BROWSER_TOKENS_TABLE)
        .map_err(|error| error.to_string())?;
    tokens
        .insert(token_hash.as_str(), challenge.session_id.as_str())
        .map_err(|error| error.to_string())?;
    drop(tokens);

    write_txn.commit().map_err(|error| error.to_string())?;

    Ok(CustomerAuthResult {
        session_id: challenge.session_id,
        browser_token: Some(browser_token),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn customer_signature_is_bound_to_purpose_and_challenge_id() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let public_key = encode_hex(signing_key.verifying_key().as_bytes());
        let message = signature_message("authenticate", "challenge-1", "aabb");
        let signature = signing_key.sign(&message);
        let signature_hex = encode_hex(&signature.to_bytes());

        assert!(verify_customer_signature(
            &public_key,
            &signature_hex,
            "authenticate",
            "challenge-1",
            "aabb"
        ).unwrap());

        assert!(!verify_customer_signature(
            &public_key,
            &signature_hex,
            "register",
            "challenge-1",
            "aabb"
        ).unwrap());

        assert!(!verify_customer_signature(
            &public_key,
            &signature_hex,
            "authenticate",
            "challenge-2",
            "aabb"
        ).unwrap());
    }

    #[test]
    fn registration_and_authentication_challenges_are_one_time() {
        use crate::utils::db::{
            Session, create_session_without_device, find_session_by_browser_token,
            generate_browser_token, generate_secure_id, init_test_db_once,
        };

        init_test_db_once();
        let session_id = generate_secure_id().unwrap();
        let browser_token = generate_browser_token().unwrap();
        let session = Session::new("Identity Test".to_string(), "127.0.0.1".to_string(), current_timestamp());
        create_session_without_device(&session_id, &session, &browser_token).unwrap();

        let signing_key = SigningKey::from_bytes(&[13u8; 32]);
        let public_key = encode_hex(signing_key.verifying_key().as_bytes());

        let (register_id, register_challenge, _) =
            issue_customer_challenge("register", &public_key, Some(&browser_token)).unwrap();
        let register_message = signature_message("register", &register_id, &register_challenge);
        let register_signature = encode_hex(&signing_key.sign(&register_message).to_bytes());

        let registered = complete_customer_challenge(
            "register",
            &register_id,
            &public_key,
            &register_signature,
            Some(&browser_token),
        ).unwrap();
        assert_eq!(registered.session_id, session_id);
        assert!(complete_customer_challenge(
            "register",
            &register_id,
            &public_key,
            &register_signature,
            Some(&browser_token),
        ).is_err());

        let (auth_id, auth_challenge, _) =
            issue_customer_challenge("authenticate", &public_key, None).unwrap();
        let auth_message = signature_message("authenticate", &auth_id, &auth_challenge);
        let auth_signature = encode_hex(&signing_key.sign(&auth_message).to_bytes());

        let authenticated = complete_customer_challenge(
            "authenticate",
            &auth_id,
            &public_key,
            &auth_signature,
            None,
        ).unwrap();
        assert_eq!(authenticated.session_id, session_id);
        let recovered_token = authenticated.browser_token.unwrap();
        assert_eq!(
            find_session_by_browser_token(&recovered_token).unwrap().as_deref(),
            Some(session_id.as_str())
        );
    }

    #[test]
    fn invalid_customer_signatures_are_rejected() {
        let signing_key = SigningKey::from_bytes(&[9u8; 32]);
        let public_key = encode_hex(signing_key.verifying_key().as_bytes());
        let message = signature_message("register", "challenge-3", "ccdd");
        let signature = signing_key.sign(&message);
        let mut signature_bytes = signature.to_bytes();
        signature_bytes[0] ^= 1;
        let signature_hex = encode_hex(&signature_bytes);

        assert!(!verify_customer_signature(
            &public_key,
            &signature_hex,
            "register",
            "challenge-3",
            "ccdd"
        ).unwrap());
    }
}
