use std::sync::Arc;

use axum::{
    Router, middleware,
    routing::{get, patch, post},
};

use crate::{
    server::AppState,
    transport::{
        middleware::{add_response_headers, check_auth},
        routes::{lyrics, player, stats, youtube},
    },
};
const API_V4: &str = "/v4";

pub fn router(state: Arc<AppState>) -> Router {
    let v4_routes = Router::new()
        .route("/loadtracks", get(stats::load_tracks))
        .route("/loadsearch", get(stats::load_search))
        .route("/info", get(stats::get_info))
        .route("/stats", get(stats::get_stats))
        .route("/decodetrack", get(stats::decode_track))
        .route("/decodetracks", post(stats::decode_tracks))
        .route("/sessions/{session_id}/players", get(player::get_players))
        .route(
            "/sessions/{session_id}/players/{guild_id}",
            get(player::get_player)
                .patch(player::update_player)
                .delete(player::destroy_player),
        )
        .route("/sessions/{session_id}", patch(player::update_session))
        .route("/lyrics", get(lyrics::get_lyrics))
        .route(
            "/sessions/{session_id}/players/{guild_id}/lyrics/subscribe",
            post(lyrics::subscribe_lyrics).delete(lyrics::unsubscribe_lyrics),
        )
        .route(
            "/sessions/{session_id}/players/{guild_id}/track/lyrics",
            get(lyrics::get_player_lyrics),
        )
        .route("/routeplanner/status", get(stats::routeplanner_status))
        .route(
            "/routeplanner/free/address",
            post(stats::routeplanner_free_address),
        )
        .route("/routeplanner/free/all", post(stats::routeplanner_free_all));

    Router::new()
        .nest(API_V4, v4_routes)
        .route("/version", get(stats::get_version))
        .route("/youtube", get(youtube::get_youtube_info))
        .route("/youtube/stream/{video_id}", get(youtube::youtube_stream))
        .route("/youtube/oauth/{refresh_token}", get(youtube::youtube_oauth_refresh))
        .layer(middleware::from_fn_with_state(state.clone(), check_auth))
        .layer(middleware::from_fn(add_response_headers))
        .with_state(state)
}
