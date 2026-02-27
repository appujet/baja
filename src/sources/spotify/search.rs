use std::{sync::Arc, time::Duration};

use serde_json::json;
use tokio::time::timeout;

use crate::{
    protocol::tracks::{PlaylistData, PlaylistInfo, SearchResult, Track},
    sources::spotify::{
        helpers::SpotifyHelpers, metadata::SpotifyMetadata, parser::SpotifyParser,
        token::SpotifyTokenTracker,
    },
};

pub struct SpotifySearch;

impl SpotifySearch {
    pub async fn get_autocomplete(
        client: &reqwest::Client,
        token_tracker: &Arc<SpotifyTokenTracker>,
        query: &str,
        types: &[String],
        search_limit: usize,
        isrc_binary_regex: &Arc<regex::Regex>,
    ) -> Option<SearchResult> {
        Self::search_full(
            client,
            token_tracker,
            query,
            types,
            search_limit,
            isrc_binary_regex,
        )
        .await
    }

    pub async fn search_full(
        client: &reqwest::Client,
        token_tracker: &Arc<SpotifyTokenTracker>,
        query: &str,
        types: &[String],
        search_limit: usize,
        isrc_binary_regex: &Arc<regex::Regex>,
    ) -> Option<SearchResult> {
        let variables = json!({
            "searchTerm": query,
            "offset": 0,
            "limit": search_limit,
            "numberOfTopResults": 5,
            "includeAudiobooks": false,
            "includeArtistHasConcertsField": false,
            "includePreReleases": false
        });

        let hash = "fcad5a3e0d5af727fb76966f06971c19cfa2275e6ff7671196753e008611873c";

        let data = match SpotifyHelpers::partner_api_request(
            client,
            token_tracker,
            "searchDesktop",
            variables,
            hash,
        )
        .await
        {
            Some(d) => d,
            None => {
                return None;
            }
        };

        let mut tracks = Vec::new();
        let mut albums = Vec::new();
        let mut artists = Vec::new();
        let mut playlists = Vec::new();

        let all_types = types.is_empty();

        // Parse Tracks
        if all_types || types.contains(&"track".to_string()) {
            if let Some(items) = data
                .pointer("/data/searchV2/tracksV2/items")
                .or_else(|| data.pointer("/data/searchV2/tracks/items"))
                .and_then(|v| v.as_array())
            {
                for item in items {
                    if let Some(track_data) = item
                        .get("item")
                        .or_else(|| item.get("itemV2"))
                        .and_then(|v| v.get("data"))
                        .or_else(|| item.get("data"))
                    {
                        if let Some(track_info) = SpotifyParser::parse_track_inner(track_data, None)
                        {
                            let mut track = Track::new(track_info);

                            track.plugin_info = crate::protocol::tracks::PluginInfo {
                                album_name: track_data
                                    .pointer("/albumOfTrack/name")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string()),
                                album_url: track_data
                                    .pointer("/albumOfTrack/uri")
                                    .and_then(|v| v.as_str())
                                    .map(|s| {
                                        format!(
                                            "https://open.spotify.com/album/{}",
                                            s.split(':').last().unwrap_or("")
                                        )
                                    }),
                                artist_url: track_data
                                    .pointer("/artists/items/0/uri")
                                    .and_then(|v| v.as_str())
                                    .map(|s| {
                                        format!(
                                            "https://open.spotify.com/artist/{}",
                                            s.split(':').last().unwrap_or("")
                                        )
                                    }),
                                artist_artwork_url: None,
                                preview_url: None,
                                is_preview: false,
                            };

                            if track.info.isrc.is_none() {
                                if let Ok(Some(isrc)) = timeout(
                                    Duration::from_secs(2),
                                    SpotifyMetadata::fetch_metadata_isrc(
                                        client,
                                        token_tracker,
                                        &track.info.identifier,
                                        isrc_binary_regex,
                                    ),
                                )
                                .await
                                {
                                    track.info.isrc = Some(isrc);
                                }
                            }

                            tracks.push(track);
                        }
                    }
                }
            }
        }
        // Parse Albums
        if all_types || types.contains(&"album".to_string()) {
            if let Some(items) = data
                .pointer("/data/searchV2/albumsV2/items")
                .or_else(|| data.pointer("/data/searchV2/albums/items"))
                .and_then(|v| v.as_array())
            {
                for item in items {
                    if let Some(album_data) = item
                        .get("item")
                        .or_else(|| item.get("itemV2"))
                        .and_then(|v| v.get("data"))
                        .or_else(|| item.get("data"))
                    {
                        let name = album_data
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown Album");
                        let uri = album_data.get("uri").and_then(|v| v.as_str()).unwrap_or("");
                        let id = uri.split(':').last().unwrap_or("");
                        let artwork = album_data
                            .pointer("/coverArt/sources/0/url")
                            .and_then(|v| v.as_str());
                        let author = album_data
                            .pointer("/artists/items/0/profile/name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown Artist");

                        albums.push(PlaylistData {
                            info: PlaylistInfo {
                                name: name.to_string(),
                                selected_track: -1,
                            },
                            plugin_info: json!({
                              "type": "album",
                              "url": format!("https://open.spotify.com/album/{}", id),
                              "artworkUrl": artwork,
                              "author": author,
                              "totalTracks": 0
                            }),
                            tracks: Vec::new(),
                        });
                    }
                }
            }
        }

