use myos_mm::{
    EarlyFrameAllocator, EarlyFrameAllocatorError, MappingOptions, PAGE_SIZE, PageTableAccessError,
    PhysAddr, PhysFrame, VirtAddr, VirtPage,
};

use super::{BootPageTable, LEVELS, PageTable, PageTableEntry, PageTableEntryError, indices};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MapPageError {
    InvalidVirtualAddress,

    FrameAllocator(EarlyFrameAllocatorError),

    Entry(PageTableEntryError),

    TableAccess(PageTableAccessError),

    AlreadyMapped,

    LeafWhereTableExpected { level: usize },

    InvalidTableEntry { level: usize },
}

impl From<EarlyFrameAllocatorError> for MapPageError {
    fn from(error: EarlyFrameAllocatorError) -> Self {
        Self::FrameAllocator(error)
    }
}

impl From<PageTableEntryError> for MapPageError {
    fn from(error: PageTableEntryError) -> Self {
        Self::Entry(error)
    }
}

impl From<PageTableAccessError> for MapPageError {
    fn from(error: PageTableAccessError) -> Self {
        Self::TableAccess(error)
    }
}

impl BootPageTable {
    pub fn map_page<const CAPACITY: usize>(
        &mut self,
        allocator: &mut EarlyFrameAllocator<CAPACITY>,
        page: VirtPage,
        frame: PhysFrame,
        options: MappingOptions,
    ) -> Result<(), MapPageError> {
        let page_indices =
            indices(page.start_address()).ok_or(MapPageError::InvalidVirtualAddress)?;

        let leaf = PageTableEntry::leaf(frame, options)?;

        let mut current = self.root;

        for level in 0..LEVELS - 1 {
            let raw = read_entry(current, page_indices[level].get())?;

            if raw == 0 {
                return self.install_missing_chain(allocator, current, level, page_indices, leaf);
            }

            let entry = PageTableEntry::from_raw(raw);

            if entry.is_leaf() {
                return Err(MapPageError::LeafWhereTableExpected { level });
            }

            if !entry.is_table() {
                return Err(MapPageError::InvalidTableEntry { level });
            }

            current = entry
                .frame()
                .ok_or(MapPageError::InvalidTableEntry { level })?;
        }

        let leaf_index = page_indices[LEVELS - 1].get();

        if read_entry(current, leaf_index)? != 0 {
            return Err(MapPageError::AlreadyMapped);
        }

        write_entry(current, leaf_index, leaf.raw())?;

        Ok(())
    }

    fn install_missing_chain<const CAPACITY: usize>(
        &mut self,
        allocator: &mut EarlyFrameAllocator<CAPACITY>,
        parent: PhysFrame,
        missing_level: usize,
        page_indices: [myos_mm::PageTableIndex; LEVELS],
        leaf: PageTableEntry,
    ) -> Result<(), MapPageError> {
        let checkpoint = allocator.checkpoint();

        let result = self.build_missing_chain(allocator, parent, missing_level, page_indices, leaf);

        if result.is_err() {
            allocator.restore(checkpoint);
        }

        result
    }

    fn build_missing_chain<const CAPACITY: usize>(
        &mut self,
        allocator: &mut EarlyFrameAllocator<CAPACITY>,
        parent: PhysFrame,
        missing_level: usize,
        page_indices: [myos_mm::PageTableIndex; LEVELS],
        leaf: PageTableEntry,
    ) -> Result<(), MapPageError> {
        /*
         * Sv39 最多需要两张新表：
         *
         * root -> level 1 -> level 2
         */
        let required = LEVELS - 1 - missing_level;

        let mut tables: [Option<PhysFrame>; 2] = [None; 2];

        for slot in tables.iter_mut().take(required) {
            let frame = allocator.allocate_frame()?;
            initialize_zero_table(frame);
            *slot = Some(frame);
        }

        let leaf_table = tables[required - 1].expect("required table is missing");

        write_entry(leaf_table, page_indices[LEVELS - 1].get(), leaf.raw())?;

        /*
         * 从最深层向上连接新表，但尚不发布到原有页表。
         */
        for index in (0..required - 1).rev() {
            let current = tables[index].expect("new table is missing");

            let child = tables[index + 1].expect("new child table is missing");

            let entry = PageTableEntry::table(child)?;

            let table_level = missing_level + 1 + index;

            write_entry(current, page_indices[table_level].get(), entry.raw())?;
        }

        let first = tables[0].expect("first new table is missing");

        let first_entry = PageTableEntry::table(first)?;

        /*
         * 最后一步才发布到已有页表。
         *
         * 在此之前失败不会留下可达的半成品页表。
         */
        write_entry(parent, page_indices[missing_level].get(), first_entry.raw())?;

        self.allocated_table_pages += required;

        Ok(())
    }

    pub fn translate(&self, address: VirtAddr) -> Result<Option<PhysAddr>, MapPageError> {
        let page_indices = indices(address).ok_or(MapPageError::InvalidVirtualAddress)?;

        let mut current = self.root;

        for (level, page_index) in page_indices.iter().enumerate().take(LEVELS) {
            let raw = read_entry(current, page_index.get())?;

            if raw == 0 {
                return Ok(None);
            }

            let entry = PageTableEntry::from_raw(raw);

            if level == LEVELS - 1 {
                if !entry.is_leaf() {
                    return Err(MapPageError::InvalidTableEntry { level });
                }

                let offset = address.get() & (PAGE_SIZE - 1);

                let physical = entry
                    .physical_address()
                    .checked_add(offset)
                    .ok_or(MapPageError::InvalidTableEntry { level })?;

                return Ok(Some(physical));
            }

            if entry.is_leaf() {
                return Err(MapPageError::LeafWhereTableExpected { level });
            }

            if !entry.is_table() {
                return Err(MapPageError::InvalidTableEntry { level });
            }

            current = entry
                .frame()
                .ok_or(MapPageError::InvalidTableEntry { level })?;
        }

        Ok(None)
    }
}

fn initialize_zero_table(frame: PhysFrame) {
    let pointer = crate::memory::phys_access::ram_mut_ptr::<PageTable>(frame.start_address())
        .expect("allocated page-table frame is not accessible");

    // SAFETY:
    // 页面来自独占的启动页帧分配器，尚未发布。
    unsafe {
        pointer.write(PageTable::zeroed());
    }
}

fn read_entry(frame: PhysFrame, index: usize) -> Result<u64, PageTableAccessError> {
    let pointer = crate::memory::phys_access::ram_ptr::<PageTable>(frame.start_address())
        .expect("page-table frame is not accessible");

    // SAFETY:
    // 页表页面由 BootPageTable 独占，并在当前地址模式下可访问。
    unsafe { (&*pointer).entry(index) }
}

fn write_entry(frame: PhysFrame, index: usize, value: u64) -> Result<(), PageTableAccessError> {
    let pointer = crate::memory::phys_access::ram_mut_ptr::<PageTable>(frame.start_address())
        .expect("allocated page-table frame is not accessible");

    // SAFETY:
    // 页表尚未发布给硬件 walker，不存在并发访问。
    unsafe { (&mut *pointer).set_entry(index, value) }
}
