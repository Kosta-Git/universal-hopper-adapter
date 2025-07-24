#![no_std]
#![no_main]

use cc_talk_core::cc_talk::MAX_BLOCK_LENGTH;
use cc_talk_device::device_impl::DeviceImpl;
use cc_talk_device::payout_device::PayoutDevice;
use defmt::{error, info};
use embassy_executor::Spawner;
use embassy_stm32::exti::ExtiInput;
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use embassy_stm32::usart::{
    BufferedUart, Config as UartConfig, DataBits, HalfDuplexReadback, OutputConfig, Parity,
    StopBits, Uart,
};
use embassy_stm32::usb::Driver;
use embassy_stm32::{bind_interrupts, peripherals, usart, usb, Config};
use embedded_io_async::{BufRead, Write};
use universal_hopper_adapter::cc_talk_usb::{configure_usb_clock, create_and_run_usb_driver};
use universal_hopper_adapter::hopper::*;
use universal_hopper_adapter::payout::init_payout_tasks;
use universal_hopper_adapter::reset::{reset_task, send_reset_signal, ResetType};
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    USB_FS => usb::InterruptHandler<peripherals::USB>;
    UART5 => usart::InterruptHandler<peripherals::UART5>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = Config::default();
    configure_usb_clock(&mut config);

    let p = embassy_stm32::init(config);
    //let driver = Driver::new(p.USB, Irqs, p.PA12, p.PA11);

    // In pins
    let in_1_pin = Output::new(p.PB11, Level::High, Speed::High);
    let in_2_pin = Output::new(p.PB10, Level::High, Speed::High);
    let in_3_pin = Output::new(p.PE15, Level::Low, Speed::High);

    // Feedback pins
    let high_level_sensor = ExtiInput::new(p.PE14, p.EXTI14, Pull::Up);
    let low_level_sensor = ExtiInput::new(p.PE12, p.EXTI12, Pull::Up);
    let exit_sensor = ExtiInput::new(p.PE10, p.EXTI10, Pull::Up);

    let user_button = ExtiInput::new(p.PC13, p.EXTI13, Pull::Down);

    // Address dip switches
    let addr_1 = Input::new(p.PG6, Pull::None);
    let addr_2 = Input::new(p.PG5, Pull::None);
    let addr_3 = Input::new(p.PG8, Pull::None);

    let address = compute_bus_address(addr_1.get_level(), addr_2.get_level(), addr_3.get_level());
    set_bus_address(address).await;

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

    let uart_config = {
        let mut conf = UartConfig::default();
        conf.baudrate = 9600;
        conf.data_bits = DataBits::DataBits8;
        conf.stop_bits = StopBits::STOP1;
        conf.parity = Parity::ParityNone;
        conf
    };
    let mut uart = Uart::new_half_duplex(
        p.UART5,
        p.PC12,
        Irqs,
        p.DMA1_CH1,
        p.DMA1_CH2,
        uart_config,
        HalfDuplexReadback::NoReadback,
    )
    .unwrap();
    info!("initializing ccTalk buffers");
    let implementation = Hopper;
    info!("ccTalk address: {}", implementation.address());
    let device = PayoutDevice::new(implementation);
    let mut read_buffer = [0u8; MAX_BLOCK_LENGTH];
    let mut reply_buffer = [0u8; MAX_BLOCK_LENGTH];
    loop {
        match uart.read_until_idle(&mut read_buffer).await {
            Ok(len) => {
                match device
                    .on_frame(&mut read_buffer[..len], reply_buffer.as_mut_slice())
                    .await
                {
                    Ok(reply_len) => {
                        let result = uart.write_all(&reply_buffer[..reply_len]).await;
                        if result.is_err() {
                            error!("Error writing reply: {:?}", result);
                        } else {
                            info!("Reply sent: {:?}", &reply_buffer[..reply_len]);
                        }
                    }
                    Err(error) => {
                        error!("Error reading packet: {:?} {}", error, read_buffer[..len]);
                        continue;
                    }
                }
            }
            Err(_) => error!("Error processing frame"),
        }
    }
    //   create_and_run_usb_driver(driver).await;
}

fn compute_bus_address(addr_1: Level, addr_2: Level, addr_3: Level) -> u8 {
    let mut address = 3;
    if addr_1 == Level::High {
        address += 1;
    }
    if addr_2 == Level::High {
        address += 2;
    }
    if addr_3 == Level::High {
        address += 4;
    }
    address
}

#[embassy_executor::task]
async fn reset_hopper(mut user_button: ExtiInput<'static>) {
    loop {
        user_button.wait_for_falling_edge().await;
        send_reset_signal(ResetType::Hopper);
    }
}
