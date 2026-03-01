use std::io::{Read, Seek};

use symphonia::core::io::MediaSource;

/// Common trait implemented by every readable audio source.
///
/// Combines `Read + Seek + MediaSource` with metadata accessors.
pub trait AudioSource: Read + Seek + MediaSource + Send {
    /// MIME / content-type of the stream, if known.
    fn content_type(&self) -> Option<String> {
        None
    }

    /// Whether the source supports seeking.
    fn seekable(&self) -> bool {
        self.is_seekable()
    }
}
