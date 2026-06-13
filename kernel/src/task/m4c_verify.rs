use core::{
    hint::spin_loop,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use crate::smp::CpuId;

use super::{Completion, WaitQueue};

static PREEMPT_HOG_STARTED: AtomicBool = AtomicBool::new(false);
static PREEMPT_PEER_RAN: AtomicBool = AtomicBool::new(false);
static PREEMPT_STOP: AtomicBool = AtomicBool::new(false);
static PREEMPT_HOG_DONE: AtomicBool = AtomicBool::new(false);

static GUARD_ENTERED: AtomicBool = AtomicBool::new(false);
static GUARD_PEER_RAN: AtomicBool = AtomicBool::new(false);
static GUARD_DONE: AtomicBool = AtomicBool::new(false);

static WAIT_QUEUE: WaitQueue = WaitQueue::new();
static WAIT_RELEASED: AtomicBool = AtomicBool::new(false);
static WAIT_READY: AtomicUsize = AtomicUsize::new(0);
static WAIT_DONE: AtomicUsize = AtomicUsize::new(0);
static IMMEDIATE_WAIT_DONE: AtomicBool = AtomicBool::new(false);
static COMPLETION: Completion = Completion::new();
static COMPLETION_DONE: AtomicBool = AtomicBool::new(false);
static COMPLETION_ALL: Completion = Completion::new();
static COMPLETION_ALL_DONE: AtomicUsize = AtomicUsize::new(0);

static SWITCH_RACE_QUEUE: WaitQueue = WaitQueue::new();
static SWITCH_RACE_CONDITION: AtomicBool = AtomicBool::new(false);
static SWITCH_RACE_HOOK_ARMED: AtomicBool = AtomicBool::new(false);
static SWITCH_RACE_HOOK_REACHED: AtomicBool = AtomicBool::new(false);
static SWITCH_RACE_HOOK_RELEASE: AtomicBool = AtomicBool::new(false);
static SWITCH_RACE_DONE: AtomicBool = AtomicBool::new(false);

fn preempt_hog() {
    assert_eq!(crate::smp::current_cpu_id(), CpuId::BOOT);
    PREEMPT_HOG_STARTED.store(true, Ordering::Release);

    while !PREEMPT_STOP.load(Ordering::Acquire) {
        spin_loop();
    }

    PREEMPT_HOG_DONE.store(true, Ordering::Release);
}

fn preempt_peer() {
    assert!(
        PREEMPT_HOG_STARTED.load(Ordering::Acquire),
        "preemption peer ran before the non-yielding task",
    );
    PREEMPT_PEER_RAN.store(true, Ordering::Release);
    PREEMPT_STOP.store(true, Ordering::Release);
}

fn preempt_guard() {
    assert_eq!(crate::smp::current_cpu_id(), CpuId::BOOT);
    let guard = super::PreemptGuard::new();
    assert_eq!(super::preempt_count(), 1);
    GUARD_ENTERED.store(true, Ordering::Release);

    let start = crate::time::timer_ticks();
    while crate::time::timer_ticks().wrapping_sub(start) < 3 {
        assert!(
            !GUARD_PEER_RAN.load(Ordering::Acquire),
            "task was preempted while preemption was disabled",
        );
        spin_loop();
    }

    drop(guard);
    assert_eq!(super::preempt_count(), 0);
    GUARD_DONE.store(true, Ordering::Release);
}

fn preempt_guard_peer() {
    assert!(GUARD_ENTERED.load(Ordering::Acquire));
    GUARD_PEER_RAN.store(true, Ordering::Release);
}

fn wait_worker() {
    WAIT_READY.fetch_add(1, Ordering::AcqRel);
    WAIT_QUEUE.wait_until(|| WAIT_RELEASED.load(Ordering::Acquire));
    WAIT_DONE.fetch_add(1, Ordering::AcqRel);
}

fn immediate_wait_worker() {
    WAIT_QUEUE.wait_until(|| WAIT_RELEASED.load(Ordering::Acquire));
    IMMEDIATE_WAIT_DONE.store(true, Ordering::Release);
}

fn completion_worker() {
    COMPLETION.wait();
    COMPLETION_DONE.store(true, Ordering::Release);
}

fn completion_all_worker() {
    COMPLETION_ALL.wait();
    COMPLETION_ALL_DONE.fetch_add(1, Ordering::AcqRel);
}

pub(super) fn before_block_context_switch() {
    if SWITCH_RACE_HOOK_ARMED
        .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }

    // The scheduler lock has been released, but the outgoing task is still in
    // SwitchingOut and local interrupts remain disabled. Holding this window
    // open makes the wake-vs-switch race deterministic instead of relying on
    // QEMU timing.
    SWITCH_RACE_HOOK_REACHED.store(true, Ordering::Release);
    while !SWITCH_RACE_HOOK_RELEASE.load(Ordering::Acquire) {
        spin_loop();
    }
}

