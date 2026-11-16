use rand::RngCore;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

pub const WIFI_GRACE_SECONDS: u64 = 7 * 60;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct User {
    pub name: String,
    pub ip: String,
    pub paused: bool,
    pub pause_attempts: u8,
    pub pause_day: u32,
    pub paused_on: u32,
    pub expires_on: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Session {
    pub name: String,
    pub ip: String,
    pub remaining_seconds: u64,
    pub last_accounted_at: u64,
    pub paused: bool,
    pub pause_attempts: u8,
    pub pause_day: u32,
    pub wifi_connected: bool,
    pub disconnected_at: Option<u64>,
    #[serde(default)]
    pub expired_at: Option<u64>,
}

impl Session {
    pub fn new(name: String, ip: String, now: u64) -> Self {
        Self {
            name,
            ip,
            remaining_seconds: 0,
            last_accounted_at: now,
            paused: false,
            pause_attempts: 0,
            pause_day: (now / 86400) as u32,
            wifi_connected: true,
            disconnected_at: None,
            expired_at: None,
        }
    }

    pub fn settle(&mut self, now: u64) {
        if self.paused || self.remaining_seconds == 0 {
            self.last_accounted_at = self.last_accounted_at.max(now);
            return;
        }

        let charge_until = if self.wifi_connected {
            now
        } else if let Some(disconnected_at) = self.disconnected_at {
            now.min(disconnected_at.saturating_add(WIFI_GRACE_SECONDS))
        } else {
            self.last_accounted_at
        };

        let elapsed = charge_until.saturating_sub(self.last_accounted_at);
        let previous_remaining = self.remaining_seconds;

        if elapsed >= previous_remaining {
            self.expired_at = Some(
                self.last_accounted_at.saturating_add(previous_remaining)
            );
        }

        self.remaining_seconds = previous_remaining.saturating_sub(elapsed);
        self.last_accounted_at = self.last_accounted_at.max(charge_until);
    }

    pub fn set_wifi_connected(&mut self, connected: bool, now: u64) {
        self.settle(now);

        if connected {
            self.wifi_connected = true;
            self.disconnected_at = None;
            self.last_accounted_at = now;
        } else if self.wifi_connected {
            self.wifi_connected = false;
            self.disconnected_at = Some(now);
            self.last_accounted_at = now;
        }
    }

    pub fn set_paused(&mut self, paused: bool, now: u64) {
        self.settle(now);
        self.paused = paused;
        self.last_accounted_at = now;
    }

    pub fn status(&self, _now: u64) -> &'static str {
        if self.remaining_seconds == 0 {
            return "expired";
        }

        if self.paused {
            return "paused";
        }

        if !self.wifi_connected {
            return "disconnected";
        }

        "connected"
    }

    pub fn remaining_at(&self, now: u64) -> u64 {
        let mut copy = self.clone();
        copy.settle(now);
        copy.remaining_seconds
    }
}

pub static DB: OnceLock<Database> = OnceLock::new();

pub const USERS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("users");
pub const RATES_TABLE: TableDefinition<u32, &str> = TableDefinition::new("rates");
pub const VOUCHERS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("vouchers");
pub const CONFIG_TABLE: TableDefinition<&str, &str> = TableDefinition::new("config");
pub const SALES_TABLE: TableDefinition<u32, u32> = TableDefinition::new("sales");

pub const SESSIONS_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("sessions");
pub const DEVICE_SESSIONS_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("device_sessions");
pub const BROWSER_TOKENS_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("browser_tokens");

pub fn init_db(db_path: &str) {
    if let Some(parent) = std::path::Path::new(db_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let db = Database::create(db_path).expect("Failed to create database");

    let write_txn = db.begin_write().unwrap();
    {
        let _ = write_txn.open_table(USERS_TABLE).unwrap();
        let _ = write_txn.open_table(RATES_TABLE).unwrap();
        let _ = write_txn.open_table(VOUCHERS_TABLE).unwrap();
        let _ = write_txn.open_table(CONFIG_TABLE).unwrap();
        let _ = write_txn.open_table(SALES_TABLE).unwrap();

        let _ = write_txn.open_table(SESSIONS_TABLE).unwrap();
        let _ = write_txn.open_table(DEVICE_SESSIONS_TABLE).unwrap();
        let _ = write_txn.open_table(BROWSER_TOKENS_TABLE).unwrap();
    }
    write_txn.commit().unwrap();

    DB.set(db).expect("Database was already initialized");
}

pub fn get_db() -> &'static Database {
    DB.get().expect("Database has not been initialized yet!")
}

