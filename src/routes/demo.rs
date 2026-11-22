use crate::network::server::{Request, Response, Server};
use crate::utils::db::{
    convert_amount_to_time, credit_vendo_payment_for_session, current_timestamp,
    find_session_by_browser_token, generate_secure_id, get_vendo,
    get_vendo_reservation_for_session, list_vendos, next_vendo_payment_sequence,
    VendoPaymentEvent, VendoPaymentResult,
};
use serde::Deserialize;
use serde_json::json;

fn demo_allowed(req: &Request, res: &mut Response) -> bool {
    let enabled = std::env::var("LIBREFI_DEMO_MODE")
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let loopback = req.ip.parse::<std::net::IpAddr>()
        .map(|address| address.is_loopback())
        .unwrap_or(false);

    if !enabled || !loopback {
        res.status = 403;
        res.body = json!({
            "ok": false,
            "error": "Local demo mode is disabled or this request is not from localhost"
        }).to_string().into_bytes();
        res.content_type = "application/json".to_string();
        return false;
    }

    true
}

fn is_demo_vendo(name: &str) -> bool {
    let normalized = name.trim().to_ascii_lowercase();
    normalized.contains("dummy") || normalized.starts_with("test-") || normalized.starts_with("test ")
}

fn respond(res: &mut Response, status: u16, value: serde_json::Value) {
    res.status = status;
    res.body = value.to_string().into_bytes();
    res.content_type = "application/json".to_string();
}

#[derive(Deserialize)]
struct DemoPaymentPayload {
    browser_token: String,
    amount: u32,
}

pub fn demo(server: &mut Server) {
    // Windows-only, localhost-only demo activation. Only explicitly named
    // dummy/test vendos are marked online; real vendo connections are untouched.
    server.post("/test/vendo/activate", |req, res| {
        if !demo_allowed(req, res) {
            return;
        }

        let vendos = match list_vendos() {
            Ok(vendos) => vendos,
            Err(error) => {
                respond(res, 500, json!({"ok": false, "error": error}));
                return;
            }
        };

        let mut activated = Vec::new();
        for vendo in vendos {
            if !vendo.enabled || !is_demo_vendo(&vendo.name) {
                continue;
            }

            if !crate::routes::ws::state::is_vendo_online(&vendo.vendo_id) {
                crate::routes::ws::state::register_vendo_connection(&vendo.vendo_id);
            }
            activated.push(json!({
                "vendo_id": vendo.vendo_id,
                "name": vendo.name,
            }));
        }

        if activated.is_empty() {
            respond(res, 404, json!({
                "ok": false,
                "error": "No enabled dummy/test vendo found. Name it dummy or TEST-VENDO-01."
            }));
            return;
        }

        respond(res, 200, json!({"ok": true, "vendos": activated}));
    });

    // Simulates a coin pulse but uses the exact production reservation,
    // idempotency, sequence, and session-credit transaction.
    server.post("/test/vendo/payment", |req, res| {
        if !demo_allowed(req, res) {
            return;
        }

        let payload: DemoPaymentPayload = match serde_json::from_slice(&req.body) {
            Ok(payload) => payload,
            Err(_) => {
                respond(res, 400, json!({"ok": false, "error": "Invalid demo payment payload"}));
                return;
            }
        };

        if payload.browser_token.len() != 64
            || !payload.browser_token.bytes().all(|byte| byte.is_ascii_hexdigit())
            || payload.amount == 0
            || payload.amount > 1000
        {
            respond(res, 400, json!({"ok": false, "error": "Invalid browser token or demo amount"}));
            return;
        }

        let session_id = match find_session_by_browser_token(&payload.browser_token) {
            Ok(Some(session_id)) => session_id,
            Ok(None) => {
                respond(res, 401, json!({"ok": false, "error": "Customer browser session was not found. Refresh the page and try again."}));
                return;
            }
            Err(error) => {
                respond(res, 500, json!({"ok": false, "error": error}));
                return;
            }
        };

        let reservation = match get_vendo_reservation_for_session(&session_id) {
            Ok(Some(reservation)) => reservation,
            Ok(None) => {
                respond(res, 409, json!({"ok": false, "error": "This customer has no active vendo reservation. Reopen Insert Coin."}));
                return;
            }
            Err(error) => {
                respond(res, 500, json!({"ok": false, "error": error}));
                return;
            }
        };

        let vendo = match get_vendo(&reservation.vendo_id) {
            Ok(Some(vendo)) if is_demo_vendo(&vendo.name) => vendo,
            Ok(_) => {
                respond(res, 403, json!({"ok": false, "error": "Simulated payments are allowed only for dummy/test vendos"}));
                return;
            }
            Err(error) => {
                respond(res, 500, json!({"ok": false, "error": error}));
                return;
            }
        };

        if !crate::routes::ws::state::is_vendo_online(&vendo.vendo_id) {
            respond(res, 409, json!({"ok": false, "error": "Dummy vendo is offline. Reopen Insert Coin to activate demo mode."}));
            return;
        }

        let seconds = convert_amount_to_time(payload.amount);
        if seconds == 0 {
            respond(res, 400, json!({"ok": false, "error": "No configured rate matches this demo coin amount. Check Admin > Rates."}));
            return;
        }

        let sequence = match next_vendo_payment_sequence(&vendo.vendo_id) {
            Ok(sequence) => sequence,
            Err(error) => {
                respond(res, 500, json!({"ok": false, "error": error}));
                return;
            }
        };

        let event = VendoPaymentEvent {
            vendo_id: vendo.vendo_id,
            event_id: match generate_secure_id() {
                Ok(event_id) => event_id,
                Err(error) => {
                    respond(res, 500, json!({"ok": false, "error": error}));
                    return;
                }
            },
            sequence,
            amount: payload.amount,
            pulse_count: 1,
            device_timestamp: current_timestamp(),
        };

        match credit_vendo_payment_for_session(&session_id, seconds as u64, event) {
            Ok(VendoPaymentResult::Credited { browser_token, .. }) => {
                let mut notification = json!({
                    "type": "coin",
                    "value": payload.amount,
                    "time": seconds,
                });
                if let Some(token) = browser_token {
                    notification["browser_token"] = json!(token);
                }
                crate::routes::ws::state::notify_customer_session(&session_id, notification);
                respond(res, 200, json!({
                    "ok": true,
                    "amount": payload.amount,
                    "credited_seconds": seconds,
                }));
            }
            Ok(VendoPaymentResult::Duplicate { original_event_id }) => {
                respond(res, 409, json!({
                    "ok": false,
                    "error": "Duplicate demo payment",
                    "original_event_id": original_event_id,
                }));
            }
            Err(error) => respond(res, 400, json!({"ok": false, "error": error})),
        }
    });
}
