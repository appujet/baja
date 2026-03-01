//! `engine/transcode.rs` — PCM → FlowController → Mixer transcode engine.
//!
//! Receives decoded, resampled PCM i16 blocks from `AudioProcessor` and sends
//! them downstream to the `FlowController` (effects chain) via a bounded
//! flume channel.  The Mixer pulls processed frames from the FlowController
//! every 20 ms using `try_pop_frame()`.

use flume::Sender;

use super::Engine;
use crate::audio::buffer::PooledBuffer;

/// Sends PCM blocks into the `FlowController`'s input channel.
///
/// Back-pressure is provided naturally by the bounded channel (capacity set
/// by the caller; default: 64 frames ≈ 1.3 s of look-ahead).
pub struct TranscodeEngine {
    pcm_tx: Sender<PooledBuffer>,
}

impl TranscodeEngine {
    /// Create a new engine that writes into `pcm_tx`.
    ///
    /// `pcm_tx` should be the *Receiver* end of the channel passed to
    /// `FlowController::for_mixer()`.
    pub fn new(pcm_tx: Sender<PooledBuffer>) -> Self {
        Self { pcm_tx }
    }
}

impl Engine for TranscodeEngine {
    /// Send a PCM block downstream.  Blocks until there is space in the
    /// channel (natural back-pressure).  Returns `false` when the channel
    /// receiver has been dropped (downstream disconnected).
    fn push_pcm(&mut self, pcm: PooledBuffer) -> bool {
        self.pcm_tx.send(pcm).is_ok()
    }
}
