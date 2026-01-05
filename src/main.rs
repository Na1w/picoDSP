#![no_std]
#![no_main]

extern crate alloc;

mod common;
mod control;
mod data;
mod dsp;
mod tasks;
mod usb;

use alloc::sync::Arc;
use core::mem::MaybeUninit;
use core::ptr::addr_of_mut;

use defmt_rtt as _;
use embassy_executor::Spawner;
use embedded_alloc::LlffHeap as Heap;
use panic_halt as _;

use embassy_rp::bind_interrupts;
use embassy_rp::flash::Flash;
use embassy_rp::multicore::{spawn_core1, Stack};
use embassy_rp::peripherals::USB;
use embassy_rp::usb::InterruptHandler;

use static_cell::StaticCell;

use crate::common::shared::{disable_denormals, CORE1_STACK_SIZE, HEAP_SIZE};
use crate::control::midi::MidiControl;
use crate::data::storage::Storage;
use crate::tasks::{core0, core1};

#[global_allocator]
pub static HEAP: Heap = Heap::empty();

bind_interrupts!(pub struct Irqs {
    USBCTRL_IRQ => InterruptHandler<USB>;
});

fn init_heap() {
    static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
    unsafe {
        HEAP.init(addr_of_mut!(HEAP_MEM) as usize, HEAP_SIZE);
    }
}

#[link_section = ".sram4"]
static mut CORE1_STACK: Stack<CORE1_STACK_SIZE> = Stack::new();

#[link_section = ".sram5"]
static EXECUTOR1: StaticCell<embassy_executor::Executor> = StaticCell::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    disable_denormals();

    init_heap();
    let p = embassy_rp::init(Default::default());

    let flash = Flash::new(p.FLASH, p.DMA_CH0);
    let mut storage = Storage::new(flash);
    storage.init().await;

    let preset = storage
        .load_preset(4)
        .await
        .unwrap_or_else(|| crate::data::presets::get_default_presets()[4]);

    let midi_control = Arc::new(MidiControl::new());

    let cutoff_norm = libm::log10f(preset.filter.cutoff / 20.0) / libm::log10f(1000.0);
    midi_control.set_parameter_1(cutoff_norm.clamp(0.0, 1.0));
    let res_norm = (preset.filter.resonance - 0.707) / 9.3;
    midi_control.set_parameter_2(res_norm.clamp(0.0, 1.0));

    let midi_control_core1 = midi_control.clone();

    unsafe {
        let stack_ptr = addr_of_mut!(CORE1_STACK) as *mut u8;
        core::ptr::write_bytes(stack_ptr, 0x55, CORE1_STACK_SIZE);
    }

    let stack_ptr_val = addr_of_mut!(CORE1_STACK) as usize;

    spawn_core1(
        p.CORE1,
        unsafe { &mut *addr_of_mut!(CORE1_STACK) },
        move || {
            let executor = EXECUTOR1.init(embassy_executor::Executor::new());
            executor.run(|spawner| {
                spawner
                    .spawn(core1::core1_task(midi_control_core1, preset, stack_ptr_val))
                    .unwrap();
            });
        },
    );

    core0::main_task(spawner, p.USB, p.PIN_25, midi_control, storage).await;
}
