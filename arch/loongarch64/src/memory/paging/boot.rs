use myos_mm::{EarlyFrameAllocator, EarlyFrameAllocatorError, PhysFrame};

use super::{LeafPageTableEntry, PageTable, PageTableEntryError, TablePointerEntry};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootPageTableError {
    FrameAllocator(EarlyFrameAllocatorError),
    Entry(PageTableEntryError),
}

impl From<EarlyFrameAllocatorError> for BootPageTableError {
    fn from(error: EarlyFrameAllocatorError) -> Self {
        Self::FrameAllocator(error)
    }
}

impl From<PageTableEntryError> for BootPageTableError {
    fn from(error: PageTableEntryError) -> Self {
        Self::Entry(error)
    }
}

/// LoongArch 四级启动页表的物理存储。
///
/// 布局：
///
/// root PGD
///   → shared invalid PUD
///       → shared invalid PMD
///           → shared invalid PTE
///
/// 后续建立真实映射时，按需将对应目录项替换为新页表。
pub struct BootPageTable {
    pub(crate) root: PhysFrame,

    invalid_pud: PhysFrame,
    invalid_pmd: PhysFrame,
    invalid_pte: PhysFrame,
    pub(crate) allocated_table_pages: usize,
}

impl BootPageTable {
    pub fn new<const CAPACITY: usize>(
        allocator: &mut EarlyFrameAllocator<CAPACITY>,
    ) -> Result<Self, BootPageTableError> {
        let checkpoint = allocator.checkpoint();

        let result = Self::allocate(allocator);

        if result.is_err() {
            allocator.restore(checkpoint);
        }

        result
    }

    fn allocate<const CAPACITY: usize>(
        allocator: &mut EarlyFrameAllocator<CAPACITY>,
    ) -> Result<Self, BootPageTableError> {
        /*
         * 从叶层向根层建立，使每一层初始化时，
         * 下一层无效表已经存在。
         */
        let invalid_pte = allocator.allocate_frame()?;
        let invalid_pmd = allocator.allocate_frame()?;
        let invalid_pud = allocator.allocate_frame()?;
        let root = allocator.allocate_frame()?;

        initialize_leaf_table(invalid_pte, LeafPageTableEntry::invalid_global().raw());

        let invalid_pte_pointer = TablePointerEntry::new(invalid_pte)?.raw();

        initialize_directory_table(invalid_pmd, invalid_pte_pointer);

        let invalid_pmd_pointer = TablePointerEntry::new(invalid_pmd)?.raw();

        initialize_directory_table(invalid_pud, invalid_pmd_pointer);

        let invalid_pud_pointer = TablePointerEntry::new(invalid_pud)?.raw();

        initialize_directory_table(root, invalid_pud_pointer);

        Ok(Self {
            root,
            invalid_pud,
            invalid_pmd,
            invalid_pte,
            allocated_table_pages: 4,
        })
    }

    pub const fn root_frame(&self) -> PhysFrame {
        self.root
    }

    pub const fn invalid_pud_frame(&self) -> PhysFrame {
        self.invalid_pud
    }

    pub const fn invalid_pmd_frame(&self) -> PhysFrame {
        self.invalid_pmd
    }

    pub const fn invalid_pte_frame(&self) -> PhysFrame {
        self.invalid_pte
    }

    pub const fn allocated_table_pages(&self) -> usize {
        self.allocated_table_pages
    }
}

fn initialize_leaf_table(frame: PhysFrame, invalid_entry: u64) {
    initialize_filled_table(frame, invalid_entry);
}

fn initialize_directory_table(frame: PhysFrame, child_pointer: u64) {
    initialize_filled_table(frame, child_pointer);
}

fn initialize_filled_table(frame: PhysFrame, value: u64) {
    let pointer = crate::memory::phys_access::ram_mut_ptr::<PageTable>(frame.start_address())
        .expect("allocated page-table frame is not accessible");

    /*
     * SAFETY:
     *
     * - 当前尚未启用页表映射模式；
     * - 分配的物理 RAM 可由当前启动环境直接访问；
     * - 页面来自独占的启动分配器；
     * - 页面尚未被发布给硬件页表遍历器。
     */
    unsafe {
        pointer.write(PageTable::zeroed());
        (*pointer).fill(value);
    }
}
