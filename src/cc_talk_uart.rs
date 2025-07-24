use defmt::info;
use embassy_stm32::{gpio::AnyPin, peripherals::LPUART1, usart::Uart, Peri};

pub async fn create_and_run_uart_driver() {
    info!("creating uart driver");
}
