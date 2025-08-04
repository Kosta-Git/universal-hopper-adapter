use cc_talk_core::cc_talk::{HopperDispenseStatus, HopperStatus};
use defmt::{debug, info, trace, warn};
use embassy_executor::Spawner;
use embassy_futures::select::{select, Either};
use embassy_stm32::{
    exti::ExtiInput,
    gpio::{Level, Output},
};
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{Duration, Timer};

static PAYOUT_SIGNAL: Signal<ThreadModeRawMutex, u8> = Signal::new();
static ENABLE_PAYOUT_SIGNAL: Signal<ThreadModeRawMutex, bool> = Signal::new();
static EMERGENCY_STOP_SIGNAL: Signal<ThreadModeRawMutex, ()> = Signal::new();

#[derive(Clone, Copy, Debug, defmt::Format, Eq, PartialEq)]
enum MotorCommand {
    Start,
    Stop,
}
static CHANGE_MOTOR_STATE_SIGNAL: Signal<ThreadModeRawMutex, MotorCommand> = Signal::new();
static SENSOR_STATE_SIGNAL: Signal<ThreadModeRawMutex, bool> = Signal::new();

static CURRENT_PAYOUT_STATUS: Mutex<ThreadModeRawMutex, HopperDispenseStatus> =
    Mutex::new(HopperDispenseStatus {
        event_counter: 0,
        coins_remaining: 0,
        paid: 0,
        unpaid: 0,
    });

static HIGH_LEVEL_SENSOR: Mutex<ThreadModeRawMutex, Level> = Mutex::new(Level::Low);
static LOW_LEVEL_SENSOR: Mutex<ThreadModeRawMutex, Level> = Mutex::new(Level::Low);

/// Hopper dispense count since last reset or power on.
static DISPENSE_COUNT: Mutex<ThreadModeRawMutex, u32> = Mutex::new(0);

pub async fn init_payout_tasks(
    spawner: Spawner,
    in_3: Output<'static>,
    exit_sensor: ExtiInput<'static>,
    low_level_sensor: ExtiInput<'static>,
    high_level_sensor: ExtiInput<'static>,
    security_output: ExtiInput<'static>,
) {
    info!("initializing payout tasks");

    spawner
        .spawn(sensor_task(low_level_sensor, high_level_sensor))
        .unwrap();
    spawner.spawn(exit_sensor_task(exit_sensor)).unwrap();
    spawner.spawn(payout_task()).unwrap();
    spawner
        .spawn(security_output_task(security_output))
        .unwrap();
    spawner.spawn(book_keeper_task()).unwrap();
    spawner.spawn(motor_control_task(in_3)).unwrap();
}

pub async fn get_dispense_count() -> u32 {
    let count = DISPENSE_COUNT.lock().await;
    *count
}

pub async fn emergency_stop() {
    EMERGENCY_STOP_SIGNAL.signal(());
}

pub async fn get_payout_status() -> HopperDispenseStatus {
    let status = CURRENT_PAYOUT_STATUS.lock().await;
    *status
}

pub fn enable_payout(enable: bool) {
    ENABLE_PAYOUT_SIGNAL.signal(enable);
}

pub fn request_payout(count: u8) {
    PAYOUT_SIGNAL.signal(count);
}

pub async fn get_sensor_status() -> HopperStatus {
    let high: Level;
    let low: Level;
    {
        let high_level = HIGH_LEVEL_SENSOR.lock().await;
        high = *high_level;
        let low_level = LOW_LEVEL_SENSOR.lock().await;
        low = *low_level;
    };
    HopperStatus::new(true, low == Level::Low, true, high == Level::Low)
}

#[embassy_executor::task]
async fn payout_task() {
    info!("payout task started");
    let mut payout_enabled = false;

    loop {
        // If somehow the emergency stop signal is done outside of payout, just clear it
        if EMERGENCY_STOP_SIGNAL.signaled() {
            EMERGENCY_STOP_SIGNAL.reset();
        }

        match select(ENABLE_PAYOUT_SIGNAL.wait(), PAYOUT_SIGNAL.wait()).await {
            Either::First(enable) => {
                payout_enabled = enable;
                info!("payout enabled status: {}", payout_enabled);
            }
            Either::Second(count) => {
                if !payout_enabled {
                    info!("Payout signal received but payouts are disabled");
                    continue;
                }

                {
                    let mut event = CURRENT_PAYOUT_STATUS.lock().await;
                    *event = event.payout_requested(count);
                }

                CHANGE_MOTOR_STATE_SIGNAL.signal(MotorCommand::Start);
            }
        }
    }
}

