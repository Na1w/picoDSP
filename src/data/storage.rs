use embassy_rp::flash::{Flash, Async, ERASE_SIZE};
use embassy_rp::peripherals::FLASH;
use embedded_storage_async::nor_flash::NorFlash;
use crate::data::presets::{Preset, get_default_presets};
use crate::usb::logger::SYSTEM_STATUS_CHANNEL;

// "PDSP"
pub const MAGIC: u32 = 0x50445350;
pub const VERSION: u32 = 7;

const FLASH_SIZE: u32 = 2 * 1024 * 1024;
const STORAGE_SIZE: u32 = 64 * 1024;
const ADDR_OFFSET: u32 = FLASH_SIZE - STORAGE_SIZE;
const SECTOR_SIZE: u32 = 4096;

#[repr(C)]
struct StorageHeader {
    magic: u32,
    version: u32,
    num_presets: u32,
    padding: u32,
}

pub struct Storage<'d> {
    flash: Flash<'d, FLASH, Async, 2097152>,
}

macro_rules! log_storage {
    ($($arg:tt)*) => {
        {
            let mut msg = heapless::String::<64>::new();
            if core::fmt::write(&mut msg, format_args!($($arg)*)).is_ok() {
                let _ = SYSTEM_STATUS_CHANNEL.try_send(msg);
            }
        }
    };
}

impl<'d> Storage<'d> {
    pub fn new(flash: Flash<'d, FLASH, Async, 2097152>) -> Self {
        Self { flash }
    }

    pub async fn init(&mut self) {
        let mut buf = [0u8; 16];
        self.flash.read(ADDR_OFFSET, &mut buf).await.unwrap();

        let header: StorageHeader = unsafe { core::ptr::read(buf.as_ptr() as *const _) };

        if header.magic != MAGIC || header.version != VERSION {
            log_storage!("Storage init/version mismatch. Formatting...\r\n");
            self.format().await;
        } else {
            log_storage!("Storage initialized. Found {} presets.\r\n", header.num_presets);
        }
    }

    pub async fn format(&mut self) {
        log_storage!("Formatting storage area...\r\n");

        self.flash.erase(ADDR_OFFSET, ADDR_OFFSET + ERASE_SIZE as u32).await.unwrap();

        let defaults = get_default_presets();
        let num_presets = defaults.len() as u32;

        let header = StorageHeader {
            magic: MAGIC,
            version: VERSION,
            num_presets,
            padding: 0,
        };

        let mut sector_buf = [0u8; 4096];

        let header_bytes: [u8; 16] = unsafe { core::mem::transmute(header) };
        sector_buf[0..16].copy_from_slice(&header_bytes);

        let mut current_pos = 16;
        for preset in defaults.iter() {
            let size = core::mem::size_of::<Preset>();
            let bytes = unsafe { core::slice::from_raw_parts(preset as *const _ as *const u8, size) };
            sector_buf[current_pos..current_pos+size].copy_from_slice(bytes);
            current_pos += size;
        }

        self.flash.write(ADDR_OFFSET, &sector_buf).await.unwrap();
        log_storage!("Formatted and wrote defaults.\r\n");
    }

    pub async fn load_preset(&mut self, index: usize) -> Option<Preset> {
        let mut buf = [0u8; 16];
        self.flash.read(ADDR_OFFSET, &mut buf).await.unwrap();
        let header: StorageHeader = unsafe { core::ptr::read(buf.as_ptr() as *const _) };

        if index >= header.num_presets as usize {
            log_storage!("Error: Preset index {} out of bounds (max {})\r\n", index, header.num_presets - 1);
            return None;
        }

        let preset_size = core::mem::size_of::<Preset>();
        let offset = ADDR_OFFSET + 16 + (index * preset_size) as u32;

        let mut preset_buf = [0u8; 256];
        if preset_size > 256 {
            log_storage!("Error: Preset too large for buffer!\r\n");
            return None;
        }

        self.flash.read(offset, &mut preset_buf[..preset_size]).await.unwrap();

        let preset: Preset = unsafe { core::ptr::read(preset_buf.as_ptr() as *const _) };

        log_storage!("Loaded preset {}: {}\r\n", index, preset.get_name());
        Some(preset)
    }

    pub async fn read_raw(&mut self, buf: &mut [u8]) {
        let len = buf.len().min(SECTOR_SIZE as usize);
        self.flash.read(ADDR_OFFSET, &mut buf[..len]).await.unwrap();
    }

    pub async fn write_raw(&mut self, data: &[u8]) {
        log_storage!("SysEx Write: Erasing sector...\r\n");
        self.flash.erase(ADDR_OFFSET, ADDR_OFFSET + ERASE_SIZE as u32).await.unwrap();

        log_storage!("SysEx Write: Writing {} bytes...\r\n", data.len());

        self.flash.write(ADDR_OFFSET, data).await.unwrap();

        log_storage!("SysEx Write: Done.\r\n");
    }
}
