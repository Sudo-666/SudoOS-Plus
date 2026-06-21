use core::{alloc::Layout, ptr::NonNull};

use crate::{PageProvider, SizeClass, SlabAllocator};

use super::{HeapError, large};

#[derive(Clone, Copy, Debug)]
pub struct HeapStats {
    pub large_allocations: usize,
    pub large_pages: usize,
}

pub struct HeapAllocator<P> {
    slab: SlabAllocator<P>,

    large_allocations: usize,
    large_pages: usize,
}

impl<P> HeapAllocator<P>
where
    P: PageProvider,
{
    pub const fn new(provider: P) -> Self {
        Self {
            slab: SlabAllocator::new(provider),

            large_allocations: 0,
            large_pages: 0,
        }
    }

    pub fn allocate(
        &mut self,
        layout: Layout,
        zeroed: bool,
    ) -> Result<NonNull<u8>, HeapError<P::Error>> {
        if layout.size() == 0 {
            return Err(HeapError::ZeroSizedLayout);
        }

        if SizeClass::for_layout(layout).is_some() {
            let pointer = self
                .slab
                .allocate(layout)?
                .ok_or(HeapError::CorruptLargeAllocation)?;

            if zeroed {
                // SAFETY: slab 返回的对象至少覆盖 layout.size() 字节且归当前 heap 独占。
                unsafe {
                    pointer.as_ptr().write_bytes(0, layout.size());
                }
            }

            return Ok(pointer);
        }

        let (pointer, pages) = large::allocate(self.slab.provider_mut(), layout, zeroed)?;

        self.large_allocations = self
            .large_allocations
            .checked_add(1)
            .ok_or(HeapError::CounterOverflow)?;

        self.large_pages = self
            .large_pages
            .checked_add(pages)
            .ok_or(HeapError::CounterOverflow)?;

        Ok(pointer)
    }

    /// # Safety
    ///
    /// pointer/layout 必须对应此前由当前 heap 返回且尚未释放
    /// 的同一次分配。
    pub unsafe fn deallocate(
        &mut self,
        pointer: NonNull<u8>,
        layout: Layout,
    ) -> Result<(), HeapError<P::Error>> {
        if layout.size() == 0 {
            return Err(HeapError::ZeroSizedLayout);
        }

        if SizeClass::for_layout(layout).is_some() {
            // SAFETY: 当前函数的 safety contract 保证 pointer/layout 来自本 heap。
            let handled = unsafe { self.slab.deallocate(pointer, layout)? };

            if !handled {
                return Err(HeapError::CorruptLargeAllocation);
            }

            return Ok(());
        }

        // SAFETY: 当前函数的 safety contract 保证 pointer/layout 来自本 heap。
        let released_pages =
            unsafe { large::deallocate(self.slab.provider_mut(), pointer, layout)? };

        self.large_allocations = self
            .large_allocations
            .checked_sub(1)
            .ok_or(HeapError::CorruptLargeAllocation)?;

        self.large_pages = self
            .large_pages
            .checked_sub(released_pages)
            .ok_or(HeapError::CorruptLargeAllocation)?;

        Ok(())
    }

    pub fn shrink(&mut self) -> Result<(), HeapError<P::Error>> {
        self.slab.shrink()?;

        Ok(())
    }

    pub const fn stats(&self) -> HeapStats {
        HeapStats {
            large_allocations: self.large_allocations,

            large_pages: self.large_pages,
        }
    }
}

// SAFETY: HeapAllocator 只有通过 &mut self 修改；跨 CPU 移动时 provider 也必须可 Send。
unsafe impl<P> Send for HeapAllocator<P> where P: PageProvider + Send {}
