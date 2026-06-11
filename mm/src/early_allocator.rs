use crate::{FrameBlock, MemoryMap, PAGE_SIZE, PhysFrame, PhysRange};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EarlyFrameAllocatorError {
    ZeroPageCount,
    SizeOverflow,
    OutOfMemory,
}

/// 启动阶段物理页分配器。
///
/// 设计约束：
///
/// - 不依赖动态内存；
/// - 不支持释放；
/// - 只消费已经规范化的 MemoryMap；
/// - 从高地址向低地址分配；
/// - 不改变原始 BootMemoryMap；
/// - 所有分配天然按 PAGE_SIZE 对齐。
///
/// 从高地址分配可以尽量保留低端内存，方便以后处理具有
/// DMA 地址限制的设备。
#[derive(Clone, Copy, Debug)]
pub struct EarlyFrameAllocator<const CAPACITY: usize> {
    free: [Option<PhysRange>; CAPACITY],
}

impl<const CAPACITY: usize> EarlyFrameAllocator<CAPACITY> {
    pub fn from_memory_map(map: &MemoryMap<CAPACITY>) -> Self {
        let mut free = [None; CAPACITY];

        for (slot, range) in free.iter_mut().zip(map.iter()) {
            debug_assert!(range.is_page_aligned());
            *slot = Some(range);
        }

        Self { free }
    }

    pub fn allocate_frame(&mut self) -> Result<PhysFrame, EarlyFrameAllocatorError> {
        Ok(self.allocate_contiguous(1)?.start())
    }

    pub fn allocate_contiguous(
        &mut self,
        page_count: usize,
    ) -> Result<FrameBlock, EarlyFrameAllocatorError> {
        if page_count == 0 {
            return Err(EarlyFrameAllocatorError::ZeroPageCount);
        }

        let size = page_count
            .checked_mul(PAGE_SIZE)
            .ok_or(EarlyFrameAllocatorError::SizeOverflow)?;

        /*
         * 从最高地址的范围开始分配。
         */
        for index in (0..CAPACITY).rev() {
            let Some(current) = self.free[index] else {
                continue;
            };

            if current.size() < size {
                continue;
            }

            let allocation_start = current
                .end()
                .checked_sub(size)
                .ok_or(EarlyFrameAllocatorError::SizeOverflow)?;

            /*
             * MemoryMap 的所有范围已经页对齐，
             * 页数乘 PAGE_SIZE 后仍然页对齐。
             */
            debug_assert!(allocation_start.is_aligned(PAGE_SIZE),);

            if allocation_start < current.start() {
                continue;
            }

            let frame = PhysFrame::from_start_address(allocation_start).expect(
                "page-aligned memory map produced \
                     an unaligned frame",
            );

            let block =
                FrameBlock::new(frame, page_count).expect("validated frame block became invalid");

            /*
             * 当前分配始终位于范围尾端，因此不需要拆成两个
             * free ranges，也不会产生额外容量需求。
             */
            self.free[index] =
                PhysRange::new(current.start(), allocation_start).filter(|range| !range.is_empty());

            return Ok(block);
        }

        Err(EarlyFrameAllocatorError::OutOfMemory)
    }

    pub fn remaining_bytes(&self) -> Option<usize> {
        self.free
            .iter()
            .flatten()
            .try_fold(0_usize, |total, range| total.checked_add(range.size()))
    }

    pub fn remaining_frames(&self) -> Option<usize> {
        self.remaining_bytes().map(|bytes| bytes / PAGE_SIZE)
    }

    pub fn free_ranges(&self) -> impl Iterator<Item = PhysRange> + '_ {
        self.free.iter().flatten().copied()
    }

    /// 保存当前分配状态。
    ///
    /// EarlyFrameAllocator 只包含固定容量数组，因此复制整个状态
    /// 是廉价且确定的。页表构造失败时可以恢复到此检查点。
    pub const fn checkpoint(&self) -> Self {
        *self
    }

    /// 恢复一个先前保存的分配状态。
    pub fn restore(&mut self, checkpoint: Self) {
        *self = checkpoint;
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        EarlyFrameAllocator, EarlyFrameAllocatorError, MemoryMap, PAGE_SIZE, PhysAddr, PhysRange,
    };

    fn range(start: usize, end: usize) -> PhysRange {
        PhysRange::new(PhysAddr::new(start), PhysAddr::new(end)).unwrap()
    }

    #[test]
    fn allocates_from_high_addresses() {
        let mut map = MemoryMap::<4>::new();

        map.add_usable(range(0x1000, 0x9000)).unwrap();

        let mut allocator = EarlyFrameAllocator::from_memory_map(&map);

        let frame = allocator.allocate_frame().unwrap();

        assert_eq!(frame.start_address().get(), 0x8000,);

        let second = allocator.allocate_frame().unwrap();

        assert_eq!(second.start_address().get(), 0x7000,);
    }

    #[test]
    fn allocates_contiguous_pages() {
        let mut map = MemoryMap::<4>::new();

        map.add_usable(range(0x1000, 0x11_000)).unwrap();

        let mut allocator = EarlyFrameAllocator::from_memory_map(&map);

        let block = allocator.allocate_contiguous(4).unwrap();

        assert_eq!(block.count(), 4);
        assert_eq!(block.start().start_address().get(), 0xd000,);
        assert_eq!(block.size(), 4 * PAGE_SIZE);
    }

    #[test]
    fn allocation_does_not_modify_source_map() {
        let mut map = MemoryMap::<2>::new();

        map.add_usable(range(0x1000, 0x5000)).unwrap();

        let original_total = map.total_bytes();

        let mut allocator = EarlyFrameAllocator::from_memory_map(&map);

        allocator.allocate_frame().unwrap();

        assert_eq!(map.total_bytes(), original_total,);
    }

    #[test]
    fn rejects_zero_pages() {
        let map = MemoryMap::<1>::new();

        let mut allocator = EarlyFrameAllocator::from_memory_map(&map);

        assert_eq!(
            allocator.allocate_contiguous(0),
            Err(EarlyFrameAllocatorError::ZeroPageCount,),
        );
    }

    #[test]
    fn reports_out_of_memory() {
        let mut map = MemoryMap::<1>::new();

        map.add_usable(range(0x1000, 0x2000)).unwrap();

        let mut allocator = EarlyFrameAllocator::from_memory_map(&map);

        allocator.allocate_frame().unwrap();

        assert_eq!(
            allocator.allocate_frame(),
            Err(EarlyFrameAllocatorError::OutOfMemory,),
        );
    }
}
