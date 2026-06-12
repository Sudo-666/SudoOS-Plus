use core::{
    cmp::min,
    mem::{align_of, size_of},
    ptr::NonNull,
    sync::atomic::Ordering,
};

use crate::{PAGE_SIZE, PhysAddr, PhysFrame, PhysRange};

use super::{
    page::{INVALID_ORDER, Page, PageState},
    zone::{DMA32_LIMIT_PFN, INVALID_PFN, MAX_ORDER, Zone, ZoneKind},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AllocationClass {
    /// 只允许低于 4 GiB 的物理页。
    Dma32,

    /// 优先 Normal，失败后回退 DMA32。
    Any,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuddyError {
    InvalidManagedRange,

    MetadataPointerIsNull,

    MetadataMisaligned { required_alignment: usize },

    MetadataTooSmall { required: usize, available: usize },

    RangeIsNotPageAligned,

    RangeOutsideManagedMemory,

    AddressOverflow,

    InvalidOrder { order: usize },

    PageStateConflict { pfn: usize, state: PageState },

    CorruptFreeList,

    InvalidAllocation,

    ReferenceCountOverflow { frame: PhysFrame },

    ReferenceCountUnderflow { frame: PhysFrame },

    PageStillReferenced { frame: PhysFrame, count: u32 },

    OutOfMemory,
}

/// 由 buddy 返回的连续物理页。
///
/// 不实现 Clone/Copy，防止复制 allocation token 后发生双重释放。
#[derive(Debug)]
pub struct PageAllocation {
    start: PhysFrame,
    order: u8,
    zone: ZoneKind,
}

impl PageAllocation {
    pub const fn start(&self) -> PhysFrame {
        self.start
    }

    pub const fn order(&self) -> usize {
        self.order as usize
    }

    pub const fn zone(&self) -> ZoneKind {
        self.zone
    }

    pub const fn page_count(&self) -> usize {
        1_usize << self.order
    }

    pub const fn size(&self) -> usize {
        self.page_count() * PAGE_SIZE
    }

    pub const fn range(&self) -> PhysRange {
        match PhysRange::from_start_size(self.start.start_address(), self.size()) {
            Some(range) => range,
            None => panic!("buddy allocation range overflows"),
        }
    }
}

/// 长期物理页分配器。
///
/// 当前限制：
///
/// - 单 NUMA node；
/// - 调用者必须提供独占 `&mut self`；
/// - 暂无 per-CPU page cache；
/// - 暂无 reclaim/compaction。
pub struct BuddyAllocator {
    metadata: NonNull<Page>,

    managed: PhysRange,
    base_pfn: usize,
    page_count: usize,

    zones: [Zone; 2],
}

impl BuddyAllocator {
    pub fn required_metadata_bytes(managed: PhysRange) -> Result<usize, BuddyError> {
        if managed.is_empty() || !managed.is_page_aligned() {
            return Err(BuddyError::InvalidManagedRange);
        }

        let pages = managed.size() / PAGE_SIZE;

        pages
            .checked_mul(size_of::<Page>())
            .ok_or(BuddyError::AddressOverflow)
    }

    pub const fn metadata_alignment() -> usize {
        align_of::<Page>()
    }

    /// # Safety
    ///
    /// `metadata_pointer` 必须指向一段独占、永久有效且可写的
    /// 内核虚拟地址空间。该内存在 allocator 存活期间不得释放。
    pub unsafe fn new(
        metadata_pointer: *mut u8,
        metadata_bytes: usize,
        managed: PhysRange,
    ) -> Result<Self, BuddyError> {
        let required = Self::required_metadata_bytes(managed)?;

        if metadata_pointer.is_null() {
            return Err(BuddyError::MetadataPointerIsNull);
        }

        if !(metadata_pointer as usize).is_multiple_of(align_of::<Page>()) {
            return Err(BuddyError::MetadataMisaligned {
                required_alignment: align_of::<Page>(),
            });
        }

        if metadata_bytes < required {
            return Err(BuddyError::MetadataTooSmall {
                required,
                available: metadata_bytes,
            });
        }

        let metadata = NonNull::new(metadata_pointer.cast::<Page>())
            .ok_or(BuddyError::MetadataPointerIsNull)?;

        let base_pfn = managed.start().get() / PAGE_SIZE;

        let page_count = managed.size() / PAGE_SIZE;

        let end_pfn = base_pfn
            .checked_add(page_count)
            .ok_or(BuddyError::AddressOverflow)?;

        for index in 0..page_count {
            // SAFETY: 调用者提供的 metadata 区域至少容纳 page_count 个 Page。
            unsafe {
                metadata.as_ptr().add(index).write(Page::absent());
            }
        }

        let dma32_end = min(end_pfn, DMA32_LIMIT_PFN);

        let dma32 = if base_pfn < DMA32_LIMIT_PFN {
            Zone::new(base_pfn, dma32_end)
        } else {
            Zone::new(base_pfn, base_pfn)
        };

        let normal_start = base_pfn.max(DMA32_LIMIT_PFN).min(end_pfn);

        let normal = Zone::new(normal_start, end_pfn);

        Ok(Self {
            metadata,
            managed,
            base_pfn,
            page_count,
            zones: [dma32, normal],
        })
    }

    pub const fn managed_range(&self) -> PhysRange {
        self.managed
    }

    /// 将真实存在的普通 RAM 标记为 reserved。
    pub fn mark_present_range(&mut self, range: PhysRange) -> Result<(), BuddyError> {
        let (start, end) = self.validate_range(range)?;

        /*
         * 第一遍验证，保证失败时不留下半修改状态。
         */
        for pfn in start..end {
            let state = self.page(pfn)?.state;

            if !matches!(state, PageState::Absent) {
                return Err(BuddyError::PageStateConflict { pfn, state });
            }
        }

        for pfn in start..end {
            let kind = self.zone_kind_for_pfn(pfn)?;

            {
                let page = self.page_mut(pfn)?;

                page.state = PageState::Reserved;
                page.order = INVALID_ORDER;
                page.zone = kind as u8;
                page.next = INVALID_PFN;
                page.previous = INVALID_PFN;

                page.reference_count.store(0, Ordering::Relaxed);
            }

            self.zones[kind.index()].present_pages += 1;
        }

        Ok(())
    }

    /// 把一段 early allocator 剩余内存交给 buddy。
    pub fn release_range(&mut self, range: PhysRange) -> Result<(), BuddyError> {
        let (start, end) = self.validate_range(range)?;

        for pfn in start..end {
            let state = self.page(pfn)?.state;

            if !matches!(state, PageState::Reserved) {
                return Err(BuddyError::PageStateConflict { pfn, state });
            }
        }

        let mut pfn = start;

        while pfn < end {
            let kind = self.zone_kind_for_pfn(pfn)?;

            let zone_end = min(end, self.zones[kind.index()].end_pfn);

            let order = largest_block_order(pfn, zone_end - pfn);

            let pages = 1_usize << order;

            self.free_reserved_block(pfn, order, kind)?;

            self.zones[kind.index()].free_pages = self.zones[kind.index()]
                .free_pages
                .checked_add(pages)
                .ok_or(BuddyError::AddressOverflow)?;

            pfn += pages;
        }

        Ok(())
    }

    pub fn allocate(
        &mut self,
        order: usize,
        class: AllocationClass,
    ) -> Result<PageAllocation, BuddyError> {
        if order >= MAX_ORDER {
            return Err(BuddyError::InvalidOrder { order });
        }

        match class {
            AllocationClass::Dma32 => self
                .allocate_from_zone(order, ZoneKind::Dma32)?
                .ok_or(BuddyError::OutOfMemory),

            AllocationClass::Any => {
                if let Some(allocation) = self.allocate_from_zone(order, ZoneKind::Normal)? {
                    return Ok(allocation);
                }

                self.allocate_from_zone(order, ZoneKind::Dma32)?
                    .ok_or(BuddyError::OutOfMemory)
            }
        }
    }

    pub fn free(&mut self, allocation: PageAllocation) -> Result<(), BuddyError> {
        let order = allocation.order as usize;

        if order >= MAX_ORDER {
            return Err(BuddyError::InvalidAllocation);
        }

        let start_pfn = allocation.start.start_address().get() / PAGE_SIZE;

        let kind = self.zone_kind_for_pfn(start_pfn)?;

        if kind != allocation.zone || !self.zones[kind.index()].contains_block(start_pfn, order) {
            return Err(BuddyError::InvalidAllocation);
        }

        let page_count = 1_usize << order;

        /*
         * 完整验证后才修改状态。
         */
        for offset in 0..page_count {
            let page = self.page(start_pfn + offset)?;

            if !matches!(page.state, PageState::Allocated)
                || page.reference_count.load(Ordering::Relaxed) != 1
            {
                return Err(BuddyError::InvalidAllocation);
            }

            if offset == 0 && page.order as usize != order {
                return Err(BuddyError::InvalidAllocation);
            }
        }

        self.coalesce_and_insert(start_pfn, order, kind)?;

        self.zones[kind.index()].free_pages = self.zones[kind.index()]
            .free_pages
            .checked_add(page_count)
            .ok_or(BuddyError::AddressOverflow)?;

        Ok(())
    }

    pub fn reference_count(&self, frame: PhysFrame) -> Result<u32, BuddyError> {
        let pfn = frame.start_address().get() / PAGE_SIZE;
        let page = self.page(pfn)?;

        if !matches!(page.state, PageState::Allocated) {
            return Err(BuddyError::InvalidAllocation);
        }

        Ok(page.reference_count.load(Ordering::Acquire))
    }

    pub fn increment_reference(&self, frame: PhysFrame) -> Result<u32, BuddyError> {
        let pfn = frame.start_address().get() / PAGE_SIZE;
        let page = self.page(pfn)?;

        if !matches!(page.state, PageState::Allocated) {
            return Err(BuddyError::InvalidAllocation);
        }

        page.reference_count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                if count == 0 {
                    None
                } else {
                    count.checked_add(1)
                }
            })
            .map(|previous| previous + 1)
            .map_err(|previous| {
                if previous == u32::MAX {
                    BuddyError::ReferenceCountOverflow { frame }
                } else {
                    BuddyError::InvalidAllocation
                }
            })
    }

    pub fn decrement_reference(&self, frame: PhysFrame) -> Result<u32, BuddyError> {
        let pfn = frame.start_address().get() / PAGE_SIZE;
        let page = self.page(pfn)?;

        if !matches!(page.state, PageState::Allocated) {
            return Err(BuddyError::InvalidAllocation);
        }

        page.reference_count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                count.checked_sub(1)
            })
            .map(|previous| previous - 1)
            .map_err(|_| BuddyError::ReferenceCountUnderflow { frame })
    }

    pub fn free_unreferenced_frame(&mut self, frame: PhysFrame) -> Result<(), BuddyError> {
        let pfn = frame.start_address().get() / PAGE_SIZE;
        let kind = self.zone_kind_for_pfn(pfn)?;

        if !self.zones[kind.index()].contains_block(pfn, 0) {
            return Err(BuddyError::InvalidAllocation);
        }

        let page = self.page(pfn)?;

        if !matches!(page.state, PageState::Allocated) || page.order != 0 {
            return Err(BuddyError::InvalidAllocation);
        }

        let count = page.reference_count.load(Ordering::Acquire);

        if count != 0 {
            return Err(BuddyError::PageStillReferenced { frame, count });
        }

        self.coalesce_and_insert(pfn, 0, kind)?;

        self.zones[kind.index()].free_pages = self.zones[kind.index()]
            .free_pages
            .checked_add(1)
            .ok_or(BuddyError::AddressOverflow)?;

        Ok(())
    }

    pub fn total_free_pages(&self) -> usize {
        self.zones.iter().map(|zone| zone.free_pages).sum()
    }

    pub const fn zone_present_pages(&self, kind: ZoneKind) -> usize {
        self.zones[kind.index()].present_pages
    }

    pub const fn zone_free_pages(&self, kind: ZoneKind) -> usize {
        self.zones[kind.index()].free_pages
    }

    pub const fn free_block_count(&self, kind: ZoneKind, order: usize) -> Option<usize> {
        if order >= MAX_ORDER {
            return None;
        }

        Some(self.zones[kind.index()].free_area[order].block_count)
    }

    fn allocate_from_zone(
        &mut self,
        requested_order: usize,
        kind: ZoneKind,
    ) -> Result<Option<PageAllocation>, BuddyError> {
        let zone_index = kind.index();

        let mut found_order = None;

        for order in requested_order..MAX_ORDER {
            if self.zones[zone_index].free_area[order].head != INVALID_PFN {
                found_order = Some(order);
                break;
            }
        }

        let Some(mut order) = found_order else {
            return Ok(None);
        };

        let pfn = self.zones[zone_index].free_area[order].head;

        self.remove_free_block(pfn, order, kind)?;

        /*
         * 左半继续拆分，右半重新挂回对应 order。
         */
        while order > requested_order {
            order -= 1;

            let right_pfn = pfn + (1_usize << order);

            self.mark_free_block(right_pfn, order, kind)?;

            self.insert_free_block(right_pfn, order, kind)?;
        }

        self.mark_allocated_block(pfn, requested_order, kind)?;

        let allocated_pages = 1_usize << requested_order;

        self.zones[zone_index].free_pages = self.zones[zone_index]
            .free_pages
            .checked_sub(allocated_pages)
            .ok_or(BuddyError::CorruptFreeList)?;

        Ok(Some(PageAllocation {
            start: frame_from_pfn(pfn)?,
            order: requested_order as u8,
            zone: kind,
        }))
    }

    fn free_reserved_block(
        &mut self,
        pfn: usize,
        order: usize,
        kind: ZoneKind,
    ) -> Result<(), BuddyError> {
        self.coalesce_and_insert(pfn, order, kind)
    }

    fn coalesce_and_insert(
        &mut self,
        mut pfn: usize,
        mut order: usize,
        kind: ZoneKind,
    ) -> Result<(), BuddyError> {
        while order + 1 < MAX_ORDER {
            let buddy = pfn ^ (1_usize << order);

            if !self.zones[kind.index()].contains_block(buddy, order) {
                break;
            }

            let buddy_page = self.page(buddy)?;

            if !matches!(buddy_page.state, PageState::FreeHead)
                || buddy_page.order as usize != order
            {
                break;
            }

            self.remove_free_block(buddy, order, kind)?;

            pfn = min(pfn, buddy);
            order += 1;
        }

        self.mark_free_block(pfn, order, kind)?;

        self.insert_free_block(pfn, order, kind)
    }

    fn mark_free_block(
        &mut self,
        pfn: usize,
        order: usize,
        kind: ZoneKind,
    ) -> Result<(), BuddyError> {
        let page_count = 1_usize << order;

        for offset in 0..page_count {
            let page = self.page_mut(pfn + offset)?;

            page.state = if offset == 0 {
                PageState::FreeHead
            } else {
                PageState::FreeTail
            };

            page.order = if offset == 0 {
                order as u8
            } else {
                INVALID_ORDER
            };

            page.zone = kind as u8;
            page.next = INVALID_PFN;
            page.previous = INVALID_PFN;

            page.reference_count.store(0, Ordering::Relaxed);
        }

        Ok(())
    }

    fn mark_allocated_block(
        &mut self,
        pfn: usize,
        order: usize,
        kind: ZoneKind,
    ) -> Result<(), BuddyError> {
        let page_count = 1_usize << order;

        for offset in 0..page_count {
            let page = self.page_mut(pfn + offset)?;

            page.state = PageState::Allocated;

            page.order = if offset == 0 {
                order as u8
            } else {
                INVALID_ORDER
            };

            page.zone = kind as u8;
            page.next = INVALID_PFN;
            page.previous = INVALID_PFN;

            page.reference_count.store(1, Ordering::Relaxed);
        }

        Ok(())
    }

    fn insert_free_block(
        &mut self,
        pfn: usize,
        order: usize,
        kind: ZoneKind,
    ) -> Result<(), BuddyError> {
        let zone_index = kind.index();

        let previous_head = self.zones[zone_index].free_area[order].head;

        {
            let page = self.page_mut(pfn)?;

            page.previous = INVALID_PFN;
            page.next = previous_head;
        }

        if previous_head != INVALID_PFN {
            self.page_mut(previous_head)?.previous = pfn;
        }

        let area = &mut self.zones[zone_index].free_area[order];

        area.head = pfn;

        area.block_count = area
            .block_count
            .checked_add(1)
            .ok_or(BuddyError::CorruptFreeList)?;

        Ok(())
    }

    fn remove_free_block(
        &mut self,
        pfn: usize,
        order: usize,
        kind: ZoneKind,
    ) -> Result<(), BuddyError> {
        let zone_index = kind.index();

        let (previous, next) = {
            let page = self.page(pfn)?;

            if !matches!(page.state, PageState::FreeHead) || page.order as usize != order {
                return Err(BuddyError::CorruptFreeList);
            }

            (page.previous, page.next)
        };

        if previous == INVALID_PFN {
            if self.zones[zone_index].free_area[order].head != pfn {
                return Err(BuddyError::CorruptFreeList);
            }

            self.zones[zone_index].free_area[order].head = next;
        } else {
            self.page_mut(previous)?.next = next;
        }

        if next != INVALID_PFN {
            self.page_mut(next)?.previous = previous;
        }

        {
            let page = self.page_mut(pfn)?;

            page.previous = INVALID_PFN;
            page.next = INVALID_PFN;
        }

        let blocks = &mut self.zones[zone_index].free_area[order].block_count;

        *blocks = blocks.checked_sub(1).ok_or(BuddyError::CorruptFreeList)?;

        Ok(())
    }

    fn validate_range(&self, range: PhysRange) -> Result<(usize, usize), BuddyError> {
        if range.is_empty() || !range.is_page_aligned() {
            return Err(BuddyError::RangeIsNotPageAligned);
        }

        if !self.managed.contains_range(range) {
            return Err(BuddyError::RangeOutsideManagedMemory);
        }

        Ok((
            range.start().get() / PAGE_SIZE,
            range.end().get() / PAGE_SIZE,
        ))
    }

    fn zone_kind_for_pfn(&self, pfn: usize) -> Result<ZoneKind, BuddyError> {
        let kind = if pfn < DMA32_LIMIT_PFN {
            ZoneKind::Dma32
        } else {
            ZoneKind::Normal
        };

        if !self.zones[kind.index()].contains_pfn(pfn) {
            return Err(BuddyError::RangeOutsideManagedMemory);
        }

        Ok(kind)
    }

    fn page(&self, pfn: usize) -> Result<&Page, BuddyError> {
        let pointer = self.page_pointer(pfn)?;

        // SAFETY:
        // pointer 位于初始化完成的 metadata 数组中。
        Ok(unsafe { &*pointer })
    }

    fn page_mut(&mut self, pfn: usize) -> Result<&mut Page, BuddyError> {
        let pointer = self.page_pointer(pfn)?;

        // SAFETY:
        // &mut self 保证此次修改期间 allocator 独占。
        Ok(unsafe { &mut *pointer })
    }

    fn page_pointer(&self, pfn: usize) -> Result<*mut Page, BuddyError> {
        let index = pfn
            .checked_sub(self.base_pfn)
            .ok_or(BuddyError::RangeOutsideManagedMemory)?;

        if index >= self.page_count {
            return Err(BuddyError::RangeOutsideManagedMemory);
        }

        // SAFETY:
        // index 已验证位于 metadata 数组内。
        Ok(unsafe { self.metadata.as_ptr().add(index) })
    }
}

