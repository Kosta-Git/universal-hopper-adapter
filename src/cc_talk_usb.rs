use cc_talk_core::cc_talk::MAX_BLOCK_LENGTH;
use cc_talk_device::payout_device::PayoutDevice;
use defmt::{panic, *};
use embassy_futures::join::join;
use embassy_stm32::peripherals::USB;
use embassy_stm32::usb::{Driver, Instance};
use embassy_stm32::Config;
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::driver::EndpointError;
use embassy_usb::Builder;

use crate::hopper::Hopper;

pub fn configure_usb_clock(config: &mut Config) {
    use embassy_stm32::rcc::*;
    config.rcc.hsi = true;
    config.rcc.sys = Sysclk::PLL1_R;
    config.rcc.pll = Some(Pll {
        // 80Mhz clock (16 / 1 * 10 / 2)
        source: PllSource::HSI,
        prediv: PllPreDiv::DIV1,
        mul: PllMul::MUL10,
        divp: None,
        divq: None,
        divr: Some(PllRDiv::DIV2),
    });
    config.rcc.hsi48 = Some(Hsi48Config {
        sync_from_usb: true,
    }); // needed for USB
    config.rcc.mux.clk48sel = mux::Clk48sel::HSI48;
}

pub async fn create_and_run_usb_driver(driver: Driver<'_, USB>) {
    info!("creating usb driver");

    // Create the driver, from the HAL.
    // Create embassy-usb Config
    let mut config = embassy_usb::Config::new(0xc057, 0xc057);
    config.manufacturer = Some("Kosta-Git");
    config.product = Some("universal-hopper-adapter");
    //config.max_packet_size_0 = 64;

    // Create embassy-usb DeviceBuilder using the driver and config.
    // It needs some buffers for building the descriptors.
    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    let mut control_buf = [0; 128];

    let mut state = State::new();

    let mut builder = Builder::new(
        driver,
        config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut [], // no msos descriptors
        &mut control_buf,
    );

    let mut class = CdcAcmClass::new(&mut builder, &mut state, MAX_BLOCK_LENGTH as u16);
    let mut usb = builder.build();

    let usb_future = usb.run();
    let cc_talk_listener_future = async {
        loop {
            info!("waiting for usb connection");
            class.wait_connection().await;
            info!("usb connected");
            let _ = cc_talk_event_listener(&mut class).await;
            info!("usb disconnected");
        }
    };

    join(usb_future, cc_talk_listener_future).await;
}

struct Disconnected {}

impl From<EndpointError> for Disconnected {
    fn from(val: EndpointError) -> Self {
        match val {
            EndpointError::BufferOverflow => panic!("Buffer overflow"),
            EndpointError::Disabled => Disconnected {},
        }
    }
}

async fn cc_talk_event_listener<'d, T: Instance + 'd>(
    class: &mut CdcAcmClass<'d, Driver<'d, T>>,
) -> Result<(), Disconnected> {
    info!("initializing ccTalk buffers");

    let device = PayoutDevice::new(Hopper);
    let mut read_buffer = [0u8; MAX_BLOCK_LENGTH];
    let mut reply_buffer = [0u8; MAX_BLOCK_LENGTH];

    info!("starting ccTalk event listener");

    loop {
        let n = match class.read_packet(&mut read_buffer).await {
            Ok(size) => size,
            Err(error) => {
                error!("Error reading packet: {:?}", error);
                continue;
            }
        };
        info!("received packet of length {}", n);
        info!("data: {:?}", &read_buffer[..n]);
        match device
            .on_frame(&mut read_buffer[..n], reply_buffer.as_mut_slice())
            .await
        {
            Ok(reply_len) => {
                let result = class.write_packet(&reply_buffer[..reply_len]).await;
                if result.is_err() {
                    error!("Error writing reply packet");
                } else {
                    info!("Replied with {}", &reply_buffer[..reply_len]);
                }
            }
            Err(_) => error!("Error processing frame"),
        };
    }
}
