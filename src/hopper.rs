use cc_talk_core::{
    cc_talk::{
        DataStorage, HopperDispenseStatus, HopperStatus, Manufacturer, MemoryType, SerialCode,
    },
    Category, ChecksumType, Device,
};
use cc_talk_device::device_impl::{DeviceImpl, SimplePayoutDevice};

use crate::{
    payout::{
        enable_payout, get_dispense_count, get_payout_status, get_sensor_status, request_payout,
    },
    reset::{send_reset_signal, ResetType},
};

pub struct Hopper;

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

    fn product_code(&self) -> &str {
        "Universal Hopper Adapter"
    }

    fn serial_number(&self) -> SerialCode {
        SerialCode::new(0, 215, 0)
    }

    fn software_revision(&self) -> &str {
        "V1.0.0"
    }

    fn build_code(&self) -> &str {
        "V1.0.0"
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
        3
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

    fn request_hopper_coin(&self) -> &str {
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
        (0, 0, 0)
    }
}
