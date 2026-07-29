#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default)]
pub struct Context {
    ra: usize,
    sp: usize,
    fp: usize,
    s0: usize,
    s1: usize,
    s2: usize,
    s3: usize,
    s4: usize,
    s5: usize,
    s6: usize,
    s7: usize,
    s8: usize,

    // LoongArch FP/LSX state. The 128-bit vector registers extend the scalar
    // floating-point register file, so one backing image covers both modes.
    fp_vector: [[u64; 2]; 32],
    fcsr0: usize,
    fcc: [usize; 8],
}

/// Bytes reserved below the end-exclusive kernel-stack boundary before a
/// fresh task can be published.
pub const FRESH_TASK_STACK_RESERVE: usize = 512;

const _: () = {
    assert!(FRESH_TASK_STACK_RESERVE >= 512);
    assert!(FRESH_TASK_STACK_RESERVE.is_multiple_of(16));
};

unsafe extern "C" {
    fn __loongarch_fresh_context_entry() -> !;
}

impl Context {
    pub fn new(initial_sp: usize, entry: unsafe extern "C" fn() -> !) -> Self {
        assert_eq!(
            initial_sp & 0xf,
            0,
            "fresh task initial SP is not ABI aligned"
        );
        Self {
            ra: __loongarch_fresh_context_entry as *const () as usize,
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
    assert!(core::mem::size_of::<Context>() == 688);
    assert!(core::mem::align_of::<Context>() == 16);
};
