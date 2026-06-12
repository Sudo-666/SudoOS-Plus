use core::{
    alloc::Layout,
    cmp::max,
    mem::{ManuallyDrop, align_of, size_of},
    ptr::NonNull,
};

use crate::{MAX_ORDER, PAGE_SIZE, PageAllocation, PageProvider};

use super::HeapError;

const LARGE_MAGIC: u64 = 0x4c41_5247_454d_594f;

const DEAD_LARGE_MAGIC: u64 = 0x4445_4144_4c41_5247;

#[cfg(debug_assertions)]
const LARGE_ALLOCATED_POISON: u8 = 0xab;

#[cfg(debug_assertions)]
const LARGE_FREED_POISON: u8 = 0xdb;

#[repr(C)]
struct LargeAllocationHeader {
    magic: u64,

    requested_size: usize,
    requested_align: usize,

    allocation: ManuallyDrop<PageAllocation>,
}

impl LargeAllocationHeader {
    fn allocation(&self) -> &PageAllocation {
        /*
         * ManuallyDrop<T> 与 T 具有相同布局。
         */
        // SAFETY: ManuallyDrop<PageAllocation> 与 PageAllocation 布局一致。
        unsafe {
            &*(&self.allocation as *const ManuallyDrop<PageAllocation> as *const PageAllocation)
        }
    }
}

pub(super) fn allocate<P>(
    provider: &mut P,
    layout: Layout,
    zeroed: bool,
) -> Result<(NonNull<u8>, usize), HeapError<P::Error>>
where
    P: PageProvider,
{
    if layout.size() == 0 {
        return Err(HeapError::ZeroSizedLayout);
    }

    let effective_alignment = max(layout.align(), align_of::<LargeAllocationHeader>());

    let required = layout
        .size()
        .checked_add(size_of::<LargeAllocationHeader>())
        .and_then(|size| size.checked_add(effective_alignment - 1))
        .ok_or(HeapError::AddressOverflow)?;

    let page_count = required
        .checked_add(PAGE_SIZE - 1)
        .ok_or(HeapError::AddressOverflow)?
        / PAGE_SIZE;

    let rounded_pages = page_count
        .checked_next_power_of_two()
        .ok_or(HeapError::AllocationTooLarge)?;

    let order = rounded_pages.trailing_zeros() as usize;

    if order >= MAX_ORDER {
        return Err(HeapError::AllocationTooLarge);
    }

    let allocation = provider
        .allocate_pages(order)
        .map_err(HeapError::Provider)?;

    let allocated_pages = allocation.page_count();

    let base_pointer = match provider.allocation_pointer(&allocation) {
        Ok(pointer) => pointer,

        Err(error) => {
            return release_with_error(provider, allocation, HeapError::Provider(error));
        }
    };

    let base = base_pointer.as_ptr() as usize;

    let block_end = base
        .checked_add(allocation.size())
        .ok_or(HeapError::AddressOverflow)?;

    let unaligned_user = base
        .checked_add(size_of::<LargeAllocationHeader>())
        .ok_or(HeapError::AddressOverflow)?;

    let user_address =
        align_up(unaligned_user, effective_alignment).ok_or(HeapError::AddressOverflow)?;

    let user_end = user_address
        .checked_add(layout.size())
        .ok_or(HeapError::AddressOverflow)?;

    if user_end > block_end {
        return release_with_error(provider, allocation, HeapError::AllocationTooLarge);
    }

    let header_address = user_address
        .checked_sub(size_of::<LargeAllocationHeader>())
        .ok_or(HeapError::AddressOverflow)?;

    if !header_address.is_multiple_of(align_of::<LargeAllocationHeader>()) {
        return release_with_error(provider, allocation, HeapError::CorruptLargeAllocation);
    }

    let header_pointer = header_address as *mut LargeAllocationHeader;

    /*
     * SAFETY:
     *
     * header 位于独占的 buddy allocation 中，并满足对齐。
     */
    unsafe {
        header_pointer.write(LargeAllocationHeader {
            magic: LARGE_MAGIC,

            requested_size: layout.size(),
            requested_align: layout.align(),

            allocation: ManuallyDrop::new(allocation),
        });
    }

    let user_pointer =
        NonNull::new(user_address as *mut u8).ok_or(HeapError::CorruptLargeAllocation)?;

    if zeroed {
        // SAFETY: user_pointer 位于独占的大对象分配内，范围覆盖 layout.size()。
        unsafe {
            user_pointer.as_ptr().write_bytes(0, layout.size());
        }
    } else {
        #[cfg(debug_assertions)]
        // SAFETY: user_pointer 位于独占的大对象分配内，范围覆盖 layout.size()。
        unsafe {
            user_pointer
                .as_ptr()
                .write_bytes(LARGE_ALLOCATED_POISON, layout.size());
        }
    }

    Ok((user_pointer, allocated_pages))
}