        // Parse Artists
        if all_types || types.contains(&"artist".to_string()) {
            if let Some(items) = data
                .pointer("/data/searchV2/artistsV2/items")
                .or_else(|| data.pointer("/data/searchV2/artists/items"))
                .or_else(|| data.pointer("/data/searchV2/profilesV2/items"))
                .or_else(|| data.pointer("/data/searchV2/profiles/items"))
                .and_then(|v| v.as_array())
            {
                for item in items {
                    if let Some(artist_data) = item
                        .get("item")
                        .or_else(|| item.get("itemV2"))
                        .and_then(|v| v.get("data"))
                        .or_else(|| item.get("data"))
                    {
                        let name = artist_data
                            .pointer("/profile/name")
                            .or_else(|| artist_data.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown Artist");
                        let uri = artist_data
                            .get("uri")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let id = uri.split(':').last().unwrap_or("");
                        let artwork = artist_data
                            .pointer("/visuals/avatarImage/sources/0/url")
                            .or_else(|| artist_data.pointer("/images/items/0/sources/0/url"))
                            .and_then(|v| v.as_str());

                        artists.push(PlaylistData {
                            info: PlaylistInfo {
                                name: format!("{}'s Top Tracks", name),
                                selected_track: -1,
                            },
                            plugin_info: json!({
                              "type": "artist",
                              "url": format!("https://open.spotify.com/artist/{}", id),
                              "artworkUrl": artwork,
                              "author": name,
                              "totalTracks": 0
                            }),
                            tracks: Vec::new(),
                        });
                    }
                }
            }
        }

        // Parse Playlists
        if all_types || types.contains(&"playlist".to_string()) {
            let playlist_paths = [
                "/data/searchV2/playlistsV2/items",
                "/data/searchV2/playlists/items",
            ];

            for path in playlist_paths {
                if let Some(items) = data.pointer(path).and_then(|v| v.as_array()) {
                    for item in items {
                        if let Some(playlist_data) = item
                            .get("item")
                            .or_else(|| item.get("itemV2"))
                            .and_then(|v| v.get("data"))
                            .or_else(|| item.get("data"))
                        {
                            let name = playlist_data
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Unknown");
                            let uri = playlist_data
                                .get("uri")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let parts: Vec<&str> = uri.split(':').collect();
                            let type_str = parts.get(1).unwrap_or(&"playlist");
                            let id = parts.last().unwrap_or(&"");

                            let artwork = playlist_data
                                .pointer("/images/items/0/sources/0/url")
                                .or_else(|| playlist_data.pointer("/coverArt/sources/0/url"))
                                .and_then(|v| v.as_str());

                            let author = playlist_data
                                .pointer("/ownerV2/data/name")
                                .or_else(|| playlist_data.pointer("/ownerV2/name"))
                                .and_then(|v| v.as_str());

                            playlists.push(PlaylistData {
                                info: PlaylistInfo {
                                    name: name.to_string(),
                                    selected_track: -1,
                                },
                                plugin_info: json!({
                                  "type": type_str,
                                  "url": format!("https://open.spotify.com/{}/{}", type_str, id),
                                  "artworkUrl": artwork,
                                  "author": author,
                                  "totalTracks": 0
                                }),
                                tracks: Vec::new(),
                            });
                        }
                    }
                }
            }
        }

        Some(SearchResult {
            tracks,
            albums,
            artists,
            playlists,
            texts: Vec::new(),
            plugin: json!({}),
        })
    }
}
