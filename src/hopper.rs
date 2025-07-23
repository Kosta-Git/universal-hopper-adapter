use cc_talk_core::{
    cc_talk::{
        DataStorage, HopperDispenseStatus, HopperStatus, Manufacturer, MemoryType, SerialCode,
    },
    Category, ChecksumType, Device,
};
use cc_talk_device::device_impl::{DeviceImpl, SimplePayoutDevice};

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
        todo!()
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
        todo!()
    }

    async fn emergency_stop(&self) {
        todo!()
    }

    fn request_hopper_coin(&self) -> &str {
        todo!()
    }

    async fn request_hopper_dispense_count(&self) -> u32 {
        todo!()
    }

    async fn dispense_hopper_coins(&self, count: u8) {
        todo!()
    }

    async fn request_payout_status(&self) -> HopperDispenseStatus {
        todo!()
    }

    async fn enable_payout(&self, enable: bool) {
        todo!()
    }

    async fn test(&self) -> (u8, u8, u8) {
        todo!()
    }
}
