use serde_json::{Value, json};

use super::DeezerSource;
use crate::protocol::tracks::{Track, TrackInfo};

impl DeezerSource {
    pub(crate) fn parse_track(&self, json: &Value) -> Option<Track> {
        let id = json.get("id")?.to_string();
        let title = json.get("title")?.as_str()?.to_string();
        let artist = json.get("artist")?.get("name")?.as_str()?.to_string();
        let duration = json.get("duration")?.as_u64()? * 1000;
        let isrc = json
            .get("isrc")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let artwork_url = json
            .get("album")
            .and_then(|a| a.get("cover_xl"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let uri = json
            .get("link")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let mut track = Track::new(TrackInfo {
            identifier: id,
            is_seekable: true,
            author: artist,
            length: duration,
            is_stream: false,
            position: 0,
            title,
            uri: uri.clone(),
            artwork_url,
            isrc,
            source_name: "deezer".to_string(),
        });

        let album_name = json.pointer("/album/title").and_then(|v| v.as_str());
        let album_url = json
            .pointer("/album/id")
            .and_then(|v| {
                v.as_str()
                    .map(|s| s.to_string())
                    .or_else(|| v.as_u64().map(|id| id.to_string()))
            })
            .map(|id| format!("https://www.deezer.com/album/{}", id));
        let artist_url = json
            .pointer("/artist/id")
            .and_then(|v| {
                v.as_str()
                    .map(|s| s.to_string())
                    .or_else(|| v.as_u64().map(|id| id.to_string()))
            })
            .map(|id| format!("https://www.deezer.com/artist/{}", id));
        let artist_artwork_url = json.pointer("/artist/picture_xl").and_then(|v| v.as_str());
        let preview_url = json.get("preview").and_then(|v| v.as_str());

        track.plugin_info = json!({
            "albumName": album_name,
            "albumUrl": album_url,
            "artistUrl": artist_url,
            "artistArtworkUrl": artist_artwork_url,
            "previewUrl": preview_url,
            "isPreview": false
        });

        Some(track)
    }

    pub(crate) fn parse_recommendation_track(&self, json: &Value) -> Option<Track> {
        let id = json
            .get("SNG_ID")?
            .as_str()
            .map(|s| s.to_string())
            .or_else(|| json.get("SNG_ID").map(|v| v.to_string()))?;
        let title = json.get("SNG_TITLE")?.as_str()?.to_string();
        let artist = json.get("ART_NAME")?.as_str()?.to_string();
        let duration = json.get("DURATION")?.as_u64()? * 1000;
        let isrc = json
            .get("ISRC")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let album_pic = json
            .get("ALB_PICTURE")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let artwork_url = if !album_pic.is_empty() {
            Some(format!(
                "https://cdn-images.dzcdn.net/images/cover/{}/1000x1000-000000-80-0-0.jpg",
                album_pic
            ))
        } else {
            None
        };

        let uri_val = Some(format!("https://deezer.com/track/{}", id));
        let mut track = Track::new(TrackInfo {
            identifier: id.clone(),
            is_seekable: true,
            author: artist,
            length: duration,
            is_stream: false,
            position: 0,
            title,
            uri: uri_val.clone(),
            artwork_url,
            isrc,
            source_name: "deezer".to_string(),
        });

        let album_name = json.get("ALB_TITLE").and_then(|v| v.as_str());
        let album_url = json
            .get("ALB_ID")
            .and_then(|v| {
                v.as_str()
                    .map(|s| s.to_string())
                    .or_else(|| v.as_u64().map(|id| id.to_string()))
            })
            .map(|id| format!("https://www.deezer.com/album/{}", id));
        let artist_url = json
            .get("ART_ID")
            .and_then(|v| {
                v.as_str()
                    .map(|s| s.to_string())
                    .or_else(|| v.as_u64().map(|id| id.to_string()))
            })
            .map(|id| format!("https://www.deezer.com/artist/{}", id));
        let artist_artwork_url = json
            .pointer("/ARTISTS/0/ART_PICTURE")
            .and_then(|v| v.as_str())
            .map(|id| {
                format!(
                    "https://cdn-images.dzcdn.net/images/cover/{}/1000x1000-000000-80-0-0.jpg",
                    id
                )
            });
        let preview_url = json.pointer("/MEDIA/0/HREF").and_then(|v| v.as_str());

        track.plugin_info = json!({
            "albumName": album_name,
            "albumUrl": album_url,
            "artistUrl": artist_url,
            "artistArtworkUrl": artist_artwork_url,
            "previewUrl": preview_url,
            "isPreview": false
        });

        Some(track)
    }
}
