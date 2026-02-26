//! `resample/hermite.rs` — high-quality Cubic Hermite (Catmull-Rom) resampler.
//!
//! Uses four-point cubic interpolation for significantly better alias
//! rejection than linear resampling, at modest extra CPU cost.
//! Well-suited for the 44 100 Hz → 48 000 Hz conversion used with Discord.

pub struct HermiteResampler {
    ratio: f32,
    /// Fractional read head within the current input block.
    index: f32,
    channels: usize,
    /// Ring of the last 4 frames (per channel) for cubic interpolation.
    hist: Vec<[i16; 4]>, // hist[channel][0..4]  newest at [3]
}

impl HermiteResampler {
    pub fn new(source_rate: u32, target_rate: u32, channels: usize) -> Self {
        Self {
            ratio: source_rate as f32 / target_rate as f32,
            index: 0.0,
            channels,
            hist: vec![[0i16; 4]; channels],
        }
    }

    /// Cubic Hermite interpolation over four evenly-spaced points.
    ///
    /// `p` = [p0, p1, p2, p3] where the output lies between p1 and p2 at `t ∈ [0, 1)`.
    #[inline]
    fn hermite(p: [f32; 4], t: f32) -> f32 {
        let c0 = p[1];
        let c1 = 0.5 * (p[2] - p[0]);
        let c2 = p[0] - 2.5 * p[1] + 2.0 * p[2] - 0.5 * p[3];
        let c3 = 0.5 * (p[3] - p[0]) + 1.5 * (p[1] - p[2]);
        ((c3 * t + c2) * t + c1) * t + c0
    }

    /// Resample `input` (interleaved i16) and **append** into `output`.
    pub fn process(&mut self, input: &[i16], output: &mut Vec<i16>) {
        let num_frames = input.len() / self.channels;

        while self.index < num_frames as f32 {
            let base = self.index as isize;
            let t = self.index.fract();

            for ch in 0..self.channels {
                let p: [f32; 4] = [
                    // p[-1]
                    if base - 1 < 0 {
                        self.hist[ch][(4 + (base - 1) as isize) as usize % 4] as f32
                    } else {
                        input[(base as usize - 1) * self.channels + ch] as f32
                    },
                    // p[0]
                    if base < 0 {
                        self.hist[ch][(4 + base) as usize % 4] as f32
                    } else {
                        input[base as usize * self.channels + ch] as f32
                    },
                    // p[1]
                    {
                        let i = (base + 1) as usize;
                        if i < num_frames {
                            input[i * self.channels + ch] as f32
                        } else {
                            input[(num_frames - 1) * self.channels + ch] as f32
                        }
                    },
                    // p[2]
                    {
                        let i = (base + 2) as usize;
                        if i < num_frames {
                            input[i * self.channels + ch] as f32
                        } else {
                            input[(num_frames - 1) * self.channels + ch] as f32
                        }
                    },
                ];
                let s = Self::hermite(p, t).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
                output.push(s);
            }

            self.index += self.ratio;
        }

        self.index -= num_frames as f32;

        // Update history: store the last 4 frames of this block.
        let kept = num_frames.min(4);
        for k in 0..kept {
            let src = num_frames - kept + k;
            for ch in 0..self.channels {
                self.hist[ch][k] = input[src * self.channels + ch];
            }
        }
    }

    pub fn reset(&mut self) {
        self.index = 0.0;
        for h in &mut self.hist {
            h.fill(0);
        }
    }

    pub fn is_passthrough(&self) -> bool {
        (self.ratio - 1.0).abs() < f32::EPSILON
    }
}
