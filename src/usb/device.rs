use core::mem::MaybeUninit;
use core::ptr::addr_of_mut;
use embassy_executor::Spawner;
use embassy_rp::peripherals::USB;
use embassy_rp::usb::Driver;
use embassy_usb::class::cdc_acm::{CdcAcmClass, Receiver, Sender, State};
use embassy_usb::class::midi::{MidiClass, Receiver as MidiReceiver, Sender as MidiSender};
use embassy_usb::class::uac1::SampleWidth;
use embassy_usb::{Builder, Config};

use crate::common::shared::{SystemCommand, COMMAND_CHANNEL};
use crate::usb::uac1::{self, Microphone, Uac1MicrophoneClass};

pub type UsbSender = Sender<'static, Driver<'static, USB>>;

static mut CONFIG_DESC: [u8; 512] = [0; 512];
static mut BOS_DESC: [u8; 256] = [0; 256];
static mut CONTROL_BUF: [u8; 64] = [0; 64];
static mut CDC_STATE: MaybeUninit<State> = MaybeUninit::uninit();
static mut UAC_STATE: MaybeUninit<uac1::State<'static>> = MaybeUninit::uninit();

pub struct UsbDevice {
    pub sender: UsbSender,
    pub microphone: Microphone<'static, Driver<'static, USB>>,
    pub midi_receiver: MidiReceiver<'static, Driver<'static, USB>>,
    pub midi_sender: MidiSender<'static, Driver<'static, USB>>,
}

pub fn init(spawner: Spawner, driver: Driver<'static, USB>) -> UsbDevice {
    let mut config = Config::new(0xdead, 0xc0de);
    config.manufacturer = Some("Sonixwave");
    config.product = Some("PicoDSP (infinitedsp 0.7.0)");
    config.serial_number = Some("12345678");
    config.max_power = 100;
    config.max_packet_size_0 = 64;

    config.device_class = 0xEF;
    config.device_sub_class = 0x02;
    config.device_protocol = 0x01;
    config.composite_with_iads = true;

    let (cdc_state, uac_state, config_desc, bos_desc, control_buf) = unsafe {
        let cdc_state_ptr = addr_of_mut!(CDC_STATE);
        (*cdc_state_ptr).write(State::new());

        let uac_state_ptr = addr_of_mut!(UAC_STATE);
        (*uac_state_ptr).write(uac1::State::new());

        (
            (*cdc_state_ptr).assume_init_mut(),
            (*uac_state_ptr).assume_init_mut(),
            &mut *addr_of_mut!(CONFIG_DESC),
            &mut *addr_of_mut!(BOS_DESC),
            &mut *addr_of_mut!(CONTROL_BUF),
        )
    };

    let mut builder = Builder::new(driver, config, config_desc, bos_desc, &mut [], control_buf);

    let uac_config = uac1::Config {
        audio_format: SampleWidth::Width2Byte,
        channel_count: 2,
        sample_rate: 48000,
        packet_size: 200,
    };
    let microphone = Uac1MicrophoneClass::new(&mut builder, uac_state, uac_config);

    let cdc_class = CdcAcmClass::new(&mut builder, cdc_state, 64);

    let midi_class = MidiClass::new(&mut builder, 1, 1, 64);

    let usb_dev = builder.build();

    spawner.spawn(usb_task(usb_dev)).unwrap();

    let (sender, receiver) = cdc_class.split();
    let (midi_sender, midi_receiver) = midi_class.split();

    spawner.spawn(command_listener_task(receiver)).unwrap();

    UsbDevice {
        sender,
        microphone,
        midi_receiver,
        midi_sender,
    }
}

#[embassy_executor::task]
async fn usb_task(mut usb: embassy_usb::UsbDevice<'static, Driver<'static, USB>>) {
    usb.run().await;
}

#[embassy_executor::task]
async fn command_listener_task(mut receiver: Receiver<'static, Driver<'static, USB>>) {
    let mut buf = [0; 64];
    let mut line_buf = heapless::String::<32>::new();

    loop {
        receiver.wait_connection().await;

        if let Ok(n) = receiver.read_packet(&mut buf).await {
            let data = &buf[..n];
            for &b in data {
                if b == b'\r' || b == b'\n' {
                    let cmd = line_buf.as_str().trim();
                    match cmd {
                        "reboot" => {
                            embassy_rp::rom_data::reset_to_usb_boot(0, 0);
                        }
                        "restart" => {
                            let mut watchdog = embassy_rp::watchdog::Watchdog::new(unsafe {
                                embassy_rp::peripherals::WATCHDOG::steal()
                            });
                            watchdog.trigger_reset();
                        }
                        "reset" => {
                            let _ = COMMAND_CHANNEL.try_send(SystemCommand::ResetStorage);
                        }
                        _ => {}
                    }
                    line_buf.clear();
                } else {
                    let _ = line_buf.push(b as char);
                }
            }
        }
    }
}
