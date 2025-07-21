use cc_talk_core::cc_talk::deserializer::deserialize;
use cc_talk_core::cc_talk::serializer::serialize;
use cc_talk_core::cc_talk::{self, deserializer, Packet, MAX_BLOCK_LENGTH};
use cc_talk_core::{Category, ChecksumType, Device, Header};
use defmt::{panic, *};
use embassy_futures::join::join;
use embassy_stm32::peripherals::USB;
use embassy_stm32::usb::{Driver, Instance};
use embassy_stm32::Config;
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::driver::EndpointError;
use embassy_usb::Builder;

use crate::{SignalPacket, PACKET_ARRIVED_SIGNAL};

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

pub async fn create_and_run_usb_driver(driver: Driver<'_, USB>, device: &Device) {
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

    let mut class = CdcAcmClass::new(&mut builder, &mut state, 64);
    let mut usb = builder.build();

    let usb_future = usb.run();
    let cc_talk_listener_future = async {
        loop {
            class.wait_connection().await;
            info!("usb connected");
            let _ = cc_talk_event_listener(&mut class, device).await;
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
    device: &Device,
) -> Result<(), Disconnected> {
    let mut buf = [0; MAX_BLOCK_LENGTH];
    let mut nack_buffer = [0u8; 5];
    let mut busy_buffer = [0u8; 5];

    let mut nack_packet = Packet::new(&mut nack_buffer);
    nack_packet.set_source(device.address()).unwrap();
    nack_packet.set_data_length(0).unwrap();
    nack_packet.set_header(Header::NACK).unwrap();

    let mut busy_packet = Packet::new(&mut busy_buffer);
    busy_packet.set_source(device.address()).unwrap();
    busy_packet.set_data_length(0).unwrap();
    busy_packet.set_header(Header::Busy).unwrap();

    info!("starting ccTalk event listener");
    info!(
        "ccTalk address: {}\nChecksum type: {}",
        device.address(),
        if device.checksum_type() == &ChecksumType::Crc8 {
            "CRC8"
        } else {
            "CRC16"
        }
    );

    loop {
        let n = class.read_packet(&mut buf).await?;
        let mut p = Packet::new(&mut buf[..n]);

        if p.get_destination().unwrap_or(0u8) != device.address() {
            continue;
        }

        // If the signal was not cleared, we just respond busy and drop the packet.
        if PACKET_ARRIVED_SIGNAL.signaled() {
            if busy_packet
                .set_destination(p.get_source().unwrap_or(1u8))
                .is_err()
            {
                error!("unable to set busy destination");
                continue;
            }

            match serialize(device, &mut busy_packet) {
                Ok(()) => {
                    let _ = class.write_packet(busy_packet.as_slice()).await;
                }
                Err(_) => {
                    error!("unable to serialize busy packet");
                }
            }

            continue;
        }

        match deserialize(&mut p, device.checksum_type().clone()) {
            Ok(reply_addr) => {
                info!("Received a valid packet with reply address: {}", reply_addr);
                // TODO: Process the packet
            }
            Err(error) => {
                match error {
                    deserializer::DeserializationError::BufferTooSmall => warn!("buffer too small"),
                    deserializer::DeserializationError::InvalidPacket => warn!("invalid packet"),
                    deserializer::DeserializationError::UnsupportedChecksumType => {
                        warn!("unsupported checksum type")
                    }
                    deserializer::DeserializationError::ChecksumMismatch(expected, actual) => {
                        warn!("checksum mismatch {} != {}", expected, actual)
                    }
                };

                // Reply NAK
                if nack_packet
                    .set_destination(p.get_source().unwrap_or(1u8))
                    .is_err()
                {
                    error!("unable to set nack destination");
                    continue;
                }

                match serialize(device, &mut nack_packet) {
                    Ok(()) => {
                        let _ = class.write_packet(nack_packet.as_slice()).await;
                    }
                    Err(_error) => error!("unable to serialize nack packet"),
                };
            }
        }
    }
}
