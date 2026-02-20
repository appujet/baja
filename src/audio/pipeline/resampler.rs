
use crate::common::types::{AnyResult};
use flume::Sender;

pub struct Resampler {
    ratio: f64,
    index: f64,
    last_samples: Vec<i16>,
    channels: usize,
}

impl Resampler {
    pub fn new(source_rate: u32, target_rate: u32, channels: usize) -> Self {
        Self {
            ratio: source_rate as f64 / target_rate as f64,
            index: 0.0,
            last_samples: vec![0; channels],
            channels,
        }
    }

    pub fn process(
        &mut self,
        input: &[i16],
        tx: &Sender<i16>,
    ) -> AnyResult<()> {
        let num_frames = input.len() / self.channels;

        while self.index < num_frames as f64 {
            let idx = self.index as usize;
            let fract = self.index.fract();

            for c in 0..self.channels {
                let s1 = if idx == 0 {
                    self.last_samples[c] as f64
                } else {
                    input[(idx - 1) * self.channels + c] as f64
                };

                let s2 = if idx < num_frames {
                    input[idx * self.channels + c] as f64
                } else {
                    input[(num_frames - 1) * self.channels + c] as f64
                };

                let s = s1 * (1.0 - fract) + s2 * fract;

                if tx.send(s as i16).is_err() {
                    return Ok(());
                }
            }

            self.index += self.ratio;
        }

        self.index -= num_frames as f64;

        if num_frames > 0 {
            for c in 0..self.channels {
                self.last_samples[c] = input[(num_frames - 1) * self.channels + c];
            }
        }

        Ok(())
    }
}
