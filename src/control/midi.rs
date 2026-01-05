use alloc::sync::Arc;
use alloc::vec;
use core::sync::atomic::{AtomicU32, AtomicBool, Ordering};
use infinitedsp_core::FrameProcessor;
use infinitedsp_core::core::channels::Mono;
use embassy_usb::class::midi::{Receiver, Sender};
use embassy_rp::usb::Driver;
use embassy_rp::peripherals::USB;
use embassy_time::Instant;
use crate::usb::logger::{MIDI_LOG_CHANNEL, LED_SIGNAL_CHANNEL};
use crate::data::storage::{Storage, MAGIC as STORAGE_MAGIC, VERSION as STORAGE_VERSION};
use crate::common::shared::{PRESET_CHANNEL, COMMAND_CHANNEL, SystemCommand};
use embassy_futures::select::{select, Either};

macro_rules! log_midi {
    ($($arg:tt)*) => {
        {
            let mut msg = heapless::String::<64>::new();
            let now = Instant::now().as_millis();
            if core::fmt::write(&mut msg, format_args!("\r\n[{}] ", now)).is_ok() {
                if core::fmt::write(&mut msg, format_args!($($arg)*)).is_ok() {
                    let _ = MIDI_LOG_CHANNEL.try_send(msg);
                }
            }
        }
    };
}

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xB0;
const PROGRAM_CHANGE: u8 = 0xC0;
const PITCH_BEND: u8 = 0xE0;
const SYSEX_START: u8 = 0xF0;
const SYSEX_END: u8 = 0xF7;

const CC_MOD_WHEEL: u8 = 1;
const CC_PORTAMENTO_TIME: u8 = 5;
const CC_SUSTAIN: u8 = 64;
const CC_FILTER_RESONANCE: u8 = 71;
const CC_FILTER_CUTOFF: u8 = 74;
const CC_ALL_SOUND_OFF: u8 = 120;
const CC_ALL_NOTES_OFF: u8 = 123;

const SYSEX_ID: u8 = 0x7D;
const SYSEX_MODEL: u8 = 0x01;
const CMD_DUMP_REQ: u8 = 0x01;
const CMD_WRITE_REQ: u8 = 0x02;
const CMD_WRITE_SUCCESS: u8 = 0x03;
const CMD_WRITE_ERROR: u8 = 0x04;

const ERR_BAD_LENGTH: u8 = 0x01;
const ERR_BAD_MAGIC: u8 = 0x02;

fn midi_to_freq(note: u8) -> f32 {
    440.0 * libm::powf(2.0, (note as f32 - 69.0) / 12.0)
}

struct NoteStack {
    notes: heapless::Vec<u8, 16>,
    pending_off: heapless::Vec<u8, 16>,
    sustain_active: bool,
}

impl NoteStack {
    fn new() -> Self {
        Self {
            notes: heapless::Vec::new(),
            pending_off: heapless::Vec::new(),
            sustain_active: false,
        }
    }

    fn note_on(&mut self, note: u8) {
        if let Some(pos) = self.pending_off.iter().position(|&n| n == note) {
            self.pending_off.remove(pos);
        }
        if !self.notes.contains(&note) {
            let _ = self.notes.push(note);
        }
    }

    fn note_off(&mut self, note: u8) {
        if self.sustain_active {
            if self.notes.contains(&note) && !self.pending_off.contains(&note) {
                let _ = self.pending_off.push(note);
            }
        } else {
            if let Some(pos) = self.notes.iter().position(|&n| n == note) {
                self.notes.remove(pos);
            }
        }
    }

    fn set_sustain(&mut self, active: bool) {
        self.sustain_active = active;
        if !active {
            for note in self.pending_off.iter() {
                if let Some(pos) = self.notes.iter().position(|n| n == note) {
                    self.notes.remove(pos);
                }
            }
            self.pending_off.clear();
        }
    }

    fn clear(&mut self) {
        self.notes.clear();
        self.pending_off.clear();
        self.sustain_active = false;
    }

