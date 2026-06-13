use core::{
    cell::UnsafeCell,
    hint::spin_loop,
    mem::MaybeUninit,
    sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering, fence},
};

use myos_mm::{AddressSpaceId, PAGE_SIZE, TlbFlush, TlbScope, TlbShootdown, VirtAddr, VirtRange};

use crate::{
    lockdep::{LockClass, LockRank},
    smp::{CpuId, MAX_CPUS},
    tracked_spin::TrackedSpinLock,
};

const SHOOTDOWN_TIMEOUT_SECONDS: u64 = 5;
const RANGE_PAGE_FLUSH_LIMIT: usize = 32;

const REQUEST_FREE: u8 = 0;
const REQUEST_PUBLISHING: u8 = 1;
const REQUEST_READY: u8 = 2;

#[derive(Clone, Copy, Debug)]
struct TlbRequest {
    shootdown: TlbShootdown,
    targets: usize,
}

impl TlbRequest {
    const fn id(self) -> u64 {
        self.shootdown.generation()
    }

    const fn flush(self) -> TlbFlush {
        self.shootdown.flush()
    }
}

struct TlbRequestSlot {
    state: AtomicU8,
    payload: UnsafeCell<MaybeUninit<TlbRequest>>,
    completed: AtomicUsize,

    // Atomic diagnostic mirrors. Panic paths may read these without racing
    // payload publication or slot reuse.
    diagnostic_id: AtomicU64,
    diagnostic_targets: AtomicUsize,
    diagnostic_kind: AtomicU8,
    diagnostic_start: AtomicUsize,
    diagnostic_end: AtomicUsize,
}

// REQUEST_PUBLISHING grants the serializer owner exclusive write access.
// REQUEST_READY publishes an immutable request to IPI readers. The owner does
// not return the slot to REQUEST_FREE until every target published completion.
unsafe impl Sync for TlbRequestSlot {}

impl TlbRequestSlot {
    const fn new() -> Self {
        Self {
            state: AtomicU8::new(REQUEST_FREE),
            payload: UnsafeCell::new(MaybeUninit::uninit()),
            completed: AtomicUsize::new(0),
            diagnostic_id: AtomicU64::new(0),
            diagnostic_targets: AtomicUsize::new(0),
            diagnostic_kind: AtomicU8::new(0),
            diagnostic_start: AtomicUsize::new(0),
            diagnostic_end: AtomicUsize::new(0),
        }
    }

    fn begin_publish(&self) {
        assert_eq!(
            self.state.compare_exchange(
                REQUEST_FREE,
                REQUEST_PUBLISHING,
                Ordering::AcqRel,
                Ordering::Acquire,
            ),
            Ok(REQUEST_FREE),
            "TLB request slot was not free while holding the serializer",
        );
        self.completed.store(0, Ordering::Relaxed);
    }

    fn publish(&self, request: TlbRequest) {
        assert_eq!(
            self.state.load(Ordering::Acquire),
            REQUEST_PUBLISHING,
            "TLB request was published without slot ownership",
        );

        let (kind, start, end) = describe_flush(request.flush());
        self.diagnostic_id.store(request.id(), Ordering::Relaxed);
        self.diagnostic_targets
            .store(request.targets, Ordering::Relaxed);
        self.diagnostic_kind.store(kind, Ordering::Relaxed);
        self.diagnostic_start.store(start, Ordering::Relaxed);
        self.diagnostic_end.store(end, Ordering::Relaxed);

        // SAFETY: REQUEST_PUBLISHING gives the serializer owner exclusive
        // write access. Readers cannot access the payload until REQUEST_READY.
        unsafe {
            (*self.payload.get()).write(request);
        }
        self.state.store(REQUEST_READY, Ordering::Release);
    }

    fn request(&self) -> TlbRequest {
        assert_eq!(
            self.state.load(Ordering::Acquire),
            REQUEST_READY,
            "TLB IPI observed no published request",
        );

        // SAFETY: the Acquire state load observes the initialized immutable
        // request. The slot cannot be reused until this CPU publishes its
        // completion bit after it has stopped accessing the payload.
        unsafe { *(*self.payload.get()).assume_init_ref() }
    }

    fn complete_for(&self, cpu: CpuId) {
        let bit = cpu_bit(cpu);
        let previous = self.completed.fetch_or(bit, Ordering::AcqRel);
        assert_eq!(
            previous & bit,
            0,
            "CPU completed one TLB request twice: request={} cpu={}",
            self.diagnostic_id.load(Ordering::Acquire),
            cpu.get(),
        );
    }

    fn release(&self, expected_targets: usize) {
        assert_eq!(
            self.completed.load(Ordering::Acquire),
            expected_targets,
            "TLB request released before every target completed",
        );
        assert_eq!(
            self.state.swap(REQUEST_FREE, Ordering::AcqRel),
            REQUEST_READY,
            "TLB request slot had an invalid release state",
        );
    }
}

