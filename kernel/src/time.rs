use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const TICKS_PER_SECOND: u64 = 100;
const VERIFY_TICKS: u64 = 8;

static CLOCK_FREQUENCY_HZ: AtomicU64 = AtomicU64::new(0);
static TICK_PERIOD_CYCLES: AtomicU64 = AtomicU64::new(0);
static NEXT_DEADLINES: [AtomicU64; crate::smp::MAX_CPUS] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static TIMER_TICKS: [AtomicU64; crate::smp::MAX_CPUS] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static TIMER_RUNNING: [AtomicBool; crate::smp::MAX_CPUS] = [
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
];

pub fn initialize(firmware_frequency: Option<u64>) {
    assert!(
        crate::arch::interrupt::are_disabled(),
        "time initialization requires local interrupts to remain disabled",
    );

    let frequency = crate::arch::time::frequency_hz(firmware_frequency)
        .unwrap_or_else(|error| panic!("unable to initialize clocksource: {error:?}"));
    let period = frequency
        .checked_div(TICKS_PER_SECOND)
        .filter(|period| *period != 0)
        .expect("clocksource frequency is too small for the configured tick rate");

    CLOCK_FREQUENCY_HZ.store(frequency, Ordering::Release);
    TICK_PERIOD_CYCLES.store(period, Ordering::Release);
    reset_current_clockevent();

    let first = crate::arch::time::counter();
    let second = crate::arch::time::counter();

    assert!(
        second >= first,
        "clocksource moved backwards during initialization: first={first} second={second}",
    );

    crate::println!("time subsystem:");
    crate::println!("  clocksource      : ready");
    crate::println!("  frequency        : {} Hz", frequency);
    crate::println!("  current counter  : {}", second);
    crate::println!("  monotonic ns     : {}", cycles_to_nanoseconds(second));
    crate::println!("  periodic timer   : not armed");
}

pub fn initialize_secondary() {
    assert!(
        crate::arch::interrupt::are_disabled(),
        "secondary time initialization requires local interrupts disabled",
    );
    assert!(
        CLOCK_FREQUENCY_HZ.load(Ordering::Acquire) != 0,
        "boot CPU did not publish the clocksource frequency",
    );

    reset_current_clockevent();
}

pub fn start_periodic() {
    arm_periodic_local();

    // SAFETY: trap entry, a kernel stack, timer acknowledgement, and all
    // currently enabled local sources are installed at this point.
    unsafe { crate::arch::interrupt::enable() };

    assert!(
        crate::arch::interrupt::are_enabled(),
        "local interrupts did not become enabled",
    );
}

pub fn arm_periodic_secondary() {
    arm_periodic_local();
}

fn arm_periodic_local() {
    assert!(
        crate::arch::interrupt::are_disabled(),
        "periodic timer must be published before enabling local interrupts",
    );

    let cpu = current_cpu_index();
    assert!(
        !TIMER_RUNNING[cpu].swap(true, Ordering::AcqRel),
        "periodic timer was started more than once on CPU {cpu}",
    );

    let period = tick_period_cycles();
    let now = crate::arch::time::counter();
    let deadline = now.wrapping_add(period);

    TIMER_TICKS[cpu].store(0, Ordering::Release);
    NEXT_DEADLINES[cpu].store(deadline, Ordering::Release);

    crate::arch::time::acknowledge();
    crate::arch::time::program_deadline(deadline)
        .unwrap_or_else(|error| panic!("unable to program first timer deadline: {error:?}"));
    crate::arch::time::enable_interrupt_source();

    assert!(
        crate::arch::time::interrupt_source_enabled(),
        "timer interrupt source did not become enabled",
    );
}

pub fn handle_timer_interrupt() {
    let cpu = current_cpu_index();

    assert!(
        TIMER_RUNNING[cpu].load(Ordering::Acquire),
        "timer interrupt arrived while CPU {cpu} clockevent was stopped",
    );

    let period = tick_period_cycles();
    let now = crate::arch::time::counter();
    let mut next = NEXT_DEADLINES[cpu].load(Ordering::Acquire);

    if !deadline_is_after(next, now) {
        let overdue = now.wrapping_sub(next);
        let periods_to_skip = overdue / period + 1;
        next = next.wrapping_add(period.wrapping_mul(periods_to_skip));
    }

    crate::arch::time::acknowledge();
    crate::arch::time::program_deadline(next)
        .unwrap_or_else(|error| panic!("unable to rearm timer interrupt: {error:?}"));

    NEXT_DEADLINES[cpu].store(next, Ordering::Release);
    TIMER_TICKS[cpu].fetch_add(1, Ordering::Release);
}

#[cfg(debug_assertions)]
pub fn verify_periodic() {
    let counter_before = crate::arch::time::counter();
    let target = timer_ticks()
        .checked_add(VERIFY_TICKS)
        .expect("timer verification target overflowed");

    while timer_ticks() < target {
        crate::arch::cpu::wait_for_interrupt();
    }

    let delivered = timer_ticks();
    let counter_after = crate::arch::time::counter();

    assert!(
        delivered >= target,
        "periodic timer delivered too few interrupts: delivered={delivered} target={target}",
    );
    assert!(
        counter_after > counter_before,
        "clocksource did not advance while waiting for timer interrupts",
    );
    assert!(
        crate::arch::interrupt::are_enabled(),
        "timer verification unexpectedly returned with interrupts disabled",
    );
    assert!(
        crate::arch::time::interrupt_source_enabled(),
        "timer verification unexpectedly returned with its source masked",
    );

    crate::println!("periodic timer test:");
    crate::println!("  clocksource      : verified");
    crate::println!("  timer interrupt  : verified ({} ticks)", delivered);
    crate::println!("  acknowledge      : verified");
    crate::println!("  rearm            : verified");
    crate::println!("  idle wakeup      : verified");
    crate::println!("  local interrupts : enabled");
    crate::println!("  periodic timer   : armed at {} Hz", TICKS_PER_SECOND);
}

pub fn timer_ticks() -> u64 {
    timer_ticks_for(crate::smp::current_cpu_id())
}

pub fn timer_ticks_for(cpu: crate::smp::CpuId) -> u64 {
    TIMER_TICKS[cpu.get()].load(Ordering::Acquire)
}

pub fn clock_frequency_hz() -> u64 {
    let frequency = CLOCK_FREQUENCY_HZ.load(Ordering::Acquire);
    assert!(frequency != 0, "clocksource has not been initialized");
    frequency
}

fn reset_current_clockevent() {
    crate::arch::time::shutdown()
        .unwrap_or_else(|error| panic!("unable to shut down local timer state: {error:?}"));

    let cpu = current_cpu_index();
    NEXT_DEADLINES[cpu].store(0, Ordering::Release);
    TIMER_TICKS[cpu].store(0, Ordering::Release);
    TIMER_RUNNING[cpu].store(false, Ordering::Release);
}

fn current_cpu_index() -> usize {
    crate::smp::current_cpu_id().get()
}

fn tick_period_cycles() -> u64 {
    let period = TICK_PERIOD_CYCLES.load(Ordering::Acquire);
    assert!(period != 0, "clockevent has not been initialized");
    period
}

fn cycles_to_nanoseconds(cycles: u64) -> u64 {
    let frequency = clock_frequency_hz();
    let nanoseconds = u128::from(cycles) * 1_000_000_000_u128 / u128::from(frequency);

    u64::try_from(nanoseconds).unwrap_or(u64::MAX)
}

fn deadline_is_after(deadline: u64, now: u64) -> bool {
    let distance = deadline.wrapping_sub(now);
    distance != 0 && distance < (1_u64 << 63)
}
