#![no_std]

use cc_talk_core::cc_talk::{Packet, MAX_BLOCK_LENGTH};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};

pub mod cc_talk_usb;
pub mod fmt;

pub type SignalPacket =
    Signal<CriticalSectionRawMutex, Packet<heapless::Vec<u8, MAX_BLOCK_LENGTH>>>;

static PACKET_ARRIVED_SIGNAL: SignalPacket = Signal::new();
static PACKET_REPLY_SIGNAL: SignalPacket = Signal::new();
