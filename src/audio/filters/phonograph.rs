use super::{biquad::{BiquadCoeffs, BiquadState}, delay_line::DelayLine, lfo::Lfo, AudioFilter};

const MAX_DELAY_MS: f32 = 60.0;
const BUFFER_SIZE: usize = ((48000.0 * MAX_DELAY_MS) / 1000.0) as usize;

struct XorShift32 {
    s: u32,
}

impl XorShift32 {
    fn new(seed: u32) -> Self {
        Self { s: seed }
    }

    fn next_u32(&mut self) -> u32 {
        let mut x = self.s;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.s = x;
        x
    }

    fn next_01(&mut self) -> f32 {
        (self.next_u32() as f64 / 4294967296.0) as f32
    }

    fn next_11(&mut self) -> f32 {
        self.next_01() * 2.0 - 1.0
    }

    fn next_noise(&mut self) -> f32 {
        (self.next_11() + self.next_11() + self.next_11()) / 3.0
    }
}

pub struct PhonographFilter {
    frequency: f32,
    depth: f32,
    crackle: f32,
    flutter: f32,
    room: f32,
    mic_agc: f32,
    drive: f32,

    wow_lfo: Lfo,
    flutter_lfo: Lfo,
    drift: f32,
    delay: DelayLine,

    hp1_state: BiquadState,
    hp2_state: BiquadState,
    lp1_state: BiquadState,
    lp2_state: BiquadState,
    peak1_state: BiquadState,
    peak2_state: BiquadState,
    hiss_hp_state: BiquadState,
    hiss_lp_state: BiquadState,

    hp1_coeffs: BiquadCoeffs,
    hp2_coeffs: BiquadCoeffs,
    lp1_coeffs: BiquadCoeffs,
    lp2_coeffs: BiquadCoeffs,
    peak1_coeffs: BiquadCoeffs,
    peak2_coeffs: BiquadCoeffs,
    hiss_hp_coeffs: BiquadCoeffs,
    hiss_lp_coeffs: BiquadCoeffs,

    r1: DelayLine,
    r2: DelayLine,
    r3: DelayLine,
    room_damp: f32,

    tick_env: f32,
    tick_amp: f32,
    scratch_env: f32,
    scratch_amp: f32,
    env: f32,
    agc_gain: f32,

    rng: XorShift32,
}

impl PhonographFilter {
    pub fn new(
        frequency: f32,
        depth: f32,
        crackle: f32,
        flutter: f32,
        room: f32,
        mic_agc: f32,
        drive: f32,
    ) -> Self {
        let mut filter = Self {
            frequency: 0.8,
            depth: 0.25,
            crackle: 0.18,
            flutter: 0.18,
            room: 0.22,
            mic_agc: 0.25,
            drive: 0.25,

            wow_lfo: Lfo::new(),
            flutter_lfo: Lfo::new(),
            drift: 0.0,
            delay: DelayLine::new(BUFFER_SIZE),

            hp1_state: BiquadState::default(),
            hp2_state: BiquadState::default(),
            lp1_state: BiquadState::default(),
            lp2_state: BiquadState::default(),
            peak1_state: BiquadState::default(),
            peak2_state: BiquadState::default(),
            hiss_hp_state: BiquadState::default(),
            hiss_lp_state: BiquadState::default(),

            hp1_coeffs: Self::make_highpass(260.0, std::f64::consts::FRAC_1_SQRT_2, 48000.0),
            hp2_coeffs: Self::make_highpass(260.0, std::f64::consts::FRAC_1_SQRT_2, 48000.0),
            lp1_coeffs: Self::make_lowpass(3300.0, std::f64::consts::FRAC_1_SQRT_2, 48000.0),
            lp2_coeffs: Self::make_lowpass(3300.0, std::f64::consts::FRAC_1_SQRT_2, 48000.0),
            peak1_coeffs: Self::make_peaking(950.0, 1.1, 7.0, 48000.0),
            peak2_coeffs: Self::make_peaking(2400.0, 1.6, 3.5, 48000.0),
            hiss_hp_coeffs: Self::make_highpass(1800.0, std::f64::consts::FRAC_1_SQRT_2, 48000.0),
            hiss_lp_coeffs: Self::make_lowpass(6500.0, std::f64::consts::FRAC_1_SQRT_2, 48000.0),

            r1: DelayLine::new((48000.0 * 0.03) as usize),
            r2: DelayLine::new((48000.0 * 0.03) as usize),
            r3: DelayLine::new((48000.0 * 0.03) as usize),
            room_damp: 0.0,

            tick_env: 0.0,
            tick_amp: 0.0,
            scratch_env: 0.0,
            scratch_amp: 0.0,
            env: 0.0,
            agc_gain: 1.0,

            rng: XorShift32::new(0x1a2b3c4d),
        };

        filter.recompute_filters();
        filter.update(frequency, depth, crackle, flutter, room, mic_agc, drive);
        filter
    }

