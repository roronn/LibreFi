mod network;
mod routes;
mod utils;

use network::server::Server;
use routes::index::index;

use crate::routes::{
    admin::admin,
    coin::coin,
    debug::debug,
    init::init,
    play_pause::play_pause,
    rates::rates,
    redeem::redeem,
    ws::ws_handler,
};

fn main() {
    #[cfg(windows)]
    let mut server = Server::new(8080);

    #[cfg(not(windows))]
    let mut server = Server::new(80);

    #[cfg(windows)]
    utils::db::init_db("librefi-test.redb");

    #[cfg(not(windows))]
    {
        utils::db::init_db("/etc/librefi/data.redb");
        let wan_iface = utils::setup_captive_portal::detect_wan_iface();
        utils::setup_captive_portal::setup_captive_portal(
            "br-lan",
            &wan_iface,
            "10.0.0.1",
            "5353",
        );
        utils::setup_captive_portal::authorize_active_users();
        utils::setup_captive_portal::restore_qos_settings();
    }

    if std::env::var("LIBREFI_DEBUG").unwrap_or_default() == "true" {
        debug(&mut server);
    }

    index(&mut server);

    #[cfg(windows)]
    server.post("/test/add-time", |_, res| {
        use crate::utils::db::{get_db, USERS_TABLE};
        use redb::ReadableTable;

        let mac = "02:00:00:00:00:01";
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as u32;

        let db = get_db();
        let write_txn = db.begin_write().unwrap();

        {
            let mut table = write_txn.open_table(USERS_TABLE).unwrap();

            let Some(record) = table.get(mac).unwrap() else {
                res.status = 404;
                res.body = b"{\"error\":\"Test user not found\"}".to_vec();
                res.content_type = "application/json".to_string();
                return;
            };

            let record_json = record.value().to_string();
            drop(record);

            let mut user: serde_json::Value =
                serde_json::from_str(&record_json).unwrap();

            let expires = user["expires_on"].as_u64().unwrap_or(0) as u32;

            user["expires_on"] = serde_json::json!(
                if expires > now { expires + 600 } else { now + 600 }
            );

            user["paused"] = serde_json::json!(false);
            user["paused_on"] = serde_json::json!(0);

            let updated_json = serde_json::to_string(&user).unwrap();
            table.insert(mac, updated_json.as_str()).unwrap();
        }

        write_txn.commit().unwrap();

        res.status = 200;
        res.body = b"{\"ok\":true,\"added_seconds\":600}".to_vec();
        res.content_type = "application/json".to_string();
    });

    play_pause(&mut server);
    init(&mut server);
    admin(&mut server);
    rates(&mut server);
    redeem(&mut server);
    coin(&mut server);
    server.ws("/ws", ws_handler);

    server.run();
}
