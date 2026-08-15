use std::net::SocketAddr;
use std::sync::Arc;

use buzz_pair_relay::{
    run_server, Relay, RelayConfig, ENV_CONN_TIMEOUT_SECS, ENV_PING_INTERVAL_SECS,
};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let addr_raw =
        std::env::var("BUZZ_PAIR_RELAY_BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:5000".to_string());
    let addr: SocketAddr = match addr_raw.parse() {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("fatal: invalid BUZZ_PAIR_RELAY_BIND_ADDR {addr_raw:?}: {e}");
            std::process::exit(1);
        }
    };
    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("fatal: failed to bind {addr}: {e}");
            std::process::exit(1);
        }
    };
    let config = RelayConfig::from_env();
    eprintln!(
        "buzz-pair-relay config: conn_timeout={:?} ping_interval={:?} \
         (override via {ENV_CONN_TIMEOUT_SECS} / {ENV_PING_INTERVAL_SECS})",
        config.conn_timeout, config.ping_interval
    );
    let relay = Arc::new(Relay::with_config(config));
    run_server(listener, relay).await;
}
