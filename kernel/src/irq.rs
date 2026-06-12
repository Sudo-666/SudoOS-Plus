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

    crate::println!("irq subsystem:");
    crate::println!("  local interrupts: disabled");
    crate::println!("  dispatch policy  : fail-fast on unhandled irq");
}

pub fn handle_timer_interrupt() {
    let _source = InterruptSource::Timer;
    crate::time::record_timer_tick();
}

pub fn handle_unhandled(source: InterruptSource, raw: usize) -> ! {
    panic!("unhandled interrupt: source={source:?} raw={raw:#x}");
}
