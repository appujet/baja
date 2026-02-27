use serde_json::Value;

use super::AppleMusicSource;
use crate::protocol::tracks::{Track, TrackInfo};

impl AppleMusicSource {
    pub(crate) fn build_track(
        &self,
        item: &Value,
        artwork_override: Option<String>,
    ) -> Option<Track> {
        let attributes = item.get("attributes")?;

        let id = item.get("id")?.as_str()?.to_string();
        let title = attributes
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Title")
            .to_string();
        let author = attributes
            .get("artistName")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Artist")
            .to_string();
        let length = attributes
            .get("durationInMillis")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let isrc = attributes
            .get("isrc")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let artwork_url = artwork_override.or_else(|| {
            attributes
                .get("artwork")
                .and_then(|a| a.get("url"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.replace("{w}", "1000").replace("{h}", "1000"))
        });

        let url = attributes
            .get("url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let mut track = Track::new(TrackInfo {
            title,
            author,
            length,
            identifier: id,
            is_stream: false,
            uri: url,
            artwork_url,
            isrc,
            source_name: "applemusic".to_string(),
            is_seekable: true,
            position: 0,
        });

        let album_name = attributes.get("albumName").and_then(|v| v.as_str());
        let artist_url = attributes.get("artistUrl").and_then(|v| v.as_str());
        let preview_url = attributes
            .pointer("/previews/0/url")
            .and_then(|v| v.as_str());

        track.plugin_info = crate::protocol::tracks::PluginInfo {
            album_name: album_name.map(|s| s.to_string()),
            album_url: track
                .info
                .uri
                .as_ref()
                .and_then(|u| u.split('?').next().map(|s| s.to_string())),
            artist_url: artist_url.map(|s| s.to_string()),
            artist_artwork_url: None,
            preview_url: preview_url.map(|s| s.to_string()),
            is_preview: false,
        };

        Some(track)
    }
}
