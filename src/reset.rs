use defmt::info;
use embassy_stm32::gpio::Output;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum ResetType {
    Hopper,
    Controller,
    All,
}

static RESET_SIGNAL: Signal<CriticalSectionRawMutex, ResetType> = Signal::new();

pub fn send_reset_signal(reset_type: ResetType) {
    info!("Sending reset signal: {}", reset_type);
    RESET_SIGNAL.signal(reset_type);
}

/// Background task that listens for reset signals and performs the appropriate reset action.
/// This task will reset the hopper, controller, or both based on the received signal.
///
/// The hopper reset is done by driving the in_1 and in_2 outputs to low and high respectively.
///
/// The system reset is performed by calling the system control block's reset function.
#[embassy_executor::task]
pub async fn reset_task(mut in_1: Output<'static>, mut in_2: Output<'static>) {
    info!("reset task started");

    loop {
        let reset_type = RESET_SIGNAL.wait().await;
        info!("reset signal received: {}", reset_type);

        match reset_type {
            ResetType::Hopper => {
                info!("Resetting hopper");
                reset_hopper(&mut in_1, &mut in_2).await;
            }
            ResetType::Controller => {
                info!("Resetting controller");
                cortex_m::peripheral::SCB::sys_reset();
            }
            ResetType::All => {
                info!("Resetting all");
                reset_hopper(&mut in_1, &mut in_2).await;
                cortex_m::peripheral::SCB::sys_reset();
            }
        }
    }
}

async fn reset_hopper(in_1: &mut Output<'static>, in_2: &mut Output<'static>) {
    info!("Resetting hopper");

    let in_1_initial_state = in_1.get_output_level();
    let in_2_initial_state = in_2.get_output_level();

    in_1.set_low();
    in_2.set_high();
    Timer::after(Duration::from_millis(50)).await;
    in_1.set_level(in_1_initial_state);
    in_2.set_level(in_2_initial_state);
}
