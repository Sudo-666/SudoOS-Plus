#![no_std]

mod address;
mod address_space;
mod buddy;
mod early_allocator;
mod fault;
mod frame;
mod heap;
mod layout;
mod map;
mod paging;
mod range;
mod slab;
mod tlb;
mod virtual_address;
mod virtual_page;
mod virtual_range;
mod vma;
mod vmalloc;

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

pub use buddy::{
    AllocationClass, BuddyAllocator, BuddyError, DMA32_LIMIT, MAX_ORDER, PageAllocation, PageState,
    ZoneKind,
};

pub use slab::{
    MAX_SLAB_OBJECT_SIZE, MIN_SLAB_OBJECT_SIZE, PageProvider, SIZE_CLASS_COUNT, SizeClass,
    SlabAllocator, SlabCacheStats, SlabError,
};

pub use heap::{HeapAllocator, HeapError, HeapStats};

pub use vma::{VmArea, VmAreaError, VmAreaFlags, VmAreaKind, VmAreaSet};

pub use address_space::{AddressSpace, AddressSpaceError, ProgramBreak};

pub use fault::{FaultAccess, FaultOutcome, FaultSource, PageFault};

pub use vmalloc::{KernelVirtualAllocator, KernelVirtualReservation, VmallocKind};

pub use tlb::{AddressSpaceId, TlbFlush, TlbScope, TlbShootdown};
