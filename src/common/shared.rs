use crate::data::presets::Preset;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;

pub const SAMPLE_RATE: f32 = 48000.0;
pub const HEAP_SIZE: usize = 400000;
pub const BLOCK_SIZE: usize = 256;
pub const CORE1_STACK_SIZE: usize = 4096;

pub struct AudioData {
    pub buffer: [f32; BLOCK_SIZE],
}

#[derive(Clone, Copy)]
pub enum SystemCommand {
    ResetStorage,
}

pub static AUDIO_CHANNEL: Channel<CriticalSectionRawMutex, AudioData, 4> = Channel::new();
pub static PRESET_CHANNEL: Channel<CriticalSectionRawMutex, Preset, 1> = Channel::new();
pub static COMMAND_CHANNEL: Channel<CriticalSectionRawMutex, SystemCommand, 2> = Channel::new();

pub fn disable_denormals() {
    unsafe {
        let fpscr: u32;
        core::arch::asm!("vmrs {}, fpscr", out(reg) fpscr);
        let new_fpscr = fpscr | (1 << 24) | (1 << 25);
        core::arch::asm!("vmsr fpscr, {}", in(reg) new_fpscr);
    }
}
