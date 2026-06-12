use alloc::vec::Vec;

use myos_mm::{MappingOptions, PhysAddr, PhysFrame, VirtAddr, VirtPage};

use crate::page_alloc::{self, GlobalPageAllocatorError, PageAllocationOptions};

#[derive(Debug)]
pub struct RuntimePageTable {
    inner: imp::RuntimePageTable,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimePageTableError {
    InvalidVirtualAddress,
    AlreadyMapped,
    NotMapped,
    LeafWhereTableExpected { level: usize },
    InvalidTableEntry { level: usize },
    PageAllocator(GlobalPageAllocatorError),
    PageTableEntry,
    PageTableAccess,
    MetadataOutOfMemory,
}

impl From<GlobalPageAllocatorError> for RuntimePageTableError {
    fn from(error: GlobalPageAllocatorError) -> Self {
        Self::PageAllocator(error)
    }
}

impl RuntimePageTable {
    pub fn from_boot(boot: crate::arch::memory::paging::BootPageTable) -> Self {
        Self {
            inner: imp::RuntimePageTable::from_boot(boot),
        }
    }

    pub fn map_page(
        &mut self,
        page: VirtPage,
        frame: PhysFrame,
        options: MappingOptions,
    ) -> Result<(), RuntimePageTableError> {
        self.inner.map_page(page, frame, options)
    }

    pub fn protect_page(
        &mut self,
        page: VirtPage,
        options: MappingOptions,
    ) -> Result<(), RuntimePageTableError> {
        self.inner.protect_page(page, options)
    }

    pub fn unmap_page(&mut self, page: VirtPage) -> Result<PhysFrame, RuntimePageTableError> {
        self.inner.unmap_page(page)
    }

    pub fn translate(&self, address: VirtAddr) -> Result<Option<PhysAddr>, RuntimePageTableError> {
        self.inner.translate(address)
    }

    pub fn allocated_runtime_tables(&self) -> usize {
        self.inner.allocated_runtime_tables()
    }
}

fn allocate_zeroed_table() -> Result<myos_mm::PageAllocation, RuntimePageTableError> {
    Ok(page_alloc::allocate(
        0,
        PageAllocationOptions::kernel_zeroed(),
    )?)
}

fn free_table(allocation: myos_mm::PageAllocation) {
    page_alloc::free(allocation).expect("unable to roll back page-table allocation");
}

#[cfg(target_arch = "riscv64")]
mod imp {
    use super::*;
    use crate::arch::memory::paging::{
        LEVELS, PageTable, PageTableEntry, PageTableEntryError, indices,
    };

    const MAX_NEW_TABLES_PER_MAPPING: usize = LEVELS - 1;

    #[derive(Debug)]
    pub struct RuntimePageTable {
        root: PhysFrame,
        runtime_tables: Vec<myos_mm::PageAllocation>,
    }

    impl RuntimePageTable {
        pub fn from_boot(boot: crate::arch::memory::paging::BootPageTable) -> Self {
            Self {
                root: boot.root_frame(),
                runtime_tables: Vec::new(),
            }
        }

        pub fn allocated_runtime_tables(&self) -> usize {
            self.runtime_tables.len()
        }

        pub fn map_page(
            &mut self,
            page: VirtPage,
            frame: PhysFrame,
            options: MappingOptions,
        ) -> Result<(), RuntimePageTableError> {
            let page_indices = indices(page.start_address())
                .ok_or(RuntimePageTableError::InvalidVirtualAddress)?;

            let leaf = PageTableEntry::leaf(frame, options)
                .map_err(|_| RuntimePageTableError::PageTableEntry)?;

            let mut current = self.root;

            for level in 0..LEVELS - 1 {
                let raw = read_entry(current, page_indices[level].get())?;

                if raw == 0 {
                    return self.install_missing_chain(current, level, page_indices, leaf);
                }

                let entry = PageTableEntry::from_raw(raw);

                if entry.is_leaf() {
                    return Err(RuntimePageTableError::LeafWhereTableExpected { level });
                }

                if !entry.is_table() {
                    return Err(RuntimePageTableError::InvalidTableEntry { level });
                }

                current = entry
                    .frame()
                    .ok_or(RuntimePageTableError::InvalidTableEntry { level })?;
            }

            let leaf_index = page_indices[LEVELS - 1].get();

            if read_entry(current, leaf_index)? != 0 {
                return Err(RuntimePageTableError::AlreadyMapped);
            }

            write_entry(current, leaf_index, leaf.raw())?;
            flush_page(page.start_address());

            Ok(())
        }

