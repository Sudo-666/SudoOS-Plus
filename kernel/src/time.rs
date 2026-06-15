// M6-B tickless scheduler-idle clockevent policy.
use core::{
    sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    time::Duration,
};

use crate::smp::MAX_CPUS;

const TICKS_PER_SECOND: u64 = 100;
const VERIFY_TICKS: u64 = 8;
const CLOCKEVENT_STOPPED: u8 = 0;
const CLOCKEVENT_RUNNING: u8 = 1;
const HALF_RANGE: u64 = 1_u64 << 63;
const MIN_CLOCKEVENT_DELTA_NS: u64 = 1_000;

static CLOCK_FREQUENCY_HZ: AtomicU64 = AtomicU64::new(0);
static TICK_PERIOD_CYCLES: AtomicU64 = AtomicU64::new(0);
static NEXT_SCHEDULER_DEADLINES: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static PROGRAMMED_CLOCKEVENT_DEADLINES: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static TIMER_TICKS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static CLOCKEVENT_MODES: [AtomicU8; MAX_CPUS] =
    [const { AtomicU8::new(CLOCKEVENT_STOPPED) }; MAX_CPUS];
static SCHEDULER_TICK_ACTIVE: [AtomicBool; MAX_CPUS] = [const { AtomicBool::new(false) }; MAX_CPUS];
static TICKLESS_IDLE_ENTRIES: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];

/// A point on the architecture's wrapping monotonic counter.
///
/// Deliberately does not implement `Ord`: wrapping counter order is only valid
/// inside a half-range window and must use the helpers below.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MonotonicInstant(u64);

impl MonotonicInstant {
    pub const fn from_cycles(cycles: u64) -> Self {
        Self(cycles)
    }

    pub const fn cycles(self) -> u64 {
        self.0
    }

    pub fn wrapping_add_cycles(self, cycles: u64) -> Self {
        assert!(cycles < HALF_RANGE, "monotonic interval exceeds half-range");
        Self(self.0.wrapping_add(cycles))
    }

