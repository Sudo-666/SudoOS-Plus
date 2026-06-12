use myos_mm::{
    EarlyFrameAllocator, EarlyFrameAllocatorError, MappingOptions, PAGE_SIZE, PageTableAccessError,
    PhysAddr, PhysFrame, VirtAddr, VirtPage,
};

use super::{
    BootPageTable, LEVELS, LeafPageTableEntry, PageTable, PageTableEntryError, TablePointerEntry,
    indices,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MapPageError {
    InvalidVirtualAddress,

    FrameAllocator(EarlyFrameAllocatorError),

    Entry(PageTableEntryError),

    TableAccess(PageTableAccessError),

    AlreadyMapped,

    InvalidDirectoryEntry { level: usize },

    InvalidLeafEntry,
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

        let leaf = LeafPageTableEntry::new(frame, options)?;

        let mut current = self.root;

        for level in 0..LEVELS - 1 {
            let raw = read_entry(current, page_indices[level].get())?;

            let pointer = TablePointerEntry::from_raw(raw);

            let next = pointer.next_table_frame();

            let invalid = self.invalid_child_for_level(level);

            if next == Some(invalid) {
                return self.install_missing_chain(allocator, current, level, page_indices, leaf);
            }

            current = next.ok_or(MapPageError::InvalidDirectoryEntry { level })?;
        }

        let leaf_index = page_indices[LEVELS - 1].get();

        let raw = read_entry(current, leaf_index)?;

        if raw != LeafPageTableEntry::invalid_global().raw() {
            return Err(MapPageError::AlreadyMapped);
        }

        write_entry(current, leaf_index, leaf.raw())?;

        Ok(())
    }

    fn invalid_child_for_level(&self, level: usize) -> PhysFrame {
        match level {
            0 => self.invalid_pud_frame(),
            1 => self.invalid_pmd_frame(),
            2 => self.invalid_pte_frame(),
            _ => panic!("invalid LoongArch page-table level"),
        }
    }

    fn install_missing_chain<const CAPACITY: usize>(
        &mut self,
        allocator: &mut EarlyFrameAllocator<CAPACITY>,
        parent: PhysFrame,
        missing_level: usize,
        page_indices: [myos_mm::PageTableIndex; LEVELS],
        leaf: LeafPageTableEntry,
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
        leaf: LeafPageTableEntry,
    ) -> Result<(), MapPageError> {
        let required = LEVELS - 1 - missing_level;

        let mut tables: [Option<PhysFrame>; 3] = [None; 3];

        for (index, slot) in tables.iter_mut().enumerate().take(required) {
            let frame = allocator.allocate_frame()?;

            let table_level = missing_level + 1 + index;

            initialize_table_for_level(frame, table_level, self)?;

            *slot = Some(frame);
        }

        let leaf_table = tables[required - 1].expect("leaf table is missing");

        write_entry(leaf_table, page_indices[LEVELS - 1].get(), leaf.raw())?;

        for index in (0..required - 1).rev() {
            let parent_table = tables[index].expect("new table is missing");

            let child = tables[index + 1].expect("new child table is missing");

            let pointer = TablePointerEntry::new(child)?;

            let table_level = missing_level + 1 + index;

            write_entry(parent_table, page_indices[table_level].get(), pointer.raw())?;
        }

        let first = tables[0].expect("first table is missing");

        let first_pointer = TablePointerEntry::new(first)?;

        write_entry(
            parent,
            page_indices[missing_level].get(),
            first_pointer.raw(),
        )?;

        self.allocated_table_pages += required;

        Ok(())
    }

    pub fn translate(&self, address: VirtAddr) -> Result<Option<PhysAddr>, MapPageError> {
        let page_indices = indices(address).ok_or(MapPageError::InvalidVirtualAddress)?;

        let mut current = self.root;

        for (level, page_index) in page_indices.iter().enumerate().take(LEVELS - 1) {
            let raw = read_entry(current, page_index.get())?;

            let pointer = TablePointerEntry::from_raw(raw);

            let next = pointer.next_table_frame();

            if next == Some(self.invalid_child_for_level(level)) {
                return Ok(None);
            }

            current = next.ok_or(MapPageError::InvalidDirectoryEntry { level })?;
        }

        let raw = read_entry(current, page_indices[LEVELS - 1].get())?;

        let leaf = LeafPageTableEntry::from_raw(raw);

        if !leaf.is_present() {
            return Ok(None);
        }

        let offset = address.get() & (PAGE_SIZE - 1);

        let physical = leaf
            .physical_address()
            .checked_add(offset)
            .ok_or(MapPageError::InvalidLeafEntry)?;

        Ok(Some(physical))
    }
}

fn initialize_table_for_level(
    frame: PhysFrame,
    table_level: usize,
    boot: &BootPageTable,
) -> Result<(), MapPageError> {
    let fill = match table_level {
        1 => TablePointerEntry::new(boot.invalid_pmd_frame())?.raw(),

        2 => TablePointerEntry::new(boot.invalid_pte_frame())?.raw(),

        3 => LeafPageTableEntry::invalid_global().raw(),

        _ => {
            return Err(MapPageError::InvalidDirectoryEntry { level: table_level });
        }
    };

    let pointer = crate::memory::phys_access::ram_mut_ptr::<PageTable>(frame.start_address())
        .expect("allocated page-table frame is not accessible");

    // SAFETY:
    // 页面来自独占启动分配器，尚未发布。
    unsafe {
        pointer.write(PageTable::zeroed());
        (*pointer).fill(fill);
    }

    Ok(())
}

fn read_entry(frame: PhysFrame, index: usize) -> Result<u64, PageTableAccessError> {
    let pointer = crate::memory::phys_access::ram_ptr::<PageTable>(frame.start_address())
        .expect("page-table frame is not accessible");

    // SAFETY: 页表页面当前可由启动地址空间直接访问。
    unsafe { (&*pointer).entry(index) }
}

fn write_entry(frame: PhysFrame, index: usize, value: u64) -> Result<(), PageTableAccessError> {
    let pointer = crate::memory::phys_access::ram_mut_ptr::<PageTable>(frame.start_address())
        .expect("allocated page-table frame is not accessible");

    // SAFETY: 页表尚未发布，不存在并发硬件 walker。
    unsafe { (&mut *pointer).set_entry(index, value) }
}
