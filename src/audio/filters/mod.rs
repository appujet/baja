pub mod biquad;
pub mod channel_mix;
pub mod delay_line;
pub mod distortion;
pub mod equalizer;
pub mod karaoke;
pub mod lfo;
pub mod low_pass;
pub mod rotation;
pub mod timescale;
pub mod tremolo;
pub mod vibrato;
pub mod volume;

use crate::playback::Filters;
use crate::config::FiltersConfig;

/// Validate if the requested filters are allowed by the server configuration.
/// Returns a list of disabled filter names that were requested.
pub fn validate_filters(filters: &Filters, config: &FiltersConfig) -> Vec<String> {
    let mut invalid = Vec::new();
    let enabled = &config.enabled;

    if filters.volume.is_some() && !enabled.volume {
        invalid.push("volume".to_string());
    }
    if filters.equalizer.is_some() && !enabled.equalizer {
        invalid.push("equalizer".to_string());
    }
    if filters.karaoke.is_some() && !enabled.karaoke {
        invalid.push("karaoke".to_string());
    }
    if filters.timescale.is_some() && !enabled.timescale {
        invalid.push("timescale".to_string());
    }
    if filters.tremolo.is_some() && !enabled.tremolo {
        invalid.push("tremolo".to_string());
    }
    if filters.vibrato.is_some() && !enabled.vibrato {
        invalid.push("vibrato".to_string());
    }
    if filters.distortion.is_some() && !enabled.distortion {
        invalid.push("distortion".to_string());
    }
    if filters.rotation.is_some() && !enabled.rotation {
        invalid.push("rotation".to_string());
    }
    if filters.channel_mix.is_some() && !enabled.channel_mix {
        invalid.push("channelMix".to_string());
    }
    if filters.low_pass.is_some() && !enabled.low_pass {
        invalid.push("lowPass".to_string());
    }

    invalid
}

/// Trait for audio filters that process interleaved stereo i16 PCM samples.
/// Buffer layout: [L, R, L, R, ...] — 960 frames × 2 channels = 1920 samples per 20ms.
pub trait AudioFilter: Send {
    /// Process samples in-place.
    fn process(&mut self, samples: &mut [i16]);
    /// Whether this filter is currently active (non-default params).
    fn is_enabled(&self) -> bool;
    /// Reset internal state (on seek or filter change).
    fn reset(&mut self);
}

/// An ordered chain of audio filters, constructed from Lavalink API `Filters`.
pub struct FilterChain {
    filters: Vec<Box<dyn AudioFilter>>,
    /// Timescale filter handled separately (changes buffer length).
    timescale: Option<timescale::TimescaleFilter>,
    /// Residual buffer for timescale output (feeds fixed-size 1920-sample frames).
    timescale_buffer: Vec<i16>,
}

