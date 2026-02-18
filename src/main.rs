use axum::{Router, routing::get};
use dashmap::DashMap;
use rustalink::server::AppState;
use rustalink::transport;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = rustalink::config::Config::load()?;

    rustalink::common::logger::init(&config);

    info!("Lavalink Server starting...");

    let routeplanner = if config.route_planner.enabled && !config.route_planner.cidrs.is_empty() {
        Some(
            Arc::new(rustalink::routeplanner::BalancingIpRoutePlanner::new(
                config.route_planner.cidrs.clone(),
            )) as Arc<dyn rustalink::routeplanner::RoutePlanner>,
        )
    } else {
        None
    };

    let shared_state = Arc::new(AppState {
        sessions: DashMap::new(),
        resumable_sessions: DashMap::new(),
        routeplanner,
        source_manager: Arc::new(rustalink::sources::SourceManager::new(&config)),
        config: config.clone(),
    });

    let app = Router::new()
        .route(
            "/v4/websocket",
            get(transport::websocket_server::websocket_handler),
        )
        .with_state(shared_state.clone())
        .merge(transport::http_server::router(shared_state.clone()))
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let ip: std::net::IpAddr = config.server.host.parse()?;
    let address = SocketAddr::from((ip, config.server.port));
    info!("Lavalink Server listening on {}", address);

    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
