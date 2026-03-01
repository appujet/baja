//! SABR (Streaming ABR) implementation for YouTube's WEB client.
//!
//! YouTube's SABR protocol works by:
//! 1. Client POSTs a protobuf `VideoPlaybackAbrRequest` to `serverAbrStreamingUrl?alr=yes&ump=1&rn=N`
//! 2. Server responds with a UMP (Universal Media Protocol) binary stream
//! 3. Client parses UMP parts: MEDIA_HEADER, MEDIA, MEDIA_END deliver audio bytes;
//!    NEXT_REQUEST_POLICY delivers the playbackCookie + backoff timing
//! 4. Loop continues until stream ends (endSegmentNumber reached)
//!
//! PoToken is fetched from the optional `yt-cipher` service and embedded
//! in `StreamerContext` (field 19) of every request.

pub mod config;
pub mod player;
pub mod pot_client;
pub mod proto;
pub mod reader;
pub mod stream;

pub use config::SabrConfig;
pub use player::fetch_sabr_config;
pub use reader::SabrReader;
pub use stream::{best_format_mime, start_sabr_stream};
