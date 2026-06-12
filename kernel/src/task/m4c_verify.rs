use core::{
    hint::spin_loop,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use crate::smp::CpuId;

use super::WaitQueue;

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
    super::preempt_disable();
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

    super::preempt_enable();
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

fn wait_until<F>(description: &str, condition: F)
where
    F: Fn() -> bool,
{
    let deadline = super::verification_deadline();

    while !condition() {
        assert!(
            !super::deadline_reached(crate::arch::time::counter(), deadline),
            "M4C verification timed out while waiting for {description}",
        );
        super::yield_now();
        if !super::current_cpu_has_work() {
            crate::arch::cpu::wait_for_interrupt();
        }
    }

    super::finish_switch();
}

fn verify_timer_preemption() {
    PREEMPT_HOG_STARTED.store(false, Ordering::Release);
    PREEMPT_PEER_RAN.store(false, Ordering::Release);
    PREEMPT_STOP.store(false, Ordering::Release);
    PREEMPT_HOG_DONE.store(false, Ordering::Release);

    let before = super::preemptions();
    super::spawn_internal(preempt_hog, Some(CpuId::BOOT), Some(CpuId::BOOT));
    super::spawn_internal(preempt_peer, Some(CpuId::BOOT), Some(CpuId::BOOT));

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

    super::spawn_internal(preempt_guard, Some(CpuId::BOOT), Some(CpuId::BOOT));
    super::spawn_internal(preempt_guard_peer, Some(CpuId::BOOT), Some(CpuId::BOOT));

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

    let target = if crate::smp::online_cpu_count() > 1 {
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

pub(super) fn verify() {
    crate::heap::shrink();
    let pages_before = crate::page_alloc::total_free_pages()
        .expect("page allocator unavailable before M4C verification");

    verify_timer_preemption();
    verify_preempt_disable();
    verify_wait_queue();

    crate::heap::shrink();
    let pages_after = crate::page_alloc::total_free_pages()
        .expect("page allocator unavailable after M4C verification");
    assert_eq!(pages_before, pages_after, "M4C task resources leaked");

    crate::println!("preemptive scheduler test:");
    crate::println!("  non-yielding task : preempted");
    crate::println!("  timer reschedule  : verified");
    crate::println!("  preempt count     : verified");
    crate::println!("wait queue test:");
    crate::println!("  block current     : verified");
    crate::println!("  remote wakeup     : verified");
    crate::println!("  no lost wakeup    : verified");
    crate::println!("  wake one/all      : verified");
    crate::println!("M4C_SCHED_TEST: PASS");
}
