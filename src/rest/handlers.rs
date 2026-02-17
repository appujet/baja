use crate::rest::models::*;
use crate::server::AppState;
use crate::sources::SourceManager;
use axum::{
    extract::{Path, Query, State},
    response::Json,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

/// Universal Track Loading Handler
///
/// Use Case:
/// This is the main entry point for resolving any identifier (URL, search query, etc.)
/// into playable tracks. It delegates to the SourceManager which tries each registered
/// source plugin in order until one can handle the request.
///
/// Supported sources (extensible):
/// - HTTP/HTTPS direct links
/// - YouTube (ytsearch: prefix or youtube.com URLs) - TODO
/// - Spotify (spsearch: prefix or spotify.com URLs) - TODO
///
/// To add a new source:
/// 1. Create a new file in src/sources/ implementing SourcePlugin trait
/// 2. Register it in SourceManager::new()
pub async fn load_tracks(
    Query(params): Query<LoadTracksQuery>,
    State(_state): State<Arc<AppState>>,
) -> Json<LoadTracksResponse> {
    let identifier = params.identifier;
    info!("Universal Load Request: '{}'", identifier);

    // Get or create the source manager
    let source_manager = SourceManager::new();

    // Delegate to source manager
    let response = source_manager.load(&identifier).await;

    Json(response)
}

/// Node Information Handler
pub async fn get_info() -> Json<InfoResponse> {
    Json(InfoResponse {
        version: Version {
            semver: "4.0.0".to_string(),
            major: 4,
            minor: 0,
            patch: 0,
            pre_release: None,
            build: None,
        },
        build_time: 0,
        git: GitInfo {
            branch: "main".to_string(),
            commit: "unknown".to_string(),
            commit_time: 0,
        },
        jvm: "n/a".to_string(),
        lavaplayer: "n/a".to_string(),
        source_managers: vec![
            "http".to_string(),
            "youtube".to_string(),
            "spotify".to_string(),
        ],
        filters: vec![],
        plugins: vec![],
    })
}

/// Player State Retrieval
pub async fn get_player(
    Path((session_id, guild_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
) -> Json<PlayerResponse> {
    info!("Get Player: sess={} guild={}", session_id, guild_id);

    let sessions = state.sessions.lock().await;
    let player = if let Some(session) = sessions.get(&session_id) {
        let players = session.players.lock().await;
        players.get(&guild_id).cloned()
    } else {
        None
    };

    match player {
        Some(p) => {
            use base64::prelude::*;
            let current_pos = if let Some(handle) = &p.track_handle {
                handle.get_position()
            } else {
                p.position
            };

            Json(PlayerResponse {
                guild_id: p.guild_id.clone(),
                track: p.track.map(|t| Track {
                    encoded: BASE64_STANDARD.encode(t.as_bytes()),
                    info: TrackInfo {
                        identifier: t.clone(),
                        is_seekable: true,
                        author: "Active".to_string(),
                        length: 0,
                        is_stream: true,
                        position: current_pos,
                        title: "Current Track".to_string(),
                        uri: t,
                        source_name: "http".to_string(),
                        artwork_url: None,
                        isrc: None,
                    },
                }),
                volume: p.volume,
                paused: p.paused,
                state: PlayerStateResponse {
                    time: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                    position: current_pos,
                    connected: true,
                    ping: 0,
                },
                voice: VoiceStateResponse {
                    token: p.voice.token,
                    endpoint: p.voice.endpoint,
                    session_id: p.voice.session_id,
                },
            })
        }
        None => Json(PlayerResponse {
            guild_id: guild_id.clone(),
            track: None,
            volume: 100,
            paused: false,
            state: PlayerStateResponse {
                time: 0,
                position: 0,
                connected: false,
                ping: 0,
            },
            voice: VoiceStateResponse {
                token: "".to_string(),
                endpoint: "".to_string(),
                session_id: "".to_string(),
            },
        }),
    }
}

/// Player Control Handler
pub async fn update_player(
    Path((session_id, guild_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<PlayerUpdateRequest>,
) -> Json<PlayerResponse> {
    use crate::server::{PlayerState, VoiceState};

    info!(
        "Update Player: sess={} guild={} body={:?}",
        session_id, guild_id, body
    );

    let mut sessions = state.sessions.lock().await;
    if let Some(session) = sessions.get_mut(&session_id) {
        let mut players = session.players.lock().await;
        let p = players
            .entry(guild_id.clone())
            .or_insert_with(|| PlayerState {
                guild_id: guild_id.clone(),
                volume: 100,
                paused: false,
                track: None,
                position: 0,
                voice: VoiceState {
                    channel_id: None,
                    ..VoiceState::default()
                },
                track_handle: None,
                engine: Arc::new(Mutex::new(crate::voice::VoiceEngine::new())),
            });

        if let Some(v) = body.volume {
            p.volume = v;
        }
        if let Some(paused) = body.paused {
            p.paused = paused;
        }
        if let Some(pos) = body.position {
            p.position = pos;
        }
        if let Some(t) = body.track {
            let track_to_play = t.encoded.clone().or(t.identifier.clone());
            if let Some(track) = track_to_play {
                crate::server::start_playback(p, track, session.sender.clone()).await;
            }
        }
        if let Some(v) = body.voice {
            p.voice = VoiceState {
                token: v.token,
                endpoint: v.endpoint,
                session_id: v.session_id,
                channel_id: v.channel_id,
            };
            if let Some(uid) = session.user_id {
                let _ = crate::server::connect_player(p, uid).await;
            }
        }
    }

    drop(sessions);
    get_player(Path((session_id, guild_id)), State(state)).await
}