    fn active_note(&self) -> Option<u8> {
        self.notes.last().copied()
    }
}

pub struct MidiControl {
    target_freq_bits: AtomicU32,
    gate: AtomicBool,
    gate_reset: AtomicBool,
    portamento_amount_bits: AtomicU32,
    pitch_bend_bits: AtomicU32,
    mod_wheel_bits: AtomicU32,
    parameter_1_bits: AtomicU32,
    parameter_2_bits: AtomicU32,
}

impl MidiControl {
    pub fn new() -> Self {
        Self {
            target_freq_bits: AtomicU32::new(440.0f32.to_bits()),
            gate: AtomicBool::new(false),
            gate_reset: AtomicBool::new(false),
            portamento_amount_bits: AtomicU32::new(0.0f32.to_bits()),
            pitch_bend_bits: AtomicU32::new(1.0f32.to_bits()),
            mod_wheel_bits: AtomicU32::new(0.0f32.to_bits()),
            parameter_1_bits: AtomicU32::new(0.5f32.to_bits()),
            parameter_2_bits: AtomicU32::new(0.0f32.to_bits()),
        }
    }

    pub fn set_freq(&self, freq: f32) {
        self.target_freq_bits.store(freq.to_bits(), Ordering::Relaxed);
    }

    pub fn set_gate(&self, gate: bool) {
        self.gate.store(gate, Ordering::Relaxed);
        if !gate {
            self.gate_reset.store(true, Ordering::Relaxed);
        }
    }

    pub fn take_gate_reset(&self) -> bool {
        self.gate_reset.swap(false, Ordering::Relaxed)
    }

    pub fn set_portamento(&self, amount: f32) {
        self.portamento_amount_bits.store(amount.to_bits(), Ordering::Relaxed);
    }

    pub fn set_pitch_bend(&self, bend_factor: f32) {
        self.pitch_bend_bits.store(bend_factor.to_bits(), Ordering::Relaxed);
    }

    pub fn set_mod_wheel(&self, value: f32) {
        self.mod_wheel_bits.store(value.to_bits(), Ordering::Relaxed);
    }

    pub fn set_parameter_1(&self, value: f32) {
        self.parameter_1_bits.store(value.to_bits(), Ordering::Relaxed);
    }

    pub fn set_parameter_2(&self, value: f32) {
        self.parameter_2_bits.store(value.to_bits(), Ordering::Relaxed);
    }

    pub fn reset(&self) {
        self.gate.store(false, Ordering::Relaxed);
        self.gate_reset.store(false, Ordering::Relaxed);
        self.pitch_bend_bits.store(1.0f32.to_bits(), Ordering::Relaxed);
        self.mod_wheel_bits.store(0.0f32.to_bits(), Ordering::Relaxed);
    }

    pub fn get_target_freq(&self) -> f32 {
        f32::from_bits(self.target_freq_bits.load(Ordering::Relaxed))
    }

    pub fn get_portamento_amount(&self) -> f32 {
        f32::from_bits(self.portamento_amount_bits.load(Ordering::Relaxed))
    }

    pub fn get_pitch_bend(&self) -> f32 {
        f32::from_bits(self.pitch_bend_bits.load(Ordering::Relaxed))
    }

    #[allow(dead_code)]
    pub fn get_mod_wheel(&self) -> f32 {
        f32::from_bits(self.mod_wheel_bits.load(Ordering::Relaxed))
    }

    pub fn get_parameter_1(&self) -> f32 {
        f32::from_bits(self.parameter_1_bits.load(Ordering::Relaxed))
    }

    pub fn get_parameter_2(&self) -> f32 {
        f32::from_bits(self.parameter_2_bits.load(Ordering::Relaxed))
    }

    pub fn get_gate(&self) -> f32 {
        if self.gate.load(Ordering::Relaxed) { 1.0 } else { 0.0 }
    }
}

pub struct MidiFreq {
    control: Arc<MidiControl>,
    current_freq: f32,
}

