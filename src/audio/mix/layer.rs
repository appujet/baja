//! `MixLayer` — a single named audio layer with its own `RingBuffer`.
//!
//! Layers are blended into the main PCM stream by `AudioMixer`.

use flume::Receiver;

use crate::audio::{RingBuffer, buffer::PooledBuffer, constants::LAYER_BUFFER_SIZE};

/// A single overlay layer: feeds PCM from a channel into a `RingBuffer` and
/// blends it into the main stream at a configurable volume.
pub struct MixLayer {
    pub id: String,
    pub rx: Receiver<PooledBuffer>,
    /// Circular buffer that decouples the producer rate from the mix tick.
    pub ring_buffer: RingBuffer,
    /// Blend volume in [0.0, 1.0].
    pub volume: f32,
    /// Set to `true` once the sender has disconnected and the buffer is drained.
    pub finished: bool,
}

impl MixLayer {
    pub fn new(id: String, rx: Receiver<PooledBuffer>, volume: f32) -> Self {
        Self {
            id,
            rx,
            ring_buffer: RingBuffer::new(LAYER_BUFFER_SIZE),
            volume: volume.clamp(0.0, 1.0),
            finished: false,
        }
    }

    /// Drain new frames from the channel into the ring buffer.
    pub fn fill(&mut self) {
        while let Ok(pooled) = self.rx.try_recv() {
            // SAFETY: i16 slice reinterpreted as u8 bytes for RingBuffer storage.
            let bytes = unsafe {
                std::slice::from_raw_parts(pooled.as_ptr() as *const u8, pooled.len() * 2)
            };
            self.ring_buffer.write(bytes);
        }
        if self.rx.is_disconnected() {
            self.finished = true;
        }
    }

    /// Return `true` if the layer is fully drained and can be removed.
    pub fn is_dead(&self) -> bool {
        self.finished && self.ring_buffer.is_empty()
    }

    /// Accumulate this layer's next `sample_count` samples into `acc` (i32).
    pub fn accumulate(&mut self, acc: &mut [i32]) {
        let byte_count = acc.len() * 2;
        if let Some(bytes) = self.ring_buffer.read(byte_count) {
            // SAFETY: u8 bytes reinterpreted as i16 samples.
            let samples = unsafe {
                std::slice::from_raw_parts(bytes.as_ptr() as *const i16, bytes.len() / 2)
            };
            for (i, &s) in samples.iter().enumerate() {
                if i < acc.len() {
                    acc[i] += (s as f32 * self.volume).round() as i32;
                }
            }
        }
    }
}
