use axum::{Router, routing::get};
use dashmap::DashMap;
use rustalink::server::AppState;
use rustalink::transport;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("debug"));

    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    let shared_state = Arc::new(AppState {
        sessions: DashMap::new(),
        resumable_sessions: DashMap::new(),
        routeplanner: None,
        source_manager: Arc::new(rustalink::sources::SourceManager::new()),
    });

    let app = Router::new()
        .route(
            "/v4/websocket",
            get(transport::websocket_server::websocket_handler),
        )
        .merge(transport::http_server::router())
        .with_state(shared_state)
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let address = SocketAddr::from(([0, 0, 0, 0], 2333));
    info!("Lavalink Server listening on {}", address);

    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