static SHOOTDOWN_SERIALIZER: TrackedSpinLock<()> = TrackedSpinLock::new_with_class(
    (),
    LockClass::new("tlb_shootdown_serializer", LockRank::CrossCpu, 2),
);

static REQUEST: TlbRequestSlot = TlbRequestSlot::new();
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(0);

static REMOTE_FLUSH_COUNTS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static REMOTE_FULL_FLUSH_COUNTS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static REMOTE_PAGE_FLUSH_COUNTS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static REMOTE_RANGE_FLUSH_COUNTS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];

static COMPLETED_SHOOTDOWNS: AtomicU64 = AtomicU64::new(0);

/// Discards every cached translation for the shared kernel address space.
pub fn shootdown_kernel_all() {
    shootdown(TlbFlush::All {
        scope: TlbScope::AddressSpace(AddressSpaceId::KERNEL),
    });
}

/// Discards one page translation for the shared kernel address space.
#[cfg_attr(not(debug_assertions), allow(dead_code))]
pub fn shootdown_kernel_page(address: VirtAddr) {
    assert!(
        address.is_aligned(PAGE_SIZE),
        "kernel TLB page request is not page-aligned: address={:#x}",
        address.get(),
    );
    shootdown(TlbFlush::Page {
        scope: TlbScope::AddressSpace(AddressSpaceId::KERNEL),
        address,
    });
}

/// Discards translations in one page-aligned half-open kernel range.
#[cfg_attr(not(debug_assertions), allow(dead_code))]
pub fn shootdown_kernel_range(range: VirtRange) {
    assert!(
        range.is_page_aligned(),
        "kernel TLB range request is not page-aligned: start={:#x} end={:#x}",
        range.start().get(),
        range.end().get(),
    );
    shootdown(TlbFlush::Range {
        scope: TlbScope::AddressSpace(AddressSpaceId::KERNEL),
        range,
    });
}

/// Executes one synchronous TLB request.
///
/// M5 has one shared kernel page table, so `AllCpus` and the kernel
/// `AddressSpaceId` target every online/IPI-ready CPU. Non-kernel address-space
/// scopes are deliberately rejected until process address spaces maintain an
/// `active_cpus` mask.
pub fn shootdown(flush: TlbFlush) {
    validate_flush(flush);

    let online = crate::smp::online_cpu_mask();

    // Early VM self-tests call this before the scheduler and secondary CPUs
    // exist. Keep the uniprocessor path strictly local and allocation-free.
    if online.count_ones() <= 1 || matches!(flush_scope(flush), TlbScope::Local) {
        flush_local(flush);
        COMPLETED_SHOOTDOWNS.fetch_add(1, Ordering::Relaxed);
        return;
    }

    crate::context::assert_interrupts_enabled();
    crate::context::assert_task_context();

    // Pin before sampling the caller CPU and lifecycle masks.
    let migration_guard = crate::task::MigrationGuard::new();
    let current = crate::smp::current_cpu_id();
    let current_bit = cpu_bit(current);

    let online = crate::smp::online_cpu_mask();
    let ready = crate::smp::ipi_ready_cpu_mask();
    assert_ne!(
        online & current_bit,
        0,
        "TLB shootdown attempted from an offline CPU: cpu={} online={online:#x}",
        current.get(),
    );
    assert_eq!(
        ready & online,
        online,
        "TLB shootdown attempted before every online CPU became IPI-ready: \
         online={online:#x} ready={ready:#x}",
    );

    let targets = target_mask(flush, online, current_bit);
    assert_ne!(
        targets, 0,
        "multi-CPU TLB request lost every remote target after migration was disabled",
    );

    // Interrupts remain enabled while contending and waiting. This CPU must be
    // able to service another CPU's shootdown/call-function request.
    let serializer = acquire_serializer(current);

    let request_id = NEXT_REQUEST_ID
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    assert_ne!(request_id, 0, "TLB request ID wrapped to zero");

    REQUEST.begin_publish();
    REQUEST.publish(TlbRequest {
        shootdown: TlbShootdown::new(flush, request_id),
        targets,
    });

    // Page-table stores before this function and request publication above
    // must be visible before a target observes the mailbox message.
    fence(Ordering::SeqCst);

    for_each_cpu(targets, crate::smp::send_tlb_shootdown);

    // The caller participates in the request but is not part of the remote
    // completion mask.
    flush_local(flush);

    wait_for_completion(request_id, targets);
    fence(Ordering::Acquire);

    REQUEST.release(targets);
    COMPLETED_SHOOTDOWNS.fetch_add(1, Ordering::Relaxed);

    drop(serializer);
    drop(migration_guard);
}

