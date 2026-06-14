use myos_mm::{PAGE_SIZE, VirtRange};

const KERNEL_STACK_SIZE: usize = 16 * 1024;
const KERNEL_STACK_ALIGNMENT: usize = PAGE_SIZE;
// M6-B r3: architecture-owned fresh-task bootstrap reserve.
//
// The architecture owns the size of the prebuilt bootstrap area.  A fresh
// context is published only after its saved SP is inside mapped memory and a
// full bootstrap reserve separates it from the upper guard page.
const FRESH_CONTEXT_HEADROOM: usize = crate::arch::task::FRESH_TASK_STACK_RESERVE;

const _: () = {
    assert!(FRESH_CONTEXT_HEADROOM >= 512);
    assert!(FRESH_CONTEXT_HEADROOM % 16 == 0);
    assert!(FRESH_CONTEXT_HEADROOM < KERNEL_STACK_SIZE);
};

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

    pub(super) fn initial_stack_pointer(&self) -> usize {
        let top = self.usable.end().get();
        let initial_sp = top
            .checked_sub(FRESH_CONTEXT_HEADROOM)
            .expect("kernel stack is smaller than the architecture bootstrap reserve");

        assert_eq!(
            initial_sp & 0xf,
            0,
            "fresh kernel-thread SP is not ABI aligned",
        );
        assert!(
            self.contains(initial_sp),
            "fresh kernel-thread SP is outside the mapped usable stack",
        );
        assert_eq!(
            initial_sp.checked_add(FRESH_CONTEXT_HEADROOM),
            Some(top),
            "fresh kernel-thread bootstrap reserve arithmetic is inconsistent",
        );
        initial_sp
    }

    pub(super) const fn contains(&self, address: usize) -> bool {
        self.usable.contains(myos_mm::VirtAddr::new(address))
    }

    pub(super) fn upper_headroom(&self, address: usize) -> usize {
        assert!(
            self.contains(address),
            "kernel stack headroom requested for an unmapped address",
        );
        self.usable
            .end()
            .get()
            .checked_sub(address)
            .expect("kernel stack headroom arithmetic underflowed")
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
