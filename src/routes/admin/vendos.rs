use crate::network::server::Server;
use crate::utils::db::{
    get_vendo_auth_verifier, list_vendos, register_vendo, revoke_vendo_credential,
    rotate_vendo_credential, set_vendo_enabled,
};
use serde::{Deserialize, Serialize};

use super::auth::parse_admin_payload;

#[derive(Deserialize)]
struct CreateVendoPayload {
    name: String,
}

#[derive(Deserialize)]
struct VendoIdPayload {
    vendo_id: String,
}

#[derive(Deserialize)]
struct SetVendoEnabledPayload {
    vendo_id: String,
    enabled: bool,
}

#[derive(Serialize)]
struct AdminVendo {
    vendo_id: String,
    name: String,
    enabled: bool,
    online: bool,
    credential_active: bool,
    created_at: u64,
}

fn parse_payload<T: for<'de> Deserialize<'de>>(
    req: &crate::network::server::Request,
    res: &mut crate::network::server::Response,
) -> Option<T> {
    let json = match parse_admin_payload(req) {
        Ok(json) => json,
        Err((status, body)) => {
            res.status = status;
            res.body = body;
            res.content_type = "application/json".to_string();
            return None;
        }
    };

    match serde_json::from_str(&json) {
        Ok(payload) => Some(payload),
        Err(error) => {
            res.status = 400;
            res.body = serde_json::json!({
                "ok": false,
                "error": format!("Invalid vendo payload: {error}")
            })
            .to_string()
            .into_bytes();
            res.content_type = "application/json".to_string();
            None
        }
    }
}

fn json_response(
    res: &mut crate::network::server::Response,
    status: u16,
    value: serde_json::Value,
) {
    res.status = status;
    res.body = value.to_string().into_bytes();
    res.content_type = "application/json".to_string();
}

pub fn handle_vendos(server: &mut Server) {
    // List vendos. Raw credentials and stored credential hashes are never returned.
    server.get("/api/admin/vendos", |req, res| {
        if let Err((status, body)) = parse_admin_payload(req) {
            res.status = status;
            res.body = body;
            res.content_type = "application/json".to_string();
            return;
        }

        let vendos = match list_vendos() {
            Ok(vendos) => vendos,
            Err(error) => {
                json_response(res, 500, serde_json::json!({"ok": false, "error": error}));
                return;
            }
        };

        let online_ids = crate::routes::ws::state::list_online_vendo_ids();
        let mut output = Vec::with_capacity(vendos.len());

        for vendo in vendos {
            let credential_active = match get_vendo_auth_verifier(&vendo.vendo_id) {
                Ok(verifier) => verifier.is_some(),
                Err(error) => {
                    json_response(res, 500, serde_json::json!({"ok": false, "error": error}));
                    return;
                }
            };

            output.push(AdminVendo {
                online: online_ids.contains(&vendo.vendo_id),
                vendo_id: vendo.vendo_id,
                name: vendo.name,
                enabled: vendo.enabled,
                credential_active,
                created_at: vendo.created_at,
            });
        }

        json_response(res, 200, serde_json::json!({"ok": true, "vendos": output}));
    });

    // Register a device. The raw credential is returned only in this response.
    server.post("/api/admin/vendos", |req, res| {
        let Some(payload) = parse_payload::<CreateVendoPayload>(req, res) else {
            return;
        };

        match register_vendo(&payload.name) {
            Ok((vendo, credential)) => {
                json_response(
                    res,
                    201,
                    serde_json::json!({
                        "ok": true,
                        "vendo": vendo,
                        "credential": credential,
                        "credential_notice": "Save this credential now. It will not be shown again."
                    }),
                );
            }
            Err(error) => {
                json_response(res, 400, serde_json::json!({"ok": false, "error": error}));
            }
        }
    });

    // Enable or disable one device. Online state is independent of enabled state.
    server.put("/api/admin/vendos", |req, res| {
        let Some(payload) = parse_payload::<SetVendoEnabledPayload>(req, res) else {
            return;
        };

        match set_vendo_enabled(&payload.vendo_id, payload.enabled) {
            Ok(vendo) => {
                json_response(res, 200, serde_json::json!({"ok": true, "vendo": vendo}));
            }
            Err(error) => {
                let status = if error == "Vendo not found" { 404 } else { 500 };
                json_response(res, status, serde_json::json!({"ok": false, "error": error}));
            }
        }
    });

    // Rotate the credential and invalidate the old authenticated socket.
    server.post("/api/admin/vendos/rotate", |req, res| {
        let Some(payload) = parse_payload::<VendoIdPayload>(req, res) else {
            return;
        };

        match rotate_vendo_credential(&payload.vendo_id) {
            Ok(credential) => {
                crate::routes::ws::state::invalidate_vendo_connection(&payload.vendo_id);
                json_response(
                    res,
                    200,
                    serde_json::json!({
                        "ok": true,
                        "vendo_id": payload.vendo_id,
                        "credential": credential,
                        "credential_notice": "Save this credential now. It will not be shown again."
                    }),
                );
            }
            Err(error) => {
                let status = if error == "Vendo credential not found" { 404 } else { 500 };
                json_response(res, status, serde_json::json!({"ok": false, "error": error}));
            }
        }
    });

    // Revoke the credential and invalidate the current authenticated socket.
    server.post("/api/admin/vendos/revoke", |req, res| {
        let Some(payload) = parse_payload::<VendoIdPayload>(req, res) else {
            return;
        };

        match revoke_vendo_credential(&payload.vendo_id) {
            Ok(()) => {
                crate::routes::ws::state::invalidate_vendo_connection(&payload.vendo_id);
                json_response(
                    res,
                    200,
                    serde_json::json!({"ok": true, "vendo_id": payload.vendo_id, "revoked": true}),
                );
            }
            Err(error) => {
                let status = if error == "Vendo credential not found" { 404 } else { 500 };
                json_response(res, status, serde_json::json!({"ok": false, "error": error}));
            }
        }
    });
}
