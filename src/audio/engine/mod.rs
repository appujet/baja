//! `engine/mod.rs` — audio engine abstraction.
//!
//! An `Engine` receives decoded, resampled PCM frames from `AudioProcessor`
//! and routes them towards Discord.  Two concrete implementations exist:
//!
//! | Engine | Description |
//! |---|---|
//! | [`PassthroughEngine`] | Raw Opus → Discord (zero transcode hot-path) |
//! | [`TranscodeEngine`] | PCM → `FlowController` → Encoder → Discord |

pub mod encoder;
pub mod passthrough;
pub mod transcode;

pub use encoder::Encoder;
pub use passthrough::PassthroughEngine;
pub use transcode::TranscodeEngine;

use crate::audio::buffer::PooledBuffer;

// ─── Engine output ────────────────────────────────────────────────────────────

/// What the engine sends downstream for each block of input.
pub enum EngineOutput {
    /// Processed PCM i16 samples (the normal transcode path).
    Pcm(PooledBuffer),
    /// Raw encoded Opus bytes (passthrough path — skip the encoder).
    Opus(std::sync::Arc<Vec<u8>>),
    /// The engine consumed the input but produced nothing this block
    /// (e.g. buffering, silence suppression).
    None,
}

// ─── Engine trait ─────────────────────────────────────────────────────────────

/// Abstraction over the two output strategies.
///
/// `AudioProcessor` calls `push_pcm` for every decoded + resampled PCM block.
/// Passthrough engines ignore PCM and instead read from their own Opus source.
pub trait Engine: Send {
    /// Push a decoded PCM block into the engine.
    ///
    /// Blocks until the downstream channel accepts the data (natural
    /// back-pressure).  Returns `false` when the downstream has disconnected
    /// and the caller should exit.
    fn push_pcm(&mut self, pcm: PooledBuffer) -> bool;

    /// Push a raw Opus packet (only meaningful for `PassthroughEngine`).
    fn push_opus(&mut self, _packet: std::sync::Arc<Vec<u8>>) -> bool {
        true // default: ignore Opus
    }
}

/// Type-erased engine — owns and drives any `Engine` implementation.
pub type BoxedEngine = Box<dyn Engine>;
