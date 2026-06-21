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

pub fn enter() {
    crate::task::irq_enter();
}

pub fn exit() {
    crate::task::irq_exit();
}

pub fn handle_timer_interrupt() {
    let event = crate::time::begin_timer_interrupt();
    let next_software_deadline = crate::timer::handle_interrupt(event.now());
    crate::time::reprogram_local(next_software_deadline);
    crate::task::on_timer_ticks(event.elapsed_ticks());
}

pub fn handle_software_interrupt() {
    crate::smp::handle_ipi();
}

pub fn handle_unhandled(source: InterruptSource, raw: usize) -> ! {
    panic!("unhandled interrupt: source={source:?} raw={raw:#x}");
}
