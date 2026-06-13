use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

use crate::smp::{CpuId, MAX_CPUS};

const MAX_HELD_LOCKS: usize = 16;
const RANK_COUNT: usize = LockRank::Console as usize + 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(usize)]
pub enum LockRank {
    Unknown = 0,
    CpuLifecycle = 10,
    Scheduler = 20,
    WaitQueue = 30,
    Vm = 40,
    PageTable = 50,
    Heap = 60,
    PageAllocator = 70,
    Console = 80,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LockClass {
    pub name: &'static str,
    pub rank: LockRank,
    pub order: usize,
}

impl LockClass {
    pub const fn new(name: &'static str, rank: LockRank, order: usize) -> Self {
        Self { name, rank, order }
    }

    pub const fn unknown() -> Self {
        Self::new("unknown", LockRank::Unknown, 0)
    }

    const fn key(self) -> usize {
        (self.rank as usize) * 1024 + self.order
    }
}

#[derive(Clone, Copy)]
struct HeldLock {
    class: LockClass,
    acquired_at: u64,
}

impl HeldLock {
    const EMPTY: Self = Self {
        class: LockClass::unknown(),
        acquired_at: 0,
    };
}

struct CpuHeldLocks {
    depth: AtomicUsize,
    entries: UnsafeCell<[HeldLock; MAX_HELD_LOCKS]>,
}

impl CpuHeldLocks {
    const fn new() -> Self {
        Self {
            depth: AtomicUsize::new(0),
            entries: UnsafeCell::new([HeldLock::EMPTY; MAX_HELD_LOCKS]),
        }
    }
}

// SAFETY: each CPU mutates only its own entry while local interrupts are
// disabled. Cross-CPU reads are not implemented yet.
unsafe impl Sync for CpuHeldLocks {}

static HELD_LOCKS: [CpuHeldLocks; MAX_CPUS] = [const { CpuHeldLocks::new() }; MAX_CPUS];
static MAX_HOLD_CYCLES: [AtomicU64; RANK_COUNT] = [const { AtomicU64::new(0) }; RANK_COUNT];
static MAX_IRQ_OFF_CYCLES: AtomicU64 = AtomicU64::new(0);

pub fn before_lock(class: LockClass, owner: usize, current: CpuId) {
    if owner == current.get() {
        panic!(
            "recursive lock acquisition: lock={} cpu={}",
            class.name,
            current.get(),
        );
    }

    let state = &HELD_LOCKS[current.get()];
    let depth = state.depth.load(Ordering::Relaxed);
    assert!(depth <= MAX_HELD_LOCKS, "held-lock stack depth corrupted");

    // SAFETY: local IRQs are disabled by IrqSpinLock before this function is
    // called, so this CPU owns its lockdep stack.
    let entries = unsafe { &*state.entries.get() };

    for held in entries.iter().take(depth) {
        if held.class.name == class.name {
            panic!(
                "recursive lock acquisition through lockdep stack: lock={} cpu={}",
                class.name,
                current.get(),
            );
        }

        let held_key = held.class.key();
        let new_key = class.key();

        assert!(
            held_key <= new_key,
            "lock order violation: held={}({:?}/#{}) new={}({:?}/#{}) cpu={}",
            held.class.name,
            held.class.rank,
            held.class.order,
            class.name,
            class.rank,
            class.order,
            current.get(),
        );
    }
}

pub fn after_lock(class: LockClass, current: CpuId) {
    let state = &HELD_LOCKS[current.get()];
    let depth = state.depth.load(Ordering::Relaxed);
    assert!(depth < MAX_HELD_LOCKS, "held-lock stack overflow");

    // SAFETY: local IRQs are disabled by IrqSpinLock while this stack is
    // updated.
    let entries = unsafe { &mut *state.entries.get() };
    entries[depth] = HeldLock {
        class,
        acquired_at: crate::arch::time::counter(),
    };
    state.depth.store(depth + 1, Ordering::Relaxed);
}

pub fn before_unlock(class: LockClass, current: CpuId) {
    let state = &HELD_LOCKS[current.get()];
    let depth = state.depth.load(Ordering::Relaxed);
    assert!(depth != 0, "unlock with empty held-lock stack");

    // SAFETY: local IRQs are still disabled while IrqSpinLock drops its guard.
    let entries = unsafe { &mut *state.entries.get() };
    let index = depth - 1;
    let held = entries[index];

    assert_eq!(
        held.class.name,
        class.name,
        "lock release order violation: top={} releasing={} cpu={}",
        held.class.name,
        class.name,
        current.get(),
    );

    let held_cycles = crate::arch::time::counter().wrapping_sub(held.acquired_at);
    update_max(&MAX_HOLD_CYCLES[class.rank as usize], held_cycles);

    entries[index] = HeldLock::EMPTY;
    state.depth.store(index, Ordering::Relaxed);
}

pub fn record_irq_off(cycles: u64) {
    update_max(&MAX_IRQ_OFF_CYCLES, cycles);
}

pub fn dump_current_cpu() {
    let raw_cpu = crate::arch::smp::current_cpu_id();
    if raw_cpu >= MAX_CPUS {
        crate::println!("lockdep:");
        crate::println!("  current CPU : invalid ({raw_cpu})");
        return;
    }

    let state = &HELD_LOCKS[raw_cpu];
    let depth = state.depth.load(Ordering::Acquire);
    crate::println!("lockdep:");
    crate::println!("  current CPU : {raw_cpu}");
    crate::println!("  held locks  : {}", depth);
    crate::println!(
        "  max IRQ-off : {} cycles",
        MAX_IRQ_OFF_CYCLES.load(Ordering::Acquire),
    );

    if depth == 0 {
        return;
    }

    let now = crate::arch::time::counter();
    let count = core::cmp::min(depth, MAX_HELD_LOCKS);

    // SAFETY: the panic owner disables local interrupts before calling this
    // function, so its own lockdep stack cannot be modified concurrently.
    let entries = unsafe { &*state.entries.get() };
    for (index, held) in entries.iter().take(count).enumerate() {
        crate::println!(
            "    #{index}: {} {:?}/#{} held={} cycles",
            held.class.name,
            held.class.rank,
            held.class.order,
            now.wrapping_sub(held.acquired_at),
        );
    }
}

#[cfg(debug_assertions)]
pub fn max_irq_off_cycles() -> u64 {
    MAX_IRQ_OFF_CYCLES.load(Ordering::Acquire)
}

#[cfg(debug_assertions)]
pub fn max_hold_cycles(rank: LockRank) -> u64 {
    MAX_HOLD_CYCLES[rank as usize].load(Ordering::Acquire)
}

fn update_max(target: &AtomicU64, value: u64) {
    let mut old = target.load(Ordering::Relaxed);
    while value > old {
        match target.compare_exchange_weak(old, value, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => break,
            Err(current) => old = current,
        }
    }
}
