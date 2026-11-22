use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

static NEXT_CUSTOMER_CONNECTION_ID: AtomicU64 = AtomicU64::new(1);
static CUSTOMER_CONNECTIONS: LazyLock<
    Mutex<HashMap<String, HashMap<u64, UnboundedSender<serde_json::Value>>>>,
> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Registers one browser WebSocket for a customer session.
/// Multiple tabs may be connected to the same session simultaneously.
pub fn register_customer_connection(
    session_id: &str,
    sender: UnboundedSender<serde_json::Value>,
) -> u64 {
    let connection_id = NEXT_CUSTOMER_CONNECTION_ID.fetch_add(1, Ordering::Relaxed);
    CUSTOMER_CONNECTIONS
        .lock()
        .unwrap()
        .entry(session_id.to_string())
        .or_default()
        .insert(connection_id, sender);
    connection_id
}

/// Removes only the specified browser connection.
pub fn unregister_customer_connection(session_id: &str, connection_id: u64) {
    let mut connections = CUSTOMER_CONNECTIONS.lock().unwrap();
    if let Some(session_connections) = connections.get_mut(session_id) {
        session_connections.remove(&connection_id);
        if session_connections.is_empty() {
            connections.remove(session_id);
        }
    }
}

/// Sends a payment/session event to every connected tab for this session.
pub fn notify_customer_session(session_id: &str, message: serde_json::Value) {
    let mut connections = CUSTOMER_CONNECTIONS.lock().unwrap();
    if let Some(session_connections) = connections.get_mut(session_id) {
        session_connections.retain(|_, sender| sender.send(message.clone()).is_ok());
        if session_connections.is_empty() {
            connections.remove(session_id);
        }
    }
}

pub enum RelayResult {
    Available,
    NotAvailable(String),
    Timeout,
}

pub struct RelayRequest {
    pub reply_tx: Option<mpsc::SyncSender<RelayResult>>,
    pub command: String,
}

pub type CoinslotSender = mpsc::SyncSender<RelayRequest>;

static COINSLOT_SENDERS: OnceLock<Mutex<HashMap<String, CoinslotSender>>> = OnceLock::new();

pub fn get_coinslot_senders() -> &'static Mutex<HashMap<String, CoinslotSender>> {
    COINSLOT_SENDERS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub static USER_EVENTS: LazyLock<Mutex<HashMap<String, Vec<WsMessage>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub const ACTIVE_USER_LEASE: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct ActiveUser {
    pub mac: String,
    pub owner_id: String,
    pub browser_token: Option<String>,
    pub expires_at: Instant,
}

pub static ACTIVE_USER: LazyLock<Mutex<HashMap<String, ActiveUser>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static NEXT_OWNER_ID: AtomicU64 = AtomicU64::new(1);

pub fn new_owner_id() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        NEXT_OWNER_ID.fetch_add(1, Ordering::Relaxed)
    )
}

pub fn reserve_active_user(subnet: &str, mac: &str, owner_id: &str, browser_token: Option<String>) -> bool {
    let mut active_map = ACTIVE_USER.lock().unwrap();
    let now = Instant::now();

    if let Some(active) = active_map.get(subnet) {
        if active.expires_at > now && active.owner_id != owner_id {
            return false;
        }
    }

    active_map.insert(
        subnet.to_string(),
        ActiveUser {
            mac: mac.to_string(),
            owner_id: owner_id.to_string(),
            browser_token,
            expires_at: now + ACTIVE_USER_LEASE,
        },
    );

    true
}

pub fn refresh_active_user(subnet: &str, owner_id: &str) -> bool {
    let mut active_map = ACTIVE_USER.lock().unwrap();

    if let Some(active) = active_map.get_mut(subnet) {
        if active.owner_id == owner_id && active.expires_at > Instant::now() {
            active.expires_at = Instant::now() + ACTIVE_USER_LEASE;
            return true;
        }
    }

    false
}

pub fn update_active_user_browser_token(
    subnet: &str,
    owner_id: &str,
    expected_token: Option<&str>,
    new_token: String,
) -> bool {
    let mut active_map = ACTIVE_USER.lock().unwrap();

    if let Some(active) = active_map.get_mut(subnet) {
        if active.owner_id == owner_id
            && active.expires_at > Instant::now()
            && active.browser_token.as_deref() == expected_token
        {
            active.browser_token = Some(new_token);
            return true;
        }
    }

    false
}
pub fn release_active_user(subnet: &str, owner_id: &str) -> bool {
    let mut active_map = ACTIVE_USER.lock().unwrap();

    if active_map
        .get(subnet)
        .is_some_and(|active| active.owner_id == owner_id)
    {
        active_map.remove(subnet);
        return true;
    }

    false
}

