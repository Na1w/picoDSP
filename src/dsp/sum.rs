use alloc::vec::Vec;
use infinitedsp_core::core::audio_param::AudioParam;
use infinitedsp_core::core::channels::Mono;
use infinitedsp_core::FrameProcessor;

pub struct Sum {
    offset: AudioParam,
    scratch_buffer: Vec<f32>,
}

impl Sum {
    pub fn new(offset: AudioParam) -> Self {
        Self {
            offset,
            scratch_buffer: Vec::new(),
        }
    }
}

impl FrameProcessor<Mono> for Sum {
    fn process(&mut self, buffer: &mut [f32], frame_index: u64) {
        let len = buffer.len();
        if self.scratch_buffer.len() < len {
            self.scratch_buffer.resize(len, 0.0);
        }

        self.offset
            .process(&mut self.scratch_buffer[0..len], frame_index);

        for (sample, offset_val) in buffer.iter_mut().zip(self.scratch_buffer.iter()) {
            *sample += *offset_val;
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.offset.set_sample_rate(sample_rate);
    }

    fn reset(&mut self) {
        self.offset.reset();
    }

    fn latency_samples(&self) -> u32 {
        0
    }

    fn name(&self) -> &str {
        "Sum"
    }

    fn visualize(&self, _indent: usize) -> alloc::string::String {
        "Sum".into()
    }
}