    pub fn duration_since(self, earlier: Self) -> Duration {
        let cycles = self.0.wrapping_sub(earlier.0);
        assert!(cycles < HALF_RANGE, "monotonic duration exceeds half-range");
        cycles_to_duration(cycles)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TimerInterrupt {
    now: MonotonicInstant,
    elapsed_ticks: u64,
}

impl TimerInterrupt {
    pub const fn now(self) -> MonotonicInstant {
        self.now
    }

    pub const fn elapsed_ticks(self) -> u64 {
        self.elapsed_ticks
    }
}

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

    let first = now();
    let second = now();
    assert!(
        !instant_is_before(second, first),
        "clocksource moved backwards during initialization: first={} second={}",
        first.cycles(),
        second.cycles(),
    );

    crate::println!("time subsystem:");
    crate::println!("  clocksource      : ready");
    crate::println!("  frequency        : {} Hz", frequency);
    crate::println!("  current counter  : {}", second.cycles());
    crate::println!(
        "  monotonic ns     : {}",
        cycles_to_nanoseconds(second.cycles())
    );
    crate::println!("  clockevent       : one-shot");
    crate::println!("  scheduler tick   : not armed");
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
    arm_scheduler_tick_local();

    // SAFETY: trap entry, a kernel stack, timer acknowledgement, and every
    // enabled local source are installed before this point.
    unsafe { crate::arch::interrupt::enable() };
    assert!(
        crate::arch::interrupt::are_enabled(),
        "local interrupts did not become enabled",
    );
}

pub fn arm_periodic_secondary() {
    arm_scheduler_tick_local();
}

fn arm_scheduler_tick_local() {
    assert!(
        crate::arch::interrupt::are_disabled(),
        "scheduler tick must be published before enabling local interrupts",
    );

    let cpu = current_cpu_index();
    assert_eq!(
        CLOCKEVENT_MODES[cpu].swap(CLOCKEVENT_RUNNING, Ordering::AcqRel),
        CLOCKEVENT_STOPPED,
        "clockevent was started more than once on CPU {cpu}",
    );

    let deadline = now().wrapping_add_cycles(tick_period_cycles());
    TIMER_TICKS[cpu].store(0, Ordering::Release);
    NEXT_SCHEDULER_DEADLINES[cpu].store(deadline.cycles(), Ordering::Release);
    SCHEDULER_TICK_ACTIVE[cpu].store(true, Ordering::Release);

    crate::arch::time::acknowledge();
    reprogram_local(crate::timer::earliest_local());
    crate::arch::time::enable_interrupt_source();
    assert!(
        crate::arch::time::interrupt_source_enabled(),
        "timer interrupt source did not become enabled",
    );
}

/// Acknowledge a local timer IRQ and account scheduler-policy ticks.
///
/// The caller must run the software timer queue before calling
/// `reprogram_local`; this keeps callback execution outside the clockevent
/// implementation and gives one place ownership of the next hardware deadline.
pub fn begin_timer_interrupt() -> TimerInterrupt {
    let cpu = current_cpu_index();
    assert_eq!(
        CLOCKEVENT_MODES[cpu].load(Ordering::Acquire),
        CLOCKEVENT_RUNNING,
        "timer interrupt arrived while CPU {cpu} clockevent was stopped",
    );

    crate::arch::time::acknowledge();
    PROGRAMMED_CLOCKEVENT_DEADLINES[cpu].store(0, Ordering::Release);
    let current = now();
    let elapsed_ticks = if SCHEDULER_TICK_ACTIVE[cpu].load(Ordering::Acquire) {
        let period = tick_period_cycles();
        let scheduled =
            MonotonicInstant::from_cycles(NEXT_SCHEDULER_DEADLINES[cpu].load(Ordering::Acquire));

        if deadline_reached(current, scheduled) {
            let overdue = current.cycles().wrapping_sub(scheduled.cycles());
            let elapsed = overdue / period + 1;
            let advance = period
                .checked_mul(elapsed)
                .expect("scheduler deadline advance overflowed");
            let next = scheduled.wrapping_add_cycles(advance);
            NEXT_SCHEDULER_DEADLINES[cpu].store(next.cycles(), Ordering::Release);
            TIMER_TICKS[cpu].fetch_add(elapsed, Ordering::AcqRel);
            elapsed
        } else {
            0
        }
    } else {
        0
    };

    TimerInterrupt {
        now: current,
        elapsed_ticks,
    }
}

/// Program the local one-shot clockevent for the earlier of the scheduler tick
/// and the per-CPU software-timer deadline.
pub fn reprogram_local(software_deadline: Option<MonotonicInstant>) {
    assert!(
        crate::arch::interrupt::are_disabled(),
        "local clockevent reprogramming requires interrupts disabled",
    );

    let cpu = current_cpu_index();
    if CLOCKEVENT_MODES[cpu].load(Ordering::Acquire) != CLOCKEVENT_RUNNING {
        return;
    }

    let scheduler_deadline = SCHEDULER_TICK_ACTIVE[cpu].load(Ordering::Acquire).then(|| {
        let cycles = NEXT_SCHEDULER_DEADLINES[cpu].load(Ordering::Acquire);
        assert_ne!(
            cycles, 0,
            "active scheduler tick has no deadline on CPU {cpu}",
        );
        MonotonicInstant::from_cycles(cycles)
    });

    let chosen = match (scheduler_deadline, software_deadline) {
        (Some(scheduler), Some(software)) => earlier_deadline(scheduler, software),
        (Some(scheduler), None) => scheduler,
        (None, Some(software)) => software,
        (None, None) => {
            crate::arch::time::shutdown().unwrap_or_else(|error| {
                panic!("unable to stop deadline-free local clockevent: {error:?}")
            });
            PROGRAMMED_CLOCKEVENT_DEADLINES[cpu].store(0, Ordering::Release);
            return;
        }
    };

    let current = now();
    let minimum_delta = minimum_clockevent_delta_cycles();
    let distance = chosen.cycles().wrapping_sub(current.cycles());
    let safe = if distance < minimum_delta || distance >= HALF_RANGE {
        current.wrapping_add_cycles(minimum_delta)
    } else {
        chosen
    };

    program_deadline(safe);
    if !crate::arch::time::interrupt_source_enabled() {
        crate::arch::time::enable_interrupt_source();
    }
}

pub(crate) fn enter_idle() {
    assert!(
        crate::arch::interrupt::are_disabled(),
        "tickless idle entry requires local interrupts disabled",
    );
    let cpu = current_cpu_index();
    if CLOCKEVENT_MODES[cpu].load(Ordering::Acquire) != CLOCKEVENT_RUNNING {
        return;
    }
    if SCHEDULER_TICK_ACTIVE[cpu].swap(false, Ordering::AcqRel) {
        NEXT_SCHEDULER_DEADLINES[cpu].store(0, Ordering::Release);
        TICKLESS_IDLE_ENTRIES[cpu].fetch_add(1, Ordering::AcqRel);
    }
    reprogram_local(crate::timer::earliest_local());
}

pub(crate) fn leave_idle() {
    assert!(
        crate::arch::interrupt::are_disabled(),
        "tickless idle exit requires local interrupts disabled",
    );

    let cpu = current_cpu_index();
    if CLOCKEVENT_MODES[cpu].load(Ordering::Acquire) != CLOCKEVENT_RUNNING {
        return;
    }

    let current = now();
    let was_active = SCHEDULER_TICK_ACTIVE[cpu].swap(true, Ordering::AcqRel);
    let published_cycles = NEXT_SCHEDULER_DEADLINES[cpu].load(Ordering::Acquire);
    let scheduler_deadline = if !was_active || published_cycles == 0 {
        let deadline = current.wrapping_add_cycles(tick_period_cycles());
        NEXT_SCHEDULER_DEADLINES[cpu].store(deadline.cycles(), Ordering::Release);
        deadline
    } else {
        MonotonicInstant::from_cycles(published_cycles)
    };

    // The active bit is a level-triggered scheduler dependency; it is not
    // proof that the one-shot hardware still owns a future deadline. A fired,
    // shut-down, or superseded event must be re-established before a non-idle
    // task resumes. This mirrors Linux NO_HZ dependency re-evaluation.
    let programmed_cycles = PROGRAMMED_CLOCKEVENT_DEADLINES[cpu].load(Ordering::Acquire);
    let programmed_deadline = MonotonicInstant::from_cycles(programmed_cycles);
    let hardware_needs_rearm = programmed_cycles == 0
        || deadline_reached(current, programmed_deadline)
        || instant_is_before(scheduler_deadline, programmed_deadline)
        || !crate::arch::time::interrupt_source_enabled();

    if !was_active || hardware_needs_rearm {
        reprogram_local(crate::timer::earliest_local());
    }
}

pub fn now() -> MonotonicInstant {
    MonotonicInstant::from_cycles(crate::arch::time::counter())
}

pub fn deadline_after(duration: Duration) -> MonotonicInstant {
    now().wrapping_add_cycles(duration_to_cycles_round_up(duration))
}

pub fn clock_frequency_hz() -> u64 {
    let frequency = CLOCK_FREQUENCY_HZ.load(Ordering::Acquire);
    assert!(frequency != 0, "clocksource has not been initialized");
    frequency
}

pub fn duration_to_cycles_round_up(duration: Duration) -> u64 {
    let frequency = u128::from(clock_frequency_hz());
    let nanos = duration.as_nanos();
    let numerator = nanos
        .checked_mul(frequency)
        .expect("duration-to-cycle conversion overflowed");
    let cycles = numerator
        .checked_add(999_999_999)
        .expect("duration-to-cycle rounding overflowed")
        / 1_000_000_000;
    let cycles = u64::try_from(cycles).expect("duration exceeds counter range");
    assert!(cycles < HALF_RANGE, "duration exceeds monotonic half-range");
    cycles
}

pub fn timer_ticks() -> u64 {
    timer_ticks_for(crate::smp::current_cpu_id())
}

pub fn timer_ticks_for(cpu: crate::smp::CpuId) -> u64 {
    TIMER_TICKS[cpu.get()].load(Ordering::Acquire)
}

#[cfg(debug_assertions)]
pub(crate) fn scheduler_tick_active_for(cpu: crate::smp::CpuId) -> bool {
    SCHEDULER_TICK_ACTIVE[cpu.get()].load(Ordering::Acquire)
}

#[cfg(debug_assertions)]
pub(crate) fn scheduler_tick_deadline_for(cpu: crate::smp::CpuId) -> u64 {
    NEXT_SCHEDULER_DEADLINES[cpu.get()].load(Ordering::Acquire)
}

#[cfg(debug_assertions)]
pub(crate) fn programmed_clockevent_deadline_for(cpu: crate::smp::CpuId) -> u64 {
    PROGRAMMED_CLOCKEVENT_DEADLINES[cpu.get()].load(Ordering::Acquire)
}

#[cfg(debug_assertions)]
pub(crate) fn clockevent_running_for(cpu: crate::smp::CpuId) -> bool {
    CLOCKEVENT_MODES[cpu.get()].load(Ordering::Acquire) == CLOCKEVENT_RUNNING
}
#[cfg(debug_assertions)]
pub fn tickless_idle_entries_for(cpu: crate::smp::CpuId) -> u64 {
    TICKLESS_IDLE_ENTRIES[cpu.get()].load(Ordering::Acquire)
}

#[cfg(debug_assertions)]
pub fn verify_periodic() {
    let counter_before = now();
    let target = timer_ticks()
        .checked_add(VERIFY_TICKS)
        .expect("timer verification target overflowed");

    while timer_ticks() < target {
        crate::arch::cpu::wait_for_interrupt();
    }

    let delivered = timer_ticks();
    let counter_after = now();
    assert!(
        delivered >= target,
        "periodic timer delivered too few interrupts"
    );
    assert!(instant_is_after(counter_after, counter_before));
    assert!(crate::arch::interrupt::are_enabled());
    assert!(crate::arch::time::interrupt_source_enabled());

    crate::println!("periodic timer test:");
    crate::println!("  clocksource      : verified");
    crate::println!("  timer interrupt  : verified ({} ticks)", delivered);
    crate::println!("  acknowledge      : verified");
    crate::println!("  rearm            : verified");
    crate::println!("  idle wakeup      : verified");
    crate::println!("  local interrupts : enabled");
    crate::println!("  periodic timer   : armed at {} Hz", TICKS_PER_SECOND);
}

pub(crate) fn deadline_reached(now: MonotonicInstant, deadline: MonotonicInstant) -> bool {
    now == deadline || instant_is_after(now, deadline)
}

pub(crate) fn instant_is_after(left: MonotonicInstant, right: MonotonicInstant) -> bool {
    let distance = left.cycles().wrapping_sub(right.cycles());
    distance != 0 && distance < HALF_RANGE
}

pub(crate) fn instant_is_before(left: MonotonicInstant, right: MonotonicInstant) -> bool {
    instant_is_after(right, left)
}

pub(crate) fn earlier_deadline(
    left: MonotonicInstant,
    right: MonotonicInstant,
) -> MonotonicInstant {
    if instant_is_before(left, right) {
        left
    } else {
        right
    }
}

fn reset_current_clockevent() {
    crate::arch::time::shutdown()
        .unwrap_or_else(|error| panic!("unable to shut down local timer state: {error:?}"));
    let cpu = current_cpu_index();
    PROGRAMMED_CLOCKEVENT_DEADLINES[cpu].store(0, Ordering::Release);
    NEXT_SCHEDULER_DEADLINES[cpu].store(0, Ordering::Release);
    TIMER_TICKS[cpu].store(0, Ordering::Release);
    SCHEDULER_TICK_ACTIVE[cpu].store(false, Ordering::Release);
    TICKLESS_IDLE_ENTRIES[cpu].store(0, Ordering::Release);
    CLOCKEVENT_MODES[cpu].store(CLOCKEVENT_STOPPED, Ordering::Release);
}

fn current_cpu_index() -> usize {
    crate::smp::current_cpu_id().get()
}

fn tick_period_cycles() -> u64 {
    let period = TICK_PERIOD_CYCLES.load(Ordering::Acquire);
    assert!(period != 0, "clockevent has not been initialized");
    period
}

fn minimum_clockevent_delta_cycles() -> u64 {
    // The current architecture API does not yet expose a device-specific
    // min_delta. Use a conservative 1 us floor so real hardware is not asked
    // to commit a deadline only one counter cycle in the future.
    duration_to_cycles_round_up(Duration::from_nanos(MIN_CLOCKEVENT_DELTA_NS)).max(1)
}

fn program_deadline(deadline: MonotonicInstant) {
    crate::arch::time::program_deadline(deadline.cycles())
        .unwrap_or_else(|error| panic!("unable to program timer deadline: {error:?}"));
    PROGRAMMED_CLOCKEVENT_DEADLINES[current_cpu_index()]
        .store(deadline.cycles(), Ordering::Release);
}

fn cycles_to_nanoseconds(cycles: u64) -> u64 {
    let nanoseconds = u128::from(cycles) * 1_000_000_000_u128 / u128::from(clock_frequency_hz());
    u64::try_from(nanoseconds).unwrap_or(u64::MAX)
}

fn cycles_to_duration(cycles: u64) -> Duration {
    Duration::from_nanos(cycles_to_nanoseconds(cycles))
}
