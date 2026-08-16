#![feature(unsigned_is_multiple_of)]
#![no_std]

mod blob;
mod error;
mod region;
mod tree;

pub use blob::{FdtBlob, FdtHeader};

pub use error::FdtError;

pub use region::{BootRamdiskRegion, MemoryRegion, PciHostBridge, VirtioMmioRegion};

pub use tree::DeviceTree;
