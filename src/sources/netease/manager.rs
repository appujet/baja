use serde_json::json;

use super::api::*;
use crate::protocol::tracks::{
    LoadResult, PlaylistData, PlaylistInfo, SearchResult, Track, TrackInfo,
};

pub fn parse_track(song: &NeteaseSong) -> Option<Track> {
    let id = song.id.to_string();

    let artist = song
        .artists
        .first()
        .map(|a| a.name.as_str())
        .unwrap_or("Unknown Artist");

    let artwork_url = song.album.as_ref().and_then(|al| al.pic_url.clone());

    Some(Track::new(TrackInfo {
        identifier: id.clone(),
        is_seekable: true,
        author: artist.to_string(),
        length: song.duration,
        is_stream: false,
        position: 0,
        title: song.name.clone(),
        uri: Some(format!("https://music.163.com/song?id={}", id)),
        artwork_url,
        isrc: None,
        source_name: "netease".to_string(),
    }))
}

pub fn parse_album(album: &NeteaseAlbum) -> PlaylistData {
    PlaylistData {
        info: PlaylistInfo {
            name: album.name.clone(),
            selected_track: -1,
        },
        plugin_info: json!({
            "type": "album",
            "url": format!("https://music.163.com/album?id={}", album.id),
            "artworkUrl": album.pic_url,
            "author": "Netease Music",
            "totalTracks": 0
        }),
        tracks: Vec::new(),
    }
}

pub fn parse_artist(artist: &NeteaseArtist) -> PlaylistData {
    PlaylistData {
        info: PlaylistInfo {
            name: format!("{}'s Top Tracks", artist.name),
            selected_track: -1,
        },
        plugin_info: json!({
            "type": "artist",
            "url": format!("https://music.163.com/artist?id={}", artist.id),
            "artworkUrl": None::<String>,
            "author": artist.name,
            "totalTracks": 0
        }),
        tracks: Vec::new(),
    }
}

pub fn parse_playlist(playlist: &NeteasePlaylist) -> PlaylistData {
    PlaylistData {
        info: PlaylistInfo {
            name: playlist.name.clone(),
            selected_track: -1,
        },
        plugin_info: json!({
            "type": "playlist",
            "url": format!("https://music.163.com/playlist?id={}", playlist.id),
            "artworkUrl": playlist.cover_img_url,
            "author": playlist.creator.as_ref().map(|c| c.nickname.clone()).unwrap_or_else(|| "Netease Music".to_string()),
            "totalTracks": playlist.track_count.unwrap_or(0)
        }),
        tracks: Vec::new(),
    }
}

#[derive(Debug, PartialEq)]
pub enum TrackUrlResult {
    Success(String),
    Code(i64),
    Trial,
    None,
}

pub async fn fetch_track_detail(
    client: &reqwest::Client,
    nuid: &str,
    device_id: &str,
    id: &str,
) -> Option<SongDetailData> {
    get_eapi_json(
        client,
        "/api/v3/song/detail",
        json!({
            "c": serde_json::to_string(&[json!({"id": id})]).unwrap(),
            "header": {
                "os": "iOS",
                "appver": "2.5.1",
                "deviceId": device_id
            }
        }),
        nuid,
        device_id,
    )
    .await
}

pub async fn fetch_album(
    client: &reqwest::Client,
    nuid: &str,
    device_id: &str,
    id: &str,
) -> LoadResult {
    match get_eapi_json::<AlbumDetailData>(
        client,
        &format!("/api/v1/album/{}", id),
        json!({}),
        nuid,
        device_id,
    )
    .await
    {
        Some(resp) if !resp.songs.is_empty() => {
            let tracks = resp.songs.iter().filter_map(parse_track).collect();
            LoadResult::Playlist(PlaylistData {
                info: PlaylistInfo {
                    name: resp.album.name,
                    selected_track: -1,
                },
                plugin_info: json!({ "type": "album", "id": id }),
                tracks,
            })
        }
        _ => LoadResult::Empty {},
    }
}

