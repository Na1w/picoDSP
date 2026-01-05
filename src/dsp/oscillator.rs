use infinitedsp_core::core::audio_param::AudioParam;
use infinitedsp_core::core::channels::Mono;
use infinitedsp_core::FrameProcessor;
use infinitedsp_core::synthesis::oscillator::Waveform;
use alloc::vec::Vec;
use core::f32::consts::PI;
use wide::f32x4;

pub struct FastOscillator {
    phase: f32,
    frequency: AudioParam,
    waveform: Waveform,
    sample_rate: f32,
    freq_buffer: Vec<f32>,
    rng_state: u32,
}

impl FastOscillator {
    pub fn new(frequency: AudioParam, waveform: Waveform) -> Self {
        FastOscillator {
            phase: 0.0,
            frequency,
            waveform,
            sample_rate: 48000.0,
            freq_buffer: Vec::new(),
            rng_state: 12345,
        }
    }

    #[inline(always)]
    fn poly_blep(t: f32, dt: f32) -> f32 {
        if t < dt {
            let t = t / dt;
            return t + t - t * t - 1.0;
        } else if t > 1.0 - dt {
            let t = (t - 1.0) / dt;
            return t * t + t + t + 1.0;
        }
        0.0
    }

    #[inline(always)]
    fn next_random(rng_state: &mut u32) -> f32 {
        *rng_state = rng_state.wrapping_mul(1103515245).wrapping_add(12345);
        let val = (*rng_state >> 16) & 0x7FFF;
        (val as f32 / 32768.0) * 2.0 - 1.0
    }
}

impl FrameProcessor<Mono> for FastOscillator {
    fn process(&mut self, buffer: &mut [f32], sample_index: u64) {
        if self.freq_buffer.len() != buffer.len() {
            self.freq_buffer.resize(buffer.len(), 0.0);
        }

        self.frequency.process(&mut self.freq_buffer, sample_index);

        let sample_rate = self.sample_rate;
        let mut phase = self.phase;
        let inv_sr = 1.0 / sample_rate;
        let inv_sr_vec = f32x4::splat(inv_sr);

        let (chunks, remainder) = buffer.as_chunks_mut::<4>();
        let (freq_chunks, _freq_rem) = self.freq_buffer.as_chunks::<4>();

        match self.waveform {
            Waveform::Sine => {
                for (out_chunk, freq_chunk) in chunks.iter_mut().zip(freq_chunks.iter()) {
                    for i in 0..4 {
                        let freq = freq_chunk[i];
                        let inc = freq * inv_sr;
                        phase += inc;
                        if phase >= 1.0 { phase -= 1.0; }
                        else if phase < 0.0 { phase += 1.0; }

                        out_chunk[i] = libm::sinf(phase * 2.0 * PI);
                    }
                }
            },
            Waveform::Triangle => {
                for (out_chunk, freq_chunk) in chunks.iter_mut().zip(freq_chunks.iter()) {
                    let freq = f32x4::from(*freq_chunk);
                    let inc = freq * inv_sr_vec;

                    let mut p = [0.0; 4];
                    let inc_arr = inc.to_array();
                    for i in 0..4 {
                        phase += inc_arr[i];
                        if phase >= 1.0 { phase -= 1.0; }
                        else if phase < 0.0 { phase += 1.0; }
                        p[i] = phase;
                    }

                    let mut out = [0.0; 4];
                    for i in 0..4 {
                        let x = p[i];
                        out[i] = if x < 0.5 {
                            4.0 * x - 1.0
                        } else {
                            4.0 * (1.0 - x) - 1.0
                        };
                    }
                    *out_chunk = out;
                }
            },
            Waveform::Saw => {
                for (out_chunk, freq_chunk) in chunks.iter_mut().zip(freq_chunks.iter()) {
                    let freq = f32x4::from(*freq_chunk);
                    let inc = freq * inv_sr_vec;

                    let mut p = [0.0; 4];
                    let inc_arr = inc.to_array();
                    for i in 0..4 {
                        phase += inc_arr[i];
                        if phase >= 1.0 { phase -= 1.0; }
                        else if phase < 0.0 { phase += 1.0; }
                        p[i] = phase;
                    }

                    let mut out = [0.0; 4];
                    for i in 0..4 {
                        let naive = 2.0 * p[i] - 1.0;
                        out[i] = naive - Self::poly_blep(p[i], inc_arr[i].abs());
                    }
                    *out_chunk = out;
                }
            },
            Waveform::Square => {
                for (out_chunk, freq_chunk) in chunks.iter_mut().zip(freq_chunks.iter()) {
                    let freq = f32x4::from(*freq_chunk);
                    let inc = freq * inv_sr_vec;

                    let mut p = [0.0; 4];
                    let inc_arr = inc.to_array();
                    for i in 0..4 {
                        phase += inc_arr[i];
                        if phase >= 1.0 { phase -= 1.0; }
                        else if phase < 0.0 { phase += 1.0; }
                        p[i] = phase;
                    }

                    let mut out = [0.0; 4];
                    for i in 0..4 {
                        let naive = if p[i] < 0.5 { 1.0 } else { -1.0 };
                        let abs_inc = inc_arr[i].abs();
                        let corr = Self::poly_blep(p[i], abs_inc)
                            - Self::poly_blep((p[i] + 0.5) % 1.0, abs_inc);
                        out[i] = naive + corr;
                    }
                    *out_chunk = out;
                }
            },
            Waveform::WhiteNoise => {
                let mut rng = self.rng_state;
                for out_chunk in chunks.iter_mut() {
                    for sample in out_chunk.iter_mut().take(4) {
                        *sample = Self::next_random(&mut rng);
                    }
                }
                self.rng_state = rng;
            }
        }

        for (i, sample) in remainder.iter_mut().enumerate() {
            let freq_idx = chunks.len() * 4 + i;
            let freq = self.freq_buffer[freq_idx];
            let inc = freq * inv_sr;

            if !matches!(self.waveform, Waveform::WhiteNoise) {
                phase += inc;
                if phase >= 1.0 { phase -= 1.0; }
                else if phase < 0.0 { phase += 1.0; }
            }

            let val = match self.waveform {
                Waveform::Sine => libm::sinf(phase * 2.0 * PI),
                Waveform::Triangle => {
                    let x = phase;
                    if x < 0.5 { 4.0 * x - 1.0 } else { 4.0 * (1.0 - x) - 1.0 }
                }
                Waveform::Saw => {
                    let naive = 2.0 * phase - 1.0;
                    let dt = inc.abs();
                    naive - Self::poly_blep(phase, dt)
                }
                Waveform::Square => {
                    let naive = if phase < 0.5 { 1.0 } else { -1.0 };
                    let dt = inc.abs();
                    let corr = Self::poly_blep(phase, dt)
                        - Self::poly_blep((phase + 0.5) % 1.0, dt);
                    naive + corr
                }
                Waveform::WhiteNoise => {
                    let mut rng = self.rng_state;
                    let v = Self::next_random(&mut rng);
                    self.rng_state = rng;
                    v
                },
            };
            *sample = val;
        }

        self.phase = phase;
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.frequency.set_sample_rate(sample_rate);
    }

    fn reset(&mut self) {
        self.phase = 0.0;
    }

    fn latency_samples(&self) -> u32 { 0 }

    fn name(&self) -> &str {
        "FastOscillator"
    }

    fn visualize(&self, _indent: usize) -> alloc::string::String { "FastOscillator".into() }
}
