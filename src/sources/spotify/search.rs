use std::sync::Arc;

use serde_json::json;

use crate::{
    api::tracks::{PlaylistData, PlaylistInfo, SearchResult, Track},
    sources::spotify::{
        helpers::SpotifyHelpers, parser::SpotifyParser, token::SpotifyTokenTracker,
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
    ) -> Option<SearchResult> {
        let token = token_tracker.get_token().await?;

        let search_types = if types.is_empty() {
            "track,album,artist,playlist"
        } else {
            &types.join(",")
        };

        let url = format!(
            "https://api.spotify.com/v1/search?q={}&type={}&limit={}",
            urlencoding::encode(query),
            search_types,
            search_limit
        );

        let res = match client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
        {
            Ok(r) => r,
            Err(_) => return None,
        };

        if !res.status().is_success() {
            if res.status() == 401 || res.status() == 403 {
                // Fallback to internal search if public API fails
                return Self::search_full(client, token_tracker, query, types, search_limit).await;
            }
            return None;
        }

        let data: serde_json::Value = res.json().await.ok()?;

        let mut tracks = Vec::new();
        let mut albums = Vec::new();
        let mut artists = Vec::new();
        let mut playlists = Vec::new();

        // Parse Tracks
        if let Some(items) = data.pointer("/tracks/items").and_then(|v| v.as_array()) {
            for item in items {
                if let Some(track_info) = SpotifyParser::parse_track_inner(item, None) {
                    let mut track = Track::new(track_info);

                    track.plugin_info = json!({
                      "save_uri": track.info.uri,
                      "albumUrl": item.pointer("/album/external_urls/spotify").and_then(|v| v.as_str()),
                      "albumName": item.pointer("/album/name").and_then(|v| v.as_str()),
                      "previewUrl": item.get("preview_url").and_then(|v| v.as_str()),
                      "isPreview": false,
                      "artistUrl": item.pointer("/artists/0/external_urls/spotify").and_then(|v| v.as_str()),
                      "artistArtworkUrl": null,
                      "isLocal": item.get("is_local").and_then(|v| v.as_bool()).unwrap_or(false)
                    });

                    tracks.push(track);
                }
            }
        }

        // Parse Albums
        if let Some(items) = data.pointer("/albums/items").and_then(|v| v.as_array()) {
            for item in items {
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown Album");
                let url = item
                    .get("external_urls")
                    .and_then(|v| v.get("spotify"))
                    .and_then(|v| v.as_str());

                if url.is_none() || name == "Unknown Album" {
                    continue;
                }

                let artwork = item
                    .get("images")
                    .and_then(|v| v.get(0))
                    .and_then(|v| v.get("url"))
                    .and_then(|v| v.as_str());
                let author = item
                    .get("artists")
                    .and_then(|v| v.get(0))
                    .and_then(|v| v.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown Artist");
                let total_tracks = item.get("total_tracks").and_then(|v| v.as_i64());

                albums.push(PlaylistData {
                    info: PlaylistInfo {
                        name: name.to_string(),
                        selected_track: -1,
                    },
                    plugin_info: json!({
                      "type": "album",
                      "url": url,
                      "artworkUrl": artwork,
                      "author": author,
                      "totalTracks": total_tracks
                    }),
                    tracks: Vec::new(),
                });
            }
        }

        // Parse Artists
        if let Some(items) = data.pointer("/artists/items").and_then(|v| v.as_array()) {
            for item in items {
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown Artist");
                let url = item
                    .get("external_urls")
                    .and_then(|v| v.get("spotify"))
                    .and_then(|v| v.as_str());

                if url.is_none() || name == "Unknown Artist" {
                    continue;
                }

                let artwork = item
                    .get("images")
                    .and_then(|v| v.get(0))
                    .and_then(|v| v.get("url"))
                    .and_then(|v| v.as_str());

                artists.push(PlaylistData {
                    info: PlaylistInfo {
                        name: format!("{}'s Top Tracks", name),
                        selected_track: -1,
                    },
                    plugin_info: json!({
                      "type": "artist",
                      "url": url,
                      "artworkUrl": artwork,
                      "author": name,
                      "totalTracks": null
                    }),
                    tracks: Vec::new(),
                });
            }
        }

        // Parse Playlists
        if let Some(items) = data.pointer("/playlists/items").and_then(|v| v.as_array()) {
            for item in items {
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown Playlist");
                let url = item
                    .get("external_urls")
                    .and_then(|v| v.get("spotify"))
                    .and_then(|v| v.as_str());

                if url.is_none() || name == "Unknown Playlist" {
                    continue;
                }

                let artwork = item
                    .get("images")
                    .and_then(|v| v.get(0))
                    .and_then(|v| v.get("url"))
                    .and_then(|v| v.as_str());
                let author = item
                    .get("owner")
                    .and_then(|v| v.get("display_name"))
                    .and_then(|v| v.as_str());
                let total_tracks = item.pointer("/tracks/total").and_then(|v| v.as_i64());

                playlists.push(PlaylistData {
                    info: PlaylistInfo {
                        name: name.to_string(),
                        selected_track: -1,
                    },
                    plugin_info: json!({
                      "type": "playlist",
                      "url": url,
                      "artworkUrl": artwork,
                      "author": author,
                      "totalTracks": total_tracks
                    }),
                    tracks: Vec::new(),
                });
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

    pub async fn search_full(
        client: &reqwest::Client,
        token_tracker: &Arc<SpotifyTokenTracker>,
        query: &str,
        types: &[String],
        search_limit: usize,
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

        let data = SpotifyHelpers::partner_api_request(
            client,
            token_tracker,
            "searchDesktop",
            variables,
            hash,
        )
        .await?;

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
                    {
                        if let Some(track_info) = SpotifyParser::parse_track_inner(track_data, None)
                        {
                            let mut track = Track::new(track_info);

                            track.plugin_info = json!({
                              "save_uri": track.info.uri,
                              "albumUrl": track_data.pointer("/albumOfTrack/uri").and_then(|v| v.as_str()).map(|s| format!("https://open.spotify.com/album/{}", s.split(':').last().unwrap_or(""))),
                              "albumName": track_data.pointer("/albumOfTrack/name").and_then(|v| v.as_str()),
                              "previewUrl": null,
                              "isPreview": false,
                              "artistUrl": track_data.pointer("/artists/items/0/uri").and_then(|v| v.as_str()).map(|s| format!("https://open.spotify.com/artist/{}", s.split(':').last().unwrap_or(""))),
                              "artistArtworkUrl": null,
                              "isLocal": false
                            });

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
                              "totalTracks": null
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
                .and_then(|v| v.as_array())
            {
                for item in items {
                    if let Some(artist_data) = item
                        .get("item")
                        .or_else(|| item.get("itemV2"))
                        .and_then(|v| v.get("data"))
                    {
                        let name = artist_data
                            .pointer("/profile/name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown Artist");
                        let uri = artist_data
                            .get("uri")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let id = uri.split(':').last().unwrap_or("");
                        let artwork = artist_data
                            .pointer("/visuals/avatar/sources/0/url")
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
                              "totalTracks": null
                            }),
                            tracks: Vec::new(),
                        });
                    }
                }
            }
        }

        // Parse Playlists
        if all_types || types.contains(&"playlist".to_string()) {
            if let Some(items) = data
                .pointer("/data/searchV2/playlistsV2/items")
                .and_then(|v| v.as_array())
            {
                for item in items {
                    if let Some(playlist_data) = item
                        .get("item")
                        .or_else(|| item.get("itemV2"))
                        .and_then(|v| v.get("data"))
                    {
                        let name = playlist_data
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown Playlist");
                        let uri = playlist_data
                            .get("uri")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let id = uri.split(':').last().unwrap_or("");
                        let artwork = playlist_data
                            .pointer("/images/items/0/sources/0/url")
                            .and_then(|v| v.as_str());
                        let author = playlist_data
                            .pointer("/ownerV2/name")
                            .and_then(|v| v.as_str());

                        playlists.push(PlaylistData {
                            info: PlaylistInfo {
                                name: name.to_string(),
                                selected_track: -1,
                            },
                            plugin_info: json!({
                              "type": "playlist",
                              "url": format!("https://open.spotify.com/playlist/{}", id),
                              "artworkUrl": artwork,
                              "author": author,
                              "totalTracks": null
                            }),
                            tracks: Vec::new(),
                        });
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