pub fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn generate_secure_id() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|e| format!("Secure random generation failed: {e}"))?;

    let mut output = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write;
        write!(&mut output, "{:02x}", byte).unwrap();
    }
    Ok(output)
}

pub fn generate_browser_token() -> Result<String, String> {
    generate_secure_id()
}

pub fn hash_recovery_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        write!(&mut output, "{:02x}", byte).unwrap();
    }
    output
}

pub fn create_session(
    session_id: &str,
    session: &Session,
    mac: &str,
    browser_token: &str,
) -> Result<(), String> {
    let db = get_db();
    let write_txn = db.begin_write().map_err(|e| e.to_string())?;
    let token_hash = hash_recovery_token(browser_token);
    let session_json = serde_json::to_string(session).map_err(|e| e.to_string())?;

    {
        let mut sessions = write_txn
            .open_table(SESSIONS_TABLE)
            .map_err(|e| e.to_string())?;

        if sessions
            .get(session_id)
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Err("Session ID already exists".to_string());
        }

        sessions
            .insert(session_id, session_json.as_str())
            .map_err(|e| e.to_string())?;
    }

    {
        let mut devices = write_txn
            .open_table(DEVICE_SESSIONS_TABLE)
            .map_err(|e| e.to_string())?;

        if devices.get(mac).map_err(|e| e.to_string())?.is_some() {
            return Err("Device is already linked to a session".to_string());
        }

        devices
            .insert(mac, session_id)
            .map_err(|e| e.to_string())?;
    }

    {
        let mut tokens = write_txn
            .open_table(BROWSER_TOKENS_TABLE)
            .map_err(|e| e.to_string())?;

        if tokens
            .get(token_hash.as_str())
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Err("Browser token is already linked to a session".to_string());
        }

        tokens
            .insert(token_hash.as_str(), session_id)
            .map_err(|e| e.to_string())?;
    }

    write_txn.commit().map_err(|e| e.to_string())
}
pub fn get_session(session_id: &str) -> Result<Option<Session>, String> {
    let db = get_db();
    let read_txn = db.begin_read().map_err(|e| e.to_string())?;
    let table = read_txn
        .open_table(SESSIONS_TABLE)
        .map_err(|e| e.to_string())?;

    match table.get(session_id).map_err(|e| e.to_string())? {
        Some(value) => serde_json::from_str(value.value())
            .map(Some)
            .map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

pub fn find_session_by_mac(mac: &str) -> Result<Option<String>, String> {
    let db = get_db();
    let read_txn = db.begin_read().map_err(|e| e.to_string())?;
    let table = read_txn
        .open_table(DEVICE_SESSIONS_TABLE)
        .map_err(|e| e.to_string())?;

    table
        .get(mac)
        .map_err(|e| e.to_string())
        .map(|value| value.map(|v| v.value().to_string()))
}

pub fn find_session_by_browser_token(token: &str) -> Result<Option<String>, String> {
    let db = get_db();
    let read_txn = db.begin_read().map_err(|e| e.to_string())?;
    let table = read_txn
        .open_table(BROWSER_TOKENS_TABLE)
        .map_err(|e| e.to_string())?;

    let token_hash = hash_recovery_token(token);
    table
        .get(token_hash.as_str())
        .map_err(|e| e.to_string())
        .map(|value| value.map(|v| v.value().to_string()))
}

pub fn link_device_and_browser_to_session(
    session_id: &str,
    mac: &str,
    browser_token: &str,
) -> Result<(), String> {
    let db = get_db();
    let write_txn = db.begin_write().map_err(|e| e.to_string())?;
    let token_hash = hash_recovery_token(browser_token);

    {
        let sessions = write_txn
            .open_table(SESSIONS_TABLE)
            .map_err(|e| e.to_string())?;

        if sessions
            .get(session_id)
            .map_err(|e| e.to_string())?
            .is_none()
        {
            return Err("Session not found".to_string());
        }
    }

    {
        let mut tokens = write_txn
            .open_table(BROWSER_TOKENS_TABLE)
            .map_err(|e| e.to_string())?;

        if let Some(existing) = tokens
            .get(token_hash.as_str())
            .map_err(|e| e.to_string())?
        {
            if existing.value() != session_id {
                return Err("Browser token already belongs to another session".to_string());
            }
        }

        tokens
            .insert(token_hash.as_str(), session_id)
            .map_err(|e| e.to_string())?;
    }

    {
        let mut devices = write_txn
            .open_table(DEVICE_SESSIONS_TABLE)
            .map_err(|e| e.to_string())?;

        if let Some(existing) = devices
            .get(mac)
            .map_err(|e| e.to_string())?
        {
            if existing.value() != session_id {
                return Err("Device is already linked to another session".to_string());
            }
        }

        devices
            .insert(mac, session_id)
            .map_err(|e| e.to_string())?;
    }

    write_txn.commit().map_err(|e| e.to_string())
}
pub fn settle_and_get_session(
    session_id: &str,
    now: u64,
) -> Result<Option<Session>, String> {
    let db = get_db();
    let write_txn = db.begin_write().map_err(|e| e.to_string())?;

    let mut session = {
        let table = write_txn
            .open_table(SESSIONS_TABLE)
            .map_err(|e| e.to_string())?;

        match table.get(session_id).map_err(|e| e.to_string())? {
            Some(value) => {
                serde_json::from_str::<Session>(value.value())
                    .map_err(|e| e.to_string())?
            }
            None => return Ok(None),
        }
    };

    session.settle(now);

    let session_json =
        serde_json::to_string(&session).map_err(|e| e.to_string())?;

    {
        let mut table = write_txn
            .open_table(SESSIONS_TABLE)
            .map_err(|e| e.to_string())?;

        table
            .insert(session_id, session_json.as_str())
            .map_err(|e| e.to_string())?;
    }

    write_txn.commit().map_err(|e| e.to_string())?;

    Ok(Some(session))
}
pub fn credit_session_for_payment(
    session_id: &str,
    mac: &str,
    seconds: u64,
    sale_amount: Option<u32>,
) -> Result<(String, Session, Option<String>), String> {
    let db = get_db();
    let write_txn = db.begin_write().map_err(|e| e.to_string())?;
    let now = current_timestamp();

    // Abort if this MAC was reassigned before the payment transaction.
    {
        let devices = write_txn
            .open_table(DEVICE_SESSIONS_TABLE)
            .map_err(|e| e.to_string())?;

        match devices.get(mac).map_err(|e| e.to_string())? {
            Some(existing) if existing.value() == session_id => {}
            Some(_) => return Err("Device session changed during coin payment".to_string()),
            None => return Err("Device session mapping not found".to_string()),
        }
    }

    let mut session = {
        let table = write_txn
            .open_table(SESSIONS_TABLE)
            .map_err(|e| e.to_string())?;

        match table.get(session_id).map_err(|e| e.to_string())? {
            Some(value) => {
                serde_json::from_str::<Session>(value.value())
                    .map_err(|e| e.to_string())?
            }
            None => return Err("Session not found".to_string()),
        }
    };

    session.settle(now);

    let (result_id, result_session, new_browser_token) =
        if session.expired_at.is_some() {
            let new_session_id = generate_secure_id()?;
            let browser_token = generate_browser_token()?;
            let token_hash = hash_recovery_token(&browser_token);

            let mut new_session =
                Session::new(session.name.clone(), session.ip.clone(), now);
            new_session.remaining_seconds = seconds;
            new_session.last_accounted_at = now;

            let old_json =
                serde_json::to_string(&session).map_err(|e| e.to_string())?;
            let new_json =
                serde_json::to_string(&new_session).map_err(|e| e.to_string())?;

            {
                let mut sessions = write_txn
                    .open_table(SESSIONS_TABLE)
                    .map_err(|e| e.to_string())?;

                if sessions
                    .get(new_session_id.as_str())
                    .map_err(|e| e.to_string())?
                    .is_some()
                {
                    return Err("Generated session ID already exists".to_string());
                }

                sessions
                    .insert(session_id, old_json.as_str())
                    .map_err(|e| e.to_string())?;

                sessions
                    .insert(new_session_id.as_str(), new_json.as_str())
                    .map_err(|e| e.to_string())?;
            }

            {
                let mut devices = write_txn
                    .open_table(DEVICE_SESSIONS_TABLE)
                    .map_err(|e| e.to_string())?;

                devices
                    .insert(mac, new_session_id.as_str())
                    .map_err(|e| e.to_string())?;
            }

            {
                let mut tokens = write_txn
                    .open_table(BROWSER_TOKENS_TABLE)
                    .map_err(|e| e.to_string())?;

                if tokens
                    .get(token_hash.as_str())
                    .map_err(|e| e.to_string())?
                    .is_some()
                {
                    return Err("Generated browser token already exists".to_string());
                }

                tokens
                    .insert(token_hash.as_str(), new_session_id.as_str())
                    .map_err(|e| e.to_string())?;
            }

            (
                new_session_id,
                new_session,
                Some(browser_token),
            )
        } else {
            session.remaining_seconds =
                session.remaining_seconds.saturating_add(seconds);
            session.last_accounted_at = now;

            let session_json =
                serde_json::to_string(&session).map_err(|e| e.to_string())?;

            {
                let mut sessions = write_txn
                    .open_table(SESSIONS_TABLE)
                    .map_err(|e| e.to_string())?;

                sessions
                    .insert(session_id, session_json.as_str())
                    .map_err(|e| e.to_string())?;
            }

            (session_id.to_string(), session, None)
        };

    if let Some(amount) = sale_amount {
        let local_time = now + 8 * 3600;
        let day = (local_time / 86400) as u32;

        let mut sales = write_txn
            .open_table(SALES_TABLE)
            .map_err(|e| e.to_string())?;

        let current_total = sales
            .get(day)
            .map_err(|e| e.to_string())?
            .map(|v| v.value())
            .unwrap_or(0);

        sales
            .insert(day, current_total.saturating_add(amount))
            .map_err(|e| e.to_string())?;
    }

    write_txn.commit().map_err(|e| e.to_string())?;

    Ok((result_id, result_session, new_browser_token))
}
/// Convert inserted amount to time in seconds using the rates table.
pub fn convert_amount_to_time(amount: u32) -> u32 {
    let db = get_db();

    let read_txn = match db.begin_read() {
        Ok(txn) => txn,
        Err(_) => return 0,
    };

    let table = match read_txn.open_table(RATES_TABLE) {
        Ok(table) => table,
        Err(_) => return 0,
    };

    let iter = match table.iter() {
        Ok(iter) => iter,
        Err(_) => return 0,
    };

    let mut rates = Vec::new();

    for item in iter {
        let (_key, value) = match item {
            Ok(entry) => entry,
            Err(_) => return 0,
        };

        let rate = match serde_json::from_str::<Rate>(value.value()) {
            Ok(rate) => rate,
            Err(_) => continue,
        };

        if rate.price.is_finite() && rate.price > 0.0 && rate.time > 0 {
            rates.push(rate);
        }
    }

    rates.sort_by(|a, b| {
        b.price
            .partial_cmp(&a.price)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut remaining = amount as f64;
    let mut total_seconds = 0u32;

    for rate in rates {
        let count = (remaining / rate.price).floor() as u32;

        if count > 0 {
            total_seconds = total_seconds.saturating_add(
                count.saturating_mul(rate.time)
            );

            remaining -= (count as f64) * rate.price;
        }
    }

    total_seconds
}
/// Legacy helper retained until all existing routes are migrated.
pub fn add_time_to_user(mac: &str, seconds: u32) -> Result<User, String> {
    let db = get_db();
    let write_txn = db.begin_write().map_err(|e| e.to_string())?;

    let mut user = {
        let table = write_txn
            .open_table(USERS_TABLE)
            .map_err(|e| e.to_string())?;

        match table.get(mac).map_err(|e| e.to_string())? {
            Some(value) => {
                serde_json::from_str::<User>(value.value()).map_err(|e| e.to_string())?
            }
            None => return Err("User not found".to_string()),
        }
    };

    let now = current_timestamp() as u32;
    if user.expires_on < now {
        user.expires_on = now;
    }
    user.expires_on = user.expires_on.saturating_add(seconds);

    let json = serde_json::to_string(&user).map_err(|e| e.to_string())?;
    {
        let mut table = write_txn
            .open_table(USERS_TABLE)
            .map_err(|e| e.to_string())?;
        table
            .insert(mac, json.as_str())
            .map_err(|e| e.to_string())?;
    }

    write_txn.commit().map_err(|e| e.to_string())?;
    Ok(user)
}

pub fn record_sale(amount: u32) {
    let now = current_timestamp();
    let local_time = now + 8 * 3600;
    let day = (local_time / 86400) as u32;

    let db = get_db();
    if let Ok(write_txn) = db.begin_write() {
        if let Ok(mut table) = write_txn.open_table(SALES_TABLE) {
            let current_total = match table.get(day) {
                Ok(Some(v)) => v.value(),
                _ => 0,
            };
            let _ = table.insert(day, current_total.saturating_add(amount));
        }
        let _ = write_txn.commit();
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Rate {
    pub price: f64,
    pub time: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Voucher {
    pub code: String,
    pub price: f64,
    pub time: u32,
    pub used: bool,
    #[serde(default)]
    pub pdf_download_count: u32,
    #[serde(default)]
    pub created_at: Option<u64>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub redeemed_at: Option<u64>,
}

/// Redeem a voucher once, or recover the session linked to an already-used voucher.
/// Voucher crediting and voucher association are committed in one transaction.
pub fn redeem_voucher_for_session(
    code: &str,
    mac: &str,
) -> Result<(String, Session, bool, Option<String>), String> {
    let db = get_db();
    let write_txn = db.begin_write().map_err(|e| e.to_string())?;
    let now = current_timestamp();

    let mut voucher: Voucher = {
        let table = write_txn
            .open_table(VOUCHERS_TABLE)
            .map_err(|e| e.to_string())?;

        let value = table
            .get(code)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Voucher not found".to_string())?;

        serde_json::from_str(value.value()).map_err(|e| e.to_string())?
    };

    // A used voucher is a recovery credential, never a second time credit.
    if voucher.used {
        let session_id = voucher
            .session_id
            .clone()
            .ok_or_else(|| "Voucher was used before session recovery was available".to_string())?;

        let mut session = {
            let table = write_txn
                .open_table(SESSIONS_TABLE)
                .map_err(|e| e.to_string())?;

            let value = table
                .get(session_id.as_str())
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "Original voucher session not found".to_string())?;

            serde_json::from_str::<Session>(value.value())
                .map_err(|e| e.to_string())?
        };

        // Settle the original session before deciding whether recovery is allowed.
        session.settle(now);

        if session.expired_at.is_some() {
            return Err("Voucher session expired".to_string());
        }

        // Never replace a MAC mapping that now belongs to another session.
        {
            let mut devices = write_txn
                .open_table(DEVICE_SESSIONS_TABLE)
                .map_err(|e| e.to_string())?;

            if let Some(existing) = devices
                .get(mac)
                .map_err(|e| e.to_string())?
            {
                if existing.value() != session_id {
                    return Err("This device is already linked to another session".to_string());
                }
            } else {
                devices
                    .insert(mac, session_id.as_str())
                    .map_err(|e| e.to_string())?;
            }
        }

        let session_json =
            serde_json::to_string(&session).map_err(|e| e.to_string())?;

        {
            let mut sessions = write_txn
                .open_table(SESSIONS_TABLE)
                .map_err(|e| e.to_string())?;

            sessions
                .insert(session_id.as_str(), session_json.as_str())
                .map_err(|e| e.to_string())?;
        }

        write_txn.commit().map_err(|e| e.to_string())?;
        return Ok((session_id, session, false, None));
    }

    // An unused voucher can only credit the session currently associated with this MAC.
    let session_id = {
        let devices = write_txn
            .open_table(DEVICE_SESSIONS_TABLE)
            .map_err(|e| e.to_string())?;

        devices
            .get(mac)
            .map_err(|e| e.to_string())?
            .map(|value| value.value().to_string())
            .ok_or_else(|| "No session found for this device".to_string())?
    };

    let mut session = {
        let table = write_txn
            .open_table(SESSIONS_TABLE)
            .map_err(|e| e.to_string())?;

        let value = table
            .get(session_id.as_str())
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Session not found".to_string())?;

        serde_json::from_str::<Session>(value.value())
            .map_err(|e| e.to_string())?
    };

    session.settle(now);

    let (credited_session_id, credited_session, new_browser_token) =
        if session.expired_at.is_some() {
            // Preserve the expired session and start a completely new session.
            let new_session_id = generate_secure_id()?;
            let browser_token = generate_browser_token()?;
            let token_hash = hash_recovery_token(&browser_token);

            let mut new_session =
                Session::new(session.name.clone(), session.ip.clone(), now);
            new_session.remaining_seconds = voucher.time as u64;
            new_session.last_accounted_at = now;

            let old_json =
                serde_json::to_string(&session).map_err(|e| e.to_string())?;
            let new_json =
                serde_json::to_string(&new_session).map_err(|e| e.to_string())?;

            {
                let mut sessions = write_txn
                    .open_table(SESSIONS_TABLE)
                    .map_err(|e| e.to_string())?;

                if sessions
                    .get(new_session_id.as_str())
                    .map_err(|e| e.to_string())?
                    .is_some()
                {
                    return Err("Generated session ID already exists".to_string());
                }

                sessions
                    .insert(session_id.as_str(), old_json.as_str())
                    .map_err(|e| e.to_string())?;

                sessions
                    .insert(new_session_id.as_str(), new_json.as_str())
                    .map_err(|e| e.to_string())?;
            }

            {
                let mut devices = write_txn
                    .open_table(DEVICE_SESSIONS_TABLE)
                    .map_err(|e| e.to_string())?;

                devices
                    .insert(mac, new_session_id.as_str())
                    .map_err(|e| e.to_string())?;
            }

            {
                let mut tokens = write_txn
                    .open_table(BROWSER_TOKENS_TABLE)
                    .map_err(|e| e.to_string())?;

                if tokens
                    .get(token_hash.as_str())
                    .map_err(|e| e.to_string())?
                    .is_some()
                {
                    return Err("Generated browser token already exists".to_string());
                }

                tokens
                    .insert(token_hash.as_str(), new_session_id.as_str())
                    .map_err(|e| e.to_string())?;
            }

            (
                new_session_id,
                new_session,
                Some(browser_token),
            )
        } else {
            // Active sessions and fresh zero-time sessions are funded in place.
            session.remaining_seconds = session
                .remaining_seconds
                .saturating_add(voucher.time as u64);
            session.last_accounted_at = now;

            let session_json =
                serde_json::to_string(&session).map_err(|e| e.to_string())?;

            {
                let mut sessions = write_txn
                    .open_table(SESSIONS_TABLE)
                    .map_err(|e| e.to_string())?;

                sessions
                    .insert(session_id.as_str(), session_json.as_str())
                    .map_err(|e| e.to_string())?;
            }

            (session_id.clone(), session, None)
        };

    // Associate the voucher with the session that actually received its time.
    voucher.used = true;
    voucher.session_id = Some(credited_session_id.clone());
    voucher.redeemed_at = Some(now);

    let voucher_json =
        serde_json::to_string(&voucher).map_err(|e| e.to_string())?;

    {
        let mut vouchers = write_txn
            .open_table(VOUCHERS_TABLE)
            .map_err(|e| e.to_string())?;

        vouchers
            .insert(code, voucher_json.as_str())
            .map_err(|e| e.to_string())?;
    }

    write_txn.commit().map_err(|e| e.to_string())?;

    Ok((
        credited_session_id,
        credited_session,
        true,
        new_browser_token,
    ))
}
#[cfg(test)]
mod session_logic_tests {
    use super::*;

    #[test]
    fn test_coin_and_voucher_session_lifecycle() {
        let db_path = std::env::temp_dir().join(format!(
            "librefi-session-logic-{}.redb",
            generate_secure_id().unwrap()
        ));

        init_db(db_path.to_str().unwrap());

        let now = current_timestamp();

        // 1. Active coin payment must add time to the existing session.
        let mut active = Session::new("Active User".into(), "10.0.0.10".into(), now);
        active.remaining_seconds = 100;
        active.paused = true;

        create_session("test-active-session", &active, "test-mac-active", "test-token-active")
            .unwrap();

        let (active_id, active_result, active_token) =
            credit_session_for_payment("test-active-session", "test-mac-active", 50, None)
                .unwrap();

        assert_eq!(active_id, "test-active-session");
        assert_eq!(active_result.remaining_seconds, 150);
        assert!(active_token.is_none());

        // 2. An expired session must be replaced by a new session.
        let mut expired =
            Session::new("Expired User".into(), "10.0.0.11".into(), now.saturating_sub(100));
        expired.remaining_seconds = 10;
        expired.last_accounted_at = now.saturating_sub(100);

        create_session(
            "test-expired-session",
            &expired,
            "test-mac-expired",
            "test-token-expired",
        )
        .unwrap();

        let (new_id, new_session, new_token) =
            credit_session_for_payment("test-expired-session", "test-mac-expired", 300, None)
                .unwrap();

        assert_ne!(new_id, "test-expired-session");
        assert_eq!(new_session.remaining_seconds, 300);
        assert!(new_token.is_some());
        assert_eq!(
            find_session_by_mac("test-mac-expired").unwrap(),
            Some(new_id.clone())
        );
        assert!(
            get_session("test-expired-session")
                .unwrap()
                .unwrap()
                .expired_at
                .is_some()
        );

        // 3. An unused voucher must add time to the existing active session.
        let mut voucher_session =
            Session::new("Voucher User".into(), "10.0.0.12".into(), now);
        voucher_session.remaining_seconds = 200;
        voucher_session.paused = true;

        create_session(
            "test-voucher-session",
            &voucher_session,
            "test-mac-voucher",
            "test-token-voucher",
        )
        .unwrap();

        let voucher = Voucher {
            code: "TESTVOUCHER1".into(),
            price: 10.0,
            time: 300,
            used: false,
            pdf_download_count: 0,
            created_at: Some(now),
            session_id: None,
            redeemed_at: None,
        };

        let voucher_json = serde_json::to_string(&voucher).unwrap();
        {
            let write_txn = get_db().begin_write().unwrap();
            {
                let mut table = write_txn.open_table(VOUCHERS_TABLE).unwrap();
                table.insert("TESTVOUCHER1", voucher_json.as_str()).unwrap();
            }
            write_txn.commit().unwrap();
        }

        let (voucher_session_id, credited_session, credited, voucher_token) =
            redeem_voucher_for_session("TESTVOUCHER1", "test-mac-voucher").unwrap();

        assert_eq!(voucher_session_id, "test-voucher-session");
        assert_eq!(credited_session.remaining_seconds, 500);
        assert!(credited);

        // 4. Recovering a used voucher must not credit its time again.
        let (recovered_id, recovered_session, credited_again, recovery_token) =
            redeem_voucher_for_session("TESTVOUCHER1", "test-mac-voucher").unwrap();

        assert_eq!(recovered_id, "test-voucher-session");
        assert_eq!(recovered_session.remaining_seconds, 500);
        assert!(!credited_again);
        assert!(recovery_token.is_none());

        // 5. An unused voucher after expiry must create a new session.
        let mut expired_voucher_session =
            Session::new("Expired Voucher User".into(), "10.0.0.13".into(), now.saturating_sub(100));
        expired_voucher_session.remaining_seconds = 10;
        expired_voucher_session.last_accounted_at = now.saturating_sub(100);

        create_session(
            "test-expired-voucher-session",
            &expired_voucher_session,
            "test-mac-expired-voucher",
            "test-token-expired-voucher",
        )
        .unwrap();

        let expired_voucher = Voucher {
            code: "TESTVOUCHER2".into(),
            price: 10.0,
            time: 600,
            used: false,
            pdf_download_count: 0,
            created_at: Some(now),
            session_id: None,
            redeemed_at: None,
        };

        let expired_voucher_json =
            serde_json::to_string(&expired_voucher).unwrap();

        {
            let write_txn = get_db().begin_write().unwrap();
            {
                let mut table = write_txn.open_table(VOUCHERS_TABLE).unwrap();
                table
                    .insert("TESTVOUCHER2", expired_voucher_json.as_str())
                    .unwrap();
            }
            write_txn.commit().unwrap();
        }

        let (replacement_id, replacement_session, replacement_credited, replacement_token) =
            redeem_voucher_for_session(
                "TESTVOUCHER2",
                "test-mac-expired-voucher",
            )
            .unwrap();

        assert_ne!(replacement_id, "test-expired-voucher-session");
        assert_eq!(replacement_session.remaining_seconds, 600);
        assert!(replacement_credited);

        let replacement_token = replacement_token.expect("Expected a new browser token");
        assert_eq!(
            find_session_by_browser_token(&replacement_token).unwrap(),
            Some(replacement_id.clone())
        );
        assert_eq!(
            find_session_by_mac("test-mac-expired-voucher").unwrap(),
            Some(replacement_id)
        );
        assert!(
            get_session("test-expired-voucher-session")
                .unwrap()
                .unwrap()
                .expired_at
                .is_some()
        );
        assert!(voucher_token.is_none());
    }
}