pub async fn fetch_playlist(
    client: &reqwest::Client,
    nuid: &str,
    device_id: &str,
    id: &str,
) -> LoadResult {
    match get_eapi_json::<PlaylistDetailData>(
        client,
        "/api/v6/playlist/detail",
        json!({ "id": id, "n": 1000 }),
        nuid,
        device_id,
    )
    .await
    {
        Some(resp) if !resp.playlist.tracks.is_empty() => {
            let tracks = resp
                .playlist
                .tracks
                .iter()
                .filter_map(parse_track)
                .collect();
            LoadResult::Playlist(PlaylistData {
                info: PlaylistInfo {
                    name: resp.playlist.name,
                    selected_track: -1,
                },
                plugin_info: json!({ "type": "playlist", "id": id }),
                tracks,
            })
        }
        _ => LoadResult::Empty {},
    }
}

pub async fn fetch_artist(
    client: &reqwest::Client,
    nuid: &str,
    device_id: &str,
    id: &str,
) -> LoadResult {
    match get_eapi_json::<ArtistDetailData>(
        client,
        &format!("/api/v1/artist/{}", id),
        json!({}),
        nuid,
        device_id,
    )
    .await
    {
        Some(resp) if !resp.hot_songs.is_empty() => {
            let tracks = resp.hot_songs.iter().filter_map(parse_track).collect();
            LoadResult::Playlist(PlaylistData {
                info: PlaylistInfo {
                    name: format!("{}'s Top Tracks", resp.artist.name),
                    selected_track: -1,
                },
                plugin_info: json!({ "type": "artist", "id": id }),
                tracks,
            })
        }
        _ => LoadResult::Empty {},
    }
}

pub async fn fetch_track_url(
    client: &reqwest::Client,
    nuid: &str,
    device_id: &str,
    id: &str,
    level: &str,
    encode_type: &str,
) -> TrackUrlResult {
    let resp: Option<TrackUrlData> = get_eapi_json(
        client,
        "/api/song/enhance/player/url/v1",
        json!({
            "ids": [id],
            "level": level,
            "encodeType": encode_type,
            "header": {
                "os": "iOS",
                "appver": "2.5.1",
                "deviceId": device_id
            }
        }),
        nuid,
        device_id,
    )
    .await;

    let data_wrap = match resp {
        Some(v) => v,
        None => return TrackUrlResult::None,
    };

    let data = match data_wrap.data.first() {
        Some(d) => d,
        None => return TrackUrlResult::None,
    };

    if data.code == -110 {
        return TrackUrlResult::Code(-110);
    }

    if let Some(trial_info) = &data.free_trial_info
        && trial_info.as_object().is_some_and(|o| !o.is_empty())
    {
        return TrackUrlResult::Trial;
    }

    if let Some(url) = &data.url
        && !url.is_empty()
    {
        return TrackUrlResult::Success(url.clone());
    }

    if data.code != 200 && data.code != 0 {
        TrackUrlResult::Code(data.code)
    } else {
        TrackUrlResult::None
    }
}

pub async fn fetch_track_url_legacy(
    client: &reqwest::Client,
    device_id: &str,
    id: &str,
    br: &str,
) -> Option<String> {
    let url = format!(
        "https://music.163.com/api/song/enhance/player/url?id={}&ids=[{}]&br={}",
        id, id, br
    );

    let resp = client.get(&url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/116.0.0.0 Safari/537.36")
        .header("Referer", "https://music.163.com/")
        .header("X-Real-IP", "118.88.88.88")
        .header("Cookie", format!("os=pc; deviceId={}", device_id))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let res: NeteaseResponse<TrackUrlData> = resp.json().await.ok()?;
    let data = res.data.data.first()?;

    if let Some(url) = &data.url
        && !url.is_empty()
    {
        return Some(url.clone());
    }

    None
}

pub async fn check_url(client: &reqwest::Client, url: &str) -> bool {
    let resp = match client.head(url)
        .header("User-Agent", "Mozilla/5.0 (iPhone; CPU iPhone OS 16_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 CloudMusic/2.5.1")
        .header("Referer", "https://music.163.com/")
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => return false,
    };

    if !resp.status().is_success() {
        return false;
    }

    let content_length = resp
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    content_length >= 100 * 1024
}

