#![no_std]

mod blob;
mod error;
mod region;
mod tree;

pub use blob::{FdtBlob, FdtHeader};

pub use error::FdtError;

pub use region::{MemoryRegion, VirtioMmioRegion};

pub use tree::DeviceTree;
