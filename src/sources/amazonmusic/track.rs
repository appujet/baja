use crate::protocol::tracks::TrackInfo;

pub struct AmazonTrackData {
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub duration_ms: u64,
    pub artwork_url: Option<String>,
    pub isrc: Option<String>,
}

impl AmazonTrackData {
    pub fn into_track_info(self) -> TrackInfo {
        let uri = format!("https://music.amazon.com/tracks/{}", self.track_id);
        TrackInfo {
            identifier: self.track_id,
            is_seekable: true,
            author: self.artist,
            length: self.duration_ms,
            is_stream: false,
            position: 0,
            title: self.title,
            uri: Some(uri),
            artwork_url: self.artwork_url,
            isrc: self.isrc,
            source_name: "amazonmusic".to_string(),
        }
    }
}
