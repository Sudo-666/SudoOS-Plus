use myos_mm::{EarlyFrameAllocator, EarlyFrameAllocatorError, PhysFrame};

use super::PageTable;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootPageTableError {
    FrameAllocator(EarlyFrameAllocatorError),
}

impl From<EarlyFrameAllocatorError> for BootPageTableError {
    fn from(error: EarlyFrameAllocatorError) -> Self {
        Self::FrameAllocator(error)
    }
}

/// 尚未发布到 SATP 的启动根页表。
///
/// 当前只表示页表物理存储，尚未包含任何映射。
pub struct BootPageTable {
    pub(crate) root: PhysFrame,
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
        let root = allocator.allocate_frame()?;

        initialize_zero_table(root);

        Ok(Self {
            root,
            allocated_table_pages: 1,
        })
    }

    pub const fn root_frame(&self) -> PhysFrame {
        self.root
    }

    pub const fn allocated_table_pages(&self) -> usize {
        self.allocated_table_pages
    }
}

fn initialize_zero_table(frame: PhysFrame) {
    let pointer = crate::memory::phys_access::ram_mut_ptr::<PageTable>(
        frame.start_address(),
    )
    .expect("allocated page-table frame is not accessible");

    /*
     * SAFETY:
     *
     * - frame 来自已经排除保留区的启动页帧分配器；
     * - 当前 RISC-V 尚未启用分页，物理 RAM 可直接访问；
     * - 该页面尚未发布，也不存在其他 Rust 引用；
     * - PageTable 大小和对齐均为一页。
     */
    unsafe {
        pointer.write(PageTable::zeroed());
    }
}
