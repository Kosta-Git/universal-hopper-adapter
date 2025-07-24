#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_stm32::exti::ExtiInput;
use embassy_stm32::gpio::{Level, Output, Pull, Speed};
use embassy_stm32::usb::Driver;
use embassy_stm32::{bind_interrupts, peripherals, usb, Config};
use universal_hopper_adapter::cc_talk_usb::{configure_usb_clock, create_and_run_usb_driver};
use universal_hopper_adapter::payout::init_payout_tasks;
use universal_hopper_adapter::reset::{reset_task, send_reset_signal, ResetType};
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    USB_FS => usb::InterruptHandler<peripherals::USB>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = Config::default();
    configure_usb_clock(&mut config);

    let p = embassy_stm32::init(config);
    let driver = Driver::new(p.USB, Irqs, p.PA12, p.PA11);

    let in_1_pin = Output::new(p.PB11, Level::High, Speed::High);
    let in_2_pin = Output::new(p.PB10, Level::High, Speed::High);
    let in_3_pin = Output::new(p.PE15, Level::Low, Speed::High);

    let high_level_sensor = ExtiInput::new(p.PE14, p.EXTI14, Pull::Up);
    let low_level_sensor = ExtiInput::new(p.PE12, p.EXTI12, Pull::Up);
    let exit_sensor = ExtiInput::new(p.PE10, p.EXTI10, Pull::Up);

    let user_button = ExtiInput::new(p.PC13, p.EXTI13, Pull::Down);

    spawner.spawn(reset_task(in_1_pin, in_2_pin)).unwrap();
    spawner.spawn(reset_hopper(user_button)).unwrap();
    init_payout_tasks(
        spawner,
        in_3_pin,
        exit_sensor,
        low_level_sensor,
        high_level_sensor,
    )
    .await;

    create_and_run_usb_driver(driver).await;
}

#[embassy_executor::task]
async fn reset_hopper(mut user_button: ExtiInput<'static>) {
    loop {
        user_button.wait_for_falling_edge().await;
        send_reset_signal(ResetType::Hopper);
    }
}
