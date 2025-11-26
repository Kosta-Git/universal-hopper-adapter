use cc_talk_core::cc_talk::{
    Category, ChecksumType, DataStorage, Device, HopperDispenseStatus, HopperStatus, Manufacturer,
    MemoryType, SerialCode,
};
use cc_talk_device::device_impl::{DeviceImpl, SimplePayoutDevice};
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, mutex::Mutex};

use crate::{
    build_info,
    payout::{
        enable_payout, get_dispense_count, get_payout_status, get_sensor_status, request_payout,
    },
    reset::{send_reset_signal, ResetType},
};

static BUS_ADDRESS: Mutex<ThreadModeRawMutex, u8> = Mutex::new(3);

const fn parse_serial_code() -> (u8, u8, u8) {
    const SERIAL_STR: &str = match option_env!("HOPPER_SERIAL_CODE") {
        Some(s) => s,
        None => "0,215,0",
    };

    let bytes = SERIAL_STR.as_bytes();
    let mut a: u8 = 0;
    let mut b: u8 = 0;
    let mut c: u8 = 0;
    let mut i = 0;
    let mut field = 0;

    while i < bytes.len() {
        if bytes[i] >= b'0' && bytes[i] <= b'9' {
            let digit = bytes[i] - b'0';
            match field {
                0 => a = a * 10 + digit,
                1 => b = b * 10 + digit,
                2 => c = c * 10 + digit,
                _ => {}
            }
        } else if bytes[i] == b',' {
            field += 1;
        }
        i += 1;
    }

    // TODO: make SerialCode::new const fn and return SerialCode directly
    (a, b, c)
}

pub struct Hopper;

pub async fn set_bus_address(address: u8) {
    let mut bus_address = BUS_ADDRESS.lock().await;
    *bus_address = address;
}

impl DeviceImpl for Hopper {
    fn manufacturer(&self) -> Manufacturer {
        Manufacturer::INOTEK
    }

    fn category(&self) -> Category {
        Category::Payout
    }

    fn checksum_type(&self) -> ChecksumType {
        ChecksumType::Crc8
    }

    fn product_code(&self) -> &'static str {
        "Universal Hopper Adapter"
    }

    fn serial_number(&self) -> SerialCode {
        let (a, b, c) = parse_serial_code();
        SerialCode::new(a, b, c)
    }

    fn software_revision(&self) -> &'static str {
        build_info::PKG_VERSION
    }

    fn build_code(&self) -> &'static str {
        build_info::PKG_VERSION
    }

    fn data_storage_availability(&self) -> DataStorage {
        DataStorage::new(MemoryType::VolatileOnReset, 0, 0, 0, 0)
    }

    fn comms_revision(&self) -> (u8, u8, u8) {
        (1, 4, 7) // From library
    }

    async fn reset(&self) {
        send_reset_signal(ResetType::All);
    }

    fn is_for_me(&self, destination_address: u8) -> bool {
        destination_address == self.address()
    }

    fn address(&self) -> u8 {
        BUS_ADDRESS.try_lock().map_or(3, |addr| *addr)
    }

    fn device(&self) -> Device {
        Device::new(self.address(), self.category(), ChecksumType::Crc8)
    }
}

impl SimplePayoutDevice for Hopper {
    async fn request_sensor_status(&self) -> HopperStatus {
        get_sensor_status().await
    }

    async fn emergency_stop(&self) {
        send_reset_signal(ResetType::Hopper);
    }

    fn request_hopper_coin(&self) -> &'static str {
        "" // The universal hopper mk2 can hold many type of coins.
    }

    async fn request_hopper_dispense_count(&self) -> u32 {
        get_dispense_count().await
    }

    async fn dispense_hopper_coins(&self, count: u8) {
        request_payout(count);
    }

    async fn request_payout_status(&self) -> HopperDispenseStatus {
        get_payout_status().await
    }

    async fn enable_payout(&self, enable: bool) {
        enable_payout(enable);
    }

    async fn test(&self) -> (u8, u8, u8) {
        // TODO: implement self test
        (0, 0, 0)
    }
}
