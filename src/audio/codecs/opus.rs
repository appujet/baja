use audiopus::{
    Channels, MutSignals, SampleRate,
    coder::Decoder as OpusDecoder,
    packet::Packet,
};
use symphonia::core::{
    audio::{AsAudioBufferRef, AudioBuffer, AudioBufferRef, Layout, Signal, SignalSpec},
    codecs::{
        CODEC_TYPE_OPUS, CodecDescriptor, CodecParameters, Decoder,
        DecoderOptions, FinalizeResult,
    },
    errors::{Error, Result},
    formats::Packet as SymphPacket,
    units::Duration,
};

/// Max decoded samples per opus frame at 48 kHz: 120 ms → 5760 samples/channel.
const MAX_FRAME_SIZE: usize = 5760;

pub struct OpusCodecDecoder {
    params: CodecParameters,
    channels: usize,
    decoder: OpusDecoder,
    buf: AudioBuffer<i16>,
    /// Reusable interleaved scratch buffer — avoids per-frame heap allocs.
    pcm: Vec<i16>,
}

// audiopus::coder::Decoder is Send but not Sync.
// We only touch it via `&mut self`, so Sync is safe.
unsafe impl Sync for OpusCodecDecoder {}

impl Decoder for OpusCodecDecoder {
    fn try_new(params: &CodecParameters, _options: &DecoderOptions) -> Result<Self> {
        if params.codec != CODEC_TYPE_OPUS {
            return Err(Error::Unsupported("not an opus stream"));
        }

        let sample_rate = params.sample_rate.unwrap_or(48000);
        let channels = params
            .channels
            .map(|c| c.count())
            .unwrap_or(2)
            .clamp(1, 2);

        let opus_channels = if channels == 1 { Channels::Mono } else { Channels::Stereo };

        let opus_rate = match sample_rate {
            8000  => SampleRate::Hz8000,
            12000 => SampleRate::Hz12000,
            16000 => SampleRate::Hz16000,
            24000 => SampleRate::Hz24000,
            _     => SampleRate::Hz48000,
        };

        let decoder = OpusDecoder::new(opus_rate, opus_channels)
            .map_err(|e| Error::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                e.to_string(),
            )))?;

        let layout = if channels == 1 { Layout::Mono } else { Layout::Stereo };
        let spec = SignalSpec::new_with_layout(sample_rate, layout);
        let buf = AudioBuffer::<i16>::new(MAX_FRAME_SIZE as Duration, spec);
        // Pre-allocate scratch buffer once; all zeroes is a safe initial state.
        let pcm = vec![0i16; MAX_FRAME_SIZE * channels];

        Ok(Self { params: params.clone(), channels, decoder, buf, pcm })
    }

    fn supported_codecs() -> &'static [CodecDescriptor] {
        &[CodecDescriptor {
            codec: CODEC_TYPE_OPUS,
            short_name: "opus",
            long_name: "Opus (via audiopus)",
            inst_func: |params, opts| {
                Ok(Box::new(OpusCodecDecoder::try_new(params, opts)?))
            },
        }]
    }

    fn reset(&mut self) {
        let channels = if self.channels == 1 { Channels::Mono } else { Channels::Stereo };
        if let Ok(dec) = OpusDecoder::new(SampleRate::Hz48000, channels) {
            self.decoder = dec;
        }
    }

    fn codec_params(&self) -> &CodecParameters {
        &self.params
    }

    fn decode(&mut self, packet: &SymphPacket) -> Result<AudioBufferRef<'_>> {
        // Decode into pre-allocated interleaved scratch buffer.
        let n = self
            .decoder
            .decode(
                Packet::try_from(packet.data.as_ref()).ok(),
                MutSignals::try_from(self.pcm.as_mut_slice()).map_err(|e| {
                    Error::IoError(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
                })?,
                false,
            )
            .map_err(|e| {
                Error::IoError(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
            })?;

        // Deinterleave into symphonia's planar AudioBuffer.
        self.buf.clear();
        self.buf.render_reserved(Some(n));

        let ch = self.channels;
        for c in 0..ch {
            let plane = self.buf.chan_mut(c);
            debug_assert_eq!(plane.len(), n, "plane length must equal decoded frame size");
            for (i, s) in plane.iter_mut().enumerate() {
                *s = self.pcm[i * ch + c];
            }
        }

        Ok(self.buf.as_audio_buffer_ref())
    }

    fn finalize(&mut self) -> FinalizeResult {
        FinalizeResult::default()
    }

    fn last_decoded(&self) -> AudioBufferRef<'_> {
        self.buf.as_audio_buffer_ref()
    }
}
