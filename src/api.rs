use crate::server::{AppState, PlayerState, VoiceState};
use axum::{
    Router,
    extract::{Path, Query, State},
    response::Json,
    routing::get,
};
use base64::prelude::*;
use serde::{Deserialize, Serialize};
use songbird::{Config, driver::Driver};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

#[derive(Deserialize)]
pub struct LoadTracksQuery {
    pub identifier: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadTracksResponse {
    pub load_type: LoadType,
    pub data: LoadData,
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum LoadData {
    Track(Track),
    Tracks(Vec<Track>),
    Playlist(PlaylistData),
    Empty(serde_json::Value), // null or empty object
    Error(Exception),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistData {
    pub info: PlaylistInfo,
    pub tracks: Vec<Track>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LoadType {
    Track,
    Playlist,
    Search,
    Empty,
    Error,
}

#[derive(Serialize, Clone)]
pub struct Track {
    pub encoded: String,
    pub info: TrackInfo,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TrackInfo {
    pub identifier: String,
    pub is_seekable: bool,
    pub author: String,
    pub length: u64,
    pub is_stream: bool,
    pub position: u64,
    pub title: String,
    pub uri: String,
    pub source_name: String,
    pub artwork_url: Option<String>,
    pub isrc: Option<String>,
}

#[derive(Serialize)]
pub struct PlaylistInfo {
    pub name: String,
    pub selected_track: i32,
}

#[derive(Serialize)]
pub struct Exception {
    pub message: String,
    pub severity: String,
    pub cause: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoResponse {
    pub version: Version,
    pub build_time: u64,
    pub git: GitInfo,
    pub jvm: String,
    pub lavaplayer: String,
    pub source_managers: Vec<String>,
    pub filters: Vec<String>,
    pub plugins: Vec<PluginInfo>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub semver: String,
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub pre_release: Option<String>,
    pub build: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitInfo {
    pub branch: String,
    pub commit: String,
    pub commit_time: u64,
}

#[derive(Serialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlayerUpdateRequest {
    pub track: Option<PlayerUpdateTrack>,
    pub position: Option<u64>,
    #[allow(dead_code)]
    pub end_time: Option<u64>,
    pub volume: Option<u32>,
    pub paused: Option<bool>,
    pub voice: Option<PlayerUpdateVoice>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlayerUpdateTrack {
    #[allow(dead_code)]
    pub encoded: Option<String>,
    pub identifier: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlayerUpdateVoice {
    pub token: String,
    pub endpoint: String,
    pub session_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerResponse {
    pub guild_id: String,
    pub track: Option<Track>,
    pub volume: u32,
    pub paused: bool,
    pub state: PlayerStateResponse,
    pub voice: VoiceStateResponse,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerStateResponse {
    pub time: u64,
    pub position: u64,
    pub connected: bool,
    pub ping: i32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceStateResponse {
    pub token: String,
    pub endpoint: String,
    pub session_id: String,
}

pub async fn load_tracks(
    Query(params): Query<LoadTracksQuery>,
    State(_state): State<Arc<AppState>>,
) -> Json<LoadTracksResponse> {
    let identifier = params.identifier;
    info!("Load tracks request: identifier='{}'", identifier);

    // Some clients wrap URLs in < >, and might have whitespace
    let clean_identifier = identifier
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>');

    if clean_identifier.starts_with("http") {
        let encoded = BASE64_STANDARD.encode(clean_identifier.as_bytes());
        Json(LoadTracksResponse {
            load_type: LoadType::Track,
            data: LoadData::Track(Track {
                encoded,
                info: TrackInfo {
                    identifier: clean_identifier.to_string(), // Use actual identifier
                    is_seekable: true, // Assuming streams are seekable for now or change to false if live
                    author: "Unknown Author".to_string(),
                    length: 0, // 0 for stream?
                    is_stream: true,
                    position: 0,
                    title: "Unknown Title".to_string(),
                    uri: clean_identifier.to_string(),
                    source_name: "http".to_string(),
                    artwork_url: None,
                    isrc: None,
                },
            }),
        })
    } else {
        Json(LoadTracksResponse {
            load_type: LoadType::Empty,
            data: LoadData::Empty(serde_json::Value::Null),
        })
    }
}

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
        source_managers: vec!["http".to_string()],
        filters: vec![],
        plugins: vec![],
    })
}

pub async fn get_player(
    Path((session_id, guild_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
) -> Json<PlayerResponse> {
    info!("Get player for session {} guild {}", session_id, guild_id);

    let sessions = state.sessions.lock().await;
    let player = if let Some(session) = sessions.get(&session_id) {
        let players = session.players.lock().await;
        players.get(&guild_id).cloned()
    } else {
        None
    };

    match player {
        Some(p) => Json(PlayerResponse {
            guild_id: p.guild_id.clone(),
            track: p.track.map(|t| Track {
                encoded: "MOCK_BASE64".to_string(),
                info: TrackInfo {
                    identifier: "mock_id".to_string(),
                    is_seekable: false,
                    author: "Unknown Author".to_string(),
                    length: 0,
                    is_stream: true,
                    position: p.position,
                    title: "Stream".to_string(),
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
                position: p.position,
                connected: true,
                ping: 0,
            },
            voice: VoiceStateResponse {
                token: p.voice.token,
                endpoint: p.voice.endpoint,
                session_id: p.voice.session_id,
            },
        }),
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

pub async fn update_player(
    Path((session_id, guild_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<PlayerUpdateRequest>,
) -> Json<PlayerResponse> {
    info!(
        "Update player for session {} guild {}: {:?}",
        session_id, guild_id, body
    );

    let mut sessions = state.sessions.lock().await;
    if let Some(session) = sessions.get_mut(&session_id) {
        let mut players = session.players.lock().await;
        let p = players.entry(guild_id.clone()).or_insert_with(|| {
            let mut config = Config::default();
            config.driver_timeout = Some(std::time::Duration::from_secs(5));
            let driver = Driver::new(config);
            PlayerState {
                guild_id: guild_id.clone(),
                volume: 100,
                paused: false,
                track: None,
                position: 0,
                voice: VoiceState::default(),
                driver: Arc::new(Mutex::new(driver)),
                track_handle: None,
            }
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
                last_connected_token: p.voice.last_connected_token.clone(),
            };
            if let Some(uid) = session.user_id {
                let _ = crate::server::connect_player(p, uid).await;
            }
        }
    }

    // Reuse get_player logic (simple way for now)
    drop(sessions);
    get_player(Path((session_id, guild_id)), State(state)).await
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v4/loadtracks", get(load_tracks))
        .route("/v4/info", get(get_info))
        .route(
            "/v4/sessions/{session_id}/players/{guild_id}",
            get(get_player).patch(update_player),
        )
}