fn switching_race_worker() {
    SWITCH_RACE_QUEUE.wait_until(|| SWITCH_RACE_CONDITION.load(Ordering::Acquire));
    SWITCH_RACE_DONE.store(true, Ordering::Release);
}

fn wait_until<F>(description: &str, condition: F)
where
    F: Fn() -> bool,
{
    let deadline = super::verification_deadline();

    while !condition() {
        if super::deadline_reached(crate::arch::time::counter(), deadline) {
            let wait_state = WAIT_QUEUE.debug_state();
            let race_state = SWITCH_RACE_QUEUE.debug_state();
            panic!(
                "M4C verification timed out while waiting for {description}: \
                 ready={} done={} waiters={} blocked={} switching={} claimed={} \
                 race_blocked={} race_switching={} race_claimed={} live={}",
                WAIT_READY.load(Ordering::Acquire),
                WAIT_DONE.load(Ordering::Acquire),
                WAIT_QUEUE.waiter_count(),
                wait_state.blocked,
                wait_state.switching,
                wait_state.claimed_switching,
                race_state.blocked,
                race_state.switching,
                race_state.claimed_switching,
                super::live_kernel_threads(),
            );
        }
        // Do not broadcast rescue IPIs here: a verifier must expose a missing
        // wakeup kick rather than repair it. Likewise, the remote completion
        // store is not an architectural wake event, so avoid depending on an
        // unrelated periodic timer to return from WFI.
        super::yield_now();
        spin_loop();
    }

    super::finish_switch();
}

fn verify_timer_preemption() {
    PREEMPT_HOG_STARTED.store(false, Ordering::Release);
    PREEMPT_PEER_RAN.store(false, Ordering::Release);
    PREEMPT_STOP.store(false, Ordering::Release);
    PREEMPT_HOG_DONE.store(false, Ordering::Release);

    let before = super::preemptions();
    let guard = super::PreemptGuard::new();
    super::spawn_internal(preempt_hog, Some(CpuId::BOOT), Some(CpuId::BOOT));
    super::spawn_internal(preempt_peer, Some(CpuId::BOOT), Some(CpuId::BOOT));
    drop(guard);

    wait_until("timer preemption workers", || {
        PREEMPT_HOG_DONE.load(Ordering::Acquire)
            && PREEMPT_PEER_RAN.load(Ordering::Acquire)
            && super::live_kernel_threads() == 0
    });

    assert!(
        super::preemptions() > before,
        "non-yielding kernel thread was not preempted",
    );
}

fn verify_preempt_disable() {
    GUARD_ENTERED.store(false, Ordering::Release);
    GUARD_PEER_RAN.store(false, Ordering::Release);
    GUARD_DONE.store(false, Ordering::Release);

    let guard = super::PreemptGuard::new();
    super::spawn_internal(preempt_guard, Some(CpuId::BOOT), Some(CpuId::BOOT));
    super::spawn_internal(preempt_guard_peer, Some(CpuId::BOOT), Some(CpuId::BOOT));
    drop(guard);

    wait_until("preempt-disable workers", || {
        GUARD_DONE.load(Ordering::Acquire)
            && GUARD_PEER_RAN.load(Ordering::Acquire)
            && super::live_kernel_threads() == 0
    });
}

fn verify_wait_queue() {
    WAIT_RELEASED.store(false, Ordering::Release);
    WAIT_READY.store(0, Ordering::Release);
    WAIT_DONE.store(0, Ordering::Release);
    IMMEDIATE_WAIT_DONE.store(false, Ordering::Release);

    let target = if super::active_cpu_count() > 1 {
        CpuId::new(1).expect("CPU1 exceeds MAX_CPUS")
    } else {
        CpuId::BOOT
    };

    super::spawn_internal(wait_worker, Some(target), Some(target));
    super::spawn_internal(wait_worker, Some(target), Some(target));

    wait_until("waiters to block", || {
        WAIT_READY.load(Ordering::Acquire) == 2 && WAIT_QUEUE.waiter_count() == 2
    });

    assert_eq!(WAIT_DONE.load(Ordering::Acquire), 0);
    WAIT_RELEASED.store(true, Ordering::Release);
    assert_eq!(WAIT_QUEUE.wake_one(), 1);

    wait_until("one waiter to wake", || {
        WAIT_DONE.load(Ordering::Acquire) == 1 && WAIT_QUEUE.waiter_count() == 1
    });

    assert_eq!(WAIT_QUEUE.wake_all(), 1);
    wait_until("all waiters to wake", || {
        WAIT_DONE.load(Ordering::Acquire) == 2
            && WAIT_QUEUE.waiter_count() == 0
            && super::live_kernel_threads() == 0
    });

    // The condition is already true. The new worker must not enqueue itself,
    // proving the condition is rechecked in the scheduler critical section.
    super::spawn_internal(immediate_wait_worker, Some(CpuId::BOOT), Some(CpuId::BOOT));
    wait_until("immediate wait condition", || {
        IMMEDIATE_WAIT_DONE.load(Ordering::Acquire) && super::live_kernel_threads() == 0
    });
    assert_eq!(WAIT_QUEUE.waiter_count(), 0);
}

