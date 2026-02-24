pub struct Resampler {
    ratio: f32,
    index: f32,
    last_samples: Vec<i16>,
    channels: usize,
}

impl Resampler {
    pub fn new(source_rate: u32, target_rate: u32, channels: usize) -> Self {
        Self {
            ratio: source_rate as f32 / target_rate as f32,
            index: 0.0,
            last_samples: vec![0; channels],
            channels,
        }
    }

    /// Resample `input` and **append** output samples into `output`
    pub fn process(&mut self, input: &[i16], output: &mut Vec<i16>) {
        let num_frames = input.len() / self.channels;

        while self.index < num_frames as f32 {
            let idx = self.index as usize;
            let fract = self.index.fract();

            for c in 0..self.channels {
                let s1 = if idx == 0 {
                    self.last_samples[c] as f32
                } else {
                    input[(idx - 1) * self.channels + c] as f32
                };

                let s2 = if idx < num_frames {
                    input[idx * self.channels + c] as f32
                } else {
                    input[(num_frames - 1) * self.channels + c] as f32
                };

                output.push((s1 * (1.0 - fract) + s2 * fract) as i16);
            }

            self.index += self.ratio;
        }

        self.index -= num_frames as f32;

        if num_frames > 0 {
            for c in 0..self.channels {
                self.last_samples[c] = input[(num_frames - 1) * self.channels + c];
            }
        }
    }

    /// Reset resampler state in-place after a seek.

    pub fn reset(&mut self) {
        self.index = 0.0;
        self.last_samples.fill(0);
    }
}