        fn install_missing_chain(
            &mut self,
            parent: PhysFrame,
            missing_level: usize,
            page_indices: [myos_mm::PageTableIndex; LEVELS],
            leaf: PageTableEntry,
        ) -> Result<(), RuntimePageTableError> {
            let required = LEVELS - 1 - missing_level;

            self.runtime_tables
                .try_reserve(required)
                .map_err(|_| RuntimePageTableError::MetadataOutOfMemory)?;

            let mut allocations: [Option<myos_mm::PageAllocation>; MAX_NEW_TABLES_PER_MAPPING] =
                core::array::from_fn(|_| None);

            let mut frames: [Option<PhysFrame>; MAX_NEW_TABLES_PER_MAPPING] =
                core::array::from_fn(|_| None);

            let result = (|| {
                for index in 0..required {
                    let allocation = allocate_zeroed_table()?;
                    frames[index] = Some(allocation.start());
                    allocations[index] = Some(allocation);
                }

                let leaf_table = frames[required - 1].expect("leaf table is missing");

                write_entry(leaf_table, page_indices[LEVELS - 1].get(), leaf.raw())?;

                for index in (0..required - 1).rev() {
                    let current = frames[index].expect("new table is missing");
                    let child = frames[index + 1].expect("new child table is missing");
                    let table_level = missing_level + 1 + index;

                    let entry = PageTableEntry::table(child)
                        .map_err(|_| RuntimePageTableError::PageTableEntry)?;

                    write_entry(current, page_indices[table_level].get(), entry.raw())?;
                }

                let first = frames[0].expect("first new table is missing");

                let first_entry = PageTableEntry::table(first)
                    .map_err(|_| RuntimePageTableError::PageTableEntry)?;

                write_entry(parent, page_indices[missing_level].get(), first_entry.raw())?;

                Ok(())
            })();

            if let Err(error) = result {
                for allocation in allocations.into_iter().flatten() {
                    free_table(allocation);
                }

                return Err(error);
            }

            for allocation in allocations.into_iter().take(required).flatten() {
                self.runtime_tables.push(allocation);
            }

            flush_all();

            Ok(())
        }

        pub fn protect_page(
            &mut self,
            page: VirtPage,
            options: MappingOptions,
        ) -> Result<(), RuntimePageTableError> {
            let (leaf_table, leaf_index, old) = self.leaf_entry(page)?;

            let frame = old
                .frame()
                .ok_or(RuntimePageTableError::InvalidTableEntry { level: LEVELS - 1 })?;

            let new = PageTableEntry::leaf(frame, options)
                .map_err(|_| RuntimePageTableError::PageTableEntry)?;

            write_entry(leaf_table, leaf_index, new.raw())?;
            flush_page(page.start_address());

            Ok(())
        }

        pub fn unmap_page(&mut self, page: VirtPage) -> Result<PhysFrame, RuntimePageTableError> {
            let (leaf_table, leaf_index, old) = self.leaf_entry(page)?;

            let frame = old
                .frame()
                .ok_or(RuntimePageTableError::InvalidTableEntry { level: LEVELS - 1 })?;

            write_entry(leaf_table, leaf_index, 0)?;
            flush_page(page.start_address());

            Ok(frame)
        }

