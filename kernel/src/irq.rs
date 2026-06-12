#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterruptSource {
    Timer,
    Software,
    External,
    Platform(usize),
    Unknown(usize),
}

pub fn initialize() {
    crate::arch::interrupt::disable();
    crate::arch::interrupt::mask_all_sources();

    crate::println!("irq subsystem:");
    crate::println!("  local interrupts: disabled");
    crate::println!("  local sources   : masked");
    crate::println!("  dispatch policy : fail-fast on unhandled irq");
}

pub fn initialize_secondary() {
    crate::arch::interrupt::disable();
    crate::arch::interrupt::mask_all_sources();
}

pub fn handle_timer_interrupt() {
    crate::time::handle_timer_interrupt();
}

pub fn handle_software_interrupt() {
    crate::smp::handle_ipi();
}

pub fn handle_unhandled(source: InterruptSource, raw: usize) -> ! {
    panic!("unhandled interrupt: source={source:?} raw={raw:#x}");
}
