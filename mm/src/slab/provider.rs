use core::ptr::NonNull;

use crate::PageAllocation;

/// slab/heap 使用的物理页后端。
///
/// allocation_pointer() 返回的映射必须：
///
/// - 在 allocation 存活期间永久有效；
/// - 覆盖完整 allocation；
/// - 对连续物理页提供连续内核虚拟地址。
pub trait PageProvider {
    type Error;

    fn allocate_pages(&mut self, order: usize) -> Result<PageAllocation, Self::Error>;

    fn free_pages(&mut self, allocation: PageAllocation) -> Result<(), Self::Error>;

    fn allocation_pointer(&self, allocation: &PageAllocation) -> Result<NonNull<u8>, Self::Error>;

    fn allocate_slab_page(&mut self) -> Result<PageAllocation, Self::Error> {
        self.allocate_pages(0)
    }

    fn free_slab_page(&mut self, allocation: PageAllocation) -> Result<(), Self::Error> {
        self.free_pages(allocation)
    }
}
