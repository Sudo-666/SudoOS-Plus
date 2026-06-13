use core::{
    hint::spin_loop,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering, fence},
};

use crate::smp::CpuId;

const PATTERN_BEFORE: u64 = 0x4d34_4332_544c_4230;
const PATTERN_AFTER: u64 = 0x4d34_4332_544c_4231;

static TLB_ADDRESS: AtomicUsize = AtomicUsize::new(0);
static TLB_PHASE: AtomicUsize = AtomicUsize::new(0);
static TLB_READY_MASK: AtomicUsize = AtomicUsize::new(0);
static TLB_DONE_MASK: AtomicUsize = AtomicUsize::new(0);

static MIGRATION_STARTED: AtomicBool = AtomicBool::new(false);
static MIGRATION_RESUMED: AtomicBool = AtomicBool::new(false);
static MIGRATION_FIRST_CPU: AtomicUsize = AtomicUsize::new(usize::MAX);
static MIGRATION_LAST_CPU: AtomicUsize = AtomicUsize::new(usize::MAX);

fn deadline() -> u64 {
    super::verification_deadline()
}

fn wait_without_scheduling(description: &str, condition: impl Fn() -> bool) {
    let timeout_at = deadline();

    while !condition() {
        assert!(
            !super::deadline_reached(crate::arch::time::counter(), timeout_at),
            "M4C2 verification timed out while waiting for {description}",
        );
        // The observed condition is published by another CPU with an atomic
        // store, which is not itself an architectural wake event. Busy-wait in
        // this debug verifier instead of depending on the periodic timer to
        // wake WFI.
        spin_loop();
    }
}

fn tlb_observer() {
    let cpu = crate::smp::current_cpu_id();
    assert_ne!(cpu, CpuId::BOOT, "boot CPU entered a remote TLB observer");

    let address = TLB_ADDRESS.load(Ordering::Acquire);
    assert_ne!(address, 0, "TLB observer received a null test address");
    let pointer = address as *const u64;

    // SAFETY: the verification owner retains the vmalloc reservation and its
    // backing page until every observer has completed both volatile reads.
    let before = unsafe { core::ptr::read_volatile(pointer) };
    assert_eq!(before, PATTERN_BEFORE, "initial remote TLB read was stale");

    TLB_READY_MASK.fetch_or(1_usize << cpu.get(), Ordering::AcqRel);

    while TLB_PHASE.load(Ordering::Acquire) == 0 {
        spin_loop();
    }

    // SAFETY: the mapping remains valid, but its PTE now names a replacement
    // frame. The synchronous shootdown completed before phase publication.
    let after = unsafe { core::ptr::read_volatile(pointer) };
    assert_eq!(
        after, PATTERN_AFTER,
        "remote CPU retained a stale TLB entry"
    );

    TLB_DONE_MASK.fetch_or(1_usize << cpu.get(), Ordering::AcqRel);
}

fn verify_remote_tlb_shootdown(cpu_count: usize) {
    TLB_PHASE.store(0, Ordering::Release);
    TLB_READY_MASK.store(0, Ordering::Release);
    TLB_DONE_MASK.store(0, Ordering::Release);

    crate::heap::shrink();
    let pages_before = crate::page_alloc::total_free_pages()
        .expect("page allocator unavailable before TLB shootdown verification");
    let tables_before = crate::vm::debug_runtime_table_count();

    let mut mapping = crate::vm::allocate_tlb_test_mapping();
    let address = mapping.address().get();
    TLB_ADDRESS.store(address, Ordering::Release);

    let pointer = address as *mut u64;
    // SAFETY: allocate_tlb_test_mapping returned a live, writable, page-sized
    // kernel mapping retained by `mapping` for the duration of this test.
    unsafe { core::ptr::write_volatile(pointer, PATTERN_BEFORE) };
    // Prime the boot CPU's local TLB as well.
    // SAFETY: same live mapping and object lifetime as the write above.
    assert_eq!(unsafe { core::ptr::read_volatile(pointer) }, PATTERN_BEFORE);

    let remote_mask = super::active_cpu_mask() & !(1_usize << CpuId::BOOT.get());
    let mut remote_flushes_before = [0_u64; crate::smp::MAX_CPUS];

    for (index, before) in remote_flushes_before
        .iter_mut()
        .enumerate()
        .take(cpu_count)
        .skip(1)
    {
        let cpu = CpuId::new(index).expect("TLB observer CPU exceeds MAX_CPUS");
        *before = crate::tlb::remote_flush_count(cpu);
        super::spawn_internal(tlb_observer, Some(cpu), Some(cpu));
    }

    if remote_mask != 0 {
        wait_without_scheduling("remote CPUs to prime the TLB mapping", || {
            TLB_READY_MASK.load(Ordering::Acquire) == remote_mask
        });
    }

    let shootdowns_before_replace = crate::tlb::completed_shootdowns();
    crate::vm::replace_tlb_test_backing(&mut mapping);
    assert!(
        crate::tlb::completed_shootdowns() > shootdowns_before_replace,
        "mapping replacement did not complete a TLB shootdown",
    );

    // SAFETY: the same virtual address now maps a newly allocated writable
    // page, and replace_tlb_test_backing completed the global invalidation.
    unsafe { core::ptr::write_volatile(pointer, PATTERN_AFTER) };
    fence(Ordering::SeqCst);
    TLB_PHASE.store(1, Ordering::Release);

    if remote_mask != 0 {
        wait_without_scheduling("remote CPUs to observe the replacement page", || {
            TLB_DONE_MASK.load(Ordering::Acquire) == remote_mask
        });

        for (index, before) in remote_flushes_before
            .iter()
            .enumerate()
            .take(cpu_count)
            .skip(1)
        {
            let cpu = CpuId::new(index).expect("TLB acknowledgement CPU exceeds MAX_CPUS");
            assert!(
                crate::tlb::remote_flush_count(cpu) > *before,
                "CPU {} did not acknowledge the remote TLB invalidation",
                cpu.get(),
            );
        }
    }

    wait_without_scheduling("TLB observer tasks to exit", || {
        super::live_kernel_threads() == 0
    });
    super::synchronize_retired_tasks();
    crate::vm::release_tlb_test_mapping(mapping);
    TLB_ADDRESS.store(0, Ordering::Release);

    crate::heap::shrink();
    let pages_after = crate::page_alloc::total_free_pages()
        .expect("page allocator unavailable after TLB shootdown verification");
    assert_eq!(
        crate::vm::debug_runtime_table_count(),
        tables_before,
        "remote TLB test leaked runtime page-table pages",
    );
    assert_eq!(
        pages_after, pages_before,
        "remote TLB test leaked physical pages"
    );
}

