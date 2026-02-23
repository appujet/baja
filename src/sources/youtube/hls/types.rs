use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct ByteRange {
    pub length: u64,
    pub offset: u64,
}

#[derive(Clone, Debug)]
pub struct Resource {
    pub url: String,
    pub range: Option<ByteRange>,
    /// Segment duration in seconds (from #EXTINF). None for map segments.
    pub duration: Option<f64>,
}

pub struct Variant {
    pub url: String,
    pub bandwidth: u64,
    pub codecs: String,
    /// True when CODECS contains audio codec but no video codec (avc1/hvc1 etc.)
    pub is_audio_only: bool,
    /// AUDIO group identifier
    pub audio_group: Option<String>,
}

pub struct Media {
    pub _type: String,
    pub _group_id: String,
    pub uri: Option<String>,
    pub is_default: bool,
}

pub enum M3u8Playlist {
    Master {
        variants: Vec<Variant>,
        audio_groups: HashMap<String, Vec<Media>>,
    },
    Media {
        segments: Vec<Resource>,
        map: Option<Resource>,
    },
}
