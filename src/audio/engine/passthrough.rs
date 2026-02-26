//! `engine/passthrough.rs` — zero-transcode Opus passthrough engine.
//!
//! Receives raw Opus packets directly from the demuxer (WebM/Opus source) and
//! forwards them to the Mixer's passthrough channel without decoding or
//! re-encoding.  This is the hot-path for YouTube WebM/Opus streams.

use flume::Sender;
use std::sync::Arc;

use super::Engine;
use crate::audio::buffer::PooledBuffer;

/// Sends raw Opus packets directly to the Mixer's passthrough lane.
///
/// `push_pcm` is a no-op — PCM data is irrelevant for this engine.
pub struct PassthroughEngine {
    opus_tx: Sender<Arc<Vec<u8>>>,
}

impl PassthroughEngine {
    pub fn new(opus_tx: Sender<Arc<Vec<u8>>>) -> Self {
        Self { opus_tx }
    }
}

impl Engine for PassthroughEngine {
    /// PCM is not used in passthrough mode — always returns `true`.
    fn push_pcm(&mut self, _pcm: PooledBuffer) -> bool {
        true
    }

    /// Forward a raw Opus packet to the Mixer.  Returns `false` when the
    /// downstream channel has closed (caller should exit).
    fn push_opus(&mut self, packet: Arc<Vec<u8>>) -> bool {
        self.opus_tx.send(packet).is_ok()
    }
}