impl FilterChain {
    /// Build a filter chain from the Lavalink API `Filters` config.
    pub fn from_config(config: &Filters) -> Self {
        let mut filters: Vec<Box<dyn AudioFilter>> = Vec::new();

        // Volume (applied first)
        if let Some(vol) = config.volume {
            let f = volume::VolumeFilter::new(vol);
            if f.is_enabled() {
                filters.push(Box::new(f));
            }
        }

        // Equalizer
        if let Some(ref bands) = config.equalizer {
            let band_tuples: Vec<(u8, f32)> = bands.iter().map(|b| (b.band, b.gain)).collect();
            let f = equalizer::EqualizerFilter::new(&band_tuples);
            if f.is_enabled() {
                filters.push(Box::new(f));
            }
        }

        // Karaoke
        if let Some(ref k) = config.karaoke {
            let f = karaoke::KaraokeFilter::new(
                k.level.unwrap_or(1.0),
                k.mono_level.unwrap_or(1.0),
                k.filter_band.unwrap_or(220.0),
                k.filter_width.unwrap_or(100.0),
            );
            if f.is_enabled() {
                filters.push(Box::new(f));
            }
        }

        // Tremolo
        if let Some(ref t) = config.tremolo {
            let f = tremolo::TremoloFilter::new(
                t.frequency.unwrap_or(2.0),
                t.depth.unwrap_or(0.5),
            );
            if f.is_enabled() {
                filters.push(Box::new(f));
            }
        }

        // Vibrato
        if let Some(ref v) = config.vibrato {
            let f = vibrato::VibratoFilter::new(
                v.frequency.unwrap_or(2.0),
                v.depth.unwrap_or(0.5),
            );
            if f.is_enabled() {
                filters.push(Box::new(f));
            }
        }

        // Rotation
        if let Some(ref r) = config.rotation {
            let f = rotation::RotationFilter::new(r.rotation_hz.unwrap_or(0.0));
            if f.is_enabled() {
                filters.push(Box::new(f));
            }
        }

        // Distortion
        if let Some(ref d) = config.distortion {
            let f = distortion::DistortionFilter::new(
                d.sin_offset.unwrap_or(0.0),
                d.sin_scale.unwrap_or(1.0),
                d.cos_offset.unwrap_or(0.0),
                d.cos_scale.unwrap_or(1.0),
                d.tan_offset.unwrap_or(0.0),
                d.tan_scale.unwrap_or(1.0),
                d.offset.unwrap_or(0.0),
                d.scale.unwrap_or(1.0),
            );
            if f.is_enabled() {
                filters.push(Box::new(f));
            }
        }

        // Channel Mix
        if let Some(ref cm) = config.channel_mix {
            let f = channel_mix::ChannelMixFilter::new(
                cm.left_to_left.unwrap_or(1.0),
                cm.left_to_right.unwrap_or(0.0),
                cm.right_to_left.unwrap_or(0.0),
                cm.right_to_right.unwrap_or(1.0),
            );
            if f.is_enabled() {
                filters.push(Box::new(f));
            }
        }

        // Low Pass
        if let Some(ref lp) = config.low_pass {
            let f = low_pass::LowPassFilter::new(lp.smoothing.unwrap_or(20.0));
            if f.is_enabled() {
                filters.push(Box::new(f));
            }
        }

        // Timescale (separate — changes buffer length)
        let timescale = config.timescale.as_ref().and_then(|t| {
            let f = timescale::TimescaleFilter::new(
                t.speed.unwrap_or(1.0),
                t.pitch.unwrap_or(1.0),
                t.rate.unwrap_or(1.0),
            );
            if f.is_enabled() { Some(f) } else { None }
        });

        Self {
            filters,
            timescale,
            timescale_buffer: Vec::new(),
        }
    }

    /// Check if any filter is active.
    pub fn is_active(&self) -> bool {
        !self.filters.is_empty() || self.timescale.is_some()
    }

    /// Process audio samples through all active filters in-place.
    /// For timescale, output is buffered internally and fed to `fill_frame`.
    pub fn process(&mut self, samples: &mut [i16]) {
        // Apply all in-place filters
        for filter in self.filters.iter_mut() {
            filter.process(samples);
        }

        // If timescale is active, resample and buffer the output
        if let Some(ref mut ts) = self.timescale {
            let resampled = ts.process_resample(samples);
            self.timescale_buffer.extend_from_slice(&resampled);
        }
    }

    /// When timescale is active, drain exactly `frame_size` samples from the
    /// internal buffer into `output`. Returns `true` if enough data was available.
    pub fn fill_frame(&mut self, output: &mut [i16]) -> bool {
        if self.timescale.is_none() {
            return false; // Not using timescale, caller should use the original buffer
        }

        if self.timescale_buffer.len() >= output.len() {
            output.copy_from_slice(&self.timescale_buffer[..output.len()]);
            self.timescale_buffer.drain(..output.len());
            true
        } else {
            false // Not enough data yet — skip this frame
        }
    }

    /// Whether timescale is active (changes the speak_loop flow).
    pub fn has_timescale(&self) -> bool {
        self.timescale.is_some()
    }

    /// Reset all filter states (e.g. on seek).
    pub fn reset(&mut self) {
        for filter in self.filters.iter_mut() {
            filter.reset();
        }
        if let Some(ref mut ts) = self.timescale {
            ts.reset();
        }
        self.timescale_buffer.clear();
    }
}
