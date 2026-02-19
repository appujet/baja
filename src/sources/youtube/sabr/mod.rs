pub mod parser;
pub mod potoken;
pub mod reader;
pub mod structs;
pub mod writer;

pub enum UmpPartId {
    MediaHeader = 20,
    Media = 21,
    MediaEnd = 22,
    NextRequestPolicy = 35,
    FormatInitializationMetadata = 42,
    SabrRedirect = 43,
    SabrError = 44,
    ReloadPlayerResponse = 46,
    PlaybackStartPolicy = 47,
    RequestIdentifier = 52,
    RequestCancellationPolicy = 53,
    SabrContextUpdate = 57,
    StreamProtectionStatus = 58,
    SabrContextSendingPolicy = 59,
    SnackbarMessage = 67,
}

impl UmpPartId {
    pub fn from_u64(v: u64) -> Option<Self> {
        match v {
            20 => Some(Self::MediaHeader),
            21 => Some(Self::Media),
            22 => Some(Self::MediaEnd),
            35 => Some(Self::NextRequestPolicy),
            42 => Some(Self::FormatInitializationMetadata),
            43 => Some(Self::SabrRedirect),
            44 => Some(Self::SabrError),
            46 => Some(Self::ReloadPlayerResponse),
            47 => Some(Self::PlaybackStartPolicy),
            52 => Some(Self::RequestIdentifier),
            53 => Some(Self::RequestCancellationPolicy),
            57 => Some(Self::SabrContextUpdate),
            58 => Some(Self::StreamProtectionStatus),
            59 => Some(Self::SabrContextSendingPolicy),
            67 => Some(Self::SnackbarMessage),
            _ => None,
        }
    }
}
