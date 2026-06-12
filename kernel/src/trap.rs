use core::sync::atomic::{AtomicUsize, Ordering};

static BREAKPOINT_COUNT: AtomicUsize = AtomicUsize::new(0);

pub fn initialize() {
    crate::arch::trap::initialize();

    crate::println!("trap subsystem:");
    crate::println!("  vector installed : yes");
    crate::println!("  interrupts       : disabled");
    crate::println!("  frame guard      : enabled");
}

#[cfg(debug_assertions)]
pub fn verify_breakpoint() {
    BREAKPOINT_COUNT.store(0, Ordering::Release);

    for expected in 1..=2 {
        crate::arch::trap::trigger_breakpoint();

        assert_eq!(
            BREAKPOINT_COUNT.load(Ordering::Acquire),
            expected,
            "breakpoint trap #{expected} was not delivered exactly once",
        );

        assert!(
            crate::arch::trap::kernel_scratch_is_clean(),
            "architecture trap scratch state was not restored after breakpoint #{expected}",
        );
    }

    assert!(
        crate::arch::trap::verify_register_restore(),
        "trap entry failed to restore register state",
    );

    assert_eq!(
        BREAKPOINT_COUNT.load(Ordering::Acquire),
        3,
        "register restore test did not traverse the breakpoint handler",
    );

    assert!(
        crate::arch::trap::kernel_scratch_is_clean(),
        "architecture trap scratch state was not restored after register self-test",
    );

    crate::println!("synchronous trap test:");
    crate::println!("  repeated entry   : verified (3 traps)");
    crate::println!("  frame alignment  : verified");
    crate::println!("  frame guard      : verified");
    crate::println!("  register restore : verified");
    crate::println!("  exception return : verified");
}

#[cfg(target_arch = "riscv64")]
#[unsafe(no_mangle)]
extern "C" fn kernel_arch_trap(frame: &mut crate::arch::trap::TrapFrame) {
    const BREAKPOINT: usize = 3;
    const INSTRUCTION_PAGE_FAULT: usize = 12;
    const LOAD_PAGE_FAULT: usize = 13;
    const STORE_PAGE_FAULT: usize = 15;
    const SUPERVISOR_SOFTWARE: usize = 1;
    const SUPERVISOR_TIMER: usize = 5;
    const SUPERVISOR_EXTERNAL: usize = 9;

    validate_trap_frame(frame);

    match (frame.is_interrupt(), frame.cause_code()) {
        (false, BREAKPOINT) => handle_breakpoint(frame),
        (false, INSTRUCTION_PAGE_FAULT) => {
            handle_riscv_page_fault(frame, myos_mm::FaultAccess::Execute)
        }
        (false, LOAD_PAGE_FAULT) => handle_riscv_page_fault(frame, myos_mm::FaultAccess::Read),
        (false, STORE_PAGE_FAULT) => handle_riscv_page_fault(frame, myos_mm::FaultAccess::Write),
        (true, SUPERVISOR_SOFTWARE) => crate::irq::handle_software_interrupt(),
        (true, SUPERVISOR_TIMER) => crate::irq::handle_timer_interrupt(),
        (true, SUPERVISOR_EXTERNAL) => {
            crate::irq::handle_unhandled(crate::irq::InterruptSource::External, frame.scause)
        }
        (true, code) => {
            crate::irq::handle_unhandled(crate::irq::InterruptSource::Unknown(code), frame.scause)
        }
        (false, code) => panic!(
            "unexpected RISC-V exception: sepc={:#x} scause={:#x} code={:#x} stval={:#x}",
            frame.sepc, frame.scause, code, frame.stval,
        ),
    }
}