#[embassy_executor::task]
async fn motor_control_task(mut in_3: Output<'static>) {
    loop {
        match select(
            CHANGE_MOTOR_STATE_SIGNAL.wait(),
            EMERGENCY_STOP_SIGNAL.wait(),
        )
        .await
        {
            Either::First(command) => match command {
                MotorCommand::Start => {
                    info!("motor command: start");
                    in_3.set_high();
                    SENSOR_STATE_SIGNAL.signal(false);
                }
                MotorCommand::Stop => {
                    info!("motor command: stop");
                    in_3.set_low();
                    SENSOR_STATE_SIGNAL.signal(true);
                }
            },
            Either::Second(_) => {
                warn!("emergency stop triggered, stopping motor");
                in_3.set_low();
                SENSOR_STATE_SIGNAL.signal(true);
            }
        };
    }
}

// Exit sensor constants
const MIN_PULSE_LENGTH: Duration = Duration::from_millis(50);
#[embassy_executor::task]
async fn exit_sensor_task(mut exit_sensor: ExtiInput<'static>) {
    loop {
        exit_sensor.wait_for_falling_edge().await;
        Timer::after(MIN_PULSE_LENGTH).await;
        if exit_sensor.get_level() == Level::High {
            trace!("exit sensor was not low for long enough, ignoring...");
            continue;
        }
        exit_sensor.wait_for_rising_edge().await;
        info!("exit sensor triggered");

        {
            let mut event = CURRENT_PAYOUT_STATUS.lock().await;
            *event = event.coin_paid(1);
            debug!("coins remaining: {}", event.coins_remaining);

            if event.coins_remaining == 0 {
                info!("all coins paid, stopping motor");
                CHANGE_MOTOR_STATE_SIGNAL.signal(MotorCommand::Stop);
            }
        };
        {
            let mut dispense_count = DISPENSE_COUNT.lock().await;
            *dispense_count = dispense_count.wrapping_add(1);
        };
    }
}

const BK_MAX_TRIES: u8 = 2;
const BK_POLL_INTERVAL: Duration = Duration::from_secs(5);
/// Makes sure the payout status is updated periodically, and resets it if no changes are detected
/// for a certain number of tries. It will mark the coins as unpaid
#[embassy_executor::task]
async fn book_keeper_task() {
    info!("bookkeeper task started");
    let mut last_remaining = 0;
    let mut tries = 0;
    loop {
        // This task can be used to log or process the payout status periodically
        Timer::after(BK_POLL_INTERVAL).await;
        let status = get_payout_status().await;
        if status.coins_remaining != 0 && last_remaining == 0 {
            last_remaining = status.coins_remaining;
            tries = 0;
            continue;
        } else if last_remaining == 0 {
            continue;
        }

        if last_remaining != status.coins_remaining {
            trace!(
                "Bookkeeper: Coins remaining: {}, Paid: {}, Unpaid: {}",
                status.coins_remaining,
                status.paid,
                status.unpaid
            );
            last_remaining = status.coins_remaining;
        } else {
            tries += 1;
        }

        if tries >= BK_MAX_TRIES {
            warn!(
                "Bookkeeper: No change in coins remaining for {} tries, resetting payout status",
                BK_MAX_TRIES
            );
            let mut event = CURRENT_PAYOUT_STATUS.lock().await;
            *event = event.coin_unpaid(event.coins_remaining);
            last_remaining = 0;
            tries = 0;

            warn!("Bookkeeper: Stopping motor state due to no change in payout status");
            CHANGE_MOTOR_STATE_SIGNAL.signal(MotorCommand::Stop);
        }
    }
}

#[embassy_executor::task]
async fn security_output_task(mut security_output: ExtiInput<'static>) {
    info!("security output task started");
    loop {
        security_output.wait_for_falling_edge().await;
    }
}

// Constants for sensor polling
const SENSOR_POLLING_INTERVAL: Duration = Duration::from_secs(5);
#[embassy_executor::task]
async fn sensor_task(low_level_sensor: ExtiInput<'static>, high_level_sensor: ExtiInput<'static>) {
    info!("sensor task started");

    let low_level_sensor_level = low_level_sensor.get_level();
    let high_level_sensor_level = high_level_sensor.get_level();
    let mut enabled = true;

    info!(
        "polling initial sensor levels, low sensor {}, high sensor {}",
        low_level_sensor_level, high_level_sensor_level
    );

    {
        let mut lll = LOW_LEVEL_SENSOR.lock().await;
        *lll = low_level_sensor_level;

        let mut hll = HIGH_LEVEL_SENSOR.lock().await;
        *hll = high_level_sensor_level;
    }

    info!("initial sensor levels set, starting event loop");
    loop {
        Timer::after(SENSOR_POLLING_INTERVAL).await;

        if SENSOR_STATE_SIGNAL.signaled() {
            enabled = SENSOR_STATE_SIGNAL.wait().await;
        }

        if !enabled {
            trace!("sensor are disabled, skipping sensor polling");
            continue;
        }

        let level = low_level_sensor.get_level();
        {
            debug!("low level sensor: {}", level);
            let mut lll = LOW_LEVEL_SENSOR.lock().await;
            *lll = level;
        }
        let level = high_level_sensor.get_level();
        {
            debug!("high level sensor: {}", level);
            let mut hll = HIGH_LEVEL_SENSOR.lock().await;
            *hll = level;
        }
    }
}