pub fn get_active_user(subnet: &str) -> Option<ActiveUser> {
    let active_map = ACTIVE_USER.lock().unwrap();

    active_map
        .get(subnet)
        .filter(|active| active.expires_at > Instant::now())
        .cloned()
}

pub fn expire_active_user(subnet: &str) -> bool {
    let mut active_map = ACTIVE_USER.lock().unwrap();

    if active_map
        .get(subnet)
        .is_some_and(|active| active.expires_at <= Instant::now())
    {
        active_map.remove(subnet);
        return true;
    }

    false
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct WsMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_token: Option<String>,
}

#[derive(Deserialize)]
pub struct EncryptedPayload {
    pub payload: String,
}

pub fn get_subnet(ip: &str) -> String {
    let parts: Vec<&str> = ip.split('.').collect();
    if parts.len() == 4 {
        format!("{}.{}.{}", parts[0], parts[1], parts[2])
    } else {
        ip.to_string()
    }
}

// Multi-vendo device connection tracking.

#[derive(Clone, Debug)]
pub struct VendoConnection {
    pub generation: u64,
    pub connected_at: Instant,
}

static VENDO_CONNECTIONS: LazyLock<Mutex<HashMap<String, VendoConnection>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static NEXT_VENDO_CONNECTION_GENERATION: AtomicU64 = AtomicU64::new(1);

/// Registers a new authenticated device connection and returns its generation.
pub fn register_vendo_connection(vendo_id: &str) -> u64 {
    let generation = NEXT_VENDO_CONNECTION_GENERATION.fetch_add(1, Ordering::Relaxed);

    VENDO_CONNECTIONS.lock().unwrap().insert(
        vendo_id.to_string(),
        VendoConnection {
            generation,
            connected_at: Instant::now(),
        },
    );

    generation
}

/// Returns true only if this generation is still the current connection.
pub fn is_current_vendo_connection(vendo_id: &str, generation: u64) -> bool {
    VENDO_CONNECTIONS
        .lock()
        .unwrap()
        .get(vendo_id)
        .is_some_and(|connection| connection.generation == generation)
}

/// Removes a device only when the supplied generation is still current.
pub fn unregister_vendo_connection(vendo_id: &str, generation: u64) -> bool {
    let mut connections = VENDO_CONNECTIONS.lock().unwrap();

    if connections
        .get(vendo_id)
        .is_some_and(|connection| connection.generation == generation)
    {
        connections.remove(vendo_id);
        return true;
    }

    false
}

/// Checks whether a vendo currently has an authenticated WebSocket connection.
pub fn is_vendo_online(vendo_id: &str) -> bool {
    VENDO_CONNECTIONS.lock().unwrap().contains_key(vendo_id)
}
// Runs an operation while the vendo's authenticated connection remains locked.
pub fn with_vendo_online<T>(
    vendo_id: &str,
    operation: impl FnOnce() -> T,
) -> Option<T> {
    let connections = VENDO_CONNECTIONS.lock().unwrap();

    if !connections.contains_key(vendo_id) {
        return None;
    }

    Some(operation())
}

/// Returns a snapshot of currently connected vendo IDs.
pub fn list_online_vendo_ids() -> Vec<String> {
    VENDO_CONNECTIONS.lock().unwrap().keys().cloned().collect()
}

#[cfg(test)]
mod vendo_connection_tests {
    use super::*;

    #[test]
    fn reconnect_replaces_old_generation_safely() {
        let vendo_id = format!("test-vendo-{}", new_owner_id());

        let first = register_vendo_connection(&vendo_id);
        assert!(is_vendo_online(&vendo_id));
        assert!(is_current_vendo_connection(&vendo_id, first));

        let second = register_vendo_connection(&vendo_id);
        assert_ne!(first, second);
        assert!(!is_current_vendo_connection(&vendo_id, first));
        assert!(is_current_vendo_connection(&vendo_id, second));

        assert!(!unregister_vendo_connection(&vendo_id, first));
        assert!(is_vendo_online(&vendo_id));
        assert!(is_current_vendo_connection(&vendo_id, second));

        assert!(unregister_vendo_connection(&vendo_id, second));
        assert!(!is_vendo_online(&vendo_id));
    }
}
