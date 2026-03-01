pub mod pool;
pub mod ring;

pub use pool::{BufferPool, get_byte_pool};
pub use ring::RingBuffer;

/// PCM buffer type: a plain `Vec<i16>` passed between the AudioProcessor
/// and the Mixer.  No custom pool needed — small fixed-size allocations.
pub type PooledBuffer = Vec<i16>;
