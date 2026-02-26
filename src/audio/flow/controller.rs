//! `FlowController` — the central PCM processing hub.
//!
//! Reassembles arbitrary PCM chunks into fixed 3840-byte (960 sample) frames
//! and pipes them through the effects chain: Filters → Tape → Volume → Fade.
//! Mirrors NodeLink's `FlowController.ts`.

use crate::audio::buffer::PooledBuffer;
use crate::audio::effects::{
    crossfade::CrossfadeController, fade::FadeEffect, tape::TapeEffect, volume::VolumeEffect,
};
use crate::audio::filters::FilterChain;

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
    /// Set to `true` once the decoder's sender is dropped.
    /// Allows the last partial frame to be retired cleanly.
    decoder_done: bool,

    // I/O
    pcm_rx: Receiver<PooledBuffer>,
    /// Output channel — `None` when used in pull mode (e.g. inside `Mixer`).
    pcm_tx: Option<Sender<PooledBuffer>>,

    _sample_rate: u32,
    _channels: usize,
}

impl FlowController {
    /// Push mode: decoded PCM flows `pcm_rx → effects → pcm_tx`.
    /// Use `run()` to drive the loop on a dedicated thread.
    pub fn new(
        pcm_rx: Receiver<PooledBuffer>,
        pcm_tx: Sender<PooledBuffer>,
        sample_rate: u32,
        channels: usize,
    ) -> Self {
        Self::build(pcm_rx, Some(pcm_tx), sample_rate, channels)
    }

    /// Pull mode: the `Mixer` calls `try_pop_frame()` each tick.
    /// No output channel is allocated.
    pub fn for_mixer(pcm_rx: Receiver<PooledBuffer>, sample_rate: u32, channels: usize) -> Self {
        Self::build(pcm_rx, None, sample_rate, channels)
    }

    fn build(
        pcm_rx: Receiver<PooledBuffer>,
        pcm_tx: Option<Sender<PooledBuffer>>,
        sample_rate: u32,
        channels: usize,
    ) -> Self {
        Self {
            tape: TapeEffect::new(sample_rate, channels),
            volume: VolumeEffect::new(1.0, sample_rate, channels),
            fade: FadeEffect::new(1.0, channels),
            crossfade: CrossfadeController::new(sample_rate, channels),
            filters: None,
            pending_pcm: Vec::with_capacity(FRAME_SIZE_SAMPLES * 2),
            decoder_done: false,
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
            self.pending_pcm.extend_from_slice(&pooled);

            while self.pending_pcm.len() >= FRAME_SIZE_SAMPLES {
                let mut frame: PooledBuffer = Vec::with_capacity(FRAME_SIZE_SAMPLES);
                frame.extend(self.pending_pcm.drain(..FRAME_SIZE_SAMPLES));
                self.process_frame(&mut frame);

                if let Some(tx) = &self.pcm_tx {
                    if tx.send(frame).is_err() {
                        return; // Downstream disconnected
                    }
                }
            }
        }
    }

    /// Pull-based variant for use inside the `Mixer` tick.
    ///
    /// Drains the channel only until `pending_pcm` has one full frame's worth
    /// of data — this preserves backpressure so the decoder runs at real-time
    /// pace rather than buffering the entire file into memory.
    ///
    /// Returns:
    /// - `Ok(Some(frame))` — a processed 960-sample (1920 i16) frame is ready
    /// - `Ok(None)`        — not enough data yet; call again next tick
    /// - `Err(())`         — decoder finished and no full frame remains
    pub fn try_pop_frame(&mut self) -> Result<Option<PooledBuffer>, ()> {
        // Only drain from the channel when we don't yet have enough for one frame.
        // This keeps the channel bounded and the decoder at ~real-time speed.
        if !self.decoder_done {
            while self.pending_pcm.len() < FRAME_SIZE_SAMPLES {
                match self.pcm_rx.try_recv() {
                    Ok(chunk) => self.pending_pcm.extend_from_slice(&chunk),
                    Err(flume::TryRecvError::Empty) => break,
                    Err(flume::TryRecvError::Disconnected) => {
                        self.decoder_done = true;
                        break;
                    }
                }
            }
        }

        if self.pending_pcm.len() >= FRAME_SIZE_SAMPLES {
            let mut frame: PooledBuffer = Vec::with_capacity(FRAME_SIZE_SAMPLES);
            frame.extend(self.pending_pcm.drain(..FRAME_SIZE_SAMPLES));
            self.process_frame(&mut frame);
            Ok(Some(frame))
        } else if self.decoder_done {
            // Decoder is done and we don't have enough for a full frame.
            // The last partial frame (< 20 ms) is silently discarded.
            Err(())
        } else {
            Ok(None) // Not enough data yet — try again next tick.
        }
    }

    /// Apply the full effects chain to a single 960-sample frame.
    pub fn process_frame(&mut self, frame: &mut [i16]) {
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
