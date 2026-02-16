use axum::{Router, routing::get};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;

mod api;
mod player;
mod server;
mod source;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let shared_state = Arc::new(server::AppState {
        sessions: Mutex::new(HashMap::new()),
    });

    let app = Router::new()
        .route("/v4/websocket", get(server::websocket_handler))
        .merge(api::router())
        .with_state(shared_state);

    let address = SocketAddr::from(([0, 0, 0, 0], 2333));
    println!("Lavalink Server listening on {}", address);

    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
