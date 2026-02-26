use std::sync::Arc;

use axum::{
    Router, middleware,
    routing::{get, patch, post},
};

use crate::{
    server::AppState,
    transport::middleware::{add_response_headers, check_auth},
    transport::routes::{lyrics, player, stats},
};

pub fn router(state: Arc<AppState>) -> Router {
    let api_routes = Router::new()
        .route("/v4/loadtracks", get(stats::load_tracks))
        .route("/v4/loadsearch", get(stats::load_search))
        .route("/v4/info", get(stats::get_info))
        .route("/v4/stats", get(stats::get_stats))
        .route("/version", get(stats::get_version))
        .route("/v4/decodetrack", get(stats::decode_track))
        .route("/v4/decodetracks", post(stats::decode_tracks))
        .route(
            "/v4/sessions/{session_id}/players",
            get(player::get_players),
        )
        .route(
            "/v4/sessions/{session_id}/players/{guild_id}",
            get(player::get_player)
                .patch(player::update_player)
                .delete(player::destroy_player),
        )
        .route("/v4/sessions/{session_id}", patch(player::update_session))
        .route("/v4/loadlyrics", get(lyrics::load_lyrics))
        .route("/v4/lyrics", get(lyrics::get_lyrics))
        .route(
            "/v4/sessions/{session_id}/players/{guild_id}/lyrics/subscribe",
            post(lyrics::subscribe_lyrics).delete(lyrics::unsubscribe_lyrics),
        )
        .route(
            "/v4/sessions/{session_id}/players/{guild_id}/track/lyrics",
            get(lyrics::get_player_lyrics),
        )
        .route("/v4/routeplanner/status", get(stats::routeplanner_status))
        .route(
            "/v4/routeplanner/free/address",
            post(stats::routeplanner_free_address),
        )
        .route(
            "/v4/routeplanner/free/all",
            post(stats::routeplanner_free_all),
        )
        .layer(middleware::from_fn_with_state(state.clone(), check_auth))
        .with_state(state);

    Router::new()
        .merge(api_routes)
        .layer(middleware::from_fn(add_response_headers))
}
