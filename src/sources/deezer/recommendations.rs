use serde_json::Value;

use super::{DeezerSource, PRIVATE_API_BASE};
use crate::api::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track};

impl DeezerSource {
    pub(crate) async fn get_recommendations(&self, query: &str) -> LoadResult {
        let tokens = match self.token_tracker.get_token().await {
            Some(t) => t,
            None => return LoadResult::Empty {},
        };

        let method;
        let payload;

        if let Some(artist_id) = query.strip_prefix(&self.rec_artist_prefix) {
            method = "song.getSmartRadio";
            payload = serde_json::json!({ "art_id": artist_id });
        } else {
            let track_id = query.strip_prefix(&self.rec_track_prefix).unwrap_or(query);
            method = "song.getSearchTrackMix";
            payload = serde_json::json!({ "sng_id": track_id, "start_with_input_track": "true" });
        }

        let url = format!(
            "{}?method={}&input=3&api_version=1.0&api_token={}",
            PRIVATE_API_BASE, method, tokens.api_token
        );
        let res = match self
            .client
            .post(&url)
            .header(
                "Cookie",
                format!(
                    "sid={}; dzr_uniq_id={}",
                    tokens.session_id, tokens.dzr_uniq_id
                ),
            )
            .json(&payload)
            .send()
            .await
        {
            Ok(r) => r,
            Err(_) => return LoadResult::Empty {},
        };

        let json: Value = res.json().await.unwrap_or(Value::Null);
        let data = json.get("results").and_then(|r| r.get("data"));

        let tracks: Vec<Track> = if let Some(arr) = data.and_then(|d| d.as_array()) {
            arr.iter()
                .filter_map(|item| self.parse_recommendation_track(item))
                .collect()
        } else if let Some(obj) = data.and_then(|d| d.as_object()) {
            obj.values()
                .filter_map(|item| self.parse_recommendation_track(item))
                .collect()
        } else {
            Vec::new()
        };

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }
        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: "Deezer Recommendations".to_string(),
                selected_track: -1,
            },
            plugin_info: serde_json::json!({
              "type": "recommendations",
              "totalTracks": tracks.len()
            }),
            tracks,
        })
    }
}