    fn soft_clip(x: f32) -> f32 {
        let x2 = x * x;
        (x * (27.0 + x2)) / (27.0 + 9.0 * x2)
    }

    fn make_highpass(fc: f64, q: f64, fs: f64) -> BiquadCoeffs {
        let w0 = 2.0 * std::f64::consts::PI * (fc / fs);
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);

        let a0 = 1.0 + alpha;
        BiquadCoeffs {
            b0: ((1.0 + cos_w0) / 2.0) / a0,
            b1: (-(1.0 + cos_w0)) / a0,
            b2: ((1.0 + cos_w0) / 2.0) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
        }
    }

    fn make_lowpass(fc: f64, q: f64, fs: f64) -> BiquadCoeffs {
        let w0 = 2.0 * std::f64::consts::PI * (fc / fs);
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);

        let a0 = 1.0 + alpha;
        BiquadCoeffs {
            b0: ((1.0 - cos_w0) / 2.0) / a0,
            b1: (1.0 - cos_w0) / a0,
            b2: ((1.0 - cos_w0) / 2.0) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
        }
    }

    fn make_peaking(fc: f64, q: f64, gain_db: f64, fs: f64) -> BiquadCoeffs {
        let a = 10f64.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * (fc / fs);
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);

        let a0 = 1.0 + alpha / a;
        BiquadCoeffs {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cos_w0) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }

    pub fn update(
        &mut self,
        frequency: f32,
        depth: f32,
        crackle: f32,
        flutter: f32,
        room: f32,
        mic_agc: f32,
        drive: f32,
    ) {
        self.frequency = frequency;
        self.depth = depth.clamp(0.0, 1.0);
        self.crackle = crackle.clamp(0.0, 1.0);
        self.flutter = flutter.clamp(0.0, 1.0);
        self.room = room.clamp(0.0, 1.0);
        self.mic_agc = mic_agc.clamp(0.0, 1.0);
        self.drive = drive.clamp(0.0, 1.0);

        self.wow_lfo.update(self.frequency.into(), 1.0);
        self.flutter_lfo.update(7.5, 1.0);
    }

    fn recompute_filters(&mut self) {
        let fs = 48000.0;
        let q = std::f64::consts::FRAC_1_SQRT_2;

        self.hp1_coeffs = Self::make_highpass(260.0, q, fs);
        self.hp2_coeffs = Self::make_highpass(260.0, q, fs);
        self.lp1_coeffs = Self::make_lowpass(3300.0, q, fs);
        self.lp2_coeffs = Self::make_lowpass(3300.0, q, fs);
        self.peak1_coeffs = Self::make_peaking(950.0, 1.1, 7.0, fs);
        self.peak2_coeffs = Self::make_peaking(2400.0, 1.6, 3.5, fs);
        self.hiss_hp_coeffs = Self::make_highpass(1800.0, q, fs);
        self.hiss_lp_coeffs = Self::make_lowpass(6500.0, q, fs);
    }
}

