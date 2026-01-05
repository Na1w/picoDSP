use alloc::sync::Arc;
use embassy_executor::Spawner;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{PIN_25, USB};
use embassy_rp::usb::Driver;
use embassy_rp::Peri;

use crate::common::shared::{AUDIO_CHANNEL, BLOCK_SIZE, HEAP_SIZE, SAMPLE_RATE};
use crate::control::midi::{midi_task, MidiControl};
use crate::data::storage::Storage;
use crate::usb::device;
use crate::usb::logger;

pub async fn main_task(
    spawner: Spawner,
    usb: Peri<'static, USB>,
    pin_25: Peri<'static, PIN_25>,
    midi_control: Arc<MidiControl>,
    storage: Storage<'static>,
) {
    let usb_device = {
        let driver = Driver::new(usb, crate::Irqs);
        Some(device::init(spawner, driver))
    };

    let led = Output::new(pin_25, Level::Low);

    if let Some(device) = usb_device {
        spawner
            .spawn(logger::logger_task(
                device.sender,
                SAMPLE_RATE,
                BLOCK_SIZE,
                HEAP_SIZE,
            ))
            .unwrap();
        spawner.spawn(logger::led_task(led)).unwrap();

        spawner
            .spawn(midi_task(
                device.midi_receiver,
                device.midi_sender,
                midi_control,
                storage,
            ))
            .unwrap();

        let mut microphone = device.microphone;

        let mut dsp_buffer: [f32; BLOCK_SIZE] = [0.0; BLOCK_SIZE];
        let mut dsp_buffer_idx = BLOCK_SIZE;

        loop {
            let mut usb_frames_collected = 0;
            let mut usb_audio_bytes = [0u8; 48 * 2 * 2];

            while usb_frames_collected < 48 {
                if dsp_buffer_idx >= BLOCK_SIZE {
                    let audio_data = AUDIO_CHANNEL.receive().await;
                    dsp_buffer = audio_data.buffer;
                    dsp_buffer_idx = 0;
                }

                if dsp_buffer_idx < BLOCK_SIZE {
                    let available_floats = BLOCK_SIZE - dsp_buffer_idx;
                    let available_frames = available_floats / 2;
                    let needed_frames = 48 - usb_frames_collected;
                    let frames_to_copy = available_frames.min(needed_frames);

                    for _ in 0..frames_to_copy {
                        let l_sample = dsp_buffer[dsp_buffer_idx];
                        let r_sample = dsp_buffer[dsp_buffer_idx + 1];
                        dsp_buffer_idx += 2;

                        let sample_l = (l_sample.clamp(-1.0, 1.0) * 32767.0) as i16;
                        let sample_r = (r_sample.clamp(-1.0, 1.0) * 32767.0) as i16;

                        let bytes_l = sample_l.to_le_bytes();
                        let bytes_r = sample_r.to_le_bytes();

                        let byte_idx = usb_frames_collected * 4;
                        usb_audio_bytes[byte_idx] = bytes_l[0];
                        usb_audio_bytes[byte_idx + 1] = bytes_l[1];
                        usb_audio_bytes[byte_idx + 2] = bytes_r[0];
                        usb_audio_bytes[byte_idx + 3] = bytes_r[1];

                        usb_frames_collected += 1;
                    }
                }
            }

            let _ = microphone.write_packet(&usb_audio_bytes).await;
        }
    } else {
        loop {
            let _ = AUDIO_CHANNEL.receive().await;
        }
    }
}