pub async fn search_tracks(
    client: &reqwest::Client,
    nuid: &str,
    device_id: &str,
    query: &str,
    limit: usize,
) -> LoadResult {
    match get_eapi_json::<SearchResultData>(
        client,
        "/api/cloudsearch/pc",
        json!({
            "s": query,
            "type": 1,
            "limit": limit,
            "offset": 0,
            "total": true
        }),
        nuid,
        device_id,
    )
    .await
    {
        Some(resp) if !resp.result.songs.is_empty() => {
            let tracks = resp.result.songs.iter().filter_map(parse_track).collect();
            LoadResult::Search(tracks)
        }
        _ => LoadResult::Empty {},
    }
}

pub async fn search_full(
    client: &reqwest::Client,
    nuid: &str,
    device_id: &str,
    query: &str,
    types: &[String],
    limit: usize,
) -> Option<SearchResult> {
    let mut tracks = Vec::new();
    let mut albums = Vec::new();
    let mut artists = Vec::new();
    let mut playlists = Vec::new();

    let all_types = types.is_empty();

    if (all_types || types.contains(&"track".to_owned()))
        && let Some(res) = fetch_search(client, nuid, device_id, query, 1, limit).await
    {
        tracks = res.songs.iter().filter_map(parse_track).collect();
    }

    if (all_types || types.contains(&"album".to_owned()))
        && let Some(res) = fetch_search(client, nuid, device_id, query, 10, limit).await
    {
        albums = res.albums.iter().map(parse_album).collect();
    }

    if (all_types || types.contains(&"artist".to_owned()))
        && let Some(res) = fetch_search(client, nuid, device_id, query, 100, limit).await
    {
        artists = res.artists.iter().map(parse_artist).collect();
    }

    if (all_types || types.contains(&"playlist".to_owned()))
        && let Some(res) = fetch_search(client, nuid, device_id, query, 1000, limit).await
    {
        playlists = res.playlists.iter().map(parse_playlist).collect();
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

async fn fetch_search(
    client: &reqwest::Client,
    nuid: &str,
    device_id: &str,
    query: &str,
    type_id: i32,
    limit: usize,
) -> Option<SearchResultInner> {
    get_eapi_json::<SearchResultData>(
        client,
        "/api/cloudsearch/pc",
        json!({
            "s": query,
            "type": type_id,
            "limit": limit,
            "offset": 0,
            "total": true
        }),
        nuid,
        device_id,
    )
    .await
    .map(|d| d.result)
}

pub async fn fetch_recommendations(
    client: &reqwest::Client,
    nuid: &str,
    device_id: &str,
    identifier: &str,
) -> LoadResult {
    let id = identifier.trim();
    if !id.chars().all(|c| c.is_ascii_digit()) || id.is_empty() {
        return LoadResult::Empty {};
    }

    match get_eapi_json::<SimilarSongsData>(
        client,
        "/api/v1/discovery/simiSong",
        json!({ "songid": id }),
        nuid,
        device_id,
    )
    .await
    {
        Some(resp) if !resp.songs.is_empty() => {
            let tracks = resp.songs.iter().filter_map(parse_track).collect();
            LoadResult::Playlist(PlaylistData {
                info: PlaylistInfo {
                    name: format!("Similar to Track {}", id),
                    selected_track: -1,
                },
                plugin_info: json!({ "type": "recommendations", "seed_id": id }),
                tracks,
            })
        }
        _ => LoadResult::Empty {},
    }
}