        fn leaf_entry(
            &self,
            page: VirtPage,
        ) -> Result<(PhysFrame, usize, PageTableEntry), RuntimePageTableError> {
            let page_indices = indices(page.start_address())
                .ok_or(RuntimePageTableError::InvalidVirtualAddress)?;

            let mut current = self.root;

            for (level, page_index) in page_indices.iter().enumerate().take(LEVELS - 1) {
                let raw = read_entry(current, page_index.get())?;

                if raw == 0 {
                    return Err(RuntimePageTableError::NotMapped);
                }

                let entry = PageTableEntry::from_raw(raw);

                if entry.is_leaf() {
                    return Err(RuntimePageTableError::LeafWhereTableExpected { level });
                }

                if !entry.is_table() {
                    return Err(RuntimePageTableError::InvalidTableEntry { level });
                }

                current = entry
                    .frame()
                    .ok_or(RuntimePageTableError::InvalidTableEntry { level })?;
            }

            let leaf_index = page_indices[LEVELS - 1].get();
            let raw = read_entry(current, leaf_index)?;

            if raw == 0 {
                return Err(RuntimePageTableError::NotMapped);
            }

            let entry = PageTableEntry::from_raw(raw);

            if !entry.is_leaf() {
                return Err(RuntimePageTableError::InvalidTableEntry { level: LEVELS - 1 });
            }

            Ok((current, leaf_index, entry))
        }

        pub fn translate(
            &self,
            address: VirtAddr,
        ) -> Result<Option<PhysAddr>, RuntimePageTableError> {
            let page_indices =
                indices(address).ok_or(RuntimePageTableError::InvalidVirtualAddress)?;

            let mut current = self.root;

            for (level, page_index) in page_indices.iter().enumerate().take(LEVELS) {
                let raw = read_entry(current, page_index.get())?;

                if raw == 0 {
                    return Ok(None);
                }

                let entry = PageTableEntry::from_raw(raw);

                if level == LEVELS - 1 {
                    if !entry.is_leaf() {
                        return Err(RuntimePageTableError::InvalidTableEntry { level });
                    }

                    let offset = address.get() & (myos_mm::PAGE_SIZE - 1);

                    return Ok(entry.physical_address().checked_add(offset));
                }

                if entry.is_leaf() {
                    return Err(RuntimePageTableError::LeafWhereTableExpected { level });
                }

                if !entry.is_table() {
                    return Err(RuntimePageTableError::InvalidTableEntry { level });
                }

                current = entry
                    .frame()
                    .ok_or(RuntimePageTableError::InvalidTableEntry { level })?;
            }

            Ok(None)
        }
    }

    fn read_entry(frame: PhysFrame, index: usize) -> Result<u64, RuntimePageTableError> {
        let pointer = crate::arch::memory::phys_access::ram_ptr::<PageTable>(frame.start_address())
            .map_err(|_| RuntimePageTableError::PageTableAccess)?;

        // SAFETY: frame 指向已初始化页表页，读取单个表项不创建可变别名。
        unsafe { (&*pointer).entry(index) }.map_err(|_| RuntimePageTableError::PageTableAccess)
    }

    fn write_entry(
        frame: PhysFrame,
        index: usize,
        value: u64,
    ) -> Result<(), RuntimePageTableError> {
        let pointer =
            crate::arch::memory::phys_access::ram_mut_ptr::<PageTable>(frame.start_address())
                .map_err(|_| RuntimePageTableError::PageTableAccess)?;

        // SAFETY: 调用者通过 runtime page-table lock 串行化页表修改。
        unsafe { (&mut *pointer).set_entry(index, value) }
            .map_err(|_| RuntimePageTableError::PageTableAccess)
    }

