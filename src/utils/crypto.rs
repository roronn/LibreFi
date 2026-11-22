use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};

/// Decrypts a byte vector containing [Nonce + Ciphertext] back into a String
pub fn decrypt_payload(encrypted_data: &[u8], secret_key: &[u8; 32]) -> Result<String, String> {
    if encrypted_data.len() < 12 {
        return Err("Data too short to contain nonce".to_string());
    }

    let key = Key::<Aes256Gcm>::from_slice(secret_key);
    let cipher = Aes256Gcm::new(key);

    // Split the data back into Nonce and Ciphertext
    let (nonce_bytes, ciphertext) = encrypted_data.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);

    // Decrypt the payload
    let plaintext_bytes = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("Decryption failed: {}", e))?;

    // Convert bytes back to a UTF-8 String
    String::from_utf8(plaintext_bytes)
        .map_err(|e| format!("Invalid UTF-8 in decrypted data: {}", e))
}

// Multi-vendo WebSocket authentication helpers.

use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Generates a cryptographically secure 32-byte challenge.
pub fn generate_vendo_challenge() -> Result<[u8; 32], String> {
    let mut challenge = [0u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut challenge)
        .map_err(|e| format!("Secure challenge generation failed: {}", e))?;
    Ok(challenge)
}

/// Calculates the device's HMAC-SHA256 authentication proof.
pub fn vendo_auth_proof(
    verifier: &[u8; 32],
    vendo_id: &str,
    challenge: &[u8; 32],
) -> Result<[u8; 32], String> {
    let id_len = u32::try_from(vendo_id.len())
        .map_err(|_| "Vendo ID is too long".to_string())?;

    let mut mac = <HmacSha256 as hmac::digest::KeyInit>::new_from_slice(verifier)
        .map_err(|_| "Invalid authentication verifier".to_string())?;

    mac.update(b"LibreFi/vendo-auth/v1");
    mac.update(&id_len.to_be_bytes());
    mac.update(vendo_id.as_bytes());
    mac.update(challenge);

    let result = mac.finalize().into_bytes();
    let mut proof = [0u8; 32];
    proof.copy_from_slice(&result);
    Ok(proof)
}

/// Verifies a device proof without a normal string comparison.
pub fn verify_vendo_auth_proof(
    verifier: &[u8; 32],
    vendo_id: &str,
    challenge: &[u8; 32],
    proof: &[u8],
) -> bool {
    let Ok(id_len) = u32::try_from(vendo_id.len()) else {
        return false;
    };

    let Ok(mut mac) = <HmacSha256 as hmac::digest::KeyInit>::new_from_slice(verifier) else {
        return false;
    };

    mac.update(b"LibreFi/vendo-auth/v1");
    mac.update(&id_len.to_be_bytes());
    mac.update(vendo_id.as_bytes());
    mac.update(challenge);

    mac.verify_slice(proof).is_ok()
}

/// Derives a separate AES-256-GCM key for this device connection.
pub fn derive_vendo_session_key(
    verifier: &[u8; 32],
    vendo_id: &str,
    challenge: &[u8; 32],
) -> Result<[u8; 32], String> {
    let id_len = u32::try_from(vendo_id.len())
        .map_err(|_| "Vendo ID is too long".to_string())?;

    let mut mac = <HmacSha256 as hmac::digest::KeyInit>::new_from_slice(verifier)
        .map_err(|_| "Invalid authentication verifier".to_string())?;

    mac.update(b"LibreFi/vendo-session/v1");
    mac.update(&id_len.to_be_bytes());
    mac.update(vendo_id.as_bytes());
    mac.update(challenge);

    let result = mac.finalize().into_bytes();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    Ok(key)
}

// Tests for multi-vendo authentication and session-key derivation.
#[cfg(test)]
mod vendo_crypto_tests {
    use super::*;

    #[test]
    fn valid_authentication_proof_is_accepted() {
        let verifier = [7u8; 32];
        let challenge = [9u8; 32];

        let proof = vendo_auth_proof(&verifier, "vendo-001", &challenge).unwrap();

        assert!(verify_vendo_auth_proof(
            &verifier,
            "vendo-001",
            &challenge,
            &proof
        ));
    }

    #[test]
    fn invalid_authentication_proofs_are_rejected() {
        let verifier = [7u8; 32];
        let wrong_verifier = [8u8; 32];
        let challenge = [9u8; 32];
        let proof = vendo_auth_proof(&verifier, "vendo-001", &challenge).unwrap();

        assert!(!verify_vendo_auth_proof(
            &verifier,
            "vendo-002",
            &challenge,
            &proof
        ));

        assert!(!verify_vendo_auth_proof(
            &wrong_verifier,
            "vendo-001",
            &challenge,
            &proof
        ));

        let wrong_challenge = [10u8; 32];
        assert!(!verify_vendo_auth_proof(
            &verifier,
            "vendo-001",
            &wrong_challenge,
            &proof
        ));

        assert!(!verify_vendo_auth_proof(
            &verifier,
            "vendo-001",
            &challenge,
            &[0u8; 31]
        ));
    }

    #[test]
    fn session_key_is_stable_for_same_connection_inputs() {
        let verifier = [7u8; 32];
        let challenge = [9u8; 32];

        let first = derive_vendo_session_key(
            &verifier,
            "vendo-001",
            &challenge
        ).unwrap();

        let second = derive_vendo_session_key(
            &verifier,
            "vendo-001",
            &challenge
        ).unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn session_key_changes_when_connection_inputs_change() {
        let verifier = [7u8; 32];
        let challenge = [9u8; 32];

        let original = derive_vendo_session_key(
            &verifier,
            "vendo-001",
            &challenge
        ).unwrap();

        let changed_challenge = derive_vendo_session_key(
            &verifier,
            "vendo-001",
            &[10u8; 32]
        ).unwrap();

        let changed_vendo = derive_vendo_session_key(
            &verifier,
            "vendo-002",
            &challenge
        ).unwrap();

        assert_ne!(original, changed_challenge);
        assert_ne!(original, changed_vendo);
        assert_ne!(original, verifier);
    }

    #[test]
    fn session_key_is_not_the_authentication_proof() {
        let verifier = [7u8; 32];
        let challenge = [9u8; 32];

        let proof = vendo_auth_proof(
            &verifier,
            "vendo-001",
            &challenge
        ).unwrap();

        let session_key = derive_vendo_session_key(
            &verifier,
            "vendo-001",
            &challenge
        ).unwrap();

        assert_ne!(proof, session_key);
    }

    #[test]
    fn generated_challenges_are_32_bytes_and_fresh() {
        let first = generate_vendo_challenge().unwrap();
        let second = generate_vendo_challenge().unwrap();

        assert_eq!(first.len(), 32);
        assert_eq!(second.len(), 32);
        assert_ne!(first, second);
    }
}
