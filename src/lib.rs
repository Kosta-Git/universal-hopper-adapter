#![no_std]
#![no_main]

use cc_talk_core::cc_talk::{Packet, MAX_BLOCK_LENGTH};
use defmt_rtt as _;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use panic_probe as _;

#[defmt::panic_handler]
fn panic() -> ! {
    cortex_m::asm::udf();
}

pub mod cc_talk_uart;
pub mod cc_talk_usb;
pub mod hopper;
pub mod payout;
pub mod reset;

pub type SignalPacket =
    Signal<CriticalSectionRawMutex, Packet<heapless::Vec<u8, MAX_BLOCK_LENGTH>>>;