fn verify_completion() {
    COMPLETION_DONE.store(false, Ordering::Release);
    COMPLETION_ALL_DONE.store(0, Ordering::Release);

    super::spawn_internal(completion_worker, Some(CpuId::BOOT), Some(CpuId::BOOT));

    wait_until("completion waiter to block", || {
        super::live_kernel_threads() == 1
    });

    assert!(!COMPLETION_DONE.load(Ordering::Acquire));
    COMPLETION.complete();

    wait_until("completion waiter to wake", || {
        COMPLETION_DONE.load(Ordering::Acquire) && super::live_kernel_threads() == 0
    });

    super::spawn_internal(completion_all_worker, Some(CpuId::BOOT), Some(CpuId::BOOT));
    super::spawn_internal(completion_all_worker, Some(CpuId::BOOT), Some(CpuId::BOOT));

    wait_until("completion-all waiters to block", || {
        super::live_kernel_threads() == 2
    });

    COMPLETION_ALL.complete_all();

    wait_until("completion-all waiters to wake", || {
        COMPLETION_ALL_DONE.load(Ordering::Acquire) == 2 && super::live_kernel_threads() == 0
    });
}

fn verify_switching_out_wakeup() {
    if super::active_cpu_count() == 1 {
        return;
    }

    SWITCH_RACE_CONDITION.store(false, Ordering::Release);
    SWITCH_RACE_HOOK_REACHED.store(false, Ordering::Release);
    SWITCH_RACE_HOOK_RELEASE.store(false, Ordering::Release);
    SWITCH_RACE_DONE.store(false, Ordering::Release);
    SWITCH_RACE_HOOK_ARMED.store(true, Ordering::Release);

    let target = CpuId::new(1).expect("CPU1 exceeds MAX_CPUS");
    super::spawn_internal(switching_race_worker, Some(target), Some(target));

    wait_until("a waiter to enter SwitchingOut", || {
        SWITCH_RACE_HOOK_REACHED.load(Ordering::Acquire)
    });

    let before = SWITCH_RACE_QUEUE.debug_state();
    assert_eq!(before.blocked, 0);
    assert_eq!(before.switching, 1);
    assert_eq!(before.claimed_switching, 0);

    SWITCH_RACE_CONDITION.store(true, Ordering::Release);
    assert_eq!(SWITCH_RACE_QUEUE.wake_one(), 1);

    // A claimed SwitchingOut waiter is no longer eligible for another wake,
    // even though schedule-tail has not yet converted it back to Runnable.
    assert_eq!(SWITCH_RACE_QUEUE.wake_one(), 0);
    assert_eq!(SWITCH_RACE_QUEUE.waiter_count(), 0);
    let claimed = SWITCH_RACE_QUEUE.debug_state();
    assert_eq!(claimed.blocked, 0);
    assert_eq!(claimed.switching, 0);
    assert_eq!(claimed.claimed_switching, 1);

    SWITCH_RACE_HOOK_RELEASE.store(true, Ordering::Release);

    wait_until("the SwitchingOut waiter to converge to runnable", || {
        SWITCH_RACE_DONE.load(Ordering::Acquire) && super::live_kernel_threads() == 0
    });
    assert_eq!(SWITCH_RACE_QUEUE.waiter_count(), 0);
    assert_eq!(
        SWITCH_RACE_QUEUE.debug_state(),
        super::WaiterDebugState::default()
    );
    super::synchronize_retired_tasks();
}

pub(super) fn verify() {
    crate::heap::shrink();
    let pages_before = crate::page_alloc::total_free_pages()
        .expect("page allocator unavailable before M4C verification");

    verify_timer_preemption();
    verify_preempt_disable();
    verify_wait_queue();
    verify_completion();
    verify_switching_out_wakeup();

    super::synchronize_retired_tasks();
    crate::heap::shrink();
    let pages_after = crate::page_alloc::total_free_pages()
        .expect("page allocator unavailable after M4C verification");
    assert_eq!(
        pages_before,
        pages_after,
        "M4C task resources leaked: active_cpus={} retired_tasks={}",
        super::active_cpu_count(),
        super::retired_task_count(),
    );

    crate::println!("preemptive scheduler test:");
    crate::println!("  non-yielding task : preempted");
    crate::println!("  timer reschedule  : verified");
    crate::println!("  preempt count     : verified");
    crate::println!("wait queue test:");
    crate::println!("  block current     : verified");
    crate::println!("  remote wakeup     : verified");
    crate::println!("  no lost wakeup    : verified");
    crate::println!("  switching-out race: verified");
    crate::println!("  wake one/all      : verified");
    crate::println!("completion test:");
    crate::println!("  complete          : verified");
    crate::println!("  complete_all      : verified");
    crate::println!("M4C_SCHED_TEST: PASS");
}