    fn flush_page(address: VirtAddr) {
        // SAFETY: sfence.vma 只刷新本 hart TLB，不访问 Rust 管理内存。
        unsafe {
            core::arch::asm!(
                "sfence.vma {address}, zero",
                address = in(reg) address.get(),
                options(nostack),
            );
        }
    }

    fn flush_all() {
        // SAFETY: sfence.vma 只刷新本 hart TLB，不访问 Rust 管理内存。
        unsafe {
            core::arch::asm!("sfence.vma zero, zero", options(nostack));
        }
    }

    impl From<PageTableEntryError> for RuntimePageTableError {
        fn from(_: PageTableEntryError) -> Self {
            Self::PageTableEntry
        }
    }
}

#[cfg(target_arch = "loongarch64")]
mod imp {
    use super::*;
    use crate::arch::memory::paging::{
        LEVELS, LeafPageTableEntry, PageTable, TablePointerEntry, indices,
    };

    const MAX_NEW_TABLES_PER_MAPPING: usize = LEVELS - 1;

    #[derive(Debug)]
    pub struct RuntimePageTable {
        root: PhysFrame,
        invalid_pud: PhysFrame,
        invalid_pmd: PhysFrame,
        invalid_pte: PhysFrame,
        runtime_tables: Vec<myos_mm::PageAllocation>,
    }

    impl RuntimePageTable {
        pub fn from_boot(boot: crate::arch::memory::paging::BootPageTable) -> Self {
            Self {
                root: boot.root_frame(),
                invalid_pud: boot.invalid_pud_frame(),
                invalid_pmd: boot.invalid_pmd_frame(),
                invalid_pte: boot.invalid_pte_frame(),
                runtime_tables: Vec::new(),
            }
        }

        pub fn allocated_runtime_tables(&self) -> usize {
            self.runtime_tables.len()
        }

        pub fn map_page(
            &mut self,
            page: VirtPage,
            frame: PhysFrame,
            options: MappingOptions,
        ) -> Result<(), RuntimePageTableError> {
            let page_indices = indices(page.start_address())
                .ok_or(RuntimePageTableError::InvalidVirtualAddress)?;

            let leaf = LeafPageTableEntry::new(frame, options)
                .map_err(|_| RuntimePageTableError::PageTableEntry)?;

            let mut current = self.root;

            for level in 0..LEVELS - 1 {
                let raw = read_entry(current, page_indices[level].get())?;
                let pointer = TablePointerEntry::from_raw(raw);
                let next = pointer.next_table_frame();
                let invalid = self.invalid_child_for_level(level);

                if next == Some(invalid) {
                    return self.install_missing_chain(current, level, page_indices, leaf);
                }

                current = next.ok_or(RuntimePageTableError::InvalidTableEntry { level })?;
            }

            let leaf_index = page_indices[LEVELS - 1].get();
            let raw = read_entry(current, leaf_index)?;

            if raw != LeafPageTableEntry::invalid_global().raw() {
                return Err(RuntimePageTableError::AlreadyMapped);
            }

            write_entry(current, leaf_index, leaf.raw())?;
            flush_page(page.start_address());

            Ok(())
        }

