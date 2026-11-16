use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

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