/// # Safety
///
/// pointer/layout 必须对应当前 heap 的有效大对象分配。
pub(super) unsafe fn deallocate<P>(
    provider: &mut P,
    pointer: NonNull<u8>,
    layout: Layout,
) -> Result<usize, HeapError<P::Error>>
where
    P: PageProvider,
{
    let user_address = pointer.as_ptr() as usize;

    let header_address = user_address
        .checked_sub(size_of::<LargeAllocationHeader>())
        .ok_or(HeapError::CorruptLargeAllocation)?;

    if !header_address.is_multiple_of(align_of::<LargeAllocationHeader>()) {
        return Err(HeapError::CorruptLargeAllocation);
    }

    let header_pointer = header_address as *mut LargeAllocationHeader;

    // SAFETY: header_address 是由 allocator 写入的有效 header 位置。
    let header = unsafe { &mut *header_pointer };

    if header.magic != LARGE_MAGIC {
        return Err(HeapError::CorruptLargeAllocation);
    }

    if header.requested_size != layout.size() || header.requested_align != layout.align() {
        return Err(HeapError::LayoutMismatch);
    }

    if !user_address.is_multiple_of(layout.align()) {
        return Err(HeapError::CorruptLargeAllocation);
    }

    let allocation = header.allocation();

    let base_pointer = provider
        .allocation_pointer(allocation)
        .map_err(HeapError::Provider)?;

    let base = base_pointer.as_ptr() as usize;

    let block_end = base
        .checked_add(allocation.size())
        .ok_or(HeapError::AddressOverflow)?;

    let user_end = user_address
        .checked_add(layout.size())
        .ok_or(HeapError::AddressOverflow)?;

    if header_address < base || user_end > block_end {
        return Err(HeapError::CorruptLargeAllocation);
    }

    let allocated_pages = allocation.page_count();

    #[cfg(debug_assertions)]
    // SAFETY: pointer/layout 对应当前有效大对象分配。
    unsafe {
        pointer
            .as_ptr()
            .write_bytes(LARGE_FREED_POISON, layout.size());
    }

    header.magic = DEAD_LARGE_MAGIC;

    // SAFETY: magic 校验通过且 header 即将失效，allocation token 只能取出一次。
    let allocation = unsafe { ManuallyDrop::take(&mut header.allocation) };

    provider
        .free_pages(allocation)
        .map_err(HeapError::Provider)?;

    Ok(allocated_pages)
}

fn release_with_error<P, T>(
    provider: &mut P,
    allocation: PageAllocation,
    error: HeapError<P::Error>,
) -> Result<T, HeapError<P::Error>>
where
    P: PageProvider,
{
    match provider.free_pages(allocation) {
        Ok(()) => Err(error),

        Err(free_error) => Err(HeapError::Provider(free_error)),
    }
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    debug_assert!(alignment.is_power_of_two());

    let mask = alignment - 1;

    value.checked_add(mask).map(|value| value & !mask)
}