        fn install_missing_chain(
            &mut self,
            parent: PhysFrame,
            missing_level: usize,
            page_indices: [myos_mm::PageTableIndex; LEVELS],
            leaf: LeafPageTableEntry,
        ) -> Result<(), RuntimePageTableError> {
            let required = LEVELS - 1 - missing_level;

            self.runtime_tables
                .try_reserve(required)
                .map_err(|_| RuntimePageTableError::MetadataOutOfMemory)?;

            let mut allocations: [Option<myos_mm::PageAllocation>; MAX_NEW_TABLES_PER_MAPPING] =
                core::array::from_fn(|_| None);

            let mut frames: [Option<PhysFrame>; MAX_NEW_TABLES_PER_MAPPING] =
                core::array::from_fn(|_| None);

            let result = (|| {
                for index in 0..required {
                    let allocation = allocate_zeroed_table()?;
                    let frame = allocation.start();
                    let table_level = missing_level + 1 + index;

                    initialize_table_for_level(
                        frame,
                        table_level,
                        self.invalid_pmd,
                        self.invalid_pte,
                    )?;

                    frames[index] = Some(frame);
                    allocations[index] = Some(allocation);
                }

                let leaf_table = frames[required - 1].expect("leaf table is missing");

                write_entry(leaf_table, page_indices[LEVELS - 1].get(), leaf.raw())?;

                for index in (0..required - 1).rev() {
                    let parent_table = frames[index].expect("new table is missing");
                    let child = frames[index + 1].expect("new child table is missing");
                    let table_level = missing_level + 1 + index;
                    let pointer = TablePointerEntry::new(child)
                        .map_err(|_| RuntimePageTableError::PageTableEntry)?;

                    write_entry(parent_table, page_indices[table_level].get(), pointer.raw())?;
                }

                let first = frames[0].expect("first table is missing");
                let first_pointer = TablePointerEntry::new(first)
                    .map_err(|_| RuntimePageTableError::PageTableEntry)?;

                write_entry(
                    parent,
                    page_indices[missing_level].get(),
                    first_pointer.raw(),
                )?;

                Ok(())
            })();

            if let Err(error) = result {
                for allocation in allocations.into_iter().flatten() {
                    free_table(allocation);
                }

                return Err(error);
            }

            for allocation in allocations.into_iter().take(required).flatten() {
                self.runtime_tables.push(allocation);
            }

            flush_all();

            Ok(())
        }

        fn invalid_child_for_level(&self, level: usize) -> PhysFrame {
            match level {
                0 => self.invalid_pud,
                1 => self.invalid_pmd,
                2 => self.invalid_pte,
                _ => panic!("invalid LoongArch page-table level"),
            }
        }

        pub fn protect_page(
            &mut self,
            page: VirtPage,
            options: MappingOptions,
        ) -> Result<(), RuntimePageTableError> {
            let (leaf_table, leaf_index, old) = self.leaf_entry(page)?;

            let frame = old
                .frame()
                .ok_or(RuntimePageTableError::InvalidTableEntry { level: LEVELS - 1 })?;

            let new = LeafPageTableEntry::new(frame, options)
                .map_err(|_| RuntimePageTableError::PageTableEntry)?;

            write_entry(leaf_table, leaf_index, new.raw())?;
            flush_page(page.start_address());

            Ok(())
        }

        pub fn unmap_page(&mut self, page: VirtPage) -> Result<PhysFrame, RuntimePageTableError> {
            let (leaf_table, leaf_index, old) = self.leaf_entry(page)?;

            let frame = old
                .frame()
                .ok_or(RuntimePageTableError::InvalidTableEntry { level: LEVELS - 1 })?;

            write_entry(
                leaf_table,
                leaf_index,
                LeafPageTableEntry::invalid_global().raw(),
            )?;
            flush_page(page.start_address());

            Ok(frame)
        }

        fn leaf_entry(
            &self,
            page: VirtPage,
        ) -> Result<(PhysFrame, usize, LeafPageTableEntry), RuntimePageTableError> {
            let page_indices = indices(page.start_address())
                .ok_or(RuntimePageTableError::InvalidVirtualAddress)?;

            let mut current = self.root;

            for (level, page_index) in page_indices.iter().enumerate().take(LEVELS - 1) {
                let raw = read_entry(current, page_index.get())?;
                let pointer = TablePointerEntry::from_raw(raw);
                let next = pointer.next_table_frame();

                if next == Some(self.invalid_child_for_level(level)) {
                    return Err(RuntimePageTableError::NotMapped);
                }

                current = next.ok_or(RuntimePageTableError::InvalidTableEntry { level })?;
            }

            let leaf_index = page_indices[LEVELS - 1].get();
            let raw = read_entry(current, leaf_index)?;
            let leaf = LeafPageTableEntry::from_raw(raw);

            if !leaf.is_present() {
                return Err(RuntimePageTableError::NotMapped);
            }

            Ok((current, leaf_index, leaf))
        }

