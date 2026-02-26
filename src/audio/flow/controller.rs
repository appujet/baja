//! `FlowController` — the central PCM processing hub.
//!
//! Reassembles arbitrary PCM chunks into fixed 3840-byte (960 sample) frames
//! and pipes them through the effects chain: Filters → Tape → Volume → Fade.
//! Mirrors NodeLink's `FlowController.ts`.

use crate::audio::buffer::PooledBuffer;
use crate::audio::filters::FilterChain;
use crate::audio::playback::effects::{
    crossfade::CrossfadeController, fade::FadeEffect, tape::TapeEffect, volume::VolumeEffect,
};
use flume::{Receiver, Sender};

use crate::audio::constants::FRAME_SIZE_SAMPLES;

pub struct FlowController {
    // Effects chain
    pub tape: TapeEffect,
    pub volume: VolumeEffect,
    pub fade: FadeEffect,
    pub crossfade: CrossfadeController,
    pub filters: Option<FilterChain>,

    // Accumulation buffer for re-framing
    pending_pcm: Vec<i16>,

    // I/O
    pcm_rx: Receiver<PooledBuffer>,
    pub pcm_tx: Sender<PooledBuffer>,

    _sample_rate: u32,
    _channels: usize,
}

impl FlowController {
    pub fn new(
        pcm_rx: Receiver<PooledBuffer>,
        pcm_tx: Sender<PooledBuffer>,
        sample_rate: u32,
        channels: usize,
    ) -> Self {
        Self {
            tape: TapeEffect::new(sample_rate, channels),
            volume: VolumeEffect::new(1.0, sample_rate, channels),
            fade: FadeEffect::new(1.0, channels),
            crossfade: CrossfadeController::new(sample_rate, channels),
            filters: None,
            pending_pcm: Vec::with_capacity(FRAME_SIZE_SAMPLES),
            pcm_rx,
            pcm_tx,
            _sample_rate: sample_rate,
            _channels: channels,
        }
    }

    /// Run the processing loop. Pulls from `pcm_rx`, assembles frames,
    /// applies effects, and pushes to `pcm_tx`.
    pub fn run(&mut self) {
        while let Ok(pooled) = self.pcm_rx.recv() {
            // 1. Accumulate into pending buffer
            self.pending_pcm.extend_from_slice(&pooled);

            // 2. Process all complete frames
            while self.pending_pcm.len() >= FRAME_SIZE_SAMPLES {
                let mut frame: PooledBuffer = Vec::with_capacity(FRAME_SIZE_SAMPLES);
                frame.extend(self.pending_pcm.drain(..FRAME_SIZE_SAMPLES));

                self.process_frame(&mut frame);

                if self.pcm_tx.send(frame).is_err() {
                    return; // Downstream disconnected
                }
            }
        }
    }

    /// Apply the full effects chain to a single 960-sample frame.
    fn process_frame(&mut self, frame: &mut [i16]) {
        // A. Filters (e.g. EQ, Reverb)
        if let Some(filters) = &mut self.filters {
            filters.process(frame);
        }

        // B. Tape Effect (Pitch/Speed ramp)
        self.tape.process(frame);

        // C. Volume (Gain + Soft Limiter)
        self.volume.process(frame);

        // D. Fade (Crossfade ramp)
        self.fade.process(frame);

        // E. Crossfade Mix (Blend with buffered next track)
        self.crossfade.fill_buffer();
        if self.crossfade.is_active() {
            self.crossfade.process(frame);
        }

        // F. Multi-layer Mixing (handled in Mixer, or we could do it here)
        // In NodeLink, FlowController calls AudioMixer.mixBuffers.
        // We'll keep layers in the Mixer for now as it's closer to the Discord send,
        // unless multi-track effects are needed.
    }
}
