use infinitedsp_core::core::channels::Mono;
use infinitedsp_core::FrameProcessor;

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(unused)]
pub enum FastLfoWaveform {
    Sine,
    Triangle,
    Saw,
    Square,
}

pub struct FastLfo {
    frequency: f32,
    waveform: FastLfoWaveform,
    min: f32,
    max: f32,
    phase: f32,
    sample_rate: f32,
}

#[allow(dead_code)]
impl FastLfo {
    pub fn new(frequency: f32, waveform: FastLfoWaveform, sample_rate: f32) -> Self {
        Self {
            frequency,
            waveform,
            min: -1.0,
            max: 1.0,
            phase: 0.0,
            sample_rate,
        }
    }

    pub fn set_range(&mut self, min: f32, max: f32) {
        self.min = min;
        self.max = max;
    }

    pub fn set_frequency(&mut self, frequency: f32) {
        self.frequency = frequency;
    }

    pub fn get_frequency(&self) -> f32 {
        self.frequency
    }

    pub fn get_waveform(&self) -> FastLfoWaveform {
        self.waveform
    }

    pub fn get_min(&self) -> f32 {
        self.min
    }

    pub fn get_max(&self) -> f32 {
        self.max
    }
}

impl FrameProcessor<Mono> for FastLfo {
    #[inline(always)]
    fn process(&mut self, buffer: &mut [f32], _frame_index: u64) {
        let inv_sr = 1.0 / self.sample_rate;
        let phase_inc = self.frequency * inv_sr;
        let range = self.max - self.min;
        let offset = self.min;

        for sample in buffer.iter_mut() {
            self.phase += phase_inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }

            let raw = match self.waveform {
                FastLfoWaveform::Sine => {
                    let mut t = self.phase * 2.0 - 1.0;
                    t = 2.0 * t.abs() - 1.0;
                    t * (1.5 - 0.5 * t * t)
                }
                FastLfoWaveform::Saw => 2.0 * self.phase - 1.0,
                FastLfoWaveform::Square => {
                    if self.phase < 0.5 {
                        1.0
                    } else {
                        -1.0
                    }
                }
                FastLfoWaveform::Triangle => {
                    let t = self.phase * 2.0 - 1.0;
                    2.0 * t.abs() - 1.0
                }
            };

            let normalized = (raw + 1.0) * 0.5;
            *sample = offset + normalized * range;
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    fn reset(&mut self) {
        self.phase = 0.0;
    }

    fn latency_samples(&self) -> u32 {
        0
    }

    fn name(&self) -> &str {
        "FastLfo"
    }

    fn visualize(&self, _indent: usize) -> alloc::string::String {
        "FastLfo".into()
    }
}
