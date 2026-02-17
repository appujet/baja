use axum::{Router, routing::get};
use dashmap::DashMap;
use std::net::SocketAddr;
use std::sync::Arc;

mod audio;
mod player;
mod rest;
mod server;
mod source;
mod sources;
mod types;
mod voice;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("debug"));

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .init();

    let shared_state = Arc::new(server::AppState {
        sessions: DashMap::new(),
        resumable_sessions: DashMap::new(),
    });

    let app = Router::new()
        .route("/v4/websocket", get(server::websocket_handler))
        .merge(rest::router())
        .with_state(shared_state);

    let address = SocketAddr::from(([0, 0, 0, 0], 2333));
    println!("Lavalink Server listening on {}", address);

    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
