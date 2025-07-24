use cc_talk_core::cc_talk::{HopperDispenseStatus, HopperStatus};
use defmt::info;
use embassy_executor::Spawner;
use embassy_futures::select::{select, Either};
use embassy_stm32::{
    exti::ExtiInput,
    gpio::{Level, Output},
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{with_timeout, Duration, Timer};

use crate::reset::{send_reset_signal, ResetType};

static PAYOUT_SIGNAL: Signal<CriticalSectionRawMutex, u8> = Signal::new();
static ENABLE_PAYOUT_SIGNAL: Signal<CriticalSectionRawMutex, bool> = Signal::new();
static EMERGENCY_STOP_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

static EXIT_SENSOR_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

static CURRENT_PAYOUT_STATUS: Mutex<CriticalSectionRawMutex, HopperDispenseStatus> =
    Mutex::new(HopperDispenseStatus {
        event_counter: 0,
        coins_remaining: 0,
        paid: 0,
        unpaid: 0,
    });

static HIGH_LEVEL_SENSOR: Mutex<CriticalSectionRawMutex, Level> = Mutex::new(Level::Low);
static LOW_LEVEL_SENSOR: Mutex<CriticalSectionRawMutex, Level> = Mutex::new(Level::Low);

static DISPENSE_COUNT: Mutex<CriticalSectionRawMutex, u32> = Mutex::new(0);

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
    HopperStatus {
        low_level_supported: true,
        high_level_supported: true,
        higher_than_low_level: low == Level::Low,
        higher_than_high_level: high == Level::Low,
    }
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

                match select(EMERGENCY_STOP_SIGNAL.wait(), payment(&mut in_3)).await {
                    Either::First(_) => {
                        send_reset_signal(ResetType::Hopper);
                        {
                            let mut event = CURRENT_PAYOUT_STATUS.lock().await;
                            *event = event.coin_unpaid(event.coins_remaining);
                        }
                    }
                    Either::Second(_) => {
                        // NOP
                    }
                }
            }
        }
    }
}

async fn payment(in_3: &mut Output<'static>) {
    let mut tries: u8 = 0;
    let mut to_pay;
    let mut success = false;

    {
        let event = CURRENT_PAYOUT_STATUS.lock().await;
        to_pay = event.coins_remaining;
    }

    while tries < MAXIMUM_TRY_COUNT && to_pay > 0 {
        in_3.set_high();
        Timer::after(Duration::from_millis(10)).await;
        in_3.set_low();

        let wait_for_exit_with_timeout =
            with_timeout(Duration::from_millis(1000), EXIT_SENSOR_SIGNAL.wait());
        match wait_for_exit_with_timeout.await {
            Ok(_) => {
                success = true;
            }
            Err(_) => {
                tries += 1;
                success = false;
            }
        }

        {
            let event = CURRENT_PAYOUT_STATUS.lock().await;
            to_pay = event.coins_remaining;
        }
    }

    if !success {
        {
            let mut event = CURRENT_PAYOUT_STATUS.lock().await;
            *event = event.coin_unpaid(event.coins_remaining);
        }
    }
}

#[embassy_executor::task]
async fn exit_sensor_task(mut exit_sensor: ExtiInput<'static>) {
    loop {
        exit_sensor.wait_for_rising_edge().await;
        info!("exit sensor triggered");

        {
            let mut event = CURRENT_PAYOUT_STATUS.lock().await;
            *event = event.coin_paid(1);
        }

        {
            let mut dispense_count = DISPENSE_COUNT.lock().await;
            *dispense_count = dispense_count.wrapping_add(1);
        }

        EXIT_SENSOR_SIGNAL.signal(());
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
                let level = low_level_sensor.get_level();
                {
                    let mut lll = LOW_LEVEL_SENSOR.lock().await;
                    *lll = level;
                }
            }
            Either::Second(_) => {
                let level = high_level_sensor.get_level();
                {
                    let mut hll = HIGH_LEVEL_SENSOR.lock().await;
                    *hll = level;
                }
            }
        }
    }
}
