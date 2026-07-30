#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Context {
    ra: usize,
    sp: usize,
    s0: usize,
    s1: usize,
    s2: usize,
    s3: usize,
    s4: usize,
    s5: usize,
    s6: usize,
    s7: usize,
    s8: usize,
    s9: usize,
    s10: usize,
    s11: usize,

    // The final-2026 glibc toolchain uses the hard-float lp64d ABI.  FP
    // registers are architectural task state even though the kernel itself is
    // built for riscv64imac and never uses floating point.  Carry all 32
    // registers plus FCSR across scheduler switches so independent rustc
    // processes and pthreads cannot corrupt one another after migration.
    fpr: [u64; 32],
    fcsr: usize,
}

/// Bytes reserved below the end-exclusive kernel-stack boundary before a
/// fresh task can be published.  This is architecture-owned so future entry
/// frames can grow without teaching generic task code about register layouts.
pub const FRESH_TASK_STACK_RESERVE: usize = 512;

const _: () = {
    assert!(FRESH_TASK_STACK_RESERVE >= 512);
    assert!(FRESH_TASK_STACK_RESERVE.is_multiple_of(16));
};

unsafe extern "C" {
    fn __riscv_fresh_context_entry() -> !;
}

impl Context {
    pub fn new(initial_sp: usize, entry: unsafe extern "C" fn() -> !) -> Self {
        assert_eq!(
            initial_sp & 0xf,
            0,
            "fresh task initial SP is not ABI aligned"
        );
        Self {
            ra: __riscv_fresh_context_entry as *const () as usize,
            // The task layer guarantees this already names mapped usable memory.
            // The assembly trampoline performs no stack memory access before Rust.
            sp: initial_sp,
            s0: entry as *const () as usize,
            ..Self::default()
        }
    }

    pub const fn saved_stack_pointer(&self) -> usize {
        self.sp
    }
}

const _: () = {
    assert!(core::mem::size_of::<Context>() == 47 * core::mem::size_of::<usize>());
    assert!(core::mem::align_of::<Context>() == core::mem::align_of::<usize>());
};