impl MidiFreq {
    pub fn new(control: Arc<MidiControl>) -> Self {
        let initial_freq = control.get_target_freq();
        Self {
            control,
            current_freq: initial_freq,
        }
    }
}

impl FrameProcessor<Mono> for MidiFreq {
    fn process(&mut self, buffer: &mut [f32], _frame_index: u64) {
        let target = self.control.get_target_freq();
        let amount = self.control.get_portamento_amount();
        let bend = self.control.get_pitch_bend();

        const CHUNK_SIZE: usize = 32;
        let factor = 1.0 - amount.clamp(0.0, 0.999);

        for chunk in buffer.chunks_mut(CHUNK_SIZE) {
            let diff = target - self.current_freq;

            if diff.abs() < 0.1 {
                self.current_freq = target;
            } else {
                self.current_freq += diff * factor;
            }

            let final_freq = self.current_freq * bend;

            for sample in chunk.iter_mut() {
                *sample = final_freq;
            }
        }
    }

    fn set_sample_rate(&mut self, _sample_rate: f32) {}

    fn reset(&mut self) {
        self.current_freq = self.control.get_target_freq();
    }

    fn latency_samples(&self) -> u32 { 0 }
    fn name(&self) -> &str { "MidiFreq" }
    fn visualize(&self, _indent: usize) -> alloc::string::String { "MidiFreq".into() }
}

pub struct MidiGate(pub Arc<MidiControl>);
impl FrameProcessor<Mono> for MidiGate {
    fn process(&mut self, buffer: &mut [f32], _frame_index: u64) {
        let g = self.0.get_gate();
        let reset = self.0.take_gate_reset();

        for (i, sample) in buffer.iter_mut().enumerate() {
            if reset && i < 4 {
                *sample = 0.0;
            } else {
                *sample = g;
            }
        }
    }
    fn set_sample_rate(&mut self, _sample_rate: f32) {}
    fn reset(&mut self) {}
    fn latency_samples(&self) -> u32 { 0 }
    fn name(&self) -> &str { "MidiGate" }
    fn visualize(&self, _indent: usize) -> alloc::string::String { "MidiGate".into() }
}

pub struct MidiFilterCutoff(pub Arc<MidiControl>);
impl FrameProcessor<Mono> for MidiFilterCutoff {
    fn process(&mut self, buffer: &mut [f32], _frame_index: u64) {
        let val = self.0.get_parameter_1();
        let freq = 20.0 * libm::powf(1000.0, val);
        for sample in buffer.iter_mut() {
            *sample = freq;
        }
    }
    fn set_sample_rate(&mut self, _sample_rate: f32) {}
    fn reset(&mut self) {}
    fn latency_samples(&self) -> u32 { 0 }
    fn name(&self) -> &str { "MidiFilterCutoff" }
    fn visualize(&self, _indent: usize) -> alloc::string::String { "MidiFilterCutoff".into() }
}

pub struct MidiFilterResonance(pub Arc<MidiControl>);
impl FrameProcessor<Mono> for MidiFilterResonance {
    fn process(&mut self, buffer: &mut [f32], _frame_index: u64) {
        let val = self.0.get_parameter_2();
        let q = 0.707 + (val * 9.3);
        for sample in buffer.iter_mut() {
            *sample = q;
        }
    }
    fn set_sample_rate(&mut self, _sample_rate: f32) {}
    fn reset(&mut self) {}
    fn latency_samples(&self) -> u32 { 0 }
    fn name(&self) -> &str { "MidiFilterResonance" }
    fn visualize(&self, _indent: usize) -> alloc::string::String { "MidiFilterResonance".into() }
}

