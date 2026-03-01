//! PCM resampling — two quality modes:
//!
//! | Type | Quality | CPU Cost |
//! |---|---|---|
//! | [`LinearResampler`] | Okay (fast path) | Very low |
//! | [`HermiteResampler`] | High (Catmull-Rom) | Low-medium |

pub mod hermite;
pub mod linear;
pub mod sinc;

pub use hermite::HermiteResampler;
pub use linear::LinearResampler;
pub use sinc::SincResampler;

use crate::audio::buffer::PooledBuffer;

/// Unified resampler enum — choose the quality at construction time.
pub enum Resampler {
    Linear(LinearResampler),
    Hermite(HermiteResampler),
    Sinc(SincResampler),
}

impl Resampler {
    /// High-quality Cubic Hermite resampler (recommended for music).
    pub fn hermite(source_rate: u32, target_rate: u32, channels: usize) -> Self {
        Self::Hermite(HermiteResampler::new(source_rate, target_rate, channels))
    }

    /// Fast linear resampler (good enough for voice / low-stakes audio).
    pub fn linear(source_rate: u32, target_rate: u32, channels: usize) -> Self {
        Self::Linear(LinearResampler::new(source_rate, target_rate, channels))
    }

    /// Professional-grade Blackman-windowed sinc resampler.
    pub fn sinc(source_rate: u32, target_rate: u32, channels: usize) -> Self {
        Self::Sinc(SincResampler::new(source_rate, target_rate, channels))
    }

    /// Returns `true` if no conversion is needed (source == target rate).
    pub fn is_passthrough(&self) -> bool {
        match self {
            Self::Linear(r) => r.is_passthrough(),
            Self::Hermite(r) => r.is_passthrough(),
            Self::Sinc(r) => r.is_passthrough(),
        }
    }

    /// Resample `input` and append resampled i16 samples into `output`.
    pub fn process(&mut self, input: &[i16], output: &mut PooledBuffer) {
        match self {
            Self::Linear(r) => r.process(input, output),
            Self::Hermite(r) => r.process(input, output),
            Self::Sinc(r) => r.process(input, output),
        }
    }

    pub fn reset(&mut self) {
        match self {
            Self::Linear(r) => r.reset(),
            Self::Hermite(r) => r.reset(),
            Self::Sinc(r) => r.reset(),
        }
    }
}
