#![no_std]
#![no_main]

use cc_talk_core::{Category, ChecksumType, Device};
use embassy_executor::Spawner;
use embassy_stm32::usb::Driver;
use embassy_stm32::{bind_interrupts, peripherals, usb, Config};
use universal_hopper_adapter::cc_talk_usb::{configure_usb_clock, create_and_run_usb_driver};
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    USB_FS => usb::InterruptHandler<peripherals::USB>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let currrent_device = Device::new(3, Category::Payout, ChecksumType::Crc8);

    let mut config = Config::default();
    configure_usb_clock(&mut config);

    let p = embassy_stm32::init(config);
    let driver = Driver::new(p.USB, Irqs, p.PA12, p.PA11);

    create_and_run_usb_driver(driver, &currrent_device).await;
}