// SAFETY: metadata 指针指向永久页元数据；并发访问由外部锁保证。
unsafe impl Send for BuddyAllocator {}

fn largest_block_order(pfn: usize, remaining_pages: usize) -> usize {
    debug_assert!(remaining_pages != 0);

    let alignment_order = pfn.trailing_zeros() as usize;

    let size_order = (usize::BITS - 1 - remaining_pages.leading_zeros()) as usize;

    min(MAX_ORDER - 1, min(alignment_order, size_order))
}

fn frame_from_pfn(pfn: usize) -> Result<PhysFrame, BuddyError> {
    let address = pfn
        .checked_mul(PAGE_SIZE)
        .ok_or(BuddyError::AddressOverflow)?;

    PhysFrame::from_start_address(PhysAddr::new(address)).ok_or(BuddyError::AddressOverflow)
}

#[cfg(test)]
mod tests {
    use core::mem::size_of_val;

    use super::*;

    fn allocator_with_pages<const PAGES: usize>() -> (BuddyAllocator, [Page; PAGES]) {
        let mut metadata = [const { Page::absent() }; PAGES];

        let managed = PhysRange::from_start_size(PhysAddr::new(0), PAGES * PAGE_SIZE).unwrap();

        // SAFETY: metadata 是测试私有数组，大小由 PAGES 保证覆盖 managed 页数。
        let mut allocator = unsafe {
            BuddyAllocator::new(
                metadata.as_mut_ptr().cast::<u8>(),
                size_of_val(&metadata),
                managed,
            )
        }
        .unwrap();

        allocator.mark_present_range(managed).unwrap();
        allocator.release_range(managed).unwrap();

        (allocator, metadata)
    }

    #[test]
    fn page_reference_count_tracks_shared_refs() {
        let (mut allocator, _metadata) = allocator_with_pages::<16>();

        let allocation = allocator.allocate(0, AllocationClass::Any).unwrap();
        let frame = allocation.start();

        assert_eq!(allocator.reference_count(frame).unwrap(), 1);
        assert_eq!(allocator.increment_reference(frame).unwrap(), 2);
        assert_eq!(allocator.decrement_reference(frame).unwrap(), 1);

        allocator.free(allocation).unwrap();
    }

    #[test]
    fn unreferenced_single_frame_can_return_to_buddy() {
        let (mut allocator, _metadata) = allocator_with_pages::<16>();

        let before = allocator.total_free_pages();
        let allocation = allocator.allocate(0, AllocationClass::Any).unwrap();
        let frame = allocation.start();

        assert_eq!(allocator.decrement_reference(frame).unwrap(), 0);

        allocator.free_unreferenced_frame(frame).unwrap();

        assert_eq!(allocator.total_free_pages(), before);
    }
}
