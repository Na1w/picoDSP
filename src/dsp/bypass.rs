use infinitedsp_core::FrameProcessor;

pub struct Bypass<T, C> {
    processor: T,
    enabled: bool,
    _marker: core::marker::PhantomData<C>,
}

impl<T, C> Bypass<T, C> {
    pub fn new(processor: T, enabled: bool) -> Self {
        Self {
            processor,
            enabled,
            _marker: core::marker::PhantomData,
        }
    }

    #[allow(dead_code)]
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

impl<T, C> FrameProcessor<C> for Bypass<T, C>
where
    T: FrameProcessor<C>,
    C: infinitedsp_core::core::channels::ChannelConfig,
{
    fn process(&mut self, buffer: &mut [f32], frame_index: u64) {
        if self.enabled {
            self.processor.process(buffer, frame_index);
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.processor.set_sample_rate(sample_rate);
    }

    fn reset(&mut self) {
        self.processor.reset();
    }

    fn latency_samples(&self) -> u32 {
        if self.enabled {
            self.processor.latency_samples()
        } else {
            0
        }
    }

    fn name(&self) -> &str {
        if self.enabled {
            self.processor.name()
        } else {
            "Bypass"
        }
    }

    fn visualize(&self, indent: usize) -> alloc::string::String {
        if self.enabled {
            self.processor.visualize(indent)
        } else {
            "Bypass".into()
        }
    }
}
