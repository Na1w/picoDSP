use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embassy_rp::gpio::Output;
use embassy_time::{Duration, Timer};
use embassy_futures::select::{select, Either};
use crate::usb::device::UsbSender;
use crate::HEAP;

extern "C" {
    static __stext: u32;
    static __sdata: u32;
    static __edata: u32;
    static __sidata: u32;
    static __estack: u32;
}

pub struct LogData {
    pub sample: f32,
    pub duration_us: u64,
    pub load_percent: f32,
}

pub static LOG_CHANNEL: Channel<CriticalSectionRawMutex, LogData, 4> = Channel::new();
pub static MIDI_LOG_CHANNEL: Channel<CriticalSectionRawMutex, heapless::String<64>, 8> = Channel::new();
pub static LED_SIGNAL_CHANNEL: Channel<CriticalSectionRawMutex, bool, 4> = Channel::new();
pub static SYSTEM_STATUS_CHANNEL: Channel<CriticalSectionRawMutex, heapless::String<64>, 4> = Channel::new();

static LOG_BUF: Mutex<CriticalSectionRawMutex, heapless::String<128>> = Mutex::new(heapless::String::new());

async fn write_bytes(sender: &mut UsbSender, data: &[u8]) {
    select(
        sender.write_packet(data),
        Timer::after(Duration::from_millis(10)),
    )
    .await;
}

async fn write_log(sender: &mut UsbSender, args: core::fmt::Arguments<'_>) {
    let mut buf = LOG_BUF.lock().await;
    buf.clear();
    let _ = core::fmt::write(&mut *buf, args);
    write_bytes(sender, buf.as_bytes()).await;
}

macro_rules! log {
    ($sender:expr, $($arg:tt)*) => {
        write_log($sender, format_args!($($arg)*)).await
    };
}

fn get_stack_pointer() -> usize {
    let sp: usize;
    unsafe {
        core::arch::asm!("mov {}, sp", out(reg) sp);
    }
    sp
}

async fn print_heap_info(sender: &mut UsbSender, total_size: usize) {
    let free = HEAP.free();
    let used = HEAP.used();
    log!(sender, "  Heap Size:   {} KB (Used: {} KB, Free: {} KB)\r\n", total_size / 1024, used / 1024, free / 1024);
}

async fn print_stack_info(sender: &mut UsbSender) {
    let stack_top = 0x20080000;
    let current_sp = get_stack_pointer();
    let stack_used = stack_top - current_sp;
    log!(sender, "  Core 0 Stack: ~{} bytes used (Current SP: {:#010X})\r\n", stack_used, current_sp);
}

pub async fn print_system_info(sender: &mut UsbSender, sample_rate: f32, block_size: usize, heap_size: usize) {
    log!(sender, "\r\n--- PicoDSP (infinitedsp-core {}) Started ---\r\n", env!("INFINITEDSP_CORE_VERSION"));
    log!(sender, "Hardware:\r\n");
    log!(sender, "  MCU:  RP2350 (Pico 2)\r\n");
    let sys_freq = embassy_rp::clocks::clk_sys_freq();
    log!(sender, "  CPU:  Dual Core Cortex-M33 @ {} MHz\r\n", sys_freq / 1_000_000);

    let text_start = unsafe { &__stext as *const u32 as usize };
    let data_start_ram = unsafe { &__sdata as *const u32 as usize };
    let data_end_ram = unsafe { &__edata as *const u32 as usize };
    let data_start_flash = unsafe { &__sidata as *const u32 as usize };

    let code_size = data_start_flash - text_start;
    let data_size = data_end_ram - data_start_ram;

    let prog_size = code_size + data_size;
    let prog_end = data_start_flash + data_size;

    const FLASH_BASE: usize = 0x10000000;
    const FLASH_SIZE_KB: usize = parse_int(env!("TOTAL_FLASH_KB"));
    const FLASH_SIZE: usize = FLASH_SIZE_KB * 1024;
    const STORAGE_SIZE: usize = 64 * 1024;

    let storage_start = FLASH_BASE + FLASH_SIZE - STORAGE_SIZE;
    let storage_end = FLASH_BASE + FLASH_SIZE;

    log!(sender, "Memory Map:\r\n");
    log!(sender, "  Flash Code: {:4} KB  [{:#010X} - {:#010X}]\r\n", prog_size / 1024, text_start, prog_end);
    log!(sender, "  Presets:    {:4} KB  [{:#010X} - {:#010X}]\r\n", STORAGE_SIZE / 1024, storage_start, storage_end);
    log!(sender, "  Total Used: {:4} KB  (of {} KB)\r\n", (prog_size + STORAGE_SIZE) / 1024, FLASH_SIZE / 1024);

    log!(sender, "  RAM:        {} KB.\r\n", env!("TOTAL_RAM_KB"));

    log!(sender, "Config:\r\n");
    log!(sender, "  Sample Rate: {:.1} kHz\r\n", sample_rate / 1000.0);
    log!(sender, "  Bit Depth:   32-bit Float (Internal)\r\n");
    let latency_ms = (block_size as f32 / 2.0) / sample_rate * 1000.0;
    log!(sender, "  Block Size:  {} samples ({:.2} ms)\r\n", block_size / 2, latency_ms);

    print_heap_info(sender, heap_size).await;
    print_stack_info(sender).await;

    log!(sender, "Topology:\r\n");
    log!(sender, "  [Osc1+2+3] -> [Mixer] -> [Filter] -> ");
    log!(sender, "[VCA] -> [Delay] -> [Reverb] -> [Out]\r\n");
    log!(sender, "------------------------------------------------\r\n");
}

const fn parse_int(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut res = 0;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b >= b'0' && b <= b'9' {
            res = res * 10 + (b - b'0') as usize;
        }
        i += 1;
    }
    res
}

#[embassy_executor::task]
pub async fn logger_task(mut sender: UsbSender, sample_rate: f32, block_size: usize, heap_size: usize) {
    Timer::after(Duration::from_secs(2)).await;

    print_system_info(&mut sender, sample_rate, block_size, heap_size).await;

    loop {
        match select(select(LOG_CHANNEL.receive(), MIDI_LOG_CHANNEL.receive()), SYSTEM_STATUS_CHANNEL.receive()).await {
            Either::First(inner) => match inner {
                Either::First(log_data) => {
                    let stack_top = 0x20080000;
                    let sp = get_stack_pointer();
                    let c0_stack = stack_top - sp;

                    log!(&mut sender, "S: {:>7.4} | Time: {:>4}us | Load: {:>4.1}% | C0: {:>5}B\r\n",
                        log_data.sample,
                        log_data.duration_us,
                        log_data.load_percent,
                        c0_stack
                    );
                }
                Either::Second(msg) => {
                    write_bytes(&mut sender, msg.as_bytes()).await;
                }
            },
            Either::Second(msg) => {
                write_bytes(&mut sender, msg.as_bytes()).await;
            }
        }
    }
}

#[embassy_executor::task]
pub async fn led_task(mut led: Output<'static>) {
    loop {
        let state = LED_SIGNAL_CHANNEL.receive().await;
        if state {
            led.set_high();
        } else {
            led.set_low();
        }
    }
}