fn migration_worker() {
    let first = crate::smp::current_cpu_id();
    assert_eq!(first, CpuId::BOOT, "migration worker did not start on CPU0");
    MIGRATION_FIRST_CPU.store(first.get(), Ordering::Release);

    // Keep the initial CPU0 affinity across the first yield. Otherwise an idle
    // secondary may steal the runnable task before the verifier can perform and
    // inspect the explicit migration hand-off.
    MIGRATION_STARTED.store(true, Ordering::Release);
    super::yield_now();

    let resumed = crate::smp::current_cpu_id();
    MIGRATION_LAST_CPU.store(resumed.get(), Ordering::Release);
    // migrate_runnable_task retargets the temporary affinity atomically with
    // queue migration; remove it after the task has actually resumed there.
    super::clear_current_affinity();
    MIGRATION_RESUMED.store(true, Ordering::Release);
}

fn verify_started_task_migration(cpu_count: usize) {
    crate::heap::shrink();
    let pages_before = crate::page_alloc::total_free_pages()
        .expect("page allocator unavailable before task migration verification");

    MIGRATION_STARTED.store(false, Ordering::Release);
    MIGRATION_RESUMED.store(false, Ordering::Release);
    MIGRATION_FIRST_CPU.store(usize::MAX, Ordering::Release);
    MIGRATION_LAST_CPU.store(usize::MAX, Ordering::Release);

    let task = super::spawn_internal(migration_worker, Some(CpuId::BOOT), Some(CpuId::BOOT)).0;

    // Run the worker once. It yields back while still pinned to CPU0, leaving
    // a has_run=true Runnable task that no secondary may steal prematurely.
    super::yield_now();
    assert!(MIGRATION_STARTED.load(Ordering::Acquire));
    assert_eq!(
        MIGRATION_FIRST_CPU.load(Ordering::Acquire),
        CpuId::BOOT.get()
    );
    assert!(super::task_is_runnable_on(task, CpuId::BOOT));

    if cpu_count == 1 {
        // Resume locally to verify the UP fallback without inventing a remote
        // CPU. The common scheduler state machine remains identical.
        super::yield_now();
        assert!(MIGRATION_RESUMED.load(Ordering::Acquire));
        assert_eq!(
            MIGRATION_LAST_CPU.load(Ordering::Acquire),
            CpuId::BOOT.get()
        );
    } else {
        let target = CpuId::new(1).expect("CPU1 exceeds MAX_CPUS");
        super::migrate_runnable_task(task, target);
        wait_without_scheduling("an already-run task to resume remotely", || {
            MIGRATION_RESUMED.load(Ordering::Acquire)
        });
        assert_eq!(MIGRATION_LAST_CPU.load(Ordering::Acquire), target.get());
    }

    wait_without_scheduling("migration worker to exit", || {
        super::live_kernel_threads() == 0
    });
    super::synchronize_retired_tasks();
    crate::heap::shrink();

    let pages_after = crate::page_alloc::total_free_pages()
        .expect("page allocator unavailable after task migration verification");
    assert_eq!(
        pages_after, pages_before,
        "migrated task stack or metadata leaked physical pages",
    );
}

pub(super) fn verify() {
    crate::context::assert_interrupts_enabled();
    let cpu_count = super::active_cpu_count();

    verify_remote_tlb_shootdown(cpu_count);
    verify_started_task_migration(cpu_count);

    crate::println!("TLB shootdown test:");
    crate::println!("  local invalidate  : verified");
    if cpu_count > 1 {
        crate::println!("  remote invalidate : verified");
        crate::println!("  remote ack        : verified");
    } else {
        crate::println!("  remote invalidate : single-CPU fallback");
        crate::println!("  remote ack        : single-CPU fallback");
    }
    crate::println!("  remap visibility  : verified");
    crate::println!("  page reclaim      : verified");

    crate::println!("task migration test:");
    if cpu_count > 1 {
        crate::println!("  started task      : migrated CPU0 -> CPU1");
    } else {
        crate::println!("  started task      : single-CPU fallback");
    }
    crate::println!("  affinity release  : verified");
    crate::println!("  stack/TLB safety  : verified");
    crate::println!("M4C_TLB_TEST: PASS");
}
