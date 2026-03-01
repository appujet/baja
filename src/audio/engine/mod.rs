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

// ─── Engine trait ─────────────────────────────────────────────────────────────

/// Abstraction over the two output strategies.
///
/// `AudioProcessor` calls `push_pcm` for every decoded + resampled PCM block.
/// An empty `pcm` is treated as a seek-flush sentinel by the downstream
/// `FlowController` (it clears stale pre-seek audio from `pending_pcm`).
pub trait Engine: Send {
    /// Push a decoded PCM block into the engine.
    ///
    /// An empty `Vec` acts as a seek-flush sentinel — the call still returns
    /// `true` (connected) even if the sentinel is silently discarded by a
    /// passthrough engine.
    fn push_pcm(&mut self, pcm: PooledBuffer) -> bool;

    /// Push a raw Opus packet (only meaningful for `PassthroughEngine`).
    fn push_opus(&mut self, _packet: std::sync::Arc<Vec<u8>>) -> bool {
        true // default: ignore Opus
    }
}

/// Type-erased engine — owns and drives any `Engine` implementation.
pub type BoxedEngine = Box<dyn Engine>;
