use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use infinitedsp_core::core::audio_param::AudioParam;
use infinitedsp_core::core::channels::Mono;
use infinitedsp_core::core::dsp_chain::DspChain;
use infinitedsp_core::effects::filter::predictive_ladder::PredictiveLadderFilter;
use infinitedsp_core::effects::utility::gain::Gain;
use infinitedsp_core::effects::utility::offset::Offset;
use infinitedsp_core::synthesis::envelope::Adsr;
use infinitedsp_core::synthesis::oscillator::Waveform;
use infinitedsp_core::FrameProcessor;

use crate::dsp::lfo::FastLfo;
use crate::dsp::oscillator::FastOscillator;
use crate::control::midi::{MidiControl, MidiFilterCutoff, MidiFilterResonance, MidiFreq, MidiGate};
use crate::data::presets::{OscSettings, Preset};
use crate::dsp::sum::Sum;

struct MoogOscillatorSection {
    osc1: FastOscillator,
    osc2: FastOscillator,
    osc3: FastOscillator,
    noise: FastOscillator,
    level1: f32,
    level2: f32,
    level3: f32,
    level_noise: f32,
    scratch_buffer: Vec<f32>,
}

impl MoogOscillatorSection {
    #[allow(clippy::too_many_arguments)]
    fn new(
        osc1: FastOscillator,
        osc2: FastOscillator,
        osc3: FastOscillator,
        noise: FastOscillator,
        l1: f32,
        l2: f32,
        l3: f32,
        ln: f32,
    ) -> Self {
        Self {
            osc1,
            osc2,
            osc3,
            noise,
            level1: l1,
            level2: l2,
            level3: l3,
            level_noise: ln,
            scratch_buffer: vec![0.0; 256],
        }
    }
}

