use myos_mm::{PAGE_SIZE, VirtRange};

const KERNEL_STACK_SIZE: usize = 16 * 1024;
const KERNEL_STACK_ALIGNMENT: usize = PAGE_SIZE;

pub struct KernelStack {
    allocation: Option<crate::vm::KernelVmAllocation>,
    usable: VirtRange,
}

impl KernelStack {
    pub fn allocate() -> Result<Self, crate::vm::KernelVmError> {
        let allocation = crate::vm::vmalloc(KERNEL_STACK_SIZE, KERNEL_STACK_ALIGNMENT)?;
        let usable = allocation.usable_range();

        assert_eq!(usable.size(), KERNEL_STACK_SIZE);
        assert!(usable.is_page_aligned());
        assert_eq!(usable.end().get() & 0xf, 0);

        Ok(Self {
            allocation: Some(allocation),
            usable,
        })
    }

    pub const fn top(&self) -> usize {
        self.usable.end().get()
    }

    pub const fn contains(&self, address: usize) -> bool {
        self.usable.contains(myos_mm::VirtAddr::new(address))
    }

    pub fn destroy(mut self) -> Result<(), crate::vm::KernelVmError> {
        let allocation = self
            .allocation
            .take()
            .expect("kernel stack allocation disappeared before destroy");

        crate::vm::vfree(allocation)
    }
}

impl Drop for KernelStack {
    fn drop(&mut self) {
        assert!(
            self.allocation.is_none(),
            "kernel stack dropped without explicit destroy",
        );
    }
}
