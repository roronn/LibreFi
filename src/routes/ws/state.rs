use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Instant;
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
) -> (u64, bool) {
    let connection_id = NEXT_CUSTOMER_CONNECTION_ID.fetch_add(1, Ordering::Relaxed);
    let mut connections = CUSTOMER_CONNECTIONS.lock().unwrap();
    let session_connections = connections.entry(session_id.to_string()).or_default();
    let first_connection = session_connections.is_empty();
    session_connections.insert(connection_id, sender);
    (connection_id, first_connection)
}

/// Removes only the specified browser connection.
pub fn unregister_customer_connection(session_id: &str, connection_id: u64) -> bool {
    let mut connections = CUSTOMER_CONNECTIONS.lock().unwrap();
    if let Some(session_connections) = connections.get_mut(session_id) {
        session_connections.remove(&connection_id);
        if session_connections.is_empty() {
            connections.remove(session_id);
            return true;
        }
    }
    false
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

// Invalidates the current authenticated connection. The handler's generation
// check rejects subsequent messages from the old socket.
pub fn invalidate_vendo_connection(vendo_id: &str) -> bool {
    VENDO_CONNECTIONS.lock().unwrap().remove(vendo_id).is_some()
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
