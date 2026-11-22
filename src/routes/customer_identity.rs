use crate::{
    network::server::{Response, Server},
    utils::customer_identity::{
        complete_customer_challenge, issue_customer_challenge,
    },
};
use serde::Deserialize;

#[derive(Deserialize)]
struct ChallengeRequest {
    purpose: String,
    public_key: String,
    browser_token: Option<String>,
}

#[derive(Deserialize)]
struct ProofRequest {
    purpose: String,
    challenge_id: String,
    public_key: String,
    signature: String,
    browser_token: Option<String>,
}

fn json_error(status: u16, message: &str, response: &mut Response) {
    response.status = status;
    response.body = serde_json::json!({ "error": message }).to_string().into_bytes();
    response.content_type = "application/json".to_string();
}

fn parse_body<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, ()> {
    if body.len() > 8192 {
        return Err(());
    }
    serde_json::from_slice(body).map_err(|_| ())
}

pub fn customer_identity(server: &mut Server) {
    server.post("/api/customer/key/challenge", |request, response| {
        let body: ChallengeRequest = match parse_body(&request.body) {
            Ok(body) => body,
            Err(()) => {
                json_error(400, "Invalid customer challenge request", response);
                return;
            }
        };

        match issue_customer_challenge(
            &body.purpose,
            &body.public_key,
            body.browser_token.as_deref(),
        ) {
            Ok((challenge_id, challenge, expires_at)) => {
                response.status = 200;
                response.body = serde_json::json!({
                    "ok": true,
                    "challenge_id": challenge_id,
                    "challenge": challenge,
                    "expires_at": expires_at
                }).to_string().into_bytes();
                response.content_type = "application/json".to_string();
            }
            Err(error) => {
                crate::debug_println!("Customer challenge request rejected: {}", error);
                json_error(401, "Customer identity could not be verified", response);
            }
        }
    });

    server.post("/api/customer/key/register", |request, response| {
        let body: ProofRequest = match parse_body(&request.body) {
            Ok(body) => body,
            Err(()) => {
                json_error(400, "Invalid customer key registration request", response);
                return;
            }
        };

        if body.purpose != "register" {
            json_error(400, "Invalid customer key registration purpose", response);
            return;
        }

        match complete_customer_challenge(
            "register",
            &body.challenge_id,
            &body.public_key,
            &body.signature,
            body.browser_token.as_deref(),
        ) {
            Ok(result) => {
                response.status = 200;
                response.body = serde_json::json!({
                    "ok": true,
                    "session_id": result.session_id
                }).to_string().into_bytes();
                response.content_type = "application/json".to_string();
            }
            Err(error) => {
                crate::debug_println!("Customer key registration rejected: {}", error);
                json_error(401, "Customer key registration could not be verified", response);
            }
        }
    });

    server.post("/api/customer/key/authenticate", |request, response| {
        let body: ProofRequest = match parse_body(&request.body) {
            Ok(body) => body,
            Err(()) => {
                json_error(400, "Invalid customer authentication request", response);
                return;
            }
        };

        if body.purpose != "authenticate" {
            json_error(400, "Invalid customer authentication purpose", response);
            return;
        }

        match complete_customer_challenge(
            "authenticate",
            &body.challenge_id,
            &body.public_key,
            &body.signature,
            None,
        ) {
            Ok(result) => {
                response.status = 200;
                response.body = serde_json::json!({
                    "ok": true,
                    "session_id": result.session_id,
                    "browser_token": result.browser_token
                }).to_string().into_bytes();
                response.content_type = "application/json".to_string();
            }
            Err(error) => {
                crate::debug_println!("Customer key authentication rejected: {}", error);
                json_error(401, "Customer identity could not be verified", response);
            }
        }
    });
}