        pub fn translate(
            &self,
            address: VirtAddr,
        ) -> Result<Option<PhysAddr>, RuntimePageTableError> {
            let page_indices =
                indices(address).ok_or(RuntimePageTableError::InvalidVirtualAddress)?;

            let mut current = self.root;

            for (level, page_index) in page_indices.iter().enumerate().take(LEVELS - 1) {
                let raw = read_entry(current, page_index.get())?;
                let pointer = TablePointerEntry::from_raw(raw);
                let next = pointer.next_table_frame();

                if next == Some(self.invalid_child_for_level(level)) {
                    return Ok(None);
                }

                current = next.ok_or(RuntimePageTableError::InvalidTableEntry { level })?;
            }

            let raw = read_entry(current, page_indices[LEVELS - 1].get())?;
            let leaf = LeafPageTableEntry::from_raw(raw);

            if !leaf.is_present() {
                return Ok(None);
            }

            let offset = address.get() & (myos_mm::PAGE_SIZE - 1);

            Ok(leaf.physical_address().checked_add(offset))
        }
    }

    fn initialize_table_for_level(
        frame: PhysFrame,
        table_level: usize,
        invalid_pmd: PhysFrame,
        invalid_pte: PhysFrame,
    ) -> Result<(), RuntimePageTableError> {
        let fill = match table_level {
            1 => TablePointerEntry::new(invalid_pmd)
                .map_err(|_| RuntimePageTableError::PageTableEntry)?
                .raw(),
            2 => TablePointerEntry::new(invalid_pte)
                .map_err(|_| RuntimePageTableError::PageTableEntry)?
                .raw(),
            3 => LeafPageTableEntry::invalid_global().raw(),
            _ => return Err(RuntimePageTableError::InvalidTableEntry { level: table_level }),
        };

        let pointer =
            crate::arch::memory::phys_access::ram_mut_ptr::<PageTable>(frame.start_address())
                .map_err(|_| RuntimePageTableError::PageTableAccess)?;

        // SAFETY: frame 是刚分配的独占页表页，尚未发布到任何上级目录。
        unsafe {
            pointer.write(PageTable::zeroed());
            (*pointer).fill(fill);
        }

        Ok(())
    }

    fn read_entry(frame: PhysFrame, index: usize) -> Result<u64, RuntimePageTableError> {
        let pointer = crate::arch::memory::phys_access::ram_ptr::<PageTable>(frame.start_address())
            .map_err(|_| RuntimePageTableError::PageTableAccess)?;

        // SAFETY: frame 指向已初始化页表页，读取单个表项不创建可变别名。
        unsafe { (&*pointer).entry(index) }.map_err(|_| RuntimePageTableError::PageTableAccess)
    }

    fn write_entry(
        frame: PhysFrame,
        index: usize,
        value: u64,
    ) -> Result<(), RuntimePageTableError> {
        let pointer =
            crate::arch::memory::phys_access::ram_mut_ptr::<PageTable>(frame.start_address())
                .map_err(|_| RuntimePageTableError::PageTableAccess)?;

        // SAFETY: 调用者通过 runtime page-table lock 串行化页表修改。
        unsafe { (&mut *pointer).set_entry(index, value) }
            .map_err(|_| RuntimePageTableError::PageTableAccess)
    }

    fn flush_page(_address: VirtAddr) {
        /*
         * 当前 LoongArch 内核仍主要依赖 DMW，高端页表映射尚未接入
         * TLB refill/页表寄存器。这里保留 flush 调用点，P1 后续接
         * 硬件页表时替换为 invtlb。
         */
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    }

    fn flush_all() {
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    }
}
