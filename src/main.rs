// Copyright (c) 2026 appujet, notdeltaxd and contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::{net::SocketAddr, sync::Arc};

use axum::{Router, routing::get};
use dashmap::DashMap;
use rustalink::{common::types::AnyResult, monitoring, rest, server::AppState, ws};
use tracing::info;

#[tokio::main]
async fn main() -> AnyResult<()> {
    let config = rustalink::config::AppConfig::load().await?;

    rustalink::common::logger::init(
        config
            .logging
            .as_ref()
            .unwrap_or(&rustalink::config::LoggingConfig::default()),
    );

    rustalink::common::banner::print_banner(&rustalink::common::banner::BannerInfo::default());

    info!("Rustalink Server starting...");

    let routeplanner = if config.route_planner.enabled && !config.route_planner.cidrs.is_empty() {
        Some(
            Arc::new(rustalink::routeplanner::BalancingIpRoutePlanner::new(
                config.route_planner.cidrs.clone(),
            )) as Arc<dyn rustalink::routeplanner::RoutePlanner>,
        )
    } else {
        None
    };

    let source_manager = Arc::new(rustalink::sources::SourceManager::new(&config));
    let lyrics_manager = Arc::new(rustalink::lyrics::LyricsManager::new(&config));
    let youtube_ctx = source_manager.youtube_stream_ctx.clone();

    let process_stat = perf_monitor::cpu::ProcessStat::cur().map_err(|e| {
        Box::new(std::io::Error::other(format!(
            "failed to init ProcessStat: {e}"
        ))) as rustalink::common::types::AnyError
    })?;

    let shared_state = Arc::new(AppState {
        start_time: std::time::Instant::now(),
        sessions: DashMap::new(),
        resumable_sessions: DashMap::new(),
        routeplanner,
        source_manager,
        lyrics_manager,
        config: config.clone(),
        youtube: youtube_ctx,
        system_state: parking_lot::Mutex::new(sysinfo::System::new_all()),
        last_system_refresh: parking_lot::Mutex::new(std::time::Instant::now()),
        process_stat: parking_lot::Mutex::new(process_stat),
    });

    monitoring::prometheus::init(shared_state.clone());

    let mut app = Router::new()
        .route("/v4/websocket", get(ws::websocket_handler))
        .with_state(shared_state.clone())
        .merge(rest::router(shared_state.clone()))
        .layer(tower_http::trace::TraceLayer::new_for_http());

    if config.metrics.prometheus.enabled {
        app = app.route(
            &config.metrics.prometheus.endpoint,
            get(monitoring::prometheus::metrics_handler),
        );
    }

    let ip: std::net::IpAddr = config.server.address.parse()?;
    let address = SocketAddr::from((ip, config.server.port));
    info!("Rustalink Server listening on {}", address);

    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
