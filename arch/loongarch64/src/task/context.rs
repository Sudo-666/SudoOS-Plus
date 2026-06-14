#[repr(C)]
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
}

unsafe extern "C" {
    fn __loongarch_fresh_context_entry() -> !;
}

impl Context {
    pub fn new(stack_top: usize, entry: unsafe extern "C" fn() -> !) -> Self {
        assert_eq!(stack_top & 0xf, 0, "fresh task stack is not ABI aligned");
        Self {
            ra: __loongarch_fresh_context_entry as *const () as usize,
            sp: stack_top,
            s0: entry as *const () as usize,
            ..Self::default()
        }
    }
}

const _: () = {
    assert!(core::mem::size_of::<Context>() == 12 * core::mem::size_of::<usize>());
    assert!(core::mem::align_of::<Context>() == core::mem::align_of::<usize>());
};