impl FrameProcessor<Mono> for MoogOscillatorSection {
    fn process(&mut self, buffer: &mut [f32], frame_index: u64) {
        let len = buffer.len();
        if self.scratch_buffer.len() < len {
            self.scratch_buffer.resize(len, 0.0);
        }

        if self.level1 > 0.0001 {
            self.osc1.process(buffer, frame_index);
            for s in buffer.iter_mut() {
                *s *= self.level1;
            }
        } else {
            buffer.fill(0.0);
        }

        if self.level2 > 0.0001 {
            self.osc2.process(&mut self.scratch_buffer[0..len], frame_index);
            for (s, scratch) in buffer.iter_mut().zip(self.scratch_buffer.iter()) {
                *s += *scratch * self.level2;
            }
        }

        if self.level3 > 0.0001 {
            self.osc3.process(&mut self.scratch_buffer[0..len], frame_index);
            for (s, scratch) in buffer.iter_mut().zip(self.scratch_buffer.iter()) {
                *s += *scratch * self.level3;
            }
        }

        if self.level_noise > 0.0001 {
            self.noise.process(&mut self.scratch_buffer[0..len], frame_index);
            for (s, scratch) in buffer.iter_mut().zip(self.scratch_buffer.iter()) {
                *s += *scratch * self.level_noise;
            }
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.osc1.set_sample_rate(sample_rate);
        self.osc2.set_sample_rate(sample_rate);
        self.osc3.set_sample_rate(sample_rate);
        self.noise.set_sample_rate(sample_rate);
    }

    fn reset(&mut self) {
        self.osc1.reset();
        self.osc2.reset();
        self.osc3.reset();
        self.noise.reset();
    }

    fn latency_samples(&self) -> u32 {
        0
    }
    fn name(&self) -> &str {
        "MoogOscillatorSection"
    }
    fn visualize(&self, _indent: usize) -> alloc::string::String {
        "MoogOscillatorSection".into()
    }
}

pub fn new_moog_voice(
    sample_rate: f32,
    midi: Arc<MidiControl>,
    preset: Preset,
) -> impl FrameProcessor<Mono> + Send {
    let cutoff_norm = libm::log10f(preset.filter.cutoff / 20.0) / libm::log10f(1000.0);
    midi.set_parameter_1(cutoff_norm.clamp(0.0, 1.0));

    let res_norm = (preset.filter.resonance - 0.707) / 9.3;
    midi.set_parameter_2(res_norm.clamp(0.0, 1.0));

    let (vibrato_node, filter_lfo_node) = if preset.lfo_enabled != 0 {
        let p = &preset.lfo;
        let mut lfo_vib = FastLfo::new(p.frequency, p.get_waveform(), sample_rate);
        lfo_vib.set_range(-p.vibrato_amount, p.vibrato_amount);

        let mut lfo_filt = FastLfo::new(p.frequency, p.get_waveform(), sample_rate);
        lfo_filt.set_range(-p.filter_amount, p.filter_amount);

        (Some(lfo_vib), Some(lfo_filt))
    } else {
        (None, None)
    };

    let create_pitch = |params: &OscSettings, vib: Option<FastLfo>| -> AudioParam {
        let mut chain = DspChain::new(MidiFreq::new(midi.clone()), sample_rate);

        if params.octave != 0.0 {
            let mult = libm::powf(2.0, params.octave);
            chain = chain.and(Gain::new_fixed(mult));
        }

        if params.detune != 0.0 {
            chain = chain.and(Offset::new(params.detune));
        }

        if params.is_vibrato_enabled() {
            if let Some(v) = vib {
                chain = chain.and(Sum::new(AudioParam::Dynamic(Box::new(v))));
            }
        }

        AudioParam::Dynamic(Box::new(chain))
    };

    let osc1_vib = vibrato_node.as_ref().map(|l| {
        let mut l = FastLfo::new(l.get_frequency(), l.get_waveform(), sample_rate);
        l.set_range(l.get_min(), l.get_max());
        l
    });
    let osc2_vib = vibrato_node.as_ref().map(|l| {
        let mut l = FastLfo::new(l.get_frequency(), l.get_waveform(), sample_rate);
        l.set_range(l.get_min(), l.get_max());
        l
    });
    let osc3_vib = vibrato_node.as_ref().map(|l| {
        let mut l = FastLfo::new(l.get_frequency(), l.get_waveform(), sample_rate);
        l.set_range(l.get_min(), l.get_max());
        l
    });

    let osc1_node = FastOscillator::new(create_pitch(&preset.osc1, osc1_vib), preset.osc1.get_waveform());
    let osc2_node = FastOscillator::new(create_pitch(&preset.osc2, osc2_vib), preset.osc2.get_waveform());
    let osc3_node = FastOscillator::new(create_pitch(&preset.osc3, osc3_vib), preset.osc3.get_waveform());
    let noise_node = FastOscillator::new(AudioParam::Static(0.0), Waveform::WhiteNoise);

    let mixer = MoogOscillatorSection::new(
        osc1_node,
        osc2_node,
        osc3_node,
        noise_node,
        preset.osc1.level,
        preset.osc2.level,
        preset.osc3.level,
        preset.noise_level,
    );

    let filter_env = Adsr::new(
        AudioParam::Dynamic(Box::new(MidiGate(midi.clone()))),
        AudioParam::Static(preset.filter.attack),
        AudioParam::Static(preset.filter.decay),
        AudioParam::Static(preset.filter.sustain),
        AudioParam::Static(preset.filter.release),
    );

    let cutoff_ctrl = MidiFilterCutoff(midi.clone());

    let mut cutoff_mod_chain = DspChain::new(cutoff_ctrl, sample_rate).and(Sum::new(
        AudioParam::Dynamic(Box::new(
            DspChain::new(filter_env, sample_rate).and(Gain::new_fixed(preset.filter.env_amount)),
        )),
    ));

    if let Some(lfo) = filter_lfo_node {
        cutoff_mod_chain = cutoff_mod_chain.and(Sum::new(AudioParam::Dynamic(Box::new(lfo))));
    }

    let resonance_ctrl = MidiFilterResonance(midi.clone());

    let filter_node = PredictiveLadderFilter::new(
        AudioParam::Dynamic(Box::new(cutoff_mod_chain)),
        AudioParam::Dynamic(Box::new(resonance_ctrl)),
    );

    let amp_env = Adsr::new(
        AudioParam::Dynamic(Box::new(MidiGate(midi.clone()))),
        AudioParam::Static(preset.amp.attack),
        AudioParam::Static(preset.amp.decay),
        AudioParam::Static(preset.amp.sustain),
        AudioParam::Static(preset.amp.release),
    );

    let vca = Gain::new(AudioParam::Dynamic(Box::new(amp_env)));

    DspChain::new(mixer, sample_rate)
        .and(filter_node)
        .and(vca)
}
