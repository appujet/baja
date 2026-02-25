//! `AudioMixer` — overlays multiple audio layers onto a main PCM stream.
//!
//! Mirrors NodeLink's `AudioMixer.ts`. Each layer has its own `RingBuffer`
//! and volume level. Useful for sound effects or secondary audio tracks.

use crate::audio::buffer::PooledBuffer;
use crate::audio::ring_buffer::RingBuffer;
use flume::Receiver;
use std::collections::HashMap;

const MAX_LAYERS: usize = 5;
const LAYER_BUFFER_SIZE: usize = 1024 * 1024; // 1 MB (~5.4s of PCM)

pub struct AudioMixer {
    pub layers: HashMap<String, Layer>,
    pub max_layers: usize,
    pub enabled: bool,
}

pub struct Layer {
    pub id: String,
    pub rx: Receiver<PooledBuffer>,
    pub ring_buffer: RingBuffer,
    pub volume: f32,
    pub finished: bool,
}

impl AudioMixer {
    pub fn new() -> Self {
        Self {
            layers: HashMap::new(),
            max_layers: MAX_LAYERS,
            enabled: true,
        }
    }

    /// Add a new audio layer.
    pub fn add_layer(
        &mut self,
        id: String,
        rx: Receiver<PooledBuffer>,
        volume: f32,
    ) -> Result<(), &'static str> {
        if self.layers.len() >= self.max_layers {
            return Err("Maximum mix layers reached");
        }

        let layer = Layer {
            id: id.clone(),
            rx,
            ring_buffer: RingBuffer::new(LAYER_BUFFER_SIZE),
            volume: volume.clamp(0.0, 1.0),
            finished: false,
        };

        self.layers.insert(id, layer);
        Ok(())
    }

    pub fn remove_layer(&mut self, id: &str) {
        self.layers.remove(id);
    }

    pub fn set_layer_volume(&mut self, id: &str, volume: f32) {
        if let Some(layer) = self.layers.get_mut(id) {
            layer.volume = volume.clamp(0.0, 1.0);
        }
    }

    /// Mix all active layers into the provided PCM frame.
    pub fn mix(&mut self, main_frame: &mut [i16]) {
        if !self.enabled || self.layers.is_empty() {
            return;
        }

        let sample_count = main_frame.len();
        let byte_count = sample_count * 2;

        // Collect samples from layers and sum them up
        // We'll use a temporary f32 accumulator to avoid intermediate clipping
        let mut i32_acc: Vec<i32> = main_frame.iter().map(|&s| s as i32).collect();

        // Drain dead layers
        self.layers.retain(|_, layer| {
            // Fill layer ring buffer from receiver
            while !layer.rx.is_empty() {
                if let Ok(pooled) = layer.rx.try_recv() {
                    let bytes = unsafe {
                        std::slice::from_raw_parts(pooled.as_ptr() as *const u8, pooled.len() * 2)
                    };
                    layer.ring_buffer.write(bytes);
                } else {
                    break;
                }
            }

            if layer.rx.is_disconnected() {
                layer.finished = true;
            }

            // If finished and buffer empty, remove it
            !(layer.finished && layer.ring_buffer.is_empty())
        });

        for layer in self.layers.values_mut() {
            if let Some(bytes) = layer.ring_buffer.read(byte_count) {
                // Safety: cast u8 vec back to i16 slice
                let samples = unsafe {
                    std::slice::from_raw_parts(bytes.as_ptr() as *const i16, bytes.len() / 2)
                };

                for (i, &s) in samples.iter().enumerate() {
                    if i < i32_acc.len() {
                        i32_acc[i] += (s as f32 * layer.volume).round() as i32;
                    }
                }
            }
        }

        // Clamp back to i16
        for (i, &sum) in i32_acc.iter().enumerate() {
            main_frame[i] = sum.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }
    }
}
