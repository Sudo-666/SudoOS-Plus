mod context;

pub use context::{Context, FRESH_TASK_STACK_RESERVE};

pub fn current_stack_pointer() -> usize {
    let stack_pointer: usize;
    // SAFETY: reading SP has no side effects and does not access memory.
    unsafe {
        core::arch::asm!(
            "ori {stack_pointer}, $r3, 0",
            stack_pointer = out(reg) stack_pointer,
            options(nomem, nostack),
        );
    }
    stack_pointer
}

/// Return address of the caller's caller: the first function frame executes
/// this helper's own `ret`, so $r1 still holds the caller's call site when the
/// asm block reads it. Used only to attribute blocked waiters to their block
/// call sites in scheduler diagnostics.
#[inline(never)]
pub fn return_address() -> usize {
    let return_address: usize;
    // SAFETY: reading $r1 has no side effects and does not access memory.
    unsafe {
        core::arch::asm!(
            "ori {return_address}, $r1, 0",
            return_address = out(reg) return_address,
            options(nomem, nostack),
        );
    }
    return_address
}

/// Switch from `previous` to `next` using the ordinary kernel-thread context.
///
/// # Safety
///
/// - both pointers must reference live, uniquely owned context records;
/// - `next` must contain a valid aligned stack pointer and return address;
/// - the caller must prevent the selected tasks from being run concurrently;
/// - local interrupts must be disabled across scheduler state publication and
///   the actual context switch.
#[inline]
pub unsafe fn switch(previous: *mut Context, next: *const Context) {
    // SAFETY: the caller establishes the context ownership and scheduling
    // invariants documented above.
    unsafe { __loongarch_switch_context(previous, next) }
}

unsafe extern "C" {
    fn __loongarch_switch_context(previous: *mut Context, next: *const Context);
}
