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
use rustalink::{common::types::AnyResult, server::AppState, transport};
use tracing::info;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[tokio::main]
async fn main() -> AnyResult<()> {
    let config = rustalink::configs::Config::load()?;

    rustalink::common::logger::init(&config);

    rustalink::log_println!(
        r#"
    [32m____            __        ___       __  [0m
   [32m/ __ \__  _______/ /_____ _/ (_)___  / /__[0m
  [32m/ /_/ / / / / ___/ __/ __ `/ / / __ \/ //_/[0m   v{}
 [32m/ _, _/ /_/ (__  ) /_/ /_/ / / / / / / ,<   [0m   Running on Rust
[32m/_/ |_|\__,_/____/\__/\__,_/_/_/_/ /_/_/|_|  [0m   
                                             
    "#,
        env!("CARGO_PKG_VERSION")
    );

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
    info!("Rustalink Server listening on {}", address);

    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
