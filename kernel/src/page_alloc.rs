use alloc::vec::Vec;
use core::ptr::write_bytes;

use myos_mm::{AllocationClass, BuddyAllocator, BuddyError, PageAllocation, PhysFrame};

use crate::{
    irq_lock::IrqSpinLock,
    lockdep::{LockClass, LockRank},
};

static PAGE_ALLOCATOR: IrqSpinLock<Option<BuddyAllocator>> = IrqSpinLock::new_with_class(
    None,
    LockClass::new("page_allocator", LockRank::PageAllocator, 1),
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlobalPageAllocatorError {
    AlreadyInitialized,
    NotInitialized,

    Buddy(BuddyError),

    PhysicalMemoryNotAccessible,
}

impl From<BuddyError> for GlobalPageAllocatorError {
    fn from(error: BuddyError) -> Self {
        Self::Buddy(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageAllocationOptions {
    class: AllocationClass,
    zeroed: bool,
}

impl PageAllocationOptions {
    pub const fn kernel() -> Self {
        Self {
            class: AllocationClass::Any,
            zeroed: false,
        }
    }

    pub const fn kernel_zeroed() -> Self {
        Self {
            class: AllocationClass::Any,
            zeroed: true,
        }
    }

    pub const fn dma32_zeroed() -> Self {
        Self {
            class: AllocationClass::Dma32,
            zeroed: true,
        }
    }

    pub const fn class(self) -> AllocationClass {
        self.class
    }

    pub const fn is_zeroed(self) -> bool {
        self.zeroed
    }
}

pub fn install(allocator: BuddyAllocator) -> Result<(), GlobalPageAllocatorError> {
    let mut slot = PAGE_ALLOCATOR.lock();

    if slot.is_some() {
        return Err(GlobalPageAllocatorError::AlreadyInitialized);
    }

    *slot = Some(allocator);

    Ok(())
}

/// Install the global page allocator during the single-CPU boot handoff.
///
/// # Safety
///
/// This must be called only before runtime allocator users, interrupt handlers,
/// and secondary CPUs can race with PAGE_ALLOCATOR. After this publication,
/// normal allocation/free/reference operations continue to use the IRQ-safe
/// lockdep-tracked PAGE_ALLOCATOR.lock() path.
pub unsafe fn install_boot(allocator: BuddyAllocator) -> Result<(), GlobalPageAllocatorError> {
    let slot = unsafe { PAGE_ALLOCATOR.get_mut_unchecked() };
    if slot.is_some() {
        return Err(GlobalPageAllocatorError::AlreadyInitialized);
    }
    *slot = Some(allocator);
    Ok(())
}

/// Inspect boot-time publication without entering the runtime IRQ lock path.
///
/// # Safety
///
/// Same contract as install_boot(): single-CPU boot phase only.
pub unsafe fn is_initialized_boot() -> bool {
    unsafe { PAGE_ALLOCATOR.get_mut_unchecked().is_some() }
}

pub fn is_initialized() -> bool {
    PAGE_ALLOCATOR.lock().is_some()
}

pub fn allocate(
    order: usize,
    options: PageAllocationOptions,
) -> Result<PageAllocation, GlobalPageAllocatorError> {
    let allocation = {
        let mut slot = PAGE_ALLOCATOR.lock();

        let allocator = slot
            .as_mut()
            .ok_or(GlobalPageAllocatorError::NotInitialized)?;

        allocator.allocate(order, options.class())?
    };

    let prepare_result = if options.is_zeroed() {
        zero_allocation(&allocation)
    } else {
        #[cfg(debug_assertions)]
        {
            poison_allocation(&allocation, ALLOCATED_POISON)
        }

        #[cfg(not(debug_assertions))]
        {
            Ok(())
        }
    };

    if let Err(error) = prepare_result {
        let mut slot = PAGE_ALLOCATOR.lock();
        let allocator = slot
            .as_mut()
            .ok_or(GlobalPageAllocatorError::NotInitialized)?;

        allocator.free(allocation)?;

        return Err(error);
    };

    Ok(allocation)
}

pub fn free(allocation: PageAllocation) -> Result<(), GlobalPageAllocatorError> {
    {
        let mut slot = PAGE_ALLOCATOR.lock();
        let allocator = slot
            .as_mut()
            .ok_or(GlobalPageAllocatorError::NotInitialized)?;

        allocator.begin_free(&allocation)?;
    }

    #[cfg(debug_assertions)]
    if let Err(error) = poison_allocation(&allocation, FREED_POISON) {
        let mut slot = PAGE_ALLOCATOR.lock();
        let allocator = slot
            .as_mut()
            .ok_or(GlobalPageAllocatorError::NotInitialized)?;

        allocator.cancel_free(&allocation)?;

        return Err(error);
    }

    let mut slot = PAGE_ALLOCATOR.lock();

    let allocator = slot
        .as_mut()
        .ok_or(GlobalPageAllocatorError::NotInitialized)?;

    allocator.finish_free(&allocation)?;

    Ok(())
}

pub fn reference_count(frame: PhysFrame) -> Result<u32, GlobalPageAllocatorError> {
    let slot = PAGE_ALLOCATOR.lock();

    let allocator = slot
        .as_ref()
        .ok_or(GlobalPageAllocatorError::NotInitialized)?;

    Ok(allocator.reference_count(frame)?)
}

pub fn increment_reference(frame: PhysFrame) -> Result<u32, GlobalPageAllocatorError> {
    let slot = PAGE_ALLOCATOR.lock();

    let allocator = slot
        .as_ref()
        .ok_or(GlobalPageAllocatorError::NotInitialized)?;

    Ok(allocator.increment_reference(frame)?)
}

/// Increments the reference count of a batch of frames under one allocator
/// lock.  COW fork retires one global lock acquisition per resident page;
/// with seven other CPUs faulting pages in, that churn dominated fork's
/// per-page cost.  All-or-nothing: on failure the already-incremented prefix
/// is rolled back so the caller never sees a half-applied batch.
pub fn increment_reference_many(frames: &[PhysFrame]) -> Result<(), GlobalPageAllocatorError> {
    let mut slot = PAGE_ALLOCATOR.lock();

    let allocator = slot
        .as_mut()
        .ok_or(GlobalPageAllocatorError::NotInitialized)?;

    let mut done = 0_usize;
    for frame in frames {
        if let Err(error) = allocator.increment_reference(*frame) {
            for frame in &frames[..done] {
                let _ = allocator.decrement_reference(*frame);
            }
            return Err(error.into());
        }
        done += 1;
    }

    Ok(())
}

pub fn decrement_reference(frame: PhysFrame) -> Result<u32, GlobalPageAllocatorError> {
    let slot = PAGE_ALLOCATOR.lock();

    let allocator = slot
        .as_ref()
        .ok_or(GlobalPageAllocatorError::NotInitialized)?;

    Ok(allocator.decrement_reference(frame)?)
}

pub fn free_unreferenced_frame(frame: PhysFrame) -> Result<(), GlobalPageAllocatorError> {
    let allocation = {
        let mut slot = PAGE_ALLOCATOR.lock();

        let allocator = slot
            .as_mut()
            .ok_or(GlobalPageAllocatorError::NotInitialized)?;

        allocator.begin_free_unreferenced_frame(frame)?
    };

    #[cfg(debug_assertions)]
    if let Err(error) = poison_allocation(&allocation, FREED_POISON) {
        let mut slot = PAGE_ALLOCATOR.lock();
        let allocator = slot
            .as_mut()
            .ok_or(GlobalPageAllocatorError::NotInitialized)?;

        allocator.cancel_free(&allocation)?;

        return Err(error);
    }

    let mut slot = PAGE_ALLOCATOR.lock();

    let allocator = slot
        .as_mut()
        .ok_or(GlobalPageAllocatorError::NotInitialized)?;

    allocator.finish_free(&allocation)?;

    Ok(())
}

/// Frees a batch of allocations with one lock acquisition per phase instead of
/// two per allocation.  Exec teardown retires tens of thousands of pages at
/// once; per-page locking turned that into the hottest global-lock churn in
/// buildstorm.  The caller moves the vector in; nothing is allocated here.
pub fn free_many(allocations: Vec<PageAllocation>) -> Result<(), GlobalPageAllocatorError> {
    {
        let mut slot = PAGE_ALLOCATOR.lock();
        let allocator = slot
            .as_mut()
            .ok_or(GlobalPageAllocatorError::NotInitialized)?;

        let mut begun = 0_usize;
        for allocation in &allocations {
            if let Err(error) = allocator.begin_free(allocation) {
                for allocation in &allocations[..begun] {
                    let _ = allocator.cancel_free(allocation);
                }
                return Err(error.into());
            }
            begun += 1;
        }
    }

    #[cfg(debug_assertions)]
    for allocation in &allocations {
        if let Err(error) = poison_allocation(allocation, FREED_POISON) {
            let mut slot = PAGE_ALLOCATOR.lock();
            let allocator = slot
                .as_mut()
                .ok_or(GlobalPageAllocatorError::NotInitialized)?;

            for allocation in &allocations {
                let _ = allocator.cancel_free(allocation);
            }

            return Err(error);
        }
    }

    let mut slot = PAGE_ALLOCATOR.lock();

    let allocator = slot
        .as_mut()
        .ok_or(GlobalPageAllocatorError::NotInitialized)?;

    for allocation in &allocations {
        allocator.finish_free(allocation)?;
    }

    Ok(())
}

/// Decrements the reference count of a batch of frames and frees the ones that
/// reach zero, all under one allocator lock.  `scratch` is a caller-provided
/// Vec reused across calls; it is cleared and filled with the begin-freed
/// allocations.  Returns the number of frames actually freed.
pub fn release_many_unreferenced(
    frames: &[PhysFrame],
    scratch: &mut Vec<PageAllocation>,
) -> Result<usize, GlobalPageAllocatorError> {
    scratch.clear();

    {
        let mut slot = PAGE_ALLOCATOR.lock();
        let allocator = slot
            .as_mut()
            .ok_or(GlobalPageAllocatorError::NotInitialized)?;

        for frame in frames {
            if allocator.decrement_reference(*frame)? == 0 {
                scratch.push(allocator.begin_free_unreferenced_frame(*frame)?);
            }
        }

        #[cfg(debug_assertions)]
        for allocation in scratch.iter() {
            if let Err(error) = poison_allocation(allocation, FREED_POISON) {
                for allocation in scratch.iter() {
                    let _ = allocator.cancel_free(allocation);
                }
                return Err(error);
            }
        }

        for allocation in scratch.iter() {
            allocator.finish_free(allocation)?;
        }
    }

    Ok(scratch.len())
}

pub fn total_free_pages() -> Result<usize, GlobalPageAllocatorError> {
    let slot = PAGE_ALLOCATOR.lock();

    let allocator = slot
        .as_ref()
        .ok_or(GlobalPageAllocatorError::NotInitialized)?;

    Ok(allocator.total_free_pages())
}

fn zero_allocation(allocation: &PageAllocation) -> Result<(), GlobalPageAllocatorError> {
    fill_allocation(allocation, 0)
}

#[cfg(debug_assertions)]
const ALLOCATED_POISON: u8 = 0xa5;

#[cfg(debug_assertions)]
const FREED_POISON: u8 = 0xdd;

#[cfg(debug_assertions)]
fn poison_allocation(
    allocation: &PageAllocation,
    value: u8,
) -> Result<(), GlobalPageAllocatorError> {
    fill_allocation(allocation, value)
}

fn fill_allocation(allocation: &PageAllocation, value: u8) -> Result<(), GlobalPageAllocatorError> {
    let range = allocation.range();

    let pointer = crate::arch::memory::phys_access::ram_mut_ptr::<u8>(range.start())
        .map_err(|_| GlobalPageAllocatorError::PhysicalMemoryNotAccessible)?;

    /*
     * SAFETY:
     *
     * - allocation 对其完整物理范围拥有独占所有权；
     * - RAM direct map/DMW 提供连续可写虚拟映射；
     * - range.size() 对应实际分配块大小。
     */
    unsafe {
        write_bytes(pointer, value, range.size());
    }

    Ok(())
}

#[cfg(debug_assertions)]
pub fn verify() {
    use core::slice;

    let before = total_free_pages().expect("page allocator unavailable");

    let zeroed = allocate(0, PageAllocationOptions::kernel_zeroed())
        .expect("unable to allocate zeroed page");

    let pointer = crate::arch::memory::phys_access::ram_ptr::<u8>(zeroed.range().start())
        .expect("allocated page is not direct-mapped");

    /*
     * SAFETY:
     *
     * zeroed 仍归测试代码独占，页面完整可读。
     */
    let bytes = unsafe { slice::from_raw_parts(pointer, zeroed.size()) };

    assert!(
        bytes.iter().all(|byte| *byte == 0),
        "zeroed page allocation contains non-zero bytes",
    );

    let block = allocate(3, PageAllocationOptions::kernel())
        .expect("unable to allocate order-3 page block");

    assert_eq!(total_free_pages().unwrap(), before - 9,);

    assert_eq!(reference_count(zeroed.start()).unwrap(), 1);
    assert_eq!(increment_reference(zeroed.start()).unwrap(), 2);
    assert_eq!(decrement_reference(zeroed.start()).unwrap(), 1);

    crate::println!("global page allocator test:");
    crate::println!(
        "  zeroed page : {:#018x}",
        zeroed.start().start_address().get(),
    );
    crate::println!(
        "  order-3     : [{:#018x}, {:#018x})",
        block.range().start().get(),
        block.range().end().get(),
    );
    crate::println!("  poisoning   : enabled",);

    free(block).expect("unable to free order-3 block");

    free(zeroed).expect("unable to free zeroed page");

    let cow_page = allocate(0, PageAllocationOptions::kernel_zeroed())
        .expect("unable to allocate test refcount page");

    let cow_frame = cow_page.start();

    assert_eq!(decrement_reference(cow_frame).unwrap(), 0);

    free_unreferenced_frame(cow_frame).expect("unable to free unreferenced frame");

    assert_eq!(total_free_pages().unwrap(), before,);

    crate::println!("  free/merge  : verified",);
    crate::println!("  refcount    : verified",);
}
