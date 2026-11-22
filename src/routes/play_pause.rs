use crate::network::server::Server;
use crate::utils::db::{
    current_timestamp, find_session_by_mac, get_db, Session, VendoReservation,
    SESSIONS_TABLE, VENDO_RESERVATIONS_TABLE,
};
use crate::utils::get_mac_from_ip::get_mac_from_ip;
use redb::ReadableTable;

pub fn play_pause(server: &mut Server) {
    server.post("/api/play_pause", |req, res| {
        let Some(mac) = get_mac_from_ip(&req.ip) else {
            res.status = 403;
            res.body = b"{\"error\":\"Device not recognized\"}".to_vec();
            res.content_type = "application/json".to_string();
            return;
        };

        let session_id = match find_session_by_mac(&mac) {
            Ok(Some(id)) => id,
            Ok(None) => {
                res.status = 404;
                res.body = b"{\"error\":\"Session not found\"}".to_vec();
                res.content_type = "application/json".to_string();
                return;
            }
            Err(_) => {
                res.status = 500;
                res.body = b"{\"error\":\"Could not find session\"}".to_vec();
                res.content_type = "application/json".to_string();
                return;
            }
        };

        let now = current_timestamp();
        let db = get_db();
        let write_txn = match db.begin_write() {
            Ok(txn) => txn,
            Err(_) => {
                res.status = 500;
                res.body = b"{\"error\":\"Could not update session\"}".to_vec();
                res.content_type = "application/json".to_string();
                return;
            }
        };

        let mut session: Session = {
            let table = match write_txn.open_table(SESSIONS_TABLE) {
                Ok(table) => table,
                Err(_) => {
                    res.status = 500;
                    res.body = b"{\"error\":\"Could not open session table\"}".to_vec();
                    res.content_type = "application/json".to_string();
                    return;
                }
            };
            match table.get(session_id.as_str()) {
                Ok(Some(value)) => match serde_json::from_str(value.value()) {
                    Ok(value) => value,
                    Err(_) => {
                        res.status = 500;
                        res.body = b"{\"error\":\"Corrupted session record\"}".to_vec();
                        res.content_type = "application/json".to_string();
                        return;
                    }
                },
                Ok(None) => {
                    res.status = 404;
                    res.body = b"{\"error\":\"Session not found\"}".to_vec();
                    res.content_type = "application/json".to_string();
                    return;
                }
                Err(_) => {
                    res.status = 500;
                    res.body = b"{\"error\":\"Could not read session\"}".to_vec();
                    res.content_type = "application/json".to_string();
                    return;
                }
            }
        };

        let paused = !session.paused;
        session.set_paused(paused, now);
        let action = if paused { "pause" } else { "play" };
        let remaining_seconds = session.remaining_seconds;
        let pause_attempts = session.pause_attempts;
        let session_json = match serde_json::to_string(&session) {
            Ok(value) => value,
            Err(_) => {
                res.status = 500;
                res.body = b"{\"error\":\"Could not serialize session\"}".to_vec();
                res.content_type = "application/json".to_string();
                return;
            }
        };
        {
            let mut table = match write_txn.open_table(SESSIONS_TABLE) {
                Ok(table) => table,
                Err(_) => {
                    res.status = 500;
                    res.body = b"{\"error\":\"Could not open session table\"}".to_vec();
                    res.content_type = "application/json".to_string();
                    return;
                }
            };
            if table.insert(session_id.as_str(), session_json.as_str()).is_err() {
                res.status = 500;
                res.body = b"{\"error\":\"Could not save session\"}".to_vec();
                res.content_type = "application/json".to_string();
                return;
            }
        }
        // Release expired session reservation.
        if session.remaining_seconds == 0 {
            let matching_vendo_ids = {
                let reservations = match write_txn.open_table(VENDO_RESERVATIONS_TABLE) {
                    Ok(table) => table,
                    Err(_) => {
                        res.status = 500;
                        res.body = b"{\"error\":\"Could not open reservation table\"}".to_vec();
                        res.content_type = "application/json".to_string();
                        return;
                    }
                };

                match reservations.iter() {
                    Ok(entries) => {
                        let mut ids = Vec::new();

                        for entry in entries {
                            let (key, value) = match entry {
                                Ok(entry) => entry,
                                Err(_) => {
                                    res.status = 500;
                                    res.body = b"{\"error\":\"Could not read reservations\"}".to_vec();
                                    res.content_type = "application/json".to_string();
                                    return;
                                }
                            };

                            let reservation: VendoReservation =
                                match serde_json::from_str(value.value()) {
                                    Ok(value) => value,
                                    Err(_) => {
                                        res.status = 500;
                                        res.body = b"{\"error\":\"Corrupted reservation record\"}".to_vec();
                                        res.content_type = "application/json".to_string();
                                        return;
                                    }
                                };

                            if reservation.session_id == session_id
                                && reservation.mac == mac
                            {
                                ids.push(key.value().to_string());
                            }
                        }

                        ids
                    }
                    Err(_) => {
                        res.status = 500;
                        res.body = b"{\"error\":\"Could not read reservations\"}".to_vec();
                        res.content_type = "application/json".to_string();
                        return;
                    }
                }
            };

            let mut reservations = match write_txn.open_table(VENDO_RESERVATIONS_TABLE) {
                Ok(table) => table,
                Err(_) => {
                    res.status = 500;
                    res.body = b"{\"error\":\"Could not update reservations\"}".to_vec();
                    res.content_type = "application/json".to_string();
                    return;
                }
            };

            for vendo_id in &matching_vendo_ids {
                if reservations.remove(vendo_id.as_str()).is_err() {
                    res.status = 500;
                    res.body = b"{\"error\":\"Could not release reservation\"}".to_vec();
                    res.content_type = "application/json".to_string();
                    return;
                }
            }
        }
        if write_txn.commit().is_err() {
            res.status = 500;
            res.body = b"{\"error\":\"Could not commit session update\"}".to_vec();
            res.content_type = "application/json".to_string();
            return;
        }

        crate::debug_println!(
            "-> [{}] MAC={} paused={} remaining_seconds={} pause_attempts={}",
            action, mac, session.paused, session.remaining_seconds, session.pause_attempts
        );

        #[cfg(not(windows))]
        {
            if session.paused || session.remaining_seconds == 0 {
                let cmd = format!("nft delete element inet librefi allowed_macs {{ {} }}", mac);
                let _ = crate::utils::setup_captive_portal::run_sh_cmd(&cmd, true);
            } else {
                let cmd = format!("nft add element inet librefi allowed_macs {{ {} timeout {}s }}", mac, session.remaining_seconds);
                let _ = crate::utils::setup_captive_portal::run_sh_cmd(&cmd, true);
            }
        }

        res.status = 200;
        res.body = serde_json::json!({
            "action": action,
            "paused": session.paused,
            "pause_attempts": pause_attempts,
            "remaining_seconds": remaining_seconds,
            "now": now,
        }).to_string().into_bytes();
        res.content_type = "application/json".to_string();
    });
}
