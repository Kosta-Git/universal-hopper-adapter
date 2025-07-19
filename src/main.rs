#![no_std]
#![no_main]

use cc_talk_core::cc_talk::deserializer::deserialize;
use cc_talk_core::cc_talk::{self, deserializer, Packet, MAX_BLOCK_LENGTH};
use defmt::{panic, *};
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_stm32::usb::{Driver, Instance};
use embassy_stm32::{bind_interrupts, peripherals, usb, Config};
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::driver::EndpointError;
use embassy_usb::Builder;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    USB_FS => usb::InterruptHandler<peripherals::USB>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let mut config = Config::default();
    {
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
    let p = embassy_stm32::init(config);

    // Create the driver, from the HAL.
    let driver = Driver::new(p.USB, Irqs, p.PA12, p.PA11);

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

    // Create classes on the builder.
    let mut class = CdcAcmClass::new(&mut builder, &mut state, 64);

    // Build the builder.
    let mut usb = builder.build();

    // Run the USB device.
    let usb_fut = usb.run();

    // Do stuff with the class!
    let echo_fut = async {
        loop {
            class.wait_connection().await;
            info!("Connected");
            let _ = echo(&mut class).await;
            info!("Disconnected");
        }
    };

    // Run everything concurrently.
    // If we had made everything `'static` above instead, we could do this using separate tasks instead.
    join(usb_fut, echo_fut).await;
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

async fn echo<'d, T: Instance + 'd>(
    class: &mut CdcAcmClass<'d, Driver<'d, T>>,
) -> Result<(), Disconnected> {
    let mut buf = [0; MAX_BLOCK_LENGTH];
    loop {
        let n = class.read_packet(&mut buf).await?;
        let mut p = Packet::new(&mut buf[..n]);
        info!("Packet: {:?}", p.as_slice());
        match deserialize(&mut p, cc_talk::ChecksumType::Crc8) {
            Ok(reply_addr) => {
                info!("Received a valid packet with reply address: {}", reply_addr);
            }
            Err(error) => match error {
                deserializer::DeserializationError::BufferTooSmall => warn!("buffer too small"),
                deserializer::DeserializationError::InvalidPacket => warn!("invalid packet"),
                deserializer::DeserializationError::UnsupportedChecksumType => {
                    warn!("unsupported checksum type")
                }
                deserializer::DeserializationError::ChecksumMismatch(expected, actual) => {
                    warn!("checksum mismatch")
                }
            },
        }
    }
}