impl AudioFilter for PhonographFilter {
    fn process(&mut self, samples: &mut [i16]) {
        let fs = 48000.0;
        let wow_max = self.depth * 0.014 * fs;
        let flutter_max = self.flutter * 0.0022 * fs;
        let center = 1.0 + wow_max + flutter_max;
        let drift_amount = self.depth * 0.0012 * fs;
        let drift_smooth = 0.00015;
        let hiss_gain = 0.01 * self.crackle;
        let tick_rate = 0.00002 * self.crackle;
        let scratch_rate = 0.0000025 * self.crackle;
        let d1 = 7.5 / 1000.0 * fs;
        let d2 = 12.0 / 1000.0 * fs;
        let d3 = 17.5 / 1000.0 * fs;
        let room_mix = 0.35 * self.room;
        let agc_on = self.mic_agc > 0.0;
        let target = 0.22;
        let atk = 0.006 + 0.01 * self.mic_agc;
        let rel = 0.0006 + 0.0012 * self.mic_agc;

        for chunk in samples.chunks_exact_mut(2) {
            let left_sample = chunk[0] as f32;
            let right_sample = chunk[1] as f32;
            
            // Phonograph natively converts stereo to mono initially
            let mut x = ((left_sample + right_sample) * 0.5) / 32768.0;

            let d_noise = self.rng.next_noise();
            self.drift += (d_noise * drift_amount - self.drift) * drift_smooth;
            let wow = self.wow_lfo.get_value() as f32;
            let flt = self.flutter_lfo.get_value() as f32;
            let mut dly = center + wow * wow_max + flt * flutter_max + self.drift;
            if dly < 1.0 { dly = 1.0; }
            if dly > BUFFER_SIZE as f32 - 2.0 { dly = BUFFER_SIZE as f32 - 2.0; }

            self.delay.write(x);
            x = self.delay.read(dly);

            if self.drive > 0.0 {
                let g = 1.0 + self.drive * 6.0;
                x = Self::soft_clip(x * g) / Self::soft_clip(g);
            }

            x = self.hp1_state.process(x as f64, &self.hp1_coeffs) as f32;
            x = self.hp2_state.process(x as f64, &self.hp2_coeffs) as f32;
            x = self.lp1_state.process(x as f64, &self.lp1_coeffs) as f32;
            x = self.lp2_state.process(x as f64, &self.lp2_coeffs) as f32;
            x = self.peak1_state.process(x as f64, &self.peak1_coeffs) as f32;
            x = self.peak2_state.process(x as f64, &self.peak2_coeffs) as f32;

            if self.crackle > 0.0 {
                let mut n = self.rng.next_noise();
                n = self.hiss_hp_state.process(n as f64, &self.hiss_hp_coeffs) as f32;
                n = self.hiss_lp_state.process(n as f64, &self.hiss_lp_coeffs) as f32;
                x += n * hiss_gain;
                
                if self.rng.next_01() < tick_rate {
                    self.tick_env = 1.0;
                    self.tick_amp = self.rng.next_11() * (0.45 + self.crackle);
                }
                self.tick_env *= 0.965;
                x += self.tick_amp * self.tick_env * 0.18;

                if self.rng.next_01() < scratch_rate {
                    self.scratch_env = 1.0;
                    self.scratch_amp = self.rng.next_11() * (0.35 + self.crackle);
                }
                self.scratch_env *= 0.992;
                x += self.scratch_amp * self.scratch_env * 0.06;
            }

            if self.room > 0.0 {
                self.room_damp += 0.08 * (x - self.room_damp);
                self.r1.write(self.room_damp);
                self.r2.write(self.room_damp);
                self.r3.write(self.room_damp);

                let a = self.r1.read(d1);
                let b = self.r2.read(d2);
                let c = self.r3.read(d3);

                x = x * (1.0 - room_mix) + (a + b + c) * (room_mix / 3.0);
            }

            if agc_on {
                let ax = x.abs();
                let coeff = if ax > self.env { atk } else { rel };
                self.env += (ax - self.env) * coeff;
                let desired = target / (self.env + 1e-6);
                self.agc_gain += (desired - self.agc_gain) * 0.0015;
                let g = self.agc_gain.clamp(0.35, 2.8);
                x *= g;
            }

            let out = (x * 32768.0).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            chunk[0] = out;
            chunk[1] = out;
        }
    }

    fn is_enabled(&self) -> bool {
        self.depth > 0.0 || self.crackle > 0.0 || self.flutter > 0.0 || self.room > 0.0 || self.drive > 0.0
    }

    fn reset(&mut self) {
        self.delay.clear();
        self.r1.clear();
        self.r2.clear();
        self.r3.clear();
        self.wow_lfo.reset();
        self.flutter_lfo.reset();
        self.drift = 0.0;
        
        self.hp1_state.reset();
        self.hp2_state.reset();
        self.lp1_state.reset();
        self.lp2_state.reset();
        self.peak1_state.reset();
        self.peak2_state.reset();
        self.hiss_hp_state.reset();
        self.hiss_lp_state.reset();
        
        self.tick_env = 0.0;
        self.tick_amp = 0.0;
        self.scratch_env = 0.0;
        self.scratch_amp = 0.0;
        self.room_damp = 0.0;
        self.env = 0.0;
        self.agc_gain = 1.0;
    }
}
