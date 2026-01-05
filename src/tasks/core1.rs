use crate::dsp::reverb::Reverb;
use alloc::boxed::Box;
use alloc::sync::Arc;
use embassy_time::Instant;
use infinitedsp_core::core::audio_param::AudioParam;
use infinitedsp_core::core::channels::{DualMono, Stereo};
use infinitedsp_core::core::dsp_chain::DspChain;
use infinitedsp_core::core::parallel_mixer::ParallelMixer;
use infinitedsp_core::effects::time::delay::Delay;
use infinitedsp_core::effects::utility::gain::Gain;
use infinitedsp_core::effects::utility::stereo_widener::StereoWidener;
use infinitedsp_core::FrameProcessor;

use crate::common::shared::{
    disable_denormals, AudioData, AUDIO_CHANNEL, BLOCK_SIZE, CORE1_STACK_SIZE, PRESET_CHANNEL,
    SAMPLE_RATE,
};
use crate::control::midi::MidiControl;
use crate::data::presets::Preset;
use crate::dsp::bypass::Bypass;
use crate::dsp::moog::new_moog_voice;
use crate::usb::logger::{LogData, LOG_CHANNEL, SYSTEM_STATUS_CHANNEL};
use crate::HEAP;

macro_rules! log_status {
    ($($arg:tt)*) => {
        {
            let mut msg = heapless::String::<64>::new();
            if core::fmt::write(&mut msg, format_args!($($arg)*)).is_ok() {
                let _ = SYSTEM_STATUS_CHANNEL.send(msg).await;
            }
        }
    };
}

fn build_synth(
    midi_control: Arc<MidiControl>,
    preset: Preset,
) -> impl FrameProcessor<Stereo> + Send {
    let voice = new_moog_voice(SAMPLE_RATE, midi_control, preset);

    let d = &preset.delay;
    let time_l = d.time;
    let time_r = d.time * 1.15;

    let delay_l = Delay::new(
        0.3,
        AudioParam::Static(time_l),
        AudioParam::Static(d.feedback),
        AudioParam::Static(d.mix),
    );

    let delay_r = Delay::new(
        0.3,
        AudioParam::Static(time_r),
        AudioParam::Static(d.feedback),
        AudioParam::Static(d.mix),
    );

    let delay_node = ParallelMixer::new(1.0, DualMono::new(delay_l, delay_r));
    let delay_bypass = Bypass::new(delay_node, d.enabled != 0);

    let r = &preset.reverb;
    let reverb =
        Reverb::new_with_params(AudioParam::Static(r.size), AudioParam::Static(r.damping), 0);

    let reverb_node = ParallelMixer::new(r.mix, reverb);
    let reverb_bypass = Bypass::new(reverb_node, r.enabled != 0);

    let widener = StereoWidener::new(AudioParam::Static(1.5));
    let gain = Gain::new_fixed(0.5);

    DspChain::new(voice, SAMPLE_RATE)
        .to_stereo()
        .and(delay_bypass)
        .and(reverb_bypass)
        .and(widener)
        .and(gain)
}

async fn print_stats(stack_ptr: usize) {
    let free = HEAP.free();
    let used = HEAP.used();
    log_status!("Core 1: DSP Chain initialized\r\n");
    log_status!("Core 1: Memory used: {} KB\r\n", used / 1024);
    log_status!("Core 1: Memory free: {} KB\r\n", free / 1024);

    let stack_slice =
        unsafe { core::slice::from_raw_parts(stack_ptr as *const u8, CORE1_STACK_SIZE) };
    let mut unused = 0;
    for &byte in stack_slice {
        if byte == 0x55 {
            unused += 1;
        } else {
            break;
        }
    }
    let used_stack = CORE1_STACK_SIZE - unused;
    log_status!(
        "Core 1: Stack used: {} / {} bytes\r\n",
        used_stack,
        CORE1_STACK_SIZE
    );
}

#[embassy_executor::task]
pub async fn core1_task(midi_control: Arc<MidiControl>, initial_preset: Preset, stack_ptr: usize) {
    disable_denormals();

    log_status!("Core 1: Starting...\r\n");
    midi_control.set_portamento(initial_preset.portamento);
    log_status!("Core 1: Measuring memory pressure.\r\n");

    let mut synth: Option<Box<dyn FrameProcessor<Stereo> + Send>> =
        Some(Box::new(build_synth(midi_control.clone(), initial_preset)));

    print_stats(stack_ptr).await;
    log_status!("Core 1: DSP Running (STEREO) with Preset\r\n");

    let mut buffer = [0.0; BLOCK_SIZE];
    let mut frame_index: u64 = 0;

    let max_duration_us = (BLOCK_SIZE as f32 / 2.0 / SAMPLE_RATE * 1_000_000.0) as u64;

    loop {
        if let Ok(new_preset) = PRESET_CHANNEL.try_receive() {
            log_status!("Core 1: Switching Preset...\r\n");
            let _ = synth.take();
            print_stats(stack_ptr).await;
            midi_control.set_portamento(new_preset.portamento);

            synth = Some(Box::new(build_synth(midi_control.clone(), new_preset)));

            log_status!("Core 1: Preset Switched.\r\n");
            print_stats(stack_ptr).await;
        }

        let start_time = Instant::now();

        if let Some(s) = &mut synth {
            s.process(&mut buffer, frame_index);
        } else {
            buffer.fill(0.0);
        }

        let end_time = Instant::now();

        if frame_index % (SAMPLE_RATE as u64) < (BLOCK_SIZE as u64 / 2) {
            let duration = (end_time - start_time).as_micros();
            let load = (duration as f32 / max_duration_us as f32) * 100.0;

            let _ = LOG_CHANNEL.try_send(LogData {
                sample: buffer[0],
                duration_us: duration,
                load_percent: load,
            });
        }

        AUDIO_CHANNEL.send(AudioData { buffer }).await;

        frame_index = frame_index.wrapping_add((BLOCK_SIZE / 2) as u64);
    }
}
