#![no_std]

mod address;
mod early_allocator;
mod frame;
mod layout;
mod map;
mod paging;
mod range;
mod virtual_address;
mod virtual_page;
mod virtual_range;

pub use address::{PAGE_SHIFT, PAGE_SIZE, PhysAddr};

pub use layout::{VirtualLayoutError, VirtualRegion, require_address_in_region, validate_regions};

pub use map::{MemoryMap, MemoryMapError};

pub use range::PhysRange;

pub use virtual_address::VirtAddr;
pub use virtual_range::VirtRange;

pub use early_allocator::{EarlyFrameAllocator, EarlyFrameAllocatorError};

pub use frame::{FrameBlock, PhysFrame};

pub use paging::{
    MappingOptions, MappingOptionsError, MemoryType, PagePermissions, PageTableAccessError,
    PageTableGeometry, PageTableIndex, RawPageTable,
};

pub use virtual_page::VirtPage;