/// Handles the TLB component of one mailbox batch on the current CPU.
pub fn handle_shootdown_ipi() {
    let cpu = crate::smp::current_cpu_id();
    let request = REQUEST.request();
    let bit = cpu_bit(cpu);

    assert_ne!(
        request.targets & bit,
        0,
        "CPU received a TLB request that did not target it: \
         request={} cpu={} targets={:#x}",
        request.id(),
        cpu.get(),
        request.targets,
    );

    flush_local(request.flush());
    fence(Ordering::SeqCst);

    REMOTE_FLUSH_COUNTS[cpu.get()].fetch_add(1, Ordering::Relaxed);
    match request.flush() {
        TlbFlush::All { .. } => {
            REMOTE_FULL_FLUSH_COUNTS[cpu.get()].fetch_add(1, Ordering::Relaxed);
        }
        TlbFlush::Page { .. } => {
            REMOTE_PAGE_FLUSH_COUNTS[cpu.get()].fetch_add(1, Ordering::Relaxed);
        }
        TlbFlush::Range { .. } => {
            REMOTE_RANGE_FLUSH_COUNTS[cpu.get()].fetch_add(1, Ordering::Relaxed);
        }
    }

    REQUEST.complete_for(cpu);
}

fn acquire_serializer(current: CpuId) -> crate::tracked_spin::TrackedSpinLockGuard<'static, ()> {
    let deadline = timeout_deadline();

    loop {
        if let Some(serializer) = SHOOTDOWN_SERIALIZER.try_lock() {
            return serializer;
        }

        if deadline_reached(crate::arch::time::counter(), deadline) {
            dump();
            crate::ipi::dump();
            crate::smp::dump_cpu_states();
            panic!(
                "timed out acquiring the TLB shootdown serializer: cpu={}",
                current.get(),
            );
        }
        spin_loop();
    }
}

fn wait_for_completion(request_id: u64, targets: usize) {
    let deadline = timeout_deadline();

    loop {
        let completed = REQUEST.completed.load(Ordering::Acquire);
        assert_eq!(
            completed & !targets,
            0,
            "TLB request completed on an unexpected CPU: \
             request={request_id} targets={targets:#x} completed={completed:#x}",
        );
        if completed == targets {
            return;
        }

        if deadline_reached(crate::arch::time::counter(), deadline) {
            dump();
            crate::ipi::dump();
            crate::smp::dump_cpu_states();
            panic!(
                "TLB request timed out: request={request_id} targets={targets:#x} \
                 completed={completed:#x} pending={:#x}",
                targets & !completed,
            );
        }
        spin_loop();
    }
}

fn validate_flush(flush: TlbFlush) {
    match flush {
        TlbFlush::All { scope } => validate_scope(scope),
        TlbFlush::Page { scope, address } => {
            validate_scope(scope);
            assert!(
                address.is_aligned(PAGE_SIZE),
                "TLB page request is not page-aligned: address={:#x}",
                address.get(),
            );
        }
        TlbFlush::Range { scope, range } => {
            validate_scope(scope);
            assert!(
                range.is_page_aligned(),
                "TLB range request is not page-aligned: start={:#x} end={:#x}",
                range.start().get(),
                range.end().get(),
            );
        }
    }
}

fn validate_scope(scope: TlbScope) {
    if let TlbScope::AddressSpace(address_space) = scope {
        assert_eq!(
            address_space,
            AddressSpaceId::KERNEL,
            "per-address-space TLB shootdown requires an active CPU mask: asid={}",
            address_space.get(),
        );
    }
}

fn target_mask(flush: TlbFlush, online: usize, current_bit: usize) -> usize {
    match flush_scope(flush) {
        TlbScope::Local => 0,
        TlbScope::AllCpus => online & !current_bit,
        TlbScope::AddressSpace(address_space) => {
            assert_eq!(address_space, AddressSpaceId::KERNEL);
            online & !current_bit
        }
    }
}

const fn flush_scope(flush: TlbFlush) -> TlbScope {
    match flush {
        TlbFlush::All { scope } | TlbFlush::Page { scope, .. } | TlbFlush::Range { scope, .. } => {
            scope
        }
    }
}

fn flush_local(flush: TlbFlush) {
    match flush {
        TlbFlush::All { .. } => crate::arch::memory::paging::flush_all(),
        TlbFlush::Page { address, .. } => {
            crate::arch::memory::paging::flush_page(address);
        }
        TlbFlush::Range { range, .. } => flush_range_local(range),
    }
}

fn flush_range_local(range: VirtRange) {
    if range.is_empty() {
        return;
    }

    let pages = range.size() / PAGE_SIZE;
    if pages > RANGE_PAGE_FLUSH_LIMIT {
        crate::arch::memory::paging::flush_all();
        return;
    }

    let mut address = range.start();
    while address.get() < range.end().get() {
        crate::arch::memory::paging::flush_page(address);
        address = address
            .checked_add(PAGE_SIZE)
            .expect("TLB range iteration overflowed");
    }
}

