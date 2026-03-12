use flume::{Receiver, Sender};

use crate::audio::{
    AudioFrame,
    buffer::{PooledBuffer, acquire_buffer},
    constants::FRAME_SIZE_SAMPLES,
    effects::{
        crossfade::CrossfadeController, fade::FadeEffect, tape::TapeEffect, volume::VolumeEffect,
    },
    error::AudioError,
};

pub struct FlowController {
    pub tape: TapeEffect,
    pub volume: VolumeEffect,
    pub fade: FadeEffect,
    pub crossfade: CrossfadeController,
    pending_pcm: Vec<i16>,
    decoder_done: bool,
    frame_rx: Receiver<AudioFrame>,
    frame_tx: Option<Sender<AudioFrame>>,
    latest_opus: Option<Vec<u8>>,
}

impl FlowController {
    pub fn new(
        frame_rx: Receiver<AudioFrame>,
        frame_tx: Sender<AudioFrame>,
        sample_rate: u32,
        channels: usize,
        volume: f32,
    ) -> Self {
        Self::build(frame_rx, Some(frame_tx), sample_rate, channels, volume)
    }

    pub fn for_mixer(
        frame_rx: Receiver<AudioFrame>,
        sample_rate: u32,
        channels: usize,
        volume: f32,
    ) -> Self {
        Self::build(frame_rx, None, sample_rate, channels, volume)
    }

    fn build(
        frame_rx: Receiver<AudioFrame>,
        frame_tx: Option<Sender<AudioFrame>>,
        sample_rate: u32,
        channels: usize,
        volume: f32,
    ) -> Self {
        Self {
            tape: TapeEffect::new(sample_rate, channels),
            volume: VolumeEffect::new(volume, sample_rate, channels),
            fade: FadeEffect::new(1.0, channels),
            crossfade: CrossfadeController::new(sample_rate, channels),
            pending_pcm: Vec::with_capacity(FRAME_SIZE_SAMPLES * 2),
            decoder_done: false,
            frame_rx,
            frame_tx,
            latest_opus: None,
        }
    }

    pub fn run(&mut self) {
        while let Ok(frame_data) = self.frame_rx.recv() {
            match frame_data {
                AudioFrame::Pcm(pooled) => {
                    self.pending_pcm.extend_from_slice(&pooled);

                    while self.pending_pcm.len() >= FRAME_SIZE_SAMPLES {
                        let mut frame = acquire_buffer(FRAME_SIZE_SAMPLES);
                        frame.extend(self.pending_pcm.drain(..FRAME_SIZE_SAMPLES));
                        self.process_frame(&mut frame);

                        if self
                            .frame_tx
                            .as_ref()
                            .is_some_and(|tx| tx.send(AudioFrame::Pcm(frame)).is_err())
                        {
                            return;
                        }
                    }
                }
                AudioFrame::Opus(packet) => {
                    if let Some(tx) = &self.frame_tx
                        && tx.send(AudioFrame::Opus(packet)).is_err()
                    {
                        return;
                    }
                }
            }
        }
    }

    /// Attempts to retrieve the next ready PCM frame from the controller without blocking.
    ///
    /// While the decoder is still active, this method consumes available incoming frames from the internal
    /// receiver into an internal buffer until at least `FRAME_SIZE_SAMPLES` samples are accumulated
    /// or the receiver becomes empty or disconnected. Incoming Opus packets are saved as the latest
    /// opus packet and incoming PCM chunks are appended to the pending buffer and then released back
    /// to the buffer pool. If enough PCM samples are available, a new `PooledBuffer` containing the
    /// next `FRAME_SIZE_SAMPLES` samples is returned after applying the controller's audio effects.
    /// If the decoder has finished and no full frame is available, returns `AudioError::DecoderFinished`.
    /// Otherwise, returns `Ok(None)` to indicate that no complete frame is ready yet.
    ///
    /// # Returns
    ///
    /// `Ok(Some(frame))` with a processed `PooledBuffer` when a full frame was produced, `Ok(None)` when
    /// no full frame is available but the decoder is still active, or `Err(AudioError::DecoderFinished)`
    /// when the decoder has finished and no more frames can be produced.
    ///
    /// # Examples
    ///
    /// ```
    /// // Assume `fc` is a mutable FlowController already receiving audio.
    /// if let Ok(Some(frame)) = fc.try_pop_frame() {
    ///     // `frame` contains FRAME_SIZE_SAMPLES processed PCM samples ready for consumption.
    /// } else {
    ///     // No full frame ready yet, or decoder finished.
    /// }
    /// ```
    pub fn try_pop_frame(&mut self) -> Result<Option<PooledBuffer>, AudioError> {
        if !self.decoder_done {
            while self.pending_pcm.len() < FRAME_SIZE_SAMPLES {
                match self.frame_rx.try_recv() {
                    Ok(AudioFrame::Pcm(chunk)) if chunk.is_empty() => {
                        self.pending_pcm.clear();
                        self.decoder_done = false;
                    }
                    Ok(AudioFrame::Pcm(chunk)) => {
                        self.pending_pcm.extend_from_slice(&chunk);
                        crate::audio::buffer::release_buffer(chunk);
                    }
                    Ok(AudioFrame::Opus(packet)) => {
                        self.latest_opus = Some(packet);
                    }
                    Err(flume::TryRecvError::Empty) => break,
                    Err(flume::TryRecvError::Disconnected) => {
                        self.decoder_done = true;
                        break;
                    }
                }
            }
        }

        if self.pending_pcm.len() >= FRAME_SIZE_SAMPLES {
            let mut frame = acquire_buffer(FRAME_SIZE_SAMPLES);
            frame.extend(self.pending_pcm.drain(..FRAME_SIZE_SAMPLES));
            self.process_frame(&mut frame);
            Ok(Some(frame))
        } else if self.decoder_done {
            Err(AudioError::DecoderFinished)
        } else {
            Ok(None)
        }
    }

    pub fn process_frame(&mut self, frame: &mut [i16]) {
        self.tape.process(frame);
        self.volume.process(frame);
        self.fade.process(frame);

        self.crossfade.fill_buffer();
        if self.crossfade.is_active() {
            self.crossfade.process(frame);
        }
    }

    pub fn take_opus(&mut self) -> Option<Vec<u8>> {
        self.latest_opus.take()
    }
}
