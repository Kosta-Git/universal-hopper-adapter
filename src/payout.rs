use cc_talk_core::cc_talk::{HopperDispenseStatus, HopperStatus};
use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_futures::select::{select, Either};
use embassy_stm32::{
    exti::ExtiInput,
    gpio::{Level, Output},
};
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{with_timeout, Duration, Timer};

use crate::reset::{send_reset_signal, ResetType};

static PAYOUT_SIGNAL: Signal<ThreadModeRawMutex, u8> = Signal::new();
static ENABLE_PAYOUT_SIGNAL: Signal<ThreadModeRawMutex, bool> = Signal::new();
static EMERGENCY_STOP_SIGNAL: Signal<ThreadModeRawMutex, ()> = Signal::new();

static EXIT_SENSOR_SIGNAL: Signal<ThreadModeRawMutex, u8> = Signal::new();

static CURRENT_PAYOUT_STATUS: Mutex<ThreadModeRawMutex, HopperDispenseStatus> =
    Mutex::new(HopperDispenseStatus {
        event_counter: 0,
        coins_remaining: 0,
        paid: 0,
        unpaid: 0,
    });

static HIGH_LEVEL_SENSOR: Mutex<ThreadModeRawMutex, Level> = Mutex::new(Level::Low);
static LOW_LEVEL_SENSOR: Mutex<ThreadModeRawMutex, Level> = Mutex::new(Level::Low);

static DISPENSE_COUNT: Mutex<ThreadModeRawMutex, u32> = Mutex::new(0);

static MAXIMUM_TRY_COUNT: u8 = 5;

pub async fn init_payout_tasks(
    spawner: Spawner,
    in_3: Output<'static>,
    exit_sensor: ExtiInput<'static>,
    low_level_sensor: ExtiInput<'static>,
    high_level_sensor: ExtiInput<'static>,
) {
    info!("initializing payout tasks");

    spawner
        .spawn(sensor_task(low_level_sensor, high_level_sensor))
        .unwrap();
    spawner.spawn(exit_sensor_task(exit_sensor)).unwrap();
    spawner.spawn(payout_task(in_3)).unwrap();
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
async fn payout_task(mut in_3: Output<'static>) {
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

                info!("starting payout for {} coins", count);
                match select(payment(&mut in_3), EMERGENCY_STOP_SIGNAL.wait()).await {
                    Either::First(_) => {
                        // NOP
                    }
                    Either::Second(_) => {
                        warn!("emergency stop triggered during payout");
                        send_reset_signal(ResetType::Hopper);
                        {
                            let mut event = CURRENT_PAYOUT_STATUS.lock().await;
                            *event = event.coin_unpaid(event.coins_remaining);
                        }
                    }
                }
            }
        }
    }
}

async fn payment(in_3: &mut Output<'static>) {
    let mut tries: u8 = 0;
    let mut to_pay = 1; // Just start with a value
    let mut success = false;

    while tries < MAXIMUM_TRY_COUNT && to_pay > 0 {
        in_3.set_high();

        match with_timeout(Duration::from_millis(500), EXIT_SENSOR_SIGNAL.wait()).await {
            Ok(remaining) => {
                success = true;
                to_pay = remaining; // Get the value from the signal
            }
            Err(_) => {
                tries += 1;
                success = false;
            }
        }

        info!("left to pay {}", to_pay);
    }

    in_3.set_low();

    if !success {
        let mut event = CURRENT_PAYOUT_STATUS.lock().await;
        *event = event.coin_unpaid(event.coins_remaining);
    }
}

#[embassy_executor::task]
async fn exit_sensor_task(mut exit_sensor: ExtiInput<'static>) {
    loop {
        exit_sensor.wait_for_low().await;
        Timer::after(Duration::from_millis(15)).await;
        info!("coin out wait");
        match with_timeout(Duration::from_millis(200), exit_sensor.wait_for_high()).await {
            Ok(_) => {}
            Err(_) => {
                warn!("coin sensor timed out waiting for high level");
                continue; // Timeout, just continue
            }
        }
        info!("coin out");
        Timer::after(Duration::from_millis(15)).await;

        let remaining = {
            let mut event = CURRENT_PAYOUT_STATUS.lock().await;
            *event = event.coin_paid(1);
            event.coins_remaining
        };
        {
            let mut dispense_count = DISPENSE_COUNT.lock().await;
            *dispense_count = dispense_count.wrapping_add(1);
        }
        info!("signal exit sensor");
        EXIT_SENSOR_SIGNAL.signal(remaining);
    }
}

#[embassy_executor::task]
async fn sensor_task(
    mut low_level_sensor: ExtiInput<'static>,
    mut high_level_sensor: ExtiInput<'static>,
) {
    info!("sensor task started");

    let low_level_sensor_level = low_level_sensor.get_level();
    let high_level_sensor_level = high_level_sensor.get_level();

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
        match select(
            low_level_sensor.wait_for_any_edge(),
            high_level_sensor.wait_for_any_edge(),
        )
        .await
        {
            Either::First(_) => {
                info!("low level sensor triggered");
                let level = low_level_sensor.get_level();
                {
                    let mut lll = LOW_LEVEL_SENSOR.lock().await;
                    *lll = level;
                }
            }
            Either::Second(_) => {
                info!("high level sensor triggered");
                let level = high_level_sensor.get_level();
                {
                    let mut hll = HIGH_LEVEL_SENSOR.lock().await;
                    *hll = level;
                }
            }
        }
        Timer::after(Duration::from_millis(50)).await;
    }
}
