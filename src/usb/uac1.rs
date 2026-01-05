use embassy_usb::class::uac1::SampleWidth;
use embassy_usb::control::{InResponse, OutResponse, Request};
use embassy_usb::driver::{Driver, EndpointIn, EndpointError};
use embassy_usb::{Builder, Handler};
use embassy_usb::descriptor::{SynchronizationType, UsageType};

#[derive(Clone, Copy)]
pub struct Config {
    pub audio_format: SampleWidth,
    pub channel_count: u8,
    pub sample_rate: u32,
    pub packet_size: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            audio_format: SampleWidth::Width2Byte,
            channel_count: 2,
            sample_rate: 48000,
            packet_size: 192,
        }
    }
}

pub struct State<'d> {
    control: Option<Control<'d>>,
}

impl<'d> State<'d> {
    pub fn new() -> Self {
        Self {
            control: None,
        }
    }
}

struct Control<'d> {
    _marker: core::marker::PhantomData<&'d ()>,
}

impl<'d> Handler for Control<'d> {
    fn control_out(&mut self, _req: Request, _data: &[u8]) -> Option<OutResponse> {
        Some(OutResponse::Accepted)
    }

    fn control_in<'a>(&'a mut self, req: Request, buf: &'a mut [u8]) -> Option<InResponse<'a>> {
        if req.request_type == embassy_usb::control::RequestType::Class
           && req.recipient == embassy_usb::control::Recipient::Interface
        {
             let cs = (req.value >> 8) as u8;

             if cs == 0x01 {
                 match req.request {
                     0x81 => {
                         buf[0] = 0;
                         return Some(InResponse::Accepted(&buf[..1]));
                     },
                     _ => return None,
                 }
             } else if cs == 0x02 {
                 match req.request {
                     0x81 => {
                         buf[0] = 0x00;
                         buf[1] = 0x00;
                         return Some(InResponse::Accepted(&buf[..2]));
                     },
                     0x82 => {
                         buf[0] = 0x00;
                         buf[1] = 0xC0;
                         return Some(InResponse::Accepted(&buf[..2]));
                     },
                     0x83 => {
                         buf[0] = 0x00;
                         buf[1] = 0x00;
                         return Some(InResponse::Accepted(&buf[..2]));
                     },
                     0x84 => {
                         buf[0] = 0x00;
                         buf[1] = 0x01;
                         return Some(InResponse::Accepted(&buf[..2]));
                     },
                     _ => return None,
                 }
             }
        }
        None
    }
}

pub struct Microphone<'d, D: Driver<'d>> {
    ep_in: D::EndpointIn,
}

impl<'d, D: Driver<'d>> Microphone<'d, D> {
    pub async fn write_packet(&mut self, data: &[u8]) -> Result<(), EndpointError> {
        self.ep_in.write(data).await
    }
}

pub struct Uac1MicrophoneClass;

impl Uac1MicrophoneClass {
    #[allow(clippy::new_ret_no_self)]
    pub fn new<'d, D: Driver<'d>>(
        builder: &mut Builder<'d, D>,
        state: &'d mut State<'d>,
        config: Config,
    ) -> Microphone<'d, D> {
        state.control = Some(Control {
            _marker: core::marker::PhantomData,
        });

        builder.handler(state.control.as_mut().unwrap());

        let mut func = builder.function(0x01, 0x01, 0x00);

        let mut ac_if = func.interface();
        let ac_if_num = ac_if.interface_number();
        let mut alt = ac_if.alt_setting(0x01, 0x01, 0x00, None);

        let total_length: u16 = 40;

        let header_desc = [
            0x09,
            0x24,
            0x01,
            0x00, 0x01,
            (total_length & 0xff) as u8, (total_length >> 8) as u8,
            0x01,
            (ac_if_num.0 + 1),
        ];
        alt.descriptor(0x24, &header_desc[2..]);

        let it_desc = [
            0x0C,
            0x24,
            0x02,
            0x01,
            0x03, 0x06,
            0x00,
            config.channel_count,
            0x03, 0x00,
            0x00,
            0x00,
        ];
        alt.descriptor(0x24, &it_desc[2..]);

        let fu_desc = [
            0x0A,
            0x24,
            0x06,
            0x02,
            0x01,
            0x01,
            0x03,
            0x00,
            0x00,
            0x00,
        ];
        alt.descriptor(0x24, &fu_desc[2..]);

        let ot_desc = [
            0x09,
            0x24,
            0x03,
            0x03,
            0x01, 0x01,
            0x00,
            0x02,
            0x00,
        ];
        alt.descriptor(0x24, &ot_desc[2..]);

        let mut as_if = func.interface();

        let _alt0 = as_if.alt_setting(0x01, 0x02, 0x00, None);

        let mut alt1 = as_if.alt_setting(0x01, 0x02, 0x00, None);

        let as_general_desc = [
            0x07,
            0x24,
            0x01,
            0x03,
            0x01,
            0x01, 0x00,
        ];
        alt1.descriptor(0x24, &as_general_desc[2..]);

        let format_desc = [
            0x08 + 3,
            0x24,
            0x02,
            0x01,
            config.channel_count,
            config.audio_format as u8,
            (config.audio_format.in_bit()) as u8,
            0x01,
            (config.sample_rate & 0xff) as u8,
            ((config.sample_rate >> 8) & 0xff) as u8,
            ((config.sample_rate >> 16) & 0xff) as u8,
        ];
        alt1.descriptor(0x24, &format_desc[2..]);

        let cs_ep_desc = [
            0x07,
            0x25,
            0x01,
            0x00,
            0x00,
            0x00, 0x00,
        ];
        alt1.descriptor(0x25, &cs_ep_desc[2..]);

        let ep_in = alt1.endpoint_isochronous_in(
            None,
            config.packet_size,
            1,
            SynchronizationType::Asynchronous,
            UsageType::DataEndpoint,
            &[]
        );

        Microphone {
            ep_in,
        }
    }
}
