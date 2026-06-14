// M6-B r5: NO_HZ-aware deterministic idle verifier.
use core::{
    hint::spin_loop,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use super::{CpuId, WaitQueue};

const NO_TARGET: usize = usize::MAX;
const GATE_DISABLED: usize = 0;
const GATE_ARMED: usize = 1;
const GATE_RECHECK_PASSED: usize = 2;
const GATE_RELEASED: usize = 3;

static TARGET_CPU: AtomicUsize = AtomicUsize::new(NO_TARGET);
static GATE_PHASE: AtomicUsize = AtomicUsize::new(GATE_DISABLED);
static WAKE_WORKER_READY: AtomicBool = AtomicBool::new(false);
static RUN_ALLOWED: AtomicBool = AtomicBool::new(false);
static WAKE_WORKER_RAN: AtomicBool = AtomicBool::new(false);
static CLEANUP_ALLOWED: AtomicBool = AtomicBool::new(false);
static RUN_QUEUE: WaitQueue = WaitQueue::new();
static CLEANUP_QUEUE: WaitQueue = WaitQueue::new();

fn target_cpu() -> CpuId {
    CpuId::new(TARGET_CPU.load(Ordering::Acquire))
        .expect("deterministic idle test has no valid target CPU")
}

fn deadline() -> u64 {
    crate::arch::time::counter().wrapping_add(
        crate::time::clock_frequency_hz()
            .checked_mul(super::VERIFY_TIMEOUT_SECONDS)
            .expect("deterministic idle verification timeout overflowed"),
    )
}

fn deadline_reached(now: u64, deadline: u64) -> bool {
    now.wrapping_sub(deadline) < (1_u64 << 63)
}

fn wait_for_flag(flag: &AtomicBool, description: &str) {
    let limit = deadline();
    while !flag.load(Ordering::Acquire) {
        assert!(
            !deadline_reached(crate::arch::time::counter(), limit),
            "deterministic idle test timed out: {description}",
        );
        spin_loop();
    }
}

fn wait_for_gate(expected: usize, description: &str) {
    let limit = deadline();
    while GATE_PHASE.load(Ordering::Acquire) != expected {
        assert!(
            !deadline_reached(crate::arch::time::counter(), limit),
            "deterministic idle test timed out: {description}; phase={}",
            GATE_PHASE.load(Ordering::Acquire),
        );
        spin_loop();
    }
}

fn wait_for_blocked(queue: &WaitQueue, expected: usize, description: &str) {
    let limit = deadline();
    loop {
        let state = super::waiter_debug_state(queue.channel());
        if state.blocked == expected && state.switching == 0 && state.claimed_switching == 0 {
            return;
        }
        assert!(
            !deadline_reached(crate::arch::time::counter(), limit),
            "deterministic idle test timed out: {description}; \
             blocked={} switching={} claimed={}",
            state.blocked,
            state.switching,
            state.claimed_switching,
        );
        spin_loop();
    }
}

fn wait_for_threads_to_exit() {
    let limit = deadline();
    while super::live_kernel_threads() != 0 {
        assert!(
            !deadline_reached(crate::arch::time::counter(), limit),
            "deterministic idle cleanup timed out: live={}",
            super::live_kernel_threads(),
        );
        super::yield_now();
    }
}

fn wake_worker() {
    let target = target_cpu();
    assert_eq!(
        crate::smp::current_cpu_id(),
        target,
        "deterministic idle wake worker ran on the wrong CPU",
    );

    WAKE_WORKER_READY.store(true, Ordering::Release);
    RUN_QUEUE.wait_until(|| RUN_ALLOWED.load(Ordering::Acquire));

    // The wait returns through block_current_on_if() and finish_switch().  M6-B
    // requires switch tail to restore the scheduler policy tick before a
    // non-idle task resumes; the worker must never repair timer state itself.
    assert!(
        crate::time::scheduler_tick_active_for(target),
        "scheduler tick was not restored before the woken task resumed",
    );

    WAKE_WORKER_RAN.store(true, Ordering::Release);

    // Keep this task alive until CPU0 samples the IPI counter.  Exiting here
    // would let stack reclamation perform a TLB shootdown inside the exact-one
    // IPI measurement interval.
    CLEANUP_QUEUE.wait_until(|| CLEANUP_ALLOWED.load(Ordering::Acquire));
}

fn idle_gate_worker() {
    let target = target_cpu();
    assert_eq!(
        crate::smp::current_cpu_id(),
        target,
        "deterministic idle gate worker ran on the wrong CPU",
    );

    // Do not manipulate the local clockevent here.  Blocking the final
    // runnable worker forces the normal idle path to perform its IRQ-disabled
    // work recheck and call time::enter_idle().
    GATE_PHASE.store(GATE_ARMED, Ordering::Release);
    CLEANUP_QUEUE.wait_until(|| CLEANUP_ALLOWED.load(Ordering::Acquire));
}

pub(super) fn before_arch_wait(cpu: CpuId) {
    if TARGET_CPU.load(Ordering::Acquire) != cpu.get() {
        return;
    }

    if GATE_PHASE
        .compare_exchange(
            GATE_ARMED,
            GATE_RECHECK_PASSED,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        return;
    }

    assert!(
        crate::arch::interrupt::are_disabled(),
        "deterministic idle gate was reached with local interrupts enabled",
    );
    assert!(
        !crate::time::scheduler_tick_active_for(cpu),
        "deterministic idle gate was reached with the scheduler tick active",
    );

    // CPU0 publishes the blocked task and sends the sole reschedule IPI before
    // releasing this gate.  The IPI is therefore pending while local IRQs
    // remain disabled and must be observed by enable-and-wait.
    while GATE_PHASE.load(Ordering::Acquire) != GATE_RELEASED {
        spin_loop();
    }
}

fn reset_state(target: CpuId) {
    assert_eq!(
        super::waiter_debug_state(RUN_QUEUE.channel()),
        super::WaiterDebugState::default(),
        "deterministic idle run queue was not quiescent",
    );
    assert_eq!(
        super::waiter_debug_state(CLEANUP_QUEUE.channel()),
        super::WaiterDebugState::default(),
        "deterministic idle cleanup queue was not quiescent",
    );

    TARGET_CPU.store(target.get(), Ordering::Release);
    GATE_PHASE.store(GATE_DISABLED, Ordering::Release);
    WAKE_WORKER_READY.store(false, Ordering::Release);
    RUN_ALLOWED.store(false, Ordering::Release);
    WAKE_WORKER_RAN.store(false, Ordering::Release);
    CLEANUP_ALLOWED.store(false, Ordering::Release);
}

pub(super) fn verify(cpu_count: usize) {
    if cpu_count == 1 {
        crate::println!("deterministic idle/IPI test:");
        crate::println!("  scheduler tick      : single-CPU fallback");
        crate::println!("  single reschedule IPI: single-CPU fallback");
        return;
    }

    assert_eq!(
        crate::smp::current_cpu_id(),
        CpuId::BOOT,
        "deterministic idle verification must run on the boot CPU",
    );
    crate::context::assert_interrupts_enabled();
    crate::context::assert_task_context();
    assert_eq!(
        super::live_kernel_threads(),
        0,
        "deterministic idle verification requires no live test threads",
    );
    super::synchronize_retired_tasks();

    let target = CpuId::new(1).expect("CPU1 exceeds MAX_CPUS");
    assert!(crate::smp::is_online(target));
    assert!(crate::smp::is_ipi_ready(target));
    reset_state(target);

    // Allocate and initially block the measured worker before the tickless
    // window.  Kernel-stack vmalloc/TLB activity is therefore outside it.
    super::spawn_internal(wake_worker, Some(target), Some(target));
    wait_for_flag(&WAKE_WORKER_READY, "wake worker did not start");
    wait_for_blocked(&RUN_QUEUE, 1, "wake worker did not block");

    // CPU1 may already be in legitimate NO_HZ idle.  Do not require an
    // initially active tick.  Wake it with a pre-measurement worker and let
    // that worker block so the real idle path owns the next tick stop.
    let idle_entries_before = crate::time::tickless_idle_entries_for(target);
    super::spawn_internal(idle_gate_worker, Some(target), Some(target));
    wait_for_gate(
        GATE_RECHECK_PASSED,
        "target CPU did not pass the IRQ-disabled idle recheck",
    );
    assert!(
        crate::time::tickless_idle_entries_for(target) > idle_entries_before,
        "target CPU reached the idle gate without a new tickless-idle entry",
    );
    assert!(
        !crate::time::scheduler_tick_active_for(target),
        "target scheduler tick was unexpectedly active at the idle gate",
    );

    let ticks_before = crate::time::timer_ticks_for(target);
    let ipis_before = crate::smp::ipi_count(target);

    RUN_ALLOWED.store(true, Ordering::Release);
    assert_eq!(
        RUN_QUEUE.wake_one(),
        1,
        "deterministic idle test did not claim exactly one blocked worker",
    );

    // wake_one() has published need_resched and emitted the one remote IPI.
    GATE_PHASE.store(GATE_RELEASED, Ordering::Release);
    wait_for_flag(
        &WAKE_WORKER_RAN,
        "single reschedule IPI did not run the target worker",
    );

    let delivered = crate::smp::ipi_count(target)
        .checked_sub(ipis_before)
        .expect("target IPI counter moved backwards");
    assert_eq!(
        delivered, 1,
        "deterministic idle window received an unexpected number of IPIs",
    );
    assert!(
        crate::time::scheduler_tick_active_for(target),
        "switch tail did not restore the target scheduler tick",
    );

    let tick_limit = deadline();
    while crate::time::timer_ticks_for(target) == ticks_before {
        assert!(
            !deadline_reached(crate::arch::time::counter(), tick_limit),
            "target scheduler tick did not advance after restoration",
        );
        spin_loop();
    }

    // Any exit/reaper/TLB IPI is deliberately moved after the exact count.
    wait_for_blocked(
        &CLEANUP_QUEUE,
        2,
        "test workers did not reach the cleanup barrier",
    );
    CLEANUP_ALLOWED.store(true, Ordering::Release);
    assert_eq!(
        CLEANUP_QUEUE.wake_all(),
        2,
        "deterministic idle cleanup did not release both workers",
    );
    wait_for_threads_to_exit();
    super::synchronize_retired_tasks();

    assert_eq!(
        super::waiter_debug_state(RUN_QUEUE.channel()),
        super::WaiterDebugState::default(),
    );
    assert_eq!(
        super::waiter_debug_state(CLEANUP_QUEUE.channel()),
        super::WaiterDebugState::default(),
    );

    TARGET_CPU.store(NO_TARGET, Ordering::Release);
    GATE_PHASE.store(GATE_DISABLED, Ordering::Release);
    RUN_ALLOWED.store(false, Ordering::Release);
    CLEANUP_ALLOWED.store(false, Ordering::Release);

    crate::println!("deterministic idle/IPI test:");
    crate::println!("  target CPU            : {}", target.get());
    crate::println!("  scheduler tick stop   : verified");
    crate::println!("  IRQ-disabled recheck  : verified");
    crate::println!("  pending IPI at wait   : verified");
    crate::println!("  single reschedule IPI : verified");
    crate::println!("  scheduler tick restore: verified");
}