#[embassy_executor::task]
pub async fn midi_task(
    mut receiver: Receiver<'static, Driver<'static, USB>>,
    mut sender: Sender<'static, Driver<'static, USB>>,
    midi_control: Arc<MidiControl>,
    mut storage: Storage<'static>
) {
    let mut buf = [0; 64];
    let mut notes = NoteStack::new();

    let mut current_preset_index = 4;

    let mut sysex_buf = vec![0u8; 8192 + 32];
    let mut sysex_idx = 0;
    let mut in_sysex = false;

    loop {
        receiver.wait_connection().await;

        loop {
            match select(receiver.read_packet(&mut buf), COMMAND_CHANNEL.receive()).await {
                Either::First(read_result) => {
                    match read_result {
                        Ok(n) => {
                            let data = &buf[..n];
                            for packet in data.chunks(4) {
                                if packet.len() < 4 { continue; }

                                let cin = packet[0] & 0x0F;
                                let status = packet[1];
                                let d1 = packet[2];
                                let d2 = packet[3];

                                if cin == 0x4 || cin == 0x5 || cin == 0x6 || cin == 0x7 {
                                    if cin == 0x4 {
                                        if !in_sysex {
                                            in_sysex = true;
                                            sysex_idx = 0;
                                        }
                                        if sysex_idx + 3 < sysex_buf.len() {
                                            sysex_buf[sysex_idx] = packet[1];
                                            sysex_buf[sysex_idx+1] = packet[2];
                                            sysex_buf[sysex_idx+2] = packet[3];
                                            sysex_idx += 3;
                                        }
                                    } else {
                                        let len = match cin {
                                            0x5 => 1,
                                            0x6 => 2,
                                            0x7 => 3,
                                            _ => 0,
                                        };

                                        if in_sysex && sysex_idx + len <= sysex_buf.len() {
                                            sysex_buf[sysex_idx..sysex_idx + len].copy_from_slice(&packet[1..1 + len]);
                                            sysex_idx += len;
                                            in_sysex = false;

                                            let msg = &sysex_buf[..sysex_idx];
                                            if msg.len() >= 5 && msg[0] == SYSEX_START && msg[msg.len()-1] == SYSEX_END {
                                                if msg[1] == SYSEX_ID && msg[2] == SYSEX_MODEL {
                                                    let cmd = msg[3];
                                                    match cmd {
                                                        CMD_DUMP_REQ => {
                                                            log_midi!("SysEx: Dump Request\r\n");
                                                            let mut raw_data = vec![0u8; 4096];
                                                            storage.read_raw(&mut raw_data).await;

                                                            let p1 = [0x04, SYSEX_START, SYSEX_ID, SYSEX_MODEL];
                                                            let _ = sender.write_packet(&p1).await;

                                                            let h0 = (raw_data[0] >> 4) & 0x0F;
                                                            let l0 = raw_data[0] & 0x0F;
                                                            let p2 = [0x04, CMD_WRITE_REQ, h0, l0];
                                                            let _ = sender.write_packet(&p2).await;

                                                            let mut encoded_buf = vec![0u8; 8190];
                                                            let mut enc_idx = 0;
                                                            for byte in raw_data.iter().skip(1) {
                                                                encoded_buf[enc_idx] = (byte >> 4) & 0x0F;
                                                                encoded_buf[enc_idx+1] = byte & 0x0F;
                                                                enc_idx += 2;
                                                            }

                                                            let mut i = 0;
                                                            let mut f7_sent = false;
                                                            let mut packet_count = 2;

                                                            while i < encoded_buf.len() {
                                                                let remaining = encoded_buf.len() - i;
                                                                if remaining >= 3 {
                                                                    let _ = sender.write_packet(&[0x04, encoded_buf[i], encoded_buf[i+1], encoded_buf[i+2]]).await;
                                                                    i += 3;
                                                                    packet_count += 1;
                                                                } else {
                                                                    let packet = if remaining == 2 {
                                                                        [0x07, encoded_buf[i], encoded_buf[i+1], 0xF7]
                                                                    } else if remaining == 1 {
                                                                        [0x06, encoded_buf[i], 0xF7, 0x00]
                                                                    } else {
                                                                        [0x05, 0xF7, 0x00, 0x00]
                                                                    };

                                                                    let _ = sender.write_packet(&packet).await;
                                                                    packet_count += 1;
                                                                    f7_sent = true;
                                                                    break;
                                                                }
                                                            }

                                                            if !f7_sent {
                                                                 let p_end = [0x05, 0xF7, 0x00, 0x00];
                                                                 let _ = sender.write_packet(&p_end).await;
                                                                 packet_count += 1;
                                                            }
                                                            log_midi!("SysEx: Dump Sent ({} packets)\r\n", packet_count);
                                                        },
                                                        CMD_WRITE_REQ => {
                                                            log_midi!("SysEx: Write Request ({} bytes)\r\n", msg.len());
                                                            let encoded_data = &msg[4..msg.len()-1];
                                                            if encoded_data.len() == 8192 {
                                                                let mut decoded_data = vec![0u8; 4096];
                                                                for i in 0..4096 {
                                                                    let h = encoded_data[i*2];
                                                                    let l = encoded_data[i*2+1];
                                                                    decoded_data[i] = (h << 4) | (l & 0x0F);
                                                                }

                                                                let magic = u32::from_le_bytes([decoded_data[0], decoded_data[1], decoded_data[2], decoded_data[3]]);
                                                                let version = u32::from_le_bytes([decoded_data[4], decoded_data[5], decoded_data[6], decoded_data[7]]);

                                                                if magic == STORAGE_MAGIC && version == STORAGE_VERSION {
                                                                    storage.write_raw(&decoded_data).await;
                                                                    log_midi!("SysEx: Write Success\r\n");

                                                                    let _ = sender.write_packet(&[0x04, SYSEX_START, SYSEX_ID, SYSEX_MODEL]).await;
                                                                    let _ = sender.write_packet(&[0x06, CMD_WRITE_SUCCESS, SYSEX_END, 0x00]).await;

                                                                    if let Some(preset) = storage.load_preset(current_preset_index).await {
                                                                        log_midi!("Reloading active preset {}\r\n", current_preset_index);
                                                                        let cutoff_norm = libm::log10f(preset.filter.cutoff / 20.0) / libm::log10f(1000.0);
                                                                        midi_control.set_parameter_1(cutoff_norm.clamp(0.0, 1.0));
                                                                        let res_norm = (preset.filter.resonance - 0.707) / 9.3;
                                                                        midi_control.set_parameter_2(res_norm.clamp(0.0, 1.0));
                                                                        midi_control.set_portamento(preset.portamento);

                                                                        let _ = PRESET_CHANNEL.try_send(preset);
                                                                    }

                                                                } else {
                                                                    log_midi!("SysEx: Invalid Magic/Version ({:X}, {})\r\n", magic, version);
                                                                    let _ = sender.write_packet(&[0x04, SYSEX_START, SYSEX_ID, SYSEX_MODEL]).await;
                                                                    let _ = sender.write_packet(&[0x07, CMD_WRITE_ERROR, ERR_BAD_MAGIC, SYSEX_END]).await;
                                                                }
                                                            } else {
                                                                log_midi!("SysEx: Invalid Length ({})\r\n", encoded_data.len());
                                                                let _ = sender.write_packet(&[0x04, SYSEX_START, SYSEX_ID, SYSEX_MODEL]).await;
                                                                let _ = sender.write_packet(&[0x07, CMD_WRITE_ERROR, ERR_BAD_LENGTH, SYSEX_END]).await;
                                                            }
                                                        },
                                                        _ => {}
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    continue;
                                }

                                log_midi!("MIDI: [{:02X}-{:02X}-{:02X}-{:02X}] - ", cin, status, d1, d2);

                                let cmd = status & 0xF0;

                                match cmd {
                                    NOTE_ON if d2 > 0 => {
                                        let freq = midi_to_freq(d1);
                                        log_midi!("NOTE ON: {} ({} Hz)", d1, freq);
                                        notes.note_on(d1);
                                        midi_control.set_freq(freq);
                                        midi_control.set_gate(true);
                                        let _ = LED_SIGNAL_CHANNEL.try_send(true);
                                    },
                                    NOTE_OFF | NOTE_ON => {
                                        let freq = midi_to_freq(d1);
                                        log_midi!("NOTE OFF: {}", freq);
                                        notes.note_off(d1);

                                        if let Some(last_note) = notes.active_note() {
                                            midi_control.set_freq(midi_to_freq(last_note));
                                            midi_control.set_gate(true);
                                        } else {
                                            midi_control.set_gate(false);
                                            let _ = LED_SIGNAL_CHANNEL.try_send(false);
                                        }
                                    },
                                    CONTROL_CHANGE => {
                                        let val_norm = d2 as f32 / 127.0;
                                        match d1 {
                                            CC_MOD_WHEEL => {
                                                log_midi!("MOD WHEEL: {:.2}", val_norm);
                                                midi_control.set_mod_wheel(val_norm);
                                            },
                                            CC_PORTAMENTO_TIME => {
                                                let amount = val_norm;
                                                log_midi!("PORTAMENTO: {:.2}", amount);
                                                midi_control.set_portamento(amount);
                                            },
                                            CC_SUSTAIN => {
                                                let sustain_on = d2 >= 64;
                                                log_midi!("SUSTAIN: {}", if sustain_on { "ON" } else { "OFF" });
                                                notes.set_sustain(sustain_on);

                                                if !sustain_on {
                                                    if let Some(last_note) = notes.active_note() {
                                                        midi_control.set_freq(midi_to_freq(last_note));
                                                        midi_control.set_gate(true);
                                                    } else {
                                                        midi_control.set_gate(false);
                                                        let _ = LED_SIGNAL_CHANNEL.try_send(false);
                                                    }
                                                }
                                            },
                                            CC_FILTER_RESONANCE => {
                                                log_midi!("RESONANCE: {:.2}", val_norm);
                                                midi_control.set_parameter_2(val_norm);
                                            },
                                            CC_FILTER_CUTOFF => {
                                                log_midi!("CUTOFF: {:.2}", val_norm);
                                                midi_control.set_parameter_1(val_norm);
                                            },
                                            CC_ALL_SOUND_OFF | CC_ALL_NOTES_OFF => {
                                                log_midi!("ALL NOTES/SOUND OFF");
                                                notes.clear();
                                                midi_control.reset();
                                                let _ = LED_SIGNAL_CHANNEL.try_send(false);
                                            },
                                            _ => {}
                                        }
                                    },
                                    PROGRAM_CHANGE => {
                                        log_midi!("PROGRAM CHANGE: {}\r\n", d1);
                                        current_preset_index = d1 as usize;
                                        if let Some(preset) = storage.load_preset(d1 as usize).await {
                                            log_midi!("Loaded: {}\r\n", preset.get_name());
                                            let cutoff_norm = libm::log10f(preset.filter.cutoff / 20.0) / libm::log10f(1000.0);
                                            midi_control.set_parameter_1(cutoff_norm.clamp(0.0, 1.0));
                                            let res_norm = (preset.filter.resonance - 0.707) / 9.3;
                                            midi_control.set_parameter_2(res_norm.clamp(0.0, 1.0));
                                            midi_control.set_portamento(preset.portamento);
                                            let _ = PRESET_CHANNEL.try_send(preset);
                                        } else {
                                            log_midi!("Preset {} not found\r\n", d1);
                                        }
                                    },
                                    PITCH_BEND => {
                                        let val = ((d2 as u16) << 7) | (d1 as u16);
                                        log_midi!("PITCHBEND: {}", val);
                                        let norm = (val as f32 - 8192.0) / 8192.0;
                                        let factor = libm::powf(2.0, (norm * 2.0) / 12.0);
                                        midi_control.set_pitch_bend(factor);
                                    },
                                    _ => {}
                                }
                            }
                        },
                        Err(_) => {
                            break;
                        }
                    }
                },
                Either::Second(cmd) => {
                    match cmd {
                        SystemCommand::ResetStorage => {
                            log_midi!("Command: Reset Storage...\r\n");
                            storage.format().await;
                            log_midi!("Storage Reset Complete.\r\n");
                        }
                    }
                }
            }
        }
    }
}