#[cfg(target_arch = "loongarch64")]
#[unsafe(no_mangle)]
extern "C" fn kernel_arch_trap(frame: &mut crate::arch::trap::TrapFrame) {
    const ECODE_INTERRUPT: usize = 0x00;
    const ECODE_LOAD_PAGE_INVALID: usize = 0x01;
    const ECODE_STORE_PAGE_INVALID: usize = 0x02;
    const ECODE_FETCH_PAGE_INVALID: usize = 0x03;
    const ECODE_PAGE_MODIFIED: usize = 0x04;
    const ECODE_PAGE_NON_READABLE: usize = 0x05;
    const ECODE_PAGE_NON_EXECUTABLE: usize = 0x06;
    const ECODE_PAGE_PRIVILEGE: usize = 0x07;
    const ECODE_BREAKPOINT: usize = 0x0c;
    const TIMER_INTERRUPT_BIT: usize = 1 << 11;
    const IPI_INTERRUPT_BIT: usize = 1 << 12;
    const SUPPORTED_INTERRUPT_BITS: usize = TIMER_INTERRUPT_BIT | IPI_INTERRUPT_BIT;

    validate_trap_frame(frame);

    assert!(
        !frame.previous_mode_was_user(),
        "LoongArch user trap arrived before a per-task kernel stack protocol was installed",
    );

    match frame.exception_code() {
        ECODE_BREAKPOINT => handle_breakpoint(frame),
        ECODE_LOAD_PAGE_INVALID => {
            handle_loongarch_page_fault(frame, myos_mm::FaultAccess::Read, false)
        }
        ECODE_STORE_PAGE_INVALID => {
            handle_loongarch_page_fault(frame, myos_mm::FaultAccess::Write, false)
        }
        ECODE_FETCH_PAGE_INVALID => {
            handle_loongarch_page_fault(frame, myos_mm::FaultAccess::Execute, false)
        }
        ECODE_PAGE_MODIFIED => {
            handle_loongarch_page_fault(frame, myos_mm::FaultAccess::Write, true)
        }
        ECODE_PAGE_NON_READABLE => {
            handle_loongarch_page_fault(frame, myos_mm::FaultAccess::Read, true)
        }
        ECODE_PAGE_NON_EXECUTABLE => {
            handle_loongarch_page_fault(frame, myos_mm::FaultAccess::Execute, true)
        }
        ECODE_PAGE_PRIVILEGE => {
            handle_loongarch_page_fault(frame, myos_mm::FaultAccess::Read, true)
        }
        ECODE_INTERRUPT => {
            let pending = frame.pending_interrupts();
            let unknown = pending & !SUPPORTED_INTERRUPT_BITS;

            if unknown != 0 {
                crate::irq::handle_unhandled(
                    crate::irq::InterruptSource::Platform(unknown),
                    frame.estat,
                );
            }

            if pending & IPI_INTERRUPT_BIT != 0 {
                crate::irq::handle_software_interrupt();
            }
            if pending & TIMER_INTERRUPT_BIT != 0 {
                crate::irq::handle_timer_interrupt();
            }
        }
        code => panic!(
            "unexpected LoongArch exception: era={:#x} ecode={:#x} esubcode={:#x} badv={:#x} badi={:#x}",
            frame.era,
            code,
            frame.exception_subcode(),
            frame.badv,
            frame.badi,
        ),
    }
}

#[unsafe(no_mangle)]
extern "C" fn kernel_trap_frame_corrupted(frame: *const crate::arch::trap::TrapFrame) -> ! {
    panic!("trap frame guard was corrupted before exception return: frame={frame:p}");
}

fn validate_trap_frame(frame: &crate::arch::trap::TrapFrame) {
    let address = frame as *const crate::arch::trap::TrapFrame as usize;

    assert_eq!(
        address & 0xf,
        0,
        "trap frame is not 16-byte aligned: {address:#x}",
    );

    assert!(frame.guard_is_valid(), "trap frame guard is corrupted");
}

fn mark_breakpoint_reached() {
    BREAKPOINT_COUNT.fetch_add(1, Ordering::AcqRel);
}

#[cfg(target_arch = "riscv64")]
fn handle_riscv_page_fault(
    frame: &crate::arch::trap::TrapFrame,
    access: myos_mm::FaultAccess,
) -> ! {
    let source = if frame.previous_mode_was_user() {
        myos_mm::FaultSource::User
    } else {
        myos_mm::FaultSource::Kernel
    };

    let fault = myos_mm::PageFault::new(myos_mm::VirtAddr::new(frame.stval), access, source, false);

    crate::fault::handle_page_fault(fault, myos_mm::VirtAddr::new(frame.sepc), frame.scause)
}

#[cfg(target_arch = "loongarch64")]
fn handle_loongarch_page_fault(
    frame: &crate::arch::trap::TrapFrame,
    access: myos_mm::FaultAccess,
    present: bool,
) -> ! {
    let source = if frame.previous_mode_was_user() {
        myos_mm::FaultSource::User
    } else {
        myos_mm::FaultSource::Kernel
    };

    let fault =
        myos_mm::PageFault::new(myos_mm::VirtAddr::new(frame.badv), access, source, present);

    crate::fault::handle_page_fault(fault, myos_mm::VirtAddr::new(frame.era), frame.estat)
}

#[cfg(target_arch = "riscv64")]
fn handle_breakpoint(frame: &mut crate::arch::trap::TrapFrame) {
    mark_breakpoint_reached();
    frame.advance_pc(4);
}

#[cfg(target_arch = "loongarch64")]
fn handle_breakpoint(frame: &mut crate::arch::trap::TrapFrame) {
    mark_breakpoint_reached();
    frame.advance_pc(4);
}
