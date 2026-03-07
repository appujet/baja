use std::{
    sync::{Arc, LazyLock},
    time::Duration,
};

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use prometheus::{Encoder, Gauge, Opts, Registry, TextEncoder};
use tokio::time::interval;
use tracing::{error, info};

use crate::server::AppState;

const NAMESPACE: &str = "lavalink";

struct NodeMetrics {
    players: Gauge,
    playing_players: Gauge,
    uptime: Gauge,
    memory_free: Gauge,
    memory_used: Gauge,
    memory_allocated: Gauge,
    memory_reservable: Gauge,
    cpu_cores: Gauge,
    cpu_system_load: Gauge,
    cpu_lavalink_load: Gauge,
}

static REGISTRY: LazyLock<Registry> = LazyLock::new(Registry::new);

static METRICS: LazyLock<NodeMetrics> = LazyLock::new(|| {
    let metrics = NodeMetrics {
        players: Gauge::with_opts(
            Opts::new("players_total", "Total connected players").namespace(NAMESPACE),
        )
        .unwrap(),
        playing_players: Gauge::with_opts(
            Opts::new("playing_players_total", "Players currently playing").namespace(NAMESPACE),
        )
        .unwrap(),
        uptime: Gauge::with_opts(
            Opts::new("uptime_milliseconds", "Node uptime in ms").namespace(NAMESPACE),
        )
        .unwrap(),
        memory_free: Gauge::with_opts(
            Opts::new("memory_free_bytes", "Free memory").namespace(NAMESPACE),
        )
        .unwrap(),
        memory_used: Gauge::with_opts(
            Opts::new("memory_used_bytes", "Used memory").namespace(NAMESPACE),
        )
        .unwrap(),
        memory_allocated: Gauge::with_opts(
            Opts::new("memory_allocated_bytes", "Allocated memory").namespace(NAMESPACE),
        )
        .unwrap(),
        memory_reservable: Gauge::with_opts(
            Opts::new("memory_reservable_bytes", "Reservable memory").namespace(NAMESPACE),
        )
        .unwrap(),
        cpu_cores: Gauge::with_opts(Opts::new("cpu_cores", "CPU cores count").namespace(NAMESPACE))
            .unwrap(),
        cpu_system_load: Gauge::with_opts(
            Opts::new("cpu_system_load_percentage", "System CPU load").namespace(NAMESPACE),
        )
        .unwrap(),
        cpu_lavalink_load: Gauge::with_opts(
            Opts::new("cpu_lavalink_load_percentage", "Process CPU load").namespace(NAMESPACE),
        )
        .unwrap(),
    };

    REGISTRY
        .register(Box::new(metrics.players.clone()))
        .unwrap();
    REGISTRY
        .register(Box::new(metrics.playing_players.clone()))
        .unwrap();
    REGISTRY.register(Box::new(metrics.uptime.clone())).unwrap();
    REGISTRY
        .register(Box::new(metrics.memory_free.clone()))
        .unwrap();
    REGISTRY
        .register(Box::new(metrics.memory_used.clone()))
        .unwrap();
    REGISTRY
        .register(Box::new(metrics.memory_allocated.clone()))
        .unwrap();
    REGISTRY
        .register(Box::new(metrics.memory_reservable.clone()))
        .unwrap();
    REGISTRY
        .register(Box::new(metrics.cpu_cores.clone()))
        .unwrap();
    REGISTRY
        .register(Box::new(metrics.cpu_system_load.clone()))
        .unwrap();
    REGISTRY
        .register(Box::new(metrics.cpu_lavalink_load.clone()))
        .unwrap();

    metrics
});

/// Initializes the metrics system and starts the background observer.
pub fn init(state: Arc<AppState>) {
    let config = &state.config.metrics.prometheus;
    if !config.enabled {
        return;
    }

    info!("Initializing Prometheus metrics at {}", config.endpoint);

    // Ensure metrics are initialized
    LazyLock::force(&METRICS);

    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(state.config.server.stats_interval));
        loop {
            ticker.tick().await;
            update_metrics(&state);
        }
    });
}

fn update_metrics(state: &AppState) {
    let stats = crate::monitoring::collect_stats(state, None);

    METRICS.players.set(stats.players as f64);
    METRICS.playing_players.set(stats.playing_players as f64);
    METRICS.uptime.set(stats.uptime as f64);
    METRICS.memory_free.set(stats.memory.free as f64);
    METRICS.memory_used.set(stats.memory.used as f64);
    METRICS.memory_allocated.set(stats.memory.allocated as f64);
    METRICS
        .memory_reservable
        .set(stats.memory.reservable as f64);
    METRICS.cpu_cores.set(stats.cpu.cores as f64);
    METRICS.cpu_system_load.set(stats.cpu.system_load);
    METRICS.cpu_lavalink_load.set(stats.cpu.lavalink_load);
}

/// Axum handler for Prometheus metrics.
pub async fn metrics_handler() -> Response {
    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();
    let mut buffer = Vec::new();

    if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
        error!("Failed to encode prometheus metrics: {}", e);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", encoder.format_type())
        .body(buffer.into())
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}
