use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};

static PANIC_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

fn stop_current_cpu() -> ! {
    crate::arch::interrupt::disable();
    crate::arch::interrupt::mask_all_sources();

    loop {
        crate::arch::cpu::wait_for_interrupt();
    }
}

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    if PANIC_IN_PROGRESS.swap(true, Ordering::AcqRel) {
        // Never let a secondary panic contend for the console or continue to
        // service scheduler/TLB interrupts after another CPU owns the report.
        stop_current_cpu();
    }

    crate::arch::interrupt::disable();
    crate::arch::interrupt::mask_all_sources();

    crate::println!();
    crate::println!("================ KERNEL PANIC ================");
    crate::println!("{info}");
    crate::lockdep::dump_current_cpu();
    crate::println!("==============================================");

    stop_current_cpu()
}
