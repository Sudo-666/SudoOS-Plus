use core::{
    hint::spin_loop,
    sync::atomic::{AtomicU64, Ordering, fence},
};

use myos_sync::SpinLock;

use crate::smp::{CpuId, MAX_CPUS};

const SHOOTDOWN_TIMEOUT_SECONDS: u64 = 5;

static SHOOTDOWN_SERIALIZER: SpinLock<()> = SpinLock::new(());
static REQUEST_GENERATION: AtomicU64 = AtomicU64::new(0);
static ACK_GENERATIONS: [AtomicU64; MAX_CPUS] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static REMOTE_FLUSH_COUNTS: [AtomicU64; MAX_CPUS] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static COMPLETED_SHOOTDOWNS: AtomicU64 = AtomicU64::new(0);

/// Makes every online CPU discard cached translations for the shared kernel
/// page table before this function returns.
///
/// The current implementation intentionally over-fences with a full local TLB
/// invalidation on each CPU. This is slower than a range request, but it gives
/// M5 a simple and correct kernel-wide invalidation primitive. Range and ASID
/// batching can be added after process address spaces exist.
pub fn shootdown_kernel_all() {
    // Before SMP is active this primitive is also used by early VM self-tests,
    // when the scheduler does not exist yet. Keep that path strictly local.
    if crate::smp::online_cpu_mask().count_ones() <= 1 {
        crate::arch::memory::paging::flush_all();
        COMPLETED_SHOOTDOWNS.fetch_add(1, Ordering::Relaxed);
        return;
    }

    crate::context::assert_interrupts_enabled();
    crate::context::assert_task_context();

    // Disable migration before sampling the logical CPU ID and target mask.
    // Sampling first is unsafe now that an already-started runnable task may be
    // stolen: a timer preemption could resume this caller on another CPU and
    // make it exclude the wrong CPU from the shootdown.
    let migration_guard = crate::task::MigrationGuard::new();

    let current = crate::smp::current_cpu_id();
    let online = crate::smp::online_cpu_mask();
    let ready = crate::smp::ipi_ready_cpu_mask();
    let current_bit = 1_usize << current.get();

    assert!(
        online & current_bit != 0,
        "TLB shootdown attempted from an offline CPU: cpu={} online={online:#x}",
        current.get(),
    );
    assert_eq!(
        ready & online,
        online,
        "TLB shootdown attempted before all online CPUs became IPI-ready: \
         online={online:#x} ready={ready:#x}",
    );

    let targets = online & !current_bit;
    assert_ne!(
        targets, 0,
        "multi-CPU TLB shootdown lost all remote targets after migration was disabled",
    );

    // Interrupts remain enabled, so this CPU can acknowledge another CPU's
    // shootdown while contending for the global serializer.
    let frequency = crate::time::clock_frequency_hz();
    let timeout_cycles = frequency
        .checked_mul(SHOOTDOWN_TIMEOUT_SECONDS)
        .expect("TLB shootdown timeout overflowed");
    let lock_deadline = crate::arch::time::counter().wrapping_add(timeout_cycles);

    let serializer = loop {
        if let Some(serializer) = SHOOTDOWN_SERIALIZER.try_lock() {
            break serializer;
        }

        assert!(
            !deadline_reached(crate::arch::time::counter(), lock_deadline),
            "timed out acquiring the TLB shootdown serializer: cpu={}",
            current.get(),
        );

        spin_loop();
    };

    let generation = REQUEST_GENERATION
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    assert_ne!(generation, 0, "TLB shootdown generation wrapped to zero");

    // Page-table stores performed before entering this function must become
    // globally visible before a target CPU observes its IPI request.
    fence(Ordering::SeqCst);

    for index in 0..crate::smp::discovered_cpu_count() {
        let bit = 1_usize << index;
        if targets & bit == 0 {
            continue;
        }

        let cpu = CpuId::new(index).expect("TLB target exceeds MAX_CPUS");
        crate::smp::send_tlb_shootdown(cpu);
    }

    crate::arch::memory::paging::flush_all();

    let frequency = crate::time::clock_frequency_hz();
    let timeout_cycles = frequency
        .checked_mul(SHOOTDOWN_TIMEOUT_SECONDS)
        .expect("TLB shootdown timeout overflowed");
    let deadline = crate::arch::time::counter().wrapping_add(timeout_cycles);

    loop {
        let mut pending = 0_usize;

        let cpu_count = crate::smp::discovered_cpu_count();

        assert!(
            cpu_count <= ACK_GENERATIONS.len(),
            "discovered CPU count exceeds TLB ACK storage",
        );

        for (index, ack) in ACK_GENERATIONS.iter().take(cpu_count).enumerate() {
            let bit = 1_usize
                .checked_shl(index as u32)
                .expect("CPU index exceeds TLB target mask width");

            if targets & bit != 0 && ack.load(Ordering::Acquire) != generation {
                pending |= bit;
            }
        }

        if pending == 0 {
            break;
        }

        assert!(
            !deadline_reached(crate::arch::time::counter(), deadline),
            "TLB shootdown timed out: generation={generation} pending={pending:#x}",
        );
        spin_loop();
    }

    fence(Ordering::Acquire);
    COMPLETED_SHOOTDOWNS.fetch_add(1, Ordering::Relaxed);

    drop(serializer);
    drop(migration_guard);
}

/// Handles the TLB component of a software interrupt on the current CPU.
pub fn handle_shootdown_ipi() {
    let cpu = crate::smp::current_cpu_id();
    let generation = REQUEST_GENERATION.load(Ordering::Acquire);

    assert_ne!(generation, 0, "TLB shootdown IPI has no published request");

    crate::arch::memory::paging::flush_all();
    fence(Ordering::SeqCst);

    REMOTE_FLUSH_COUNTS[cpu.get()].fetch_add(1, Ordering::Relaxed);
    ACK_GENERATIONS[cpu.get()].store(generation, Ordering::Release);
}

#[cfg(debug_assertions)]
pub fn completed_shootdowns() -> u64 {
    COMPLETED_SHOOTDOWNS.load(Ordering::Acquire)
}

#[cfg(debug_assertions)]
pub fn remote_flush_count(cpu: CpuId) -> u64 {
    REMOTE_FLUSH_COUNTS[cpu.get()].load(Ordering::Acquire)
}

fn deadline_reached(now: u64, deadline: u64) -> bool {
    now.wrapping_sub(deadline) < (1_u64 << 63)
}
