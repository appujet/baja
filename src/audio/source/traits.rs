use std::io::{Read, Seek};

use symphonia::core::io::MediaSource;

pub trait AudioSource: Read + Seek + MediaSource + Send {
    fn content_type(&self) -> Option<String> {
        None
    }

    fn seekable(&self) -> bool {
        self.is_seekable()
    }
}
