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
    pub customer_id: Option<String>,
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
            customer_id: None,
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
            self.expired_at = Some(self.last_accounted_at.saturating_add(previous_remaining));
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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Customer {
    pub customer_id: String,
    pub created_at: u64,
    pub active_session_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Vendo {
    pub vendo_id: String,
    pub name: String,
    pub enabled: bool,
    pub created_at: u64,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VendoReservation {
    pub vendo_id: String,
    pub session_id: String,
    pub mac: String,
    pub reserved_at: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VendoCredential {
    pub vendo_id: String,
    pub credential_hash: String,
    pub created_at: u64,
    pub revoked_at: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VendoTransaction {
    pub vendo_id: String,
    pub event_id: String,
    pub sequence: u64,
    pub amount: u32,
    pub pulse_count: u32,
    pub device_timestamp: u64,
    pub received_at: u64,
    pub status: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VendoSequence {
    pub vendo_id: String,
    pub sequence: u64,
    pub event_id: String,
    pub received_at: u64,
}
#[derive(Debug, Clone)]
pub struct VendoPaymentEvent {
    pub vendo_id: String,
    pub event_id: String,
    pub sequence: u64,
    pub amount: u32,
    pub pulse_count: u32,
    pub device_timestamp: u64,
}

#[derive(Debug, Clone)]
pub enum VendoPaymentResult {
    Credited {
        session_id: String,
        session: Session,
        browser_token: Option<String>,
    },
    Duplicate {
        original_event_id: String,
    },
}
pub static DB: OnceLock<Database> = OnceLock::new();

pub const USERS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("users");
pub const RATES_TABLE: TableDefinition<u32, &str> = TableDefinition::new("rates");
pub const VOUCHERS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("vouchers");
pub const CONFIG_TABLE: TableDefinition<&str, &str> = TableDefinition::new("config");
pub const SALES_TABLE: TableDefinition<u32, u32> = TableDefinition::new("sales");

pub const SESSIONS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("sessions");
pub const DEVICE_SESSIONS_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("device_sessions");
pub const BROWSER_TOKENS_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("browser_tokens");
pub const CUSTOMERS_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("customers");

// Multi-vendo registry and payment tables
pub const VENDOS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("vendos");

pub const VENDO_CREDENTIALS_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("vendo_credentials");

pub const VENDO_TRANSACTIONS_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("vendo_transactions");

pub const VENDO_SEQUENCES_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("vendo_sequences");
pub const VENDO_RESERVATIONS_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("vendo_reservations");

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
        let _ = write_txn.open_table(CUSTOMERS_TABLE).unwrap();

        // Multi-vendo tables
        let _ = write_txn.open_table(VENDOS_TABLE).unwrap();
        let _ = write_txn.open_table(VENDO_CREDENTIALS_TABLE).unwrap();
        let _ = write_txn.open_table(VENDO_TRANSACTIONS_TABLE).unwrap();
        let _ = write_txn.open_table(VENDO_SEQUENCES_TABLE).unwrap();
        let _ = write_txn.open_table(VENDO_RESERVATIONS_TABLE).unwrap();
    }
    write_txn.commit().unwrap();

    DB.set(db).expect("Database was already initialized");
}

#[cfg(test)]
pub(crate) fn init_test_db_once() {
    static TEST_DB_INIT: OnceLock<()> = OnceLock::new();

    TEST_DB_INIT.get_or_init(|| {
        let db_path = std::env::temp_dir().join(format!(
            "librefi-tests-{}.redb",
            generate_secure_id().expect("Failed to generate test database ID")
        ));
        init_db(db_path.to_str().expect("Invalid test database path"));
    });
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
// Register a vendo and generate its individual device credential.
// The raw credential is returned once and is never stored in the database.
pub fn register_vendo(name: &str) -> Result<(Vendo, String), String> {
    let name = name.trim();

    if name.is_empty() || name.chars().count() > 64 {
        return Err("Vendo name must contain between 1 and 64 characters".to_string());
    }

    if name.chars().any(|character| character.is_control()) {
        return Err("Vendo name contains invalid characters".to_string());
    }

    let vendo_id = generate_secure_id()?;
    let device_credential = generate_secure_id()?;
    let credential_hash = hash_recovery_token(&device_credential);
    let now = current_timestamp();

    let vendo = Vendo {
        vendo_id: vendo_id.clone(),
        name: name.to_string(),
        enabled: true,
        created_at: now,
    };

    let credential_record = VendoCredential {
        vendo_id: vendo_id.clone(),
        credential_hash,
        created_at: now,
        revoked_at: None,
    };

    let vendo_json = serde_json::to_string(&vendo).map_err(|error| error.to_string())?;

    let credential_json =
        serde_json::to_string(&credential_record).map_err(|error| error.to_string())?;

    let db = get_db();
    let write_txn = db.begin_write().map_err(|error| error.to_string())?;

    {
        let mut vendos = write_txn
            .open_table(VENDOS_TABLE)
            .map_err(|error| error.to_string())?;

        if vendos
            .get(vendo_id.as_str())
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return Err("Generated vendo ID already exists".to_string());
        }

        vendos
            .insert(vendo_id.as_str(), vendo_json.as_str())
            .map_err(|error| error.to_string())?;
    }

    {
        let mut credentials = write_txn
            .open_table(VENDO_CREDENTIALS_TABLE)
            .map_err(|error| error.to_string())?;

        if credentials
            .get(vendo_id.as_str())
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return Err("Vendo credential already exists".to_string());
        }

        credentials
            .insert(vendo_id.as_str(), credential_json.as_str())
            .map_err(|error| error.to_string())?;
    }

    write_txn.commit().map_err(|error| error.to_string())?;

    Ok((vendo, device_credential))
}

fn ensure_customer_for_session(
    write_txn: &redb::WriteTransaction,
    session_id: &str,
    session: &mut Session,
    now: u64,
) -> Result<(), String> {
    let customer_id = match session.customer_id.clone() {
        Some(id) if !id.is_empty() => id,
        _ => generate_secure_id()?,
    };

    let mut customers = write_txn
        .open_table(CUSTOMERS_TABLE)
        .map_err(|e| e.to_string())?;

    let customer = match customers
        .get(customer_id.as_str())
        .map_err(|e| e.to_string())?
    {
        Some(value) => {
            let mut record: Customer =
                serde_json::from_str(value.value()).map_err(|e| e.to_string())?;
            if record.customer_id != customer_id {
                return Err("Customer record ID mismatch".to_string());
            }
            record.active_session_id = session_id.to_string();
            record
        }
        None => Customer {
            customer_id: customer_id.clone(),
            created_at: now,
            active_session_id: session_id.to_string(),
        },
    };

    let customer_json = serde_json::to_string(&customer).map_err(|e| e.to_string())?;
    customers
        .insert(customer_id.as_str(), customer_json.as_str())
        .map_err(|e| e.to_string())?;

    session.customer_id = Some(customer_id);
    Ok(())
}

fn create_session_internal(
    session_id: &str,
    session: &Session,
    mac: Option<&str>,
    browser_token: &str,
) -> Result<(), String> {
    let db = get_db();
    let write_txn = db.begin_write().map_err(|e| e.to_string())?;
    let token_hash = hash_recovery_token(browser_token);
    let mut session_record = session.clone();
    ensure_customer_for_session(&write_txn, session_id, &mut session_record, current_timestamp())?;
    let session_json = serde_json::to_string(&session_record).map_err(|e| e.to_string())?;

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

    if let Some(mac) = mac {
        let mut devices = write_txn
            .open_table(DEVICE_SESSIONS_TABLE)
            .map_err(|e| e.to_string())?;

        if devices.get(mac).map_err(|e| e.to_string())?.is_some() {
            return Err("Device is already linked to a session".to_string());
        }

        devices.insert(mac, session_id).map_err(|e| e.to_string())?;
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

/// Creates a session with a legacy device mapping. New customer identity flows
/// should use create_session_without_device so a MAC address is never identity.
pub fn create_session(
    session_id: &str,
    session: &Session,
    mac: &str,
    browser_token: &str,
) -> Result<(), String> {
    create_session_internal(session_id, session, Some(mac), browser_token)
}

/// Creates a session authenticated only by its browser recovery token.
pub fn create_session_without_device(
    session_id: &str,
    session: &Session,
    browser_token: &str,
) -> Result<(), String> {
    create_session_internal(session_id, session, None, browser_token)
}
fn constant_time_string_eq(left: &str, right: &str) -> bool {
    let left_bytes = left.as_bytes();
    let right_bytes = right.as_bytes();

    if left_bytes.len() != right_bytes.len() {
        return false;
    }

    let difference = left_bytes
        .iter()
        .zip(right_bytes.iter())
        .fold(0u8, |diff, (a, b)| diff | (a ^ b));

    difference == 0
}

pub fn revoke_vendo_credential(vendo_id: &str) -> Result<(), String> {
    let db = get_db();
    let write_txn = db.begin_write().map_err(|e| e.to_string())?;

    let mut credentials = write_txn
        .open_table(VENDO_CREDENTIALS_TABLE)
        .map_err(|e| e.to_string())?;

    let mut record: VendoCredential = match credentials.get(vendo_id).map_err(|e| e.to_string())? {
        Some(value) => serde_json::from_str(value.value()).map_err(|e| e.to_string())?,
        None => return Err("Vendo credential not found".to_string()),
    };

    if record.revoked_at.is_none() {
        record.revoked_at = Some(current_timestamp());

        let record_json = serde_json::to_string(&record).map_err(|e| e.to_string())?;

        credentials
            .insert(vendo_id, record_json.as_str())
            .map_err(|e| e.to_string())?;
    }

    drop(credentials);

    write_txn.commit().map_err(|e| e.to_string())
}
pub fn rotate_vendo_credential(vendo_id: &str) -> Result<String, String> {
    let new_credential = generate_secure_id()?;
    let credential_hash = hash_recovery_token(&new_credential);
    let now = current_timestamp();

    let db = get_db();
    let write_txn = db.begin_write().map_err(|e| e.to_string())?;

    let mut credentials = write_txn
        .open_table(VENDO_CREDENTIALS_TABLE)
        .map_err(|e| e.to_string())?;

    let existing: VendoCredential = match credentials.get(vendo_id).map_err(|e| e.to_string())? {
        Some(value) => serde_json::from_str(value.value()).map_err(|e| e.to_string())?,
        None => return Err("Vendo credential not found".to_string()),
    };

    let replacement = VendoCredential {
        vendo_id: existing.vendo_id,
        credential_hash,
        created_at: now,
        revoked_at: None,
    };

    let replacement_json = serde_json::to_string(&replacement).map_err(|e| e.to_string())?;

    credentials
        .insert(vendo_id, replacement_json.as_str())
        .map_err(|e| e.to_string())?;

    drop(credentials);

    write_txn.commit().map_err(|e| e.to_string())?;

    Ok(new_credential)
}
pub fn get_vendo_auth_verifier(vendo_id: &str) -> Result<Option<[u8; 32]>, String> {
    let db = get_db();
    let read_txn = db.begin_read().map_err(|e| e.to_string())?;

    let credential_record = {
        let table = read_txn
            .open_table(VENDO_CREDENTIALS_TABLE)
            .map_err(|e| e.to_string())?;

        match table.get(vendo_id).map_err(|e| e.to_string())? {
            Some(value) => {
                serde_json::from_str::<VendoCredential>(value.value()).map_err(|e| e.to_string())?
            }
            None => return Ok(None),
        }
    };

    if credential_record.vendo_id != vendo_id || credential_record.revoked_at.is_some() {
        return Ok(None);
    }

    let vendo = {
        let table = read_txn
            .open_table(VENDOS_TABLE)
            .map_err(|e| e.to_string())?;

        match table.get(vendo_id).map_err(|e| e.to_string())? {
            Some(value) => {
                serde_json::from_str::<Vendo>(value.value()).map_err(|e| e.to_string())?
            }
            None => return Ok(None),
        }
    };

    if vendo.vendo_id != vendo_id {
        return Ok(None);
    }

    let hash = credential_record.credential_hash.as_bytes();

    if hash.len() != 64 {
        return Err("Stored vendo credential hash has an invalid length".to_string());
    }

    let mut verifier = [0u8; 32];

    for (index, chunk) in hash.chunks_exact(2).enumerate() {
        let hex = std::str::from_utf8(chunk)
            .map_err(|_| "Stored vendo credential hash is not valid hexadecimal".to_string())?;

        verifier[index] = u8::from_str_radix(hex, 16)
            .map_err(|_| "Stored vendo credential hash is not valid hexadecimal".to_string())?;
    }

    Ok(Some(verifier))
}
pub fn authenticate_vendo(vendo_id: &str, credential: &str) -> Result<Option<Vendo>, String> {
    let db = get_db();
    let read_txn = db.begin_read().map_err(|e| e.to_string())?;

    let credential_record = {
        let table = read_txn
            .open_table(VENDO_CREDENTIALS_TABLE)
            .map_err(|e| e.to_string())?;

        match table.get(vendo_id).map_err(|e| e.to_string())? {
            Some(value) => {
                serde_json::from_str::<VendoCredential>(value.value()).map_err(|e| e.to_string())?
            }
            None => return Ok(None),
        }
    };

    if credential_record.revoked_at.is_some() {
        return Ok(None);
    }

    let supplied_hash = hash_recovery_token(credential);

    if !constant_time_string_eq(&supplied_hash, &credential_record.credential_hash) {
        return Ok(None);
    }

    let table = read_txn
        .open_table(VENDOS_TABLE)
        .map_err(|e| e.to_string())?;

    match table.get(vendo_id).map_err(|e| e.to_string())? {
        Some(value) => {
            let vendo: Vendo = serde_json::from_str(value.value()).map_err(|e| e.to_string())?;

            if vendo.vendo_id != credential_record.vendo_id {
                return Ok(None);
            }

            Ok(Some(vendo))
        }
        None => Ok(None),
    }
}
pub fn get_vendo(vendo_id: &str) -> Result<Option<Vendo>, String> {
    let db = get_db();
    let read_txn = db.begin_read().map_err(|e| e.to_string())?;
    let table = read_txn
        .open_table(VENDOS_TABLE)
        .map_err(|e| e.to_string())?;

    match table.get(vendo_id).map_err(|e| e.to_string())? {
        Some(value) => serde_json::from_str(value.value())
            .map(Some)
            .map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

pub fn list_vendos() -> Result<Vec<Vendo>, String> {
    let db = get_db();
    let read_txn = db.begin_read().map_err(|e| e.to_string())?;
    let table = read_txn
        .open_table(VENDOS_TABLE)
        .map_err(|e| e.to_string())?;

    let mut vendos = Vec::new();

    for entry in table.iter().map_err(|e| e.to_string())? {
        let (_, value) = entry.map_err(|e| e.to_string())?;
        let vendo: Vendo = serde_json::from_str(value.value()).map_err(|e| e.to_string())?;
        vendos.push(vendo);
    }

    Ok(vendos)
}
pub fn list_available_vendos() -> Result<Vec<Vendo>, String> {
    let online_ids = crate::routes::ws::state::list_online_vendo_ids();

    let db = get_db();
    let read_txn = db.begin_read().map_err(|e| e.to_string())?;

    let vendos_table = read_txn
        .open_table(VENDOS_TABLE)
        .map_err(|e| e.to_string())?;

    let reservations_table = read_txn
        .open_table(VENDO_RESERVATIONS_TABLE)
        .map_err(|e| e.to_string())?;

    let mut available = Vec::new();

    for entry in vendos_table.iter().map_err(|e| e.to_string())? {
        let (key, value) = entry.map_err(|e| e.to_string())?;
        let vendo: Vendo =
            serde_json::from_str(value.value()).map_err(|e| e.to_string())?;

        if !vendo.enabled || !online_ids.contains(&vendo.vendo_id) {
            continue;
        }

        if reservations_table
            .get(key.value())
            .map_err(|e| e.to_string())?
            .is_some()
        {
            continue;
        }

        available.push(vendo);
    }

    Ok(available)
}
pub fn reserve_vendo_for_session(
    vendo_id: &str,
    session_id: &str,
    mac: &str,
) -> Result<VendoReservation, String> {
    let result = crate::routes::ws::state::with_vendo_online(vendo_id, || {
        let db = get_db();
        let write_txn = db.begin_write().map_err(|e| e.to_string())?;

        {
            let vendos = write_txn
                .open_table(VENDOS_TABLE)
                .map_err(|e| e.to_string())?;

            let vendo: Vendo = match vendos
                .get(vendo_id)
                .map_err(|e| e.to_string())?
            {
                Some(value) => serde_json::from_str(value.value())
                    .map_err(|e| e.to_string())?,
                None => return Err("Vendo not found".to_string()),
            };

            if !vendo.enabled {
                return Err("Vendo is disabled".to_string());
            }
        }

        if !mac.is_empty() {
            let devices = write_txn
                .open_table(DEVICE_SESSIONS_TABLE)
                .map_err(|e| e.to_string())?;

            match devices.get(mac).map_err(|e| e.to_string())? {
                Some(value) if value.value() == session_id => {}
                _ => return Err("Customer session does not belong to this device".to_string()),
            }
        }

        {
            let sessions = write_txn
                .open_table(SESSIONS_TABLE)
                .map_err(|e| e.to_string())?;

            if sessions
                .get(session_id)
                .map_err(|e| e.to_string())?
                .is_none()
            {
                return Err("Customer session not found".to_string());
            }
        }

        let reservation = VendoReservation {
            vendo_id: vendo_id.to_string(),
            session_id: session_id.to_string(),
            mac: mac.to_string(),
            reserved_at: current_timestamp(),
        };

        {
            let mut reservations = write_txn
                .open_table(VENDO_RESERVATIONS_TABLE)
                .map_err(|e| e.to_string())?;

            if reservations
                .get(vendo_id)
                .map_err(|e| e.to_string())?
                .is_some()
            {
                return Err("Vendo is already reserved".to_string());
            }

            for entry in reservations.iter().map_err(|e| e.to_string())? {
                let (_, value) = entry.map_err(|e| e.to_string())?;
                let existing: VendoReservation =
                    serde_json::from_str(value.value()).map_err(|e| e.to_string())?;
                if existing.session_id == session_id {
                    return Err("Customer session already has a vendo reservation".to_string());
                }
            }

            let reservation_json =
                serde_json::to_string(&reservation).map_err(|e| e.to_string())?;

            reservations
                .insert(vendo_id, reservation_json.as_str())
                .map_err(|e| e.to_string())?;
        }

        write_txn.commit().map_err(|e| e.to_string())?;

        Ok(reservation)
    });

    match result {
        Some(result) => result,
        None => Err("Vendo is offline".to_string()),
    }
}
/// Reserve a vendo for a customer authenticated by its browser token.
/// The session ID is the reservation owner; a client MAC is not required.
pub fn reserve_vendo_for_customer_session(
    vendo_id: &str,
    session_id: &str,
) -> Result<VendoReservation, String> {
    reserve_vendo_for_session(vendo_id, session_id, "")
}

pub fn set_vendo_enabled(vendo_id: &str, enabled: bool) -> Result<Vendo, String> {
    let db = get_db();
    let write_txn = db.begin_write().map_err(|e| e.to_string())?;

    let mut vendos = write_txn
        .open_table(VENDOS_TABLE)
        .map_err(|e| e.to_string())?;

    let mut vendo: Vendo = match vendos.get(vendo_id).map_err(|e| e.to_string())? {
        Some(value) => serde_json::from_str(value.value()).map_err(|e| e.to_string())?,
        None => return Err("Vendo not found".to_string()),
    };

    vendo.enabled = enabled;

    let vendo_json = serde_json::to_string(&vendo).map_err(|e| e.to_string())?;

    vendos
        .insert(vendo_id, vendo_json.as_str())
        .map_err(|e| e.to_string())?;

    drop(vendos);

    write_txn.commit().map_err(|e| e.to_string())?;

    Ok(vendo)
}
pub fn release_vendo_reservation(
    session_id: &str,
    mac: &str,
) -> Result<bool, String> {
    let db = get_db();
    let write_txn = db.begin_write().map_err(|e| e.to_string())?;

    let mut reservations = write_txn
        .open_table(VENDO_RESERVATIONS_TABLE)
        .map_err(|e| e.to_string())?;

    let mut matching_vendo_ids = Vec::new();

    for entry in reservations.iter().map_err(|e| e.to_string())? {
        let (key, value) = entry.map_err(|e| e.to_string())?;
        let reservation: VendoReservation =
            serde_json::from_str(value.value()).map_err(|e| e.to_string())?;

        if reservation.session_id == session_id
            && (mac.is_empty() || reservation.mac == mac)
        {
            matching_vendo_ids.push(key.value().to_string());
        }
    }

    for vendo_id in &matching_vendo_ids {
        reservations
            .remove(vendo_id.as_str())
            .map_err(|e| e.to_string())?;
    }

    drop(reservations);

    write_txn.commit().map_err(|e| e.to_string())?;

    Ok(!matching_vendo_ids.is_empty())
}
/// Release a reservation using the authenticated customer session ID.
pub fn release_vendo_reservation_for_session(session_id: &str) -> Result<bool, String> {
    release_vendo_reservation(session_id, "")
}

pub fn get_vendo_reservation_for_session(
    session_id: &str,
) -> Result<Option<VendoReservation>, String> {
    let db = get_db();
    let read_txn = db.begin_read().map_err(|e| e.to_string())?;
    let reservations = read_txn
        .open_table(VENDO_RESERVATIONS_TABLE)
        .map_err(|e| e.to_string())?;

    for entry in reservations.iter().map_err(|e| e.to_string())? {
        let (_, value) = entry.map_err(|e| e.to_string())?;
        let reservation: VendoReservation =
            serde_json::from_str(value.value()).map_err(|e| e.to_string())?;

        if reservation.session_id == session_id {
            return Ok(Some(reservation));
        }
    }

    Ok(None)
}

/// Looks up the active customer reservation for a specific vendo.
pub fn get_vendo_reservation_for_vendo(
    vendo_id: &str,
) -> Result<Option<VendoReservation>, String> {
    let db = get_db();
    let read_txn = db.begin_read().map_err(|e| e.to_string())?;
    let reservations = read_txn
        .open_table(VENDO_RESERVATIONS_TABLE)
        .map_err(|e| e.to_string())?;

    match reservations.get(vendo_id).map_err(|e| e.to_string())? {
        Some(value) => {
            let reservation: VendoReservation =
                serde_json::from_str(value.value()).map_err(|e| e.to_string())?;
            if reservation.vendo_id != vendo_id {
                return Err("Vendo reservation ID mismatch".to_string());
            }
            Ok(Some(reservation))
        }
        None => Ok(None),
    }
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

        if let Some(existing) = tokens.get(token_hash.as_str()).map_err(|e| e.to_string())? {
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

        if let Some(existing) = devices.get(mac).map_err(|e| e.to_string())? {
            if existing.value() != session_id {
                return Err("Device is already linked to another session".to_string());
            }
        }

        devices.insert(mac, session_id).map_err(|e| e.to_string())?;
    }

    write_txn.commit().map_err(|e| e.to_string())
}
pub fn settle_and_get_session(session_id: &str, now: u64) -> Result<Option<Session>, String> {
    let db = get_db();
    let write_txn = db.begin_write().map_err(|e| e.to_string())?;

    let mut session = {
        let table = write_txn
            .open_table(SESSIONS_TABLE)
            .map_err(|e| e.to_string())?;

        match table.get(session_id).map_err(|e| e.to_string())? {
            Some(value) => {
                serde_json::from_str::<Session>(value.value()).map_err(|e| e.to_string())?
            }
            None => return Ok(None),
        }
    };

    session.settle(now);

    let session_json = serde_json::to_string(&session).map_err(|e| e.to_string())?;

    {
        let mut table = write_txn
            .open_table(SESSIONS_TABLE)
            .map_err(|e| e.to_string())?;

        table
            .insert(session_id, session_json.as_str())
            .map_err(|e| e.to_string())?;
    }

    // Release reservations belonging to an expired session.
    if session.remaining_seconds == 0 {
        let matching_vendo_ids = {
            let reservations = write_txn
                .open_table(VENDO_RESERVATIONS_TABLE)
                .map_err(|e| e.to_string())?;

            reservations
                .iter()
                .map_err(|e| e.to_string())?
                .map(|entry| {
                    let (key, value) = entry.map_err(|e| e.to_string())?;
                    let reservation: VendoReservation =
                        serde_json::from_str(value.value())
                            .map_err(|e| e.to_string())?;

                    Ok(if reservation.session_id == session_id {
                        Some(key.value().to_string())
                    } else {
                        None
                    })
                })
                .collect::<Result<Vec<_>, String>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
        };

        let mut reservations = write_txn
            .open_table(VENDO_RESERVATIONS_TABLE)
            .map_err(|e| e.to_string())?;

        for vendo_id in &matching_vendo_ids {
            reservations
                .remove(vendo_id.as_str())
                .map_err(|e| e.to_string())?;
        }
    }
    write_txn.commit().map_err(|e| e.to_string())?;

    Ok(Some(session))
}
/// Persist customer WebSocket connectivity so voucher recovery can enforce
/// the seven-minute disconnect grace period.
pub fn set_session_wifi_connected(session_id: &str, connected: bool) -> Result<(), String> {
    let db = get_db();
    let write_txn = db.begin_write().map_err(|error| error.to_string())?;
    let now = current_timestamp();

    let mut session = {
        let sessions = write_txn
            .open_table(SESSIONS_TABLE)
            .map_err(|error| error.to_string())?;
        let value = sessions
            .get(session_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Customer session not found".to_string())?;
        serde_json::from_str::<Session>(value.value()).map_err(|error| error.to_string())?
    };

    session.set_wifi_connected(connected, now);
    let session_json = serde_json::to_string(&session).map_err(|error| error.to_string())?;
    {
        let mut sessions = write_txn
            .open_table(SESSIONS_TABLE)
            .map_err(|error| error.to_string())?;
        sessions
            .insert(session_id, session_json.as_str())
            .map_err(|error| error.to_string())?;
    }

    write_txn.commit().map_err(|error| error.to_string())
}

/// A process restart drops all in-memory WebSocket connections. Mark any
/// persisted connected sessions disconnected at startup, beginning a fresh
/// grace window without charging time while the server was offline.
pub fn mark_all_customer_sessions_disconnected() -> Result<(), String> {
    let db = get_db();
    let write_txn = db.begin_write().map_err(|error| error.to_string())?;
    let now = current_timestamp();

    let updates = {
        let sessions = write_txn
            .open_table(SESSIONS_TABLE)
            .map_err(|error| error.to_string())?;
        sessions
            .iter()
            .map_err(|error| error.to_string())?
            .map(|entry| {
                let (key, value) = entry.map_err(|error| error.to_string())?;
                let session: Session =
                    serde_json::from_str(value.value()).map_err(|error| error.to_string())?;
                Ok(if session.wifi_connected {
                    let mut disconnected = session;
                    disconnected.wifi_connected = false;
                    disconnected.disconnected_at = Some(now);
                    disconnected.last_accounted_at = now;
                    Some((key.value().to_string(), disconnected))
                } else {
                    None
                })
            })
            .collect::<Result<Vec<_>, String>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
    };

    {
        let mut sessions = write_txn
            .open_table(SESSIONS_TABLE)
            .map_err(|error| error.to_string())?;
        for (session_id, session) in updates {
            let session_json =
                serde_json::to_string(&session).map_err(|error| error.to_string())?;
            sessions
                .insert(session_id.as_str(), session_json.as_str())
                .map_err(|error| error.to_string())?;
        }
    }

    write_txn.commit().map_err(|error| error.to_string())
}

pub fn credit_session_for_payment(
    session_id: &str,
    mac: &str,
    seconds: u64,
    sale_amount: Option<u32>,
) -> Result<(String, Session, Option<String>), String> {
    let db = get_db();
    let write_txn = db.begin_write().map_err(|e| e.to_string())?;

    let result = credit_session_for_payment_in_transaction(
        &write_txn,
        session_id,
        Some(mac),
        seconds,
        sale_amount,
    )?;

    write_txn.commit().map_err(|e| e.to_string())?;

    Ok(result)
}

fn credit_session_for_payment_in_transaction(
    write_txn: &redb::WriteTransaction,
    session_id: &str,
    mac: Option<&str>,
    seconds: u64,
    sale_amount: Option<u32>,
) -> Result<(String, Session, Option<String>), String> {
    let now = current_timestamp();

    // Retain MAC mapping validation for legacy callers. The new
    // browser-token-authenticated flow does not treat MAC as identity.
    if let Some(mac) = mac {
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
                serde_json::from_str::<Session>(value.value()).map_err(|e| e.to_string())?
            }
            None => return Err("Session not found".to_string()),
        }
    };

    session.settle(now);
    ensure_customer_for_session(&write_txn, session_id, &mut session, now)?;

    let (result_id, result_session, new_browser_token) = if session.expired_at.is_some() {
        let new_session_id = generate_secure_id()?;
        let browser_token = generate_browser_token()?;
        let token_hash = hash_recovery_token(&browser_token);

        let mut new_session = Session::new(session.name.clone(), session.ip.clone(), now);
        new_session.customer_id = session.customer_id.clone();
        ensure_customer_for_session(&write_txn, &new_session_id, &mut new_session, now)?;
        new_session.remaining_seconds = seconds;
        new_session.last_accounted_at = now;

        let old_json = serde_json::to_string(&session).map_err(|e| e.to_string())?;
        let new_json = serde_json::to_string(&new_session).map_err(|e| e.to_string())?;

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

        if let Some(mac) = mac {
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

        (new_session_id, new_session, Some(browser_token))
    } else {
        session.remaining_seconds = session.remaining_seconds.saturating_add(seconds);
        session.last_accounted_at = now;

        let session_json = serde_json::to_string(&session).map_err(|e| e.to_string())?;

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

    Ok((result_id, result_session, new_browser_token))
}
/// Atomically records and credits one vendo payment event using a legacy
/// MAC mapping. Retained for existing callers.
pub fn credit_vendo_payment(
    session_id: &str,
    mac: &str,
    seconds: u64,
    event: VendoPaymentEvent,
) -> Result<VendoPaymentResult, String> {
    credit_vendo_payment_internal(session_id, Some(mac), seconds, event)
}

/// Credits a payment for a browser-token-authenticated customer session.
/// The active reservation binds the vendo to the session; MAC is not used
/// as a customer identity or authorization factor.
pub fn credit_vendo_payment_for_session(
    session_id: &str,
    seconds: u64,
    event: VendoPaymentEvent,
) -> Result<VendoPaymentResult, String> {
    credit_vendo_payment_internal(session_id, None, seconds, event)
}

fn credit_vendo_payment_internal(
    session_id: &str,
    mac: Option<&str>,
    seconds: u64,
    event: VendoPaymentEvent,
) -> Result<VendoPaymentResult, String> {
    if event.vendo_id.trim().is_empty()
        || event.vendo_id.len() > 128
        || event.vendo_id.chars().any(|c| c.is_control())
    {
        return Err("Invalid vendo ID".to_string());
    }

    if event.event_id.trim().is_empty()
        || event.event_id.len() > 128
        || event.event_id.chars().any(|c| c.is_control())
    {
        return Err("Invalid payment event ID".to_string());
    }

    if event.sequence == 0 {
        return Err("Payment sequence must be greater than zero".to_string());
    }

    if event.amount == 0 || event.pulse_count == 0 || seconds == 0 {
        return Err(
            "Payment amount, pulse count, and credited time must be greater than zero".to_string(),
        );
    }

    let db = get_db();
    let write_txn = db.begin_write().map_err(|e| e.to_string())?;

    // Confirm that this vendo exists. Administrative enabled status is
    // intentionally not checked: an existing reservation may finish.
    {
        let vendos = write_txn
            .open_table(VENDOS_TABLE)
            .map_err(|e| e.to_string())?;

        let vendo: Vendo = match vendos
            .get(event.vendo_id.as_str())
            .map_err(|e| e.to_string())?
        {
            Some(value) => serde_json::from_str(value.value()).map_err(|e| e.to_string())?,
            None => return Err("Vendo not found".to_string()),
        };

        if vendo.vendo_id != event.vendo_id {
            return Err("Vendo record ID mismatch".to_string());
        }
    }

    // Length-prefix the vendo ID to avoid ambiguous composite keys.
    let transaction_key = format!(
        "{}:{}{}",
        event.vendo_id.len(),
        event.vendo_id,
        event.event_id
    );

    // Check duplicate event IDs before checking sequence numbers.
    {
        let transactions = write_txn
            .open_table(VENDO_TRANSACTIONS_TABLE)
            .map_err(|e| e.to_string())?;

        if let Some(value) = transactions
            .get(transaction_key.as_str())
            .map_err(|e| e.to_string())?
        {
            let existing: VendoTransaction =
                serde_json::from_str(value.value()).map_err(|e| e.to_string())?;

            if existing.vendo_id == event.vendo_id
                && existing.event_id == event.event_id
                && existing.sequence == event.sequence
                && existing.amount == event.amount
                && existing.pulse_count == event.pulse_count
                && existing.device_timestamp == event.device_timestamp
            {
                return Ok(VendoPaymentResult::Duplicate {
                    original_event_id: existing.event_id,
                });
            }

            return Err("Payment event ID was reused with different payment details".to_string());
        }
    }

    // The payment must belong to the vendo currently reserved by this session.
    {
        let reservations = write_txn
            .open_table(VENDO_RESERVATIONS_TABLE)
            .map_err(|e| e.to_string())?;

        let reservation: VendoReservation = match reservations
            .get(event.vendo_id.as_str())
            .map_err(|e| e.to_string())?
        {
            Some(value) => serde_json::from_str(value.value())
                .map_err(|e| e.to_string())?,
            None => return Err("Vendo has no active reservation".to_string()),
        };

        if reservation.vendo_id != event.vendo_id
            || reservation.session_id != session_id
        {
            return Err("Vendo reservation does not belong to this session".to_string());
        }
    }
    // Enforce strictly sequential events for each vendo.
    {
        let sequences = write_txn
            .open_table(VENDO_SEQUENCES_TABLE)
            .map_err(|e| e.to_string())?;

        let expected_sequence = match sequences
            .get(event.vendo_id.as_str())
            .map_err(|e| e.to_string())?
        {
            Some(value) => {
                let previous: VendoSequence =
                    serde_json::from_str(value.value()).map_err(|e| e.to_string())?;

                if previous.vendo_id != event.vendo_id {
                    return Err("Vendo sequence record ID mismatch".to_string());
                }

                previous
                    .sequence
                    .checked_add(1)
                    .ok_or_else(|| "Vendo sequence number exhausted".to_string())?
            }
            None => 1,
        };

        if event.sequence != expected_sequence {
            return Err(format!(
                "Invalid payment sequence: expected {}, received {}",
                expected_sequence, event.sequence
            ));
        }
    }

    // Credit the existing or replacement session using the shared helper.
    // Any later failure rolls back these changes with the same transaction.
    let (credited_session_id, credited_session, browser_token) =
        credit_session_for_payment_in_transaction(
            &write_txn,
            session_id,
            mac,
            seconds,
            Some(event.amount),
        )?;

    let received_at = current_timestamp();

    let transaction = VendoTransaction {
        vendo_id: event.vendo_id.clone(),
        event_id: event.event_id.clone(),
        sequence: event.sequence,
        amount: event.amount,
        pulse_count: event.pulse_count,
        device_timestamp: event.device_timestamp,
        received_at,
        status: "credited".to_string(),
    };

    let transaction_json = serde_json::to_string(&transaction).map_err(|e| e.to_string())?;

    {
        let mut transactions = write_txn
            .open_table(VENDO_TRANSACTIONS_TABLE)
            .map_err(|e| e.to_string())?;

        transactions
            .insert(transaction_key.as_str(), transaction_json.as_str())
            .map_err(|e| e.to_string())?;
    }

    let sequence_record = VendoSequence {
        vendo_id: event.vendo_id.clone(),
        sequence: event.sequence,
        event_id: event.event_id.clone(),
        received_at,
    };

    let sequence_json = serde_json::to_string(&sequence_record).map_err(|e| e.to_string())?;

    {
        let mut sequences = write_txn
            .open_table(VENDO_SEQUENCES_TABLE)
            .map_err(|e| e.to_string())?;

        sequences
            .insert(event.vendo_id.as_str(), sequence_json.as_str())
            .map_err(|e| e.to_string())?;
    }

    // Session credit, daily sales, transaction, and sequence commit together.
    // Release the reservation after successful payment processing.
    {
        let mut reservations = write_txn
            .open_table(VENDO_RESERVATIONS_TABLE)
            .map_err(|e| e.to_string())?;

        reservations
            .remove(event.vendo_id.as_str())
            .map_err(|e| e.to_string())?;
    }
    write_txn.commit().map_err(|e| e.to_string())?;

    Ok(VendoPaymentResult::Credited {
        session_id: credited_session_id,
        session: credited_session,
        browser_token,
    })
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
            total_seconds = total_seconds.saturating_add(count.saturating_mul(rate.time));

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
    pub customer_id: Option<String>,
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
/// Redeem an unused voucher for the session authenticated by its browser token,
/// or recover an active session with its original used voucher after the
/// seven-minute disconnect grace period. MAC addresses are not identity.
pub fn redeem_voucher_for_customer_session(
    code: &str,
    browser_token: Option<&str>,
) -> Result<(String, Session, bool, Option<String>), String> {
    let db = get_db();
    let write_txn = db.begin_write().map_err(|error| error.to_string())?;
    let now = current_timestamp();

    let mut voucher: Voucher = {
        let vouchers = write_txn
            .open_table(VOUCHERS_TABLE)
            .map_err(|error| error.to_string())?;
        let value = vouchers
            .get(code)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Voucher not found".to_string())?;
        serde_json::from_str(value.value()).map_err(|error| error.to_string())?
    };

    if voucher.used {
        let session_id = voucher
            .session_id
            .clone()
            .ok_or_else(|| "Used voucher has no registered session".to_string())?;

        let mut session = {
            let sessions = write_txn
                .open_table(SESSIONS_TABLE)
                .map_err(|error| error.to_string())?;
            let value = sessions
                .get(session_id.as_str())
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "Original voucher session not found".to_string())?;
            serde_json::from_str::<Session>(value.value())
                .map_err(|error| error.to_string())?
        };

        ensure_customer_for_session(&write_txn, &session_id, &mut session, now)?;
        let voucher_customer_id = voucher
            .customer_id
            .as_deref()
            .ok_or_else(|| "Used voucher has no registered customer identity".to_string())?;
        if session.customer_id.as_deref() != Some(voucher_customer_id) {
            return Err("Voucher belongs to a different customer".to_string());
        }

        session.settle(now);
        if session.expired_at.is_some() {
            return Err("Voucher session expired".to_string());
        }

        let disconnected_at = session
            .disconnected_at
            .ok_or_else(|| "Voucher recovery is not yet eligible".to_string())?;
        if session.wifi_connected
            || now.saturating_sub(disconnected_at) < WIFI_GRACE_SECONDS
        {
            return Err("Voucher recovery is available after the seven-minute disconnect grace period".to_string());
        }

        // The original voucher authorizes one recovery transfer. Revoke every
        // prior bearer token and device key before issuing access to this browser.
        crate::utils::customer_identity::revoke_customer_credentials_in_transaction(
            &write_txn,
            &session_id,
        )?;

        let new_browser_token = generate_browser_token()?;
        let token_hash = hash_recovery_token(&new_browser_token);
        {
            let mut tokens = write_txn
                .open_table(BROWSER_TOKENS_TABLE)
                .map_err(|error| error.to_string())?;
            tokens
                .insert(token_hash.as_str(), session_id.as_str())
                .map_err(|error| error.to_string())?;
        }

        let session_json = serde_json::to_string(&session).map_err(|error| error.to_string())?;
        {
            let mut sessions = write_txn
                .open_table(SESSIONS_TABLE)
                .map_err(|error| error.to_string())?;
            sessions
                .insert(session_id.as_str(), session_json.as_str())
                .map_err(|error| error.to_string())?;
        }

        write_txn.commit().map_err(|error| error.to_string())?;
        return Ok((session_id, session, false, Some(new_browser_token)));
    }

    // An unused voucher is claimed only by the browser token already bound
    // to the customer session. A MAC address cannot claim or select a session.
    let browser_token = browser_token
        .ok_or_else(|| "A valid browser token is required".to_string())?;
    if browser_token.len() != 64
        || !browser_token.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("A valid browser token is required".to_string());
    }
    let token_hash = hash_recovery_token(browser_token);
    let session_id = {
        let tokens = write_txn
            .open_table(BROWSER_TOKENS_TABLE)
            .map_err(|error| error.to_string())?;
        tokens
            .get(token_hash.as_str())
            .map_err(|error| error.to_string())?
            .map(|value| value.value().to_string())
            .ok_or_else(|| "Browser token is not recognized".to_string())?
    };

    let mut session = {
        let sessions = write_txn
            .open_table(SESSIONS_TABLE)
            .map_err(|error| error.to_string())?;
        let value = sessions
            .get(session_id.as_str())
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Customer session not found".to_string())?;
        serde_json::from_str::<Session>(value.value()).map_err(|error| error.to_string())?
    };

    session.settle(now);
    ensure_customer_for_session(&write_txn, &session_id, &mut session, now)?;

    let (credited_session_id, credited_session, new_browser_token) =
        if session.expired_at.is_some() {
            let new_session_id = generate_secure_id()?;
            let new_browser_token = generate_browser_token()?;
            let new_token_hash = hash_recovery_token(&new_browser_token);

            let mut new_session = Session::new(session.name.clone(), session.ip.clone(), now);
            new_session.customer_id = session.customer_id.clone();
            ensure_customer_for_session(
                &write_txn,
                &new_session_id,
                &mut new_session,
                now,
            )?;
            new_session.remaining_seconds = voucher.time as u64;
            new_session.last_accounted_at = now;

            // A replacement session invalidates all old browser and device keys.
            crate::utils::customer_identity::revoke_customer_credentials_in_transaction(
                &write_txn,
                &session_id,
            )?;

            let old_json = serde_json::to_string(&session).map_err(|error| error.to_string())?;
            let new_json =
                serde_json::to_string(&new_session).map_err(|error| error.to_string())?;
            {
                let mut sessions = write_txn
                    .open_table(SESSIONS_TABLE)
                    .map_err(|error| error.to_string())?;
                sessions
                    .insert(session_id.as_str(), old_json.as_str())
                    .map_err(|error| error.to_string())?;
                sessions
                    .insert(new_session_id.as_str(), new_json.as_str())
                    .map_err(|error| error.to_string())?;
            }

            {
                let mut tokens = write_txn
                    .open_table(BROWSER_TOKENS_TABLE)
                    .map_err(|error| error.to_string())?;
                tokens
                    .insert(new_token_hash.as_str(), new_session_id.as_str())
                    .map_err(|error| error.to_string())?;
            }

            // Move an existing coin-vendo reservation with the customer's
            // session identity; it is not tied to a physical MAC address.
            let matching_reservations = {
                let reservations = write_txn
                    .open_table(VENDO_RESERVATIONS_TABLE)
                    .map_err(|error| error.to_string())?;
                reservations
                    .iter()
                    .map_err(|error| error.to_string())?
                    .map(|entry| {
                        let (key, value) = entry.map_err(|error| error.to_string())?;
                        let reservation: VendoReservation =
                            serde_json::from_str(value.value())
                                .map_err(|error| error.to_string())?;
                        Ok(if reservation.session_id == session_id {
                            Some((key.value().to_string(), reservation))
                        } else {
                            None
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
            };
            {
                let mut reservations = write_txn
                    .open_table(VENDO_RESERVATIONS_TABLE)
                    .map_err(|error| error.to_string())?;
                for (vendo_id, mut reservation) in matching_reservations {
                    reservation.session_id = new_session_id.clone();
                    let reservation_json =
                        serde_json::to_string(&reservation).map_err(|error| error.to_string())?;
                    reservations
                        .insert(vendo_id.as_str(), reservation_json.as_str())
                        .map_err(|error| error.to_string())?;
                }
            }

            (new_session_id, new_session, Some(new_browser_token))
        } else {
            session.remaining_seconds = session
                .remaining_seconds
                .saturating_add(voucher.time as u64);
            session.last_accounted_at = now;

            let session_json =
                serde_json::to_string(&session).map_err(|error| error.to_string())?;
            {
                let mut sessions = write_txn
                    .open_table(SESSIONS_TABLE)
                    .map_err(|error| error.to_string())?;
                sessions
                    .insert(session_id.as_str(), session_json.as_str())
                    .map_err(|error| error.to_string())?;
            }

            (session_id.clone(), session, None)
        };

    voucher.used = true;
    voucher.customer_id = credited_session.customer_id.clone();
    voucher.session_id = Some(credited_session_id.clone());
    voucher.redeemed_at = Some(now);

    let voucher_json = serde_json::to_string(&voucher).map_err(|error| error.to_string())?;
    {
        let mut vouchers = write_txn
            .open_table(VOUCHERS_TABLE)
            .map_err(|error| error.to_string())?;
        vouchers
            .insert(code, voucher_json.as_str())
            .map_err(|error| error.to_string())?;
    }

    write_txn.commit().map_err(|error| error.to_string())?;

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
        init_test_db_once();

        let now = current_timestamp();

        // 1. Active coin payment must add time to the existing session.
        let mut active = Session::new("Active User".into(), "10.0.0.10".into(), now);
        active.remaining_seconds = 100;
        active.paused = true;

        create_session(
            "test-active-session",
            &active,
            "test-mac-active",
            "test-token-active",
        )
        .unwrap();

        let (active_id, active_result, active_token) =
            credit_session_for_payment("test-active-session", "test-mac-active", 50, None).unwrap();

        assert_eq!(active_id, "test-active-session");
        assert_eq!(active_result.remaining_seconds, 150);
        assert!(active_token.is_none());

        // 2. An expired session must be replaced by a new session.
        let mut expired = Session::new(
            "Expired User".into(),
            "10.0.0.11".into(),
            now.saturating_sub(100),
        );
        expired.remaining_seconds = 10;
        expired.last_accounted_at = now.saturating_sub(100);

        create_session(
            "test-expired-session",
            &expired,
            "test-mac-expired",
            "test-token-expired",
        )
        .unwrap();

        let expired_customer_id = get_session("test-expired-session")
            .unwrap()
            .unwrap()
            .customer_id
            .expect("Created session must have a customer ID");

        let (new_id, new_session, new_token) =
            credit_session_for_payment("test-expired-session", "test-mac-expired", 300, None)
                .unwrap();

        assert_ne!(new_id, "test-expired-session");
        assert_eq!(new_session.remaining_seconds, 300);
        assert_eq!(new_session.customer_id.as_deref(), Some(expired_customer_id.as_str()));
        {
            let read_txn = get_db().begin_read().unwrap();
            let customers = read_txn.open_table(CUSTOMERS_TABLE).unwrap();
            let record = customers.get(expired_customer_id.as_str()).unwrap().unwrap();
            let customer: Customer = serde_json::from_str(record.value()).unwrap();
            assert_eq!(customer.active_session_id, new_id);
        }
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
        let mut voucher_session = Session::new("Voucher User".into(), "10.0.0.12".into(), now);
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
            customer_id: None,
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

        let voucher_customer_id = credited_session
            .customer_id
            .clone()
            .expect("Voucher session must have a permanent customer ID");
        {
            let read_txn = get_db().begin_read().unwrap();
            let vouchers = read_txn.open_table(VOUCHERS_TABLE).unwrap();
            let record = vouchers.get("TESTVOUCHER1").unwrap().unwrap();
            let stored_voucher: Voucher = serde_json::from_str(record.value()).unwrap();
            assert_eq!(
                stored_voucher.customer_id.as_deref(),
                Some(voucher_customer_id.as_str())
            );

            let customers = read_txn.open_table(CUSTOMERS_TABLE).unwrap();
            let record = customers.get(voucher_customer_id.as_str()).unwrap().unwrap();
            let customer: Customer = serde_json::from_str(record.value()).unwrap();
            assert_eq!(customer.active_session_id, "test-voucher-session");
        }

        // 4. Recovering a used voucher must not credit its time again.
        let (recovered_id, recovered_session, credited_again, recovery_token) =
            redeem_voucher_for_session("TESTVOUCHER1", "test-mac-voucher").unwrap();

        assert_eq!(recovered_id, "test-voucher-session");
        assert_eq!(recovered_session.remaining_seconds, 500);
        assert!(!credited_again);
        assert!(recovery_token.is_none());

        // 5. An unused voucher after expiry must create a new session.
        let mut expired_voucher_session = Session::new(
            "Expired Voucher User".into(),
            "10.0.0.13".into(),
            now.saturating_sub(100),
        );
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
            customer_id: None,
            pdf_download_count: 0,
            created_at: Some(now),
            session_id: None,
            redeemed_at: None,
        };

        let expired_voucher_json = serde_json::to_string(&expired_voucher).unwrap();

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
            redeem_voucher_for_session("TESTVOUCHER2", "test-mac-expired-voucher").unwrap();

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

        // MULTI-VENDO PAYMENT TESTS

        let (vendo_one, vendo_one_credential) = register_vendo("Payment Test Vendo One").unwrap();

        // Reservation requires an authenticated online vendo connection.
        let vendo_connection_generation =
            crate::routes::ws::state::register_vendo_connection(&vendo_one.vendo_id);

        // Registration stores the SHA-256 verifier, not the raw credential.
        let initial_verifier = get_vendo_auth_verifier(&vendo_one.vendo_id)
            .unwrap()
            .expect("Registered vendo should have an authentication verifier");

        let initial_verifier_hex = initial_verifier
            .iter()
            .map(|byte| format!("{:02x}", byte))
            .collect::<String>();

        assert_eq!(
            initial_verifier_hex,
            hash_recovery_token(&vendo_one_credential)
        );

        let mut payment_session = Session::new("Payment Test User".into(), "10.0.0.20".into(), now);
        payment_session.remaining_seconds = 100;
        payment_session.paused = true;

        create_session(
            "test-vendo-payment-session",
            &payment_session,
            "test-vendo-payment-mac",
            "test-vendo-payment-token",
        )
        .unwrap();

        // Reserve this vendo for the payment test session.
        {
            let write_txn = get_db().begin_write().unwrap();
            {
                let mut reservations = write_txn
                    .open_table(VENDO_RESERVATIONS_TABLE)
                    .unwrap();

                let reservation = VendoReservation {
                    vendo_id: vendo_one.vendo_id.clone(),
                    session_id: "test-vendo-payment-session".into(),
                    mac: "test-vendo-payment-mac".into(),
                    reserved_at: now,
                };

                let reservation_json = serde_json::to_string(&reservation).unwrap();

                reservations
                    .insert(vendo_one.vendo_id.as_str(), reservation_json.as_str())
                    .unwrap();
            }
            write_txn.commit().unwrap();
        }
        let event_one = VendoPaymentEvent {
            vendo_id: vendo_one.vendo_id.clone(),
            event_id: "payment-event-1".into(),
            sequence: 1,
            amount: 10,
            pulse_count: 1,
            device_timestamp: 1000,
        };

        // First payment credits the existing session.
        match credit_vendo_payment(
            "test-vendo-payment-session",
            "test-vendo-payment-mac",
            60,
            event_one.clone(),
        )
        .unwrap()
        {
            VendoPaymentResult::Credited { session, .. } => {
                assert_eq!(session.remaining_seconds, 160);
            }
            _ => panic!("First payment should be credited"),
        }

        // A successful payment must release the reservation immediately.
        assert!(
            get_vendo_reservation_for_vendo(&vendo_one.vendo_id)
                .unwrap()
                .is_none(),
            "Successful payment must release the vendo reservation"
        );

        // An identical retry must not credit the session again.
        assert!(matches!(
            credit_vendo_payment(
                "test-vendo-payment-session",
                "test-vendo-payment-mac",
                60,
                event_one.clone(),
            )
            .unwrap(),
            VendoPaymentResult::Duplicate { .. }
        ));

        assert_eq!(
            get_session("test-vendo-payment-session")
                .unwrap()
                .unwrap()
                .remaining_seconds,
            160
        );

        // Reusing the event ID with different details must be rejected.
        let mut conflicting_event = event_one.clone();
        conflicting_event.amount = 11;

        assert!(
            credit_vendo_payment(
                "test-vendo-payment-session",
                "test-vendo-payment-mac",
                60,
                conflicting_event,
            )
            .is_err()
        );

        // Sequence 3 must be rejected because sequence 2 is expected.
        let event_three = VendoPaymentEvent {
            vendo_id: vendo_one.vendo_id.clone(),
            event_id: "payment-event-3".into(),
            sequence: 3,
            amount: 5,
            pulse_count: 1,
            device_timestamp: 3000,
        };

        assert!(
            credit_vendo_payment(
                "test-vendo-payment-session",
                "test-vendo-payment-mac",
                30,
                event_three,
            )
            .is_err()
        );

        // A successful payment releases its reservation. A new payment
        // requires a fresh reservation before it can be accepted.
        set_vendo_enabled(&vendo_one.vendo_id, true).unwrap();
        reserve_vendo_for_session(
            &vendo_one.vendo_id,
            "test-vendo-payment-session",
            "test-vendo-payment-mac",
        )
        .unwrap();

        // Disabling a vendo must not block a payment that is already reserved.
        set_vendo_enabled(&vendo_one.vendo_id, false).unwrap();

        // Disabled vendos retain valid device authentication.
        assert!(
            get_vendo_auth_verifier(&vendo_one.vendo_id)
                .unwrap()
                .is_some()
        );

        let event_two = VendoPaymentEvent {
            vendo_id: vendo_one.vendo_id.clone(),
            event_id: "payment-event-2".into(),
            sequence: 2,
            amount: 5,
            pulse_count: 1,
            device_timestamp: 2000,
        };

        assert!(matches!(
            credit_vendo_payment(
                "test-vendo-payment-session",
                "test-vendo-payment-mac",
                30,
                event_two,
            )
            .unwrap(),
            VendoPaymentResult::Credited { .. }
        ));

        assert_eq!(
            get_session("test-vendo-payment-session")
                .unwrap()
                .unwrap()
                .remaining_seconds,
            190
        );

        // Rotating a credential replaces the previous verifier.
        let rotated_credential = rotate_vendo_credential(&vendo_one.vendo_id).unwrap();

        let rotated_verifier = get_vendo_auth_verifier(&vendo_one.vendo_id)
            .unwrap()
            .expect("Rotated credential should have a verifier");

        let rotated_verifier_hex = rotated_verifier
            .iter()
            .map(|byte| format!("{:02x}", byte))
            .collect::<String>();

        assert_eq!(
            rotated_verifier_hex,
            hash_recovery_token(&rotated_credential)
        );
        assert_ne!(initial_verifier, rotated_verifier);

        // Revocation removes the verifier from authentication use.
        revoke_vendo_credential(&vendo_one.vendo_id).unwrap();

        assert!(
            get_vendo_auth_verifier(&vendo_one.vendo_id)
                .unwrap()
                .is_none()
        );

        assert!(
            crate::routes::ws::state::unregister_vendo_connection(
                &vendo_one.vendo_id,
                vendo_connection_generation,
            )
        );

        // A second vendo starts its own independent sequence at 1.
        let (vendo_two, _) = register_vendo("Payment Test Vendo Two").unwrap();
        let vendo_two_connection_generation =
            crate::routes::ws::state::register_vendo_connection(&vendo_two.vendo_id);

        // Give the second vendo its own independent customer session.
        let mut second_payment_session =
            Session::new("Second Vendo Test User".into(), "10.0.0.21".into(), now);
        second_payment_session.remaining_seconds = 0;

        create_session(
            "test-second-vendo-session",
            &second_payment_session,
            "test-second-vendo-mac",
            "test-second-vendo-token",
        )
        .unwrap();

        // Reserve the second vendo for its own session.
        {
            let write_txn = get_db().begin_write().unwrap();
            {
                let mut reservations = write_txn
                    .open_table(VENDO_RESERVATIONS_TABLE)
                    .unwrap();

                let reservation = VendoReservation {
                    vendo_id: vendo_two.vendo_id.clone(),
                    session_id: "test-second-vendo-session".into(),
                    mac: "test-second-vendo-mac".into(),
                    reserved_at: now,
                };

                let reservation_json = serde_json::to_string(&reservation).unwrap();

                reservations
                    .insert(vendo_two.vendo_id.as_str(), reservation_json.as_str())
                    .unwrap();
            }
            write_txn.commit().unwrap();
        }
        let second_vendo_event_one = VendoPaymentEvent {
            vendo_id: vendo_two.vendo_id.clone(),
            event_id: "second-vendo-event-1".into(),
            sequence: 1,
            amount: 2,
            pulse_count: 1,
            device_timestamp: 4000,
        };

        assert!(matches!(
            credit_vendo_payment_for_session(
                "test-second-vendo-session",
                10,
                second_vendo_event_one,
            )
            .unwrap(),
            VendoPaymentResult::Credited { .. }
        ));

        // The first successful payment released the reservation.
        // Reserve again before accepting the next payment event.
        reserve_vendo_for_session(
            &vendo_two.vendo_id,
            "test-second-vendo-session",
            "test-second-vendo-mac",
        )
        .unwrap();

        // A failed payment must not consume the next sequence number.
        let second_vendo_event_two = VendoPaymentEvent {
            vendo_id: vendo_two.vendo_id.clone(),
            event_id: "second-vendo-event-2".into(),
            sequence: 2,
            amount: 3,
            pulse_count: 1,
            device_timestamp: 5000,
        };

        assert!(
            credit_vendo_payment_for_session(
                "missing-session",
                15,
                second_vendo_event_two.clone(),
            )
            .is_err()
        );

        // A failed payment must leave the reservation intact for retry.
        let retained_reservation =
            get_vendo_reservation_for_vendo(&vendo_two.vendo_id)
                .unwrap()
                .expect("Failed payment must retain the reservation");
        assert_eq!(
            retained_reservation.session_id,
            "test-second-vendo-session"
        );

        // The same event can succeed after the session mapping is corrected.
        assert!(matches!(
            credit_vendo_payment_for_session(
                "test-second-vendo-session",
                15,
                second_vendo_event_two,
            )
            .unwrap(),
            VendoPaymentResult::Credited { .. }
        ));

        assert_eq!(
            get_session("test-second-vendo-session")
                .unwrap()
                .unwrap()
                .remaining_seconds,
            25
        );

        assert!(
            get_vendo_reservation_for_vendo(&vendo_two.vendo_id)
                .unwrap()
                .is_none(),
            "Successful retry must release the reservation"
        );

        assert!(
            crate::routes::ws::state::unregister_vendo_connection(
                &vendo_two.vendo_id,
                vendo_two_connection_generation,
            )
        );
    }

    #[test]
    fn concurrent_reservations_are_exclusive_per_vendo_and_session() {
        init_test_db_once();

        use std::sync::{Arc, Barrier};
        use std::thread;

        let (vendo, _) = register_vendo("Reservation Race Test").unwrap();
        let generation = crate::routes::ws::state::register_vendo_connection(&vendo.vendo_id);

        let session_one = generate_secure_id().unwrap();
        let session_two = generate_secure_id().unwrap();

        let mut first = Session::new("Reservation Customer One".into(), "10.0.0.31".into(), current_timestamp());
        first.remaining_seconds = 120;
        first.paused = true;
        create_session(
            &session_one,
            &first,
            &format!("mac-{}", session_one),
            &format!("token-{}", session_one),
        ).unwrap();

        let mut second = Session::new("Reservation Customer Two".into(), "10.0.0.32".into(), current_timestamp());
        second.remaining_seconds = 120;
        second.paused = true;
        create_session(
            &session_two,
            &second,
            &format!("mac-{}", session_two),
            &format!("token-{}", session_two),
        ).unwrap();

        let barrier = Arc::new(Barrier::new(3));
        let vendo_one = vendo.vendo_id.clone();
        let session_one_for_thread = session_one.clone();
        let barrier_one = Arc::clone(&barrier);
        let first_attempt = thread::spawn(move || {
            barrier_one.wait();
            reserve_vendo_for_customer_session(&vendo_one, &session_one_for_thread)
        });

        let vendo_two = vendo.vendo_id.clone();
        let session_two_for_thread = session_two.clone();
        let barrier_two = Arc::clone(&barrier);
        let second_attempt = thread::spawn(move || {
            barrier_two.wait();
            reserve_vendo_for_customer_session(&vendo_two, &session_two_for_thread)
        });

        barrier.wait();

        let first_result = first_attempt.join().unwrap();
        let second_result = second_attempt.join().unwrap();

        assert_ne!(
            first_result.is_ok(),
            second_result.is_ok(),
            "Exactly one customer must reserve a vendo when requests race"
        );

        let winning_session = if first_result.is_ok() {
            session_one.as_str()
        } else {
            session_two.as_str()
        };

        let stored = get_vendo_reservation_for_vendo(&vendo.vendo_id)
            .unwrap()
            .expect("The winning reservation must be stored");
        assert_eq!(stored.session_id, winning_session);

        // A session that already owns one reservation cannot reserve another vendo.
        let (second_vendo, _) = register_vendo("Second Reservation Race Test").unwrap();
        let second_generation =
            crate::routes::ws::state::register_vendo_connection(&second_vendo.vendo_id);

        assert!(
            reserve_vendo_for_customer_session(&second_vendo.vendo_id, winning_session).is_err(),
            "A customer session must not reserve two vendos"
        );

        assert!(
            get_vendo_reservation_for_vendo(&second_vendo.vendo_id)
                .unwrap()
                .is_none()
        );

        assert!(release_vendo_reservation_for_session(winning_session).unwrap());

        assert!(crate::routes::ws::state::unregister_vendo_connection(
            &vendo.vendo_id,
            generation,
        ));
        assert!(crate::routes::ws::state::unregister_vendo_connection(
            &second_vendo.vendo_id,
            second_generation,
        ));
    }


    #[test]
    fn expired_session_releases_vendo_reservation() {
        init_test_db_once();

        let now = current_timestamp();
        let (vendo, _) = register_vendo("Expired Reservation Cleanup Test").unwrap();
        let generation = crate::routes::ws::state::register_vendo_connection(&vendo.vendo_id);

        let expired_session_id = generate_secure_id().unwrap();
        let mut expired_session =
            Session::new("Expiring Reservation Customer".into(), "10.0.0.41".into(), now);
        expired_session.remaining_seconds = 1;
        expired_session.last_accounted_at = now;
        create_session(
            &expired_session_id,
            &expired_session,
            &format!("mac-{}", expired_session_id),
            &format!("token-{}", expired_session_id),
        )
        .unwrap();

        reserve_vendo_for_customer_session(&vendo.vendo_id, &expired_session_id).unwrap();
        assert!(
            get_vendo_reservation_for_vendo(&vendo.vendo_id).unwrap().is_some(),
            "The reservation must exist before the session expires"
        );

        let settled = settle_and_get_session(&expired_session_id, now + 2)
            .unwrap()
            .expect("Expired session should still be retrievable");
        assert_eq!(settled.remaining_seconds, 0);
        assert!(settled.expired_at.is_some());
        assert!(
            get_vendo_reservation_for_vendo(&vendo.vendo_id).unwrap().is_none(),
            "Session expiry must release its vendo reservation"
        );

        let replacement_session_id = generate_secure_id().unwrap();
        let mut replacement_session =
            Session::new("Replacement Reservation Customer".into(), "10.0.0.42".into(), now + 2);
        replacement_session.remaining_seconds = 120;
        replacement_session.paused = true;
        create_session(
            &replacement_session_id,
            &replacement_session,
            &format!("mac-{}", replacement_session_id),
            &format!("token-{}", replacement_session_id),
        )
        .unwrap();

        reserve_vendo_for_customer_session(&vendo.vendo_id, &replacement_session_id)
            .expect("A different customer must be able to reserve the vendo after expiry");

        let new_reservation = get_vendo_reservation_for_vendo(&vendo.vendo_id)
            .unwrap()
            .expect("Replacement reservation must be stored");
        assert_eq!(new_reservation.session_id, replacement_session_id);

        assert!(release_vendo_reservation_for_session(&replacement_session_id).unwrap());
        assert!(crate::routes::ws::state::unregister_vendo_connection(
            &vendo.vendo_id,
            generation,
        ));
    }


    #[test]
    fn stress_test_ten_vendos_process_payments_concurrently() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        init_test_db_once();

        const VENDO_COUNT: usize = 10;
        let test_id = generate_secure_id().expect("test ID generation should succeed");
        let barrier = Arc::new(Barrier::new(VENDO_COUNT + 1));
        let mut workers = Vec::with_capacity(VENDO_COUNT);
        let mut generations = Vec::with_capacity(VENDO_COUNT);
        let mut expected_sessions = Vec::with_capacity(VENDO_COUNT);

        // Prepare ten independently authenticated, online vendos. Each one
        // owns a different customer session and active reservation.
        for index in 0..VENDO_COUNT {
            let (vendo, _credential) =
                register_vendo(&format!("Stress Vendo {index}"))
                    .expect("vendo registration should succeed");
            let generation =
                crate::routes::ws::state::register_vendo_connection(&vendo.vendo_id);

            let session_id = format!("concurrent-payment-{test_id}-{index}");
            let mac = format!("02:10:00:{:02x}:{:02x}:{:02x}", index / 256, index % 256, index + 1);
            let token = format!("concurrent-token-{test_id}-{index}");
            let mut session = Session::new(
                format!("Concurrent Customer {index}"),
                "127.0.0.1".to_string(),
                current_timestamp(),
            );
            session.remaining_seconds = 100;
            session.paused = true;

            create_session(&session_id, &session, &mac, &token)
                .expect("customer session creation should succeed");
            reserve_vendo_for_customer_session(&vendo.vendo_id, &session_id)
                .expect("each vendo should reserve its customer session");

            let barrier_for_worker = Arc::clone(&barrier);
            let session_for_worker = session_id.clone();
            let vendo_for_worker = vendo.vendo_id.clone();
            let event_id = format!("payment-{test_id}-{index}");

            workers.push(thread::spawn(move || {
                barrier_for_worker.wait();

                credit_vendo_payment_for_session(
                    &session_for_worker,
                    60,
                    VendoPaymentEvent {
                        vendo_id: vendo_for_worker,
                        event_id,
                        sequence: 1,
                        amount: 10,
                        pulse_count: 1,
                        device_timestamp: 1,
                    },
                )
            }));

            generations.push((vendo.vendo_id, generation));
            expected_sessions.push(session_id);
        }

        // Release all ten payment workers together to contend for redb's
        // serialized write transaction and verify every payment completes.
        barrier.wait();

        for (index, worker) in workers.into_iter().enumerate() {
            let result = worker
                .join()
                .expect("payment worker thread should not panic")
                .expect("concurrent payment should be accepted");

            assert!(
                matches!(result, VendoPaymentResult::Credited { .. }),
                "Payment {index} should be credited exactly once"
            );

            let session = get_session(&expected_sessions[index])
                .expect("session lookup should succeed")
                .expect("customer session should still exist");
            assert_eq!(
                session.remaining_seconds, 160,
                "Payment {index} should add 60 seconds to its own session"
            );
        }

        for (vendo_id, generation) in generations {
            assert!(
                crate::routes::ws::state::unregister_vendo_connection(
                    &vendo_id,
                    generation,
                ),
                "Test vendo connection should unregister cleanly"
            );
        }
    }

}