const fn describe_flush(flush: TlbFlush) -> (u8, usize, usize) {
    match flush {
        TlbFlush::All { .. } => (1, 0, 0),
        TlbFlush::Page { address, .. } => {
            (2, address.get(), address.get().saturating_add(PAGE_SIZE))
        }
        TlbFlush::Range { range, .. } => (3, range.start().get(), range.end().get()),
    }
}

fn flush_kind_name(kind: u8) -> &'static str {
    match kind {
        1 => "all",
        2 => "page",
        3 => "range",
        _ => "unknown",
    }
}

fn for_each_cpu(mask: usize, mut function: impl FnMut(CpuId)) {
    for logical in 0..crate::smp::discovered_cpu_count() {
        let bit = 1_usize << logical;
        if mask & bit == 0 {
            continue;
        }
        function(CpuId::new(logical).expect("TLB target exceeds MAX_CPUS"));
    }
}

fn cpu_bit(cpu: CpuId) -> usize {
    1_usize
        .checked_shl(cpu.get() as u32)
        .expect("CPU ID exceeds TLB target-mask width")
}

fn timeout_deadline() -> u64 {
    let cycles = crate::time::clock_frequency_hz()
        .checked_mul(SHOOTDOWN_TIMEOUT_SECONDS)
        .expect("TLB shootdown timeout overflowed");
    crate::arch::time::counter().wrapping_add(cycles)
}

fn deadline_reached(now: u64, deadline: u64) -> bool {
    now.wrapping_sub(deadline) < (1_u64 << 63)
}

pub fn dump() {
    let state = REQUEST.state.load(Ordering::Acquire);
    let state_name = match state {
        REQUEST_FREE => "free",
        REQUEST_PUBLISHING => "publishing",
        REQUEST_READY => "ready",
        _ => "invalid",
    };

    crate::println!("TLB request:");
    crate::println!(
        "  state={} id={} kind={} targets={:#x} completed={:#x} \
         start={:#x} end={:#x}",
        state_name,
        REQUEST.diagnostic_id.load(Ordering::Acquire),
        flush_kind_name(REQUEST.diagnostic_kind.load(Ordering::Acquire)),
        REQUEST.diagnostic_targets.load(Ordering::Acquire),
        REQUEST.completed.load(Ordering::Acquire),
        REQUEST.diagnostic_start.load(Ordering::Acquire),
        REQUEST.diagnostic_end.load(Ordering::Acquire),
    );
}

#[cfg(debug_assertions)]
pub fn completed_shootdowns() -> u64 {
    COMPLETED_SHOOTDOWNS.load(Ordering::Acquire)
}

#[cfg(debug_assertions)]
pub fn remote_flush_count(cpu: CpuId) -> u64 {
    REMOTE_FLUSH_COUNTS[cpu.get()].load(Ordering::Acquire)
}

#[cfg(debug_assertions)]
pub fn verify_request_model() {
    crate::context::assert_task_context();
    crate::context::assert_interrupts_enabled();

    let completed_before = completed_shootdowns();
    let mut remote_before = [0_u64; MAX_CPUS];
    for logical in 0..crate::smp::discovered_cpu_count() {
        let cpu = CpuId::new(logical).expect("TLB verifier CPU exceeds MAX_CPUS");
        remote_before[logical] = remote_flush_count(cpu);
    }

    shootdown_kernel_page(VirtAddr::new(0));
    shootdown_kernel_range(VirtRange::from_bounds(0, PAGE_SIZE * 2));

    assert_eq!(
        completed_shootdowns(),
        completed_before + 2,
        "TLB request verifier lost a completed request",
    );

    let targets = crate::smp::ipi_ready_cpu_mask() & !cpu_bit(crate::smp::current_cpu_id());
    for logical in 0..crate::smp::discovered_cpu_count() {
        let bit = 1_usize << logical;
        if targets & bit == 0 {
            continue;
        }

        let cpu = CpuId::new(logical).expect("TLB verifier target exceeds MAX_CPUS");
        assert_eq!(
            remote_flush_count(cpu),
            remote_before[logical] + 2,
            "remote CPU did not execute page and range TLB requests",
        );
    }

    assert_eq!(
        REQUEST.state.load(Ordering::Acquire),
        REQUEST_FREE,
        "TLB request verifier leaked the request slot",
    );

    crate::println!("TLB request v2 test:");
    crate::println!("  explicit request ID : verified");
    crate::println!("  target/completion mask: verified");
    crate::println!("  page request        : verified");
    crate::println!("  range request       : verified");
    crate::println!("  long-range fallback : {} pages", RANGE_PAGE_FLUSH_LIMIT);
}
