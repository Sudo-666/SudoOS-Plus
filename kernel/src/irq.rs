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

/*
 * 板级 (platform-visionfive2)：per-CPU supervisor-timer 中断计数。
 *
 * 通用 timer handler 在此累计每个逻辑 CPU 收到的 supervisor-timer IRQ，
 * 供启动后的 CPU-COUNTERS 真机检查确认 boot CPU 的定时器中断真正进入
 * 处理器。副核进入 tickless NO_HZ idle 后本地硬件定时器被 shutdown，
 * 其计数恒为 0（有意设计，与 ls2k1000 同款检查一致）；只有 boot CPU
 * 的计数在检查窗口内必然增长。仅 visionfive2 平台读取，其余平台不受影响。
 */
#[cfg(feature = "platform-visionfive2")]
static TIMER_IRQ_COUNT: [core::sync::atomic::AtomicU64; crate::smp::MAX_CPUS] =
    [const { core::sync::atomic::AtomicU64::new(0) }; crate::smp::MAX_CPUS];

#[cfg(feature = "platform-visionfive2")]
pub fn timer_irq_count(cpu: usize) -> u64 {
    TIMER_IRQ_COUNT[cpu].load(core::sync::atomic::Ordering::Relaxed)
}

pub fn handle_timer_interrupt() {
    #[cfg(feature = "platform-visionfive2")]
    TIMER_IRQ_COUNT[crate::smp::current_cpu_id().get()]
        .fetch_add(1, core::sync::atomic::Ordering::Relaxed);

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
