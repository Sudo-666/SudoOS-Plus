use core::ptr::write_bytes;

use myos_mm::{AllocationClass, BuddyAllocator, BuddyError, PageAllocation, PhysFrame};

use crate::irq_lock::IrqSpinLock;

static PAGE_ALLOCATOR: IrqSpinLock<Option<BuddyAllocator>> = IrqSpinLock::new(None);

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

pub fn is_initialized() -> bool {
    PAGE_ALLOCATOR.lock().is_some()
}

pub fn allocate(
    order: usize,
    options: PageAllocationOptions,
) -> Result<PageAllocation, GlobalPageAllocatorError> {
    let mut slot = PAGE_ALLOCATOR.lock();

    let allocator = slot
        .as_mut()
        .ok_or(GlobalPageAllocatorError::NotInitialized)?;

    let allocation = allocator.allocate(order, options.class())?;

    if options.is_zeroed() {
        if zero_allocation(&allocation).is_err() {
            /*
             * 地址转换失败时，不允许泄漏已分配页。
             */
            allocator.free(allocation)?;

            return Err(GlobalPageAllocatorError::PhysicalMemoryNotAccessible);
        }
    } else {
        #[cfg(debug_assertions)]
        poison_allocation(&allocation, ALLOCATED_POISON)?;
    }

    Ok(allocation)
}

pub fn free(allocation: PageAllocation) -> Result<(), GlobalPageAllocatorError> {
    let mut slot = PAGE_ALLOCATOR.lock();

    let allocator = slot
        .as_mut()
        .ok_or(GlobalPageAllocatorError::NotInitialized)?;

    #[cfg(debug_assertions)]
    poison_allocation(&allocation, FREED_POISON)?;

    allocator.free(allocation)?;

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

pub fn decrement_reference(frame: PhysFrame) -> Result<u32, GlobalPageAllocatorError> {
    let slot = PAGE_ALLOCATOR.lock();

    let allocator = slot
        .as_ref()
        .ok_or(GlobalPageAllocatorError::NotInitialized)?;

    Ok(allocator.decrement_reference(frame)?)
}

pub fn free_unreferenced_frame(frame: PhysFrame) -> Result<(), GlobalPageAllocatorError> {
    let mut slot = PAGE_ALLOCATOR.lock();

    let allocator = slot
        .as_mut()
        .ok_or(GlobalPageAllocatorError::NotInitialized)?;

    allocator.free_unreferenced_frame(frame)?;

    Ok(())
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
