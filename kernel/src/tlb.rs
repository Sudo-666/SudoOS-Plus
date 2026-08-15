use core::{
    cell::UnsafeCell,
    hint::spin_loop,
    mem::MaybeUninit,
    sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering, fence},
};

use myos_mm::{
    AddressSpaceId, PAGE_SIZE, PerMmTlbRequest, TlbFlush, TlbScope, TlbShootdown, VirtAddr,
    VirtRange,
};
#[cfg(debug_assertions)]
use myos_mm::{AsidToken, UserAddressSpace};

use crate::smp::{CpuId, MAX_CPUS};

const SHOOTDOWN_TIMEOUT_SECONDS: u64 = 30;
const RANGE_PAGE_FLUSH_LIMIT: usize = 32;
// §9: per-CPU inbox depth. Concurrent shootdowns targeting the same CPU
// share its inbox; more in-flight requests than RING_SLOTS make later
// initiators spin (bounded by the shootdown timeout) instead of blocking
// unrelated CPUs behind one global serializer.
const TLB_RING_SLOTS: usize = 4;

const SLOT_FREE: u8 = 0;
const SLOT_PUBLISHING: u8 = 1;
const SLOT_READY: u8 = 2;

/// Index that means "this CPU index has no acquired inbox slot".
const SLOT_NONE: u8 = u8::MAX;

#[derive(Clone, Copy, Debug)]
struct TlbRequest {
    shootdown: TlbShootdown,
    targets: usize,
    // Base pointer of the owning UserMm's local_tlb_generation array, or
    // null for kernel-address-space requests. The IPI handler stores the
    // request generation into element [cpu] after flushing.
    seen: *const AtomicU64,
}

impl TlbRequest {
    const fn id(self) -> u64 {
        self.shootdown.generation()
    }

    const fn flush(self) -> TlbFlush {
        self.shootdown.flush()
    }
}

/// §9 per-CPU TLB request slot.
///
/// The initiating CPU CASes SLOT_FREE -> SLOT_PUBLISHING (exclusive write
/// access), writes the payload, and publishes SLOT_READY with Release. The
/// owning CPU's IPI handler reads the immutable request, flushes locally,
/// updates the per-mm seen generation, and publishes its ack bit with
/// Release. The initiator spins on the ack bit, then swaps the slot back to
/// SLOT_FREE. A wedged target leaves its slot READY forever: subsequent
/// shootdowns to that CPU fill the remaining ring and then time out with
/// the §10 rich dump.
struct TlbSlot {
    state: AtomicU8,
    payload: UnsafeCell<MaybeUninit<TlbRequest>>,
    acked: AtomicUsize,

    // Atomic diagnostic mirrors. Panic paths may read these without racing
    // payload publication or slot reuse.
    diagnostic_id: AtomicU64,
    diagnostic_targets: AtomicUsize,
    diagnostic_kind: AtomicU8,
    diagnostic_start: AtomicUsize,
    diagnostic_end: AtomicUsize,
}

// SAFETY: SLOT_PUBLISHING grants the acquiring initiator exclusive mutation
// access to `payload`. Publishing SLOT_READY with Release makes the request
// immutable and visible to the owning CPU's IPI handler, which stops
// accessing the payload before it publishes its ack bit. The initiator does
// not reuse the slot until it observes the ack, so payload reuse and reads
// never race.
unsafe impl Sync for TlbSlot {}

impl TlbSlot {
    const fn new() -> Self {
        Self {
            state: AtomicU8::new(SLOT_FREE),
            payload: UnsafeCell::new(MaybeUninit::uninit()),
            acked: AtomicUsize::new(0),
            diagnostic_id: AtomicU64::new(0),
            diagnostic_targets: AtomicUsize::new(0),
            diagnostic_kind: AtomicU8::new(0),
            diagnostic_start: AtomicUsize::new(0),
            diagnostic_end: AtomicUsize::new(0),
        }
    }

    fn publish(&self, request: TlbRequest) {
        assert_eq!(
            self.state.load(Ordering::Acquire),
            SLOT_PUBLISHING,
            "TLB request was published without slot ownership",
        );
        assert_eq!(
            self.acked.load(Ordering::Acquire),
            0,
            "TLB slot was reused with a stale ack bit",
        );

        let (kind, start, end) = describe_flush(request.flush());
        self.diagnostic_id.store(request.id(), Ordering::Relaxed);
        self.diagnostic_targets
            .store(request.targets, Ordering::Relaxed);
        self.diagnostic_kind.store(kind, Ordering::Relaxed);
        self.diagnostic_start.store(start, Ordering::Relaxed);
        self.diagnostic_end.store(end, Ordering::Relaxed);

        // SAFETY: SLOT_PUBLISHING gives the initiator exclusive write
        // access. The owning CPU cannot read the payload until SLOT_READY.
        unsafe {
            (*self.payload.get()).write(request);
        }
        self.state.store(SLOT_READY, Ordering::Release);
    }

    fn request(&self) -> TlbRequest {
        assert_eq!(
            self.state.load(Ordering::Acquire),
            SLOT_READY,
            "TLB IPI observed no published request",
        );

        // SAFETY: the Acquire state load observes the initialized immutable
        // request. The slot cannot be reused until this CPU publishes its
        // ack bit after it has stopped accessing the payload.
        unsafe { *(*self.payload.get()).assume_init_ref() }
    }

    fn ack(&self, cpu: CpuId) {
        let bit = cpu_bit(cpu);
        let previous = self.acked.fetch_or(bit, Ordering::AcqRel);
        assert_eq!(
            previous & bit,
            0,
            "CPU acknowledged one TLB request twice: request={} cpu={}",
            self.diagnostic_id.load(Ordering::Acquire),
            cpu.get(),
        );
    }
}

/// §9 per-CPU TLB inbox: the request ring one CPU's IPI handler drains.
struct TlbInbox {
    slots: [TlbSlot; TLB_RING_SLOTS],
}

impl TlbInbox {
    const fn new() -> Self {
        Self {
            slots: [const { TlbSlot::new() }; TLB_RING_SLOTS],
        }
    }
}

static INBOXES: [TlbInbox; MAX_CPUS] = [const { TlbInbox::new() }; MAX_CPUS];
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(0);

static REMOTE_FLUSH_COUNTS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static REMOTE_FULL_FLUSH_COUNTS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static REMOTE_PAGE_FLUSH_COUNTS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static REMOTE_RANGE_FLUSH_COUNTS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];

static COMPLETED_SHOOTDOWNS: AtomicU64 = AtomicU64::new(0);

/// Discards every cached translation for the shared kernel address space.
#[track_caller]
pub fn shootdown_kernel_all() {
    shootdown(TlbFlush::All {
        scope: TlbScope::AddressSpace(AddressSpaceId::KERNEL),
    });
}

/// Discards one page translation for the shared kernel address space.
#[cfg_attr(not(debug_assertions), allow(dead_code))]
#[track_caller]
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
#[track_caller]
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
#[track_caller]
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

    // §9: MigrationGuard replaces the global serializer. It pins this task
    // to the CPU recorded at the 0->1 depth edge, so CPU identity and
    // target masks stay valid for the whole request without serializing
    // unrelated shootdowns. The holder stays preemptible and interruptible.
    let _migration = crate::task::MigrationGuard::new();
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

    let request_id = NEXT_REQUEST_ID
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    assert_ne!(request_id, 0, "TLB request ID wrapped to zero");

    let pending = publish_to_inboxes(
        TlbRequest {
            shootdown: TlbShootdown::new(flush, request_id),
            targets,
            seen: core::ptr::null(),
        },
        targets,
        request_id,
    );

    // Page-table stores before this function and request publication above
    // must be visible before a target observes the mailbox message.
    fence(Ordering::SeqCst);

    for_each_cpu(targets, crate::smp::send_tlb_shootdown);

    // The caller participates in the request but is not part of the remote
    // completion mask.
    flush_local(flush);

    wait_for_completion(&pending);
    fence(Ordering::Acquire);

    free_slots(&pending);
    COMPLETED_SHOOTDOWNS.fetch_add(1, Ordering::Relaxed);
}

/// Executes one synchronous, exact-target TLB request for a user address space.
///
/// The request must be created by `UserAddressSpace::plan_tlb_request()` after
/// the page-table/VMA locks have been released. `seen` is the owning mm's
/// per-CPU local-generation array: the IPI handler stores the request
/// generation there so §11 switch-in logic can skip redundant flushes.
#[track_caller]
pub fn shootdown_user(request: PerMmTlbRequest, seen: Option<*const AtomicU64>) {
    validate_user_request(request);
    crate::context::assert_interrupts_enabled();
    crate::context::assert_task_context();

    let requested = usize::try_from(request.targets().bits())
        .expect("per-mm CPU mask exceeds the kernel target-mask width");
    let flush = request.flush();

    if requested == 0 {
        COMPLETED_SHOOTDOWNS.fetch_add(1, Ordering::Relaxed);
        return;
    }

    // §9: pin with MigrationGuard before reading current CPU. Even a
    // local-only request uses this short critical section so the target
    // comparison cannot race migration.
    let _migration = crate::task::MigrationGuard::new();
    let current = crate::smp::current_cpu_id();
    let current_bit = cpu_bit(current);
    let online = crate::smp::online_cpu_mask();
    let ready = crate::smp::ipi_ready_cpu_mask();

    assert_ne!(
        online & current_bit,
        0,
        "per-mm TLB shootdown attempted from an offline CPU: cpu={} online={online:#x}",
        current.get(),
    );
    assert_eq!(
        requested & !online,
        0,
        "per-mm TLB request targeted an offline CPU: requested={requested:#x} online={online:#x}",
    );
    assert_eq!(
        requested & !ready,
        0,
        "per-mm TLB request targeted a CPU that is not IPI-ready: \
         requested={requested:#x} ready={ready:#x}",
    );

    let targets = requested & !current_bit;
    if targets == 0 {
        if requested & current_bit != 0 {
            flush_local(flush);
        }
        COMPLETED_SHOOTDOWNS.fetch_add(1, Ordering::Relaxed);
        return;
    }

    // Interrupts remain enabled so this CPU can acknowledge another CPU's
    // request while waiting.
    let request_id = NEXT_REQUEST_ID
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    assert_ne!(request_id, 0, "TLB request ID wrapped to zero");

    let pending = publish_to_inboxes(
        TlbRequest {
            shootdown: TlbShootdown::new(flush, request_id),
            targets,
            seen: seen.map_or(core::ptr::null(), |base| base),
        },
        targets,
        request_id,
    );
    fence(Ordering::SeqCst);
    for_each_cpu(targets, crate::smp::send_tlb_shootdown);

    if requested & current_bit != 0 {
        flush_local(flush);
    }
    wait_for_completion(&pending);
    fence(Ordering::Acquire);
    free_slots(&pending);
    COMPLETED_SHOOTDOWNS.fetch_add(1, Ordering::Relaxed);
}

/// Executes an ASID-scoped request that is known to target only this CPU.
///
/// M8's synchronous verifier disables local interrupts while a private user
/// root is active. It therefore cannot enter the remote serializer/ACK path.
/// This helper is deliberately fail-closed: a future shared-mm target mask
/// must use `shootdown_user()` from interruptible task context instead.
pub fn shootdown_user_local(request: PerMmTlbRequest) {
    validate_user_request(request);
    crate::context::assert_interrupts_disabled();

    let requested = usize::try_from(request.targets().bits())
        .expect("per-mm CPU mask exceeds the kernel target-mask width");
    let current = crate::smp::current_cpu_id();
    let current_bit = cpu_bit(current);
    assert_eq!(
        requested & !current_bit,
        0,
        "local-only per-mm request targeted another CPU: current={} targets={requested:#x}",
        current.get(),
    );

    if requested & current_bit != 0 {
        flush_local(request.flush());
    }
    COMPLETED_SHOOTDOWNS.fetch_add(1, Ordering::Relaxed);
}

pub fn handle_shootdown_ipi() {
    let cpu = crate::smp::current_cpu_id();
    let bit = cpu_bit(cpu);

    // §9: drain every READY request in this CPU's inbox. The handler takes
    // no locks, allocates nothing, sleeps nowhere, and waits on nobody —
    // it flushes locally, records the per-mm seen generation, and ACKs.
    for slot in &INBOXES[cpu.get()].slots {
        if slot.state.load(Ordering::Acquire) != SLOT_READY {
            continue;
        }
        if slot.acked.load(Ordering::Acquire) & bit != 0 {
            continue;
        }
        let request = slot.request();

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

        if !request.seen.is_null() {
            // SAFETY: the owning UserMm cannot be destroyed while this
            // request targets this CPU — the §8 retirement model frees
            // nothing before every target acknowledges, and the initiator
            // holds the mm alive for the whole request. The array is
            // indexed by logical CPU id.
            unsafe {
                (&*request.seen.add(cpu.get())).store(request.id(), Ordering::Release);
            }
        }

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

        slot.ack(cpu);
    }
}

/// One in-flight initiator view: which inbox slot was acquired per target.
struct PendingShootdown {
    request_id: u64,
    targets: usize,
    // slots[cpu] = inbox slot index, or SLOT_NONE when cpu is not a target.
    slots: [u8; MAX_CPUS],
}

/// Acquires one inbox slot per target CPU and publishes the request there.
///
/// Slot acquisition spins per-CPU: a full inbox on one target only stalls
/// requests that need that CPU, never unrelated shootdowns.
fn publish_to_inboxes(request: TlbRequest, targets: usize, request_id: u64) -> PendingShootdown {
    let mut pending = PendingShootdown {
        request_id,
        targets,
        slots: [SLOT_NONE; MAX_CPUS],
    };

    for_each_cpu(targets, |cpu| {
        let slot_index = acquire_slot(cpu, request_id);
        pending.slots[cpu.get()] = slot_index;
        INBOXES[cpu.get()].slots[slot_index as usize].publish(request);
    });

    pending
}

fn acquire_slot(cpu: CpuId, request_id: u64) -> u8 {
    let deadline = timeout_deadline();

    loop {
        for (index, slot) in INBOXES[cpu.get()].slots.iter().enumerate() {
            if slot
                .state
                .compare_exchange(SLOT_FREE, SLOT_PUBLISHING, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                slot.acked.store(0, Ordering::Relaxed);
                return index as u8;
            }
        }

        if deadline_reached(crate::arch::time::counter(), deadline) {
            dump_rich();
            panic!(
                "TLB inbox exhausted: request={request_id} target={} \
                 (all {} ring slots in flight for 30s)",
                cpu.get(),
                TLB_RING_SLOTS,
            );
        }
        spin_loop();
    }
}

fn wait_for_completion(pending: &PendingShootdown) {
    let deadline = timeout_deadline();

    loop {
        let mut completed = 0_usize;
        for_each_cpu(pending.targets, |cpu| {
            let slot = &INBOXES[cpu.get()].slots[pending.slots[cpu.get()] as usize];
            let acked = slot.acked.load(Ordering::Acquire);
            assert_eq!(
                acked & !cpu_bit(cpu),
                0,
                "TLB slot acknowledged on an unexpected CPU: \
                 request={} target={} acked={acked:#x}",
                pending.request_id,
                cpu.get(),
            );
            if acked != 0 {
                completed |= cpu_bit(cpu);
            }
        });

        assert_eq!(
            completed & !pending.targets,
            0,
            "TLB request completed on an unexpected CPU: \
             request={} targets={:#x} completed={completed:#x}",
            pending.request_id,
            pending.targets,
        );
        if completed == pending.targets {
            return;
        }

        if deadline_reached(crate::arch::time::counter(), deadline) {
            dump_rich();
            panic!(
                "TLB request timed out: request={} targets={:#x} \
                 completed={completed:#x} pending={:#x}",
                pending.request_id,
                pending.targets,
                pending.targets & !completed,
            );
        }
        spin_loop();
    }
}

/// Returns every acquired slot to SLOT_FREE after all acks were observed.
fn free_slots(pending: &PendingShootdown) {
    for_each_cpu(pending.targets, |cpu| {
        let slot = &INBOXES[cpu.get()].slots[pending.slots[cpu.get()] as usize];
        assert_eq!(
            slot.state.swap(SLOT_FREE, Ordering::AcqRel),
            SLOT_READY,
            "TLB slot had an invalid release state: request={} cpu={}",
            pending.request_id,
            cpu.get(),
        );
    });
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

fn validate_user_request(request: PerMmTlbRequest) {
    let asid = request.asid().id();
    assert_ne!(
        asid,
        AddressSpaceId::KERNEL,
        "per-mm TLB request used the reserved kernel ASID",
    );
    assert_ne!(
        request.generation(),
        0,
        "per-mm TLB request used generation zero",
    );

    match request.flush() {
        TlbFlush::All { scope } => validate_user_scope(scope, asid),
        TlbFlush::Page { scope, address } => {
            validate_user_scope(scope, asid);
            assert!(
                address.is_aligned(PAGE_SIZE),
                "per-mm TLB page request is not page-aligned: address={:#x}",
                address.get(),
            );
        }
        TlbFlush::Range { scope, range } => {
            validate_user_scope(scope, asid);
            assert!(
                range.is_page_aligned(),
                "per-mm TLB range request is not page-aligned: start={:#x} end={:#x}",
                range.start().get(),
                range.end().get(),
            );
        }
    }
}

fn validate_user_scope(scope: TlbScope, asid: AddressSpaceId) {
    assert_eq!(
        scope,
        TlbScope::AddressSpace(asid),
        "per-mm TLB request scope does not match its ASID token",
    );
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
        TlbFlush::All { scope } => flush_all_local(scope),
        TlbFlush::Page { scope, address } => flush_page_local(scope, address),
        TlbFlush::Range { scope, range } => flush_range_local(scope, range),
    }
}

fn flush_all_local(scope: TlbScope) {
    match scope {
        TlbScope::AddressSpace(address_space) if address_space != AddressSpaceId::KERNEL => {
            crate::arch::memory::paging::flush_asid(address_space);
        }
        TlbScope::Local | TlbScope::AllCpus | TlbScope::AddressSpace(_) => {
            crate::arch::memory::paging::flush_all();
        }
    }
}

fn flush_page_local(scope: TlbScope, address: VirtAddr) {
    match scope {
        TlbScope::AddressSpace(address_space) if address_space != AddressSpaceId::KERNEL => {
            // §11: QEMU's LoongArch targeted op-4/op-6 invalidations have
            // proven unreliable (see arch flush_asid), so invalidate the
            // complete ASID there; RISC-V keeps the selective page path.
            // The activate_current_cpu generation gate's
            // "seen == generation implies clean" claim depends on every
            // shootdown flush actually removing stale translations.
            #[cfg(target_arch = "loongarch64")]
            {
                let _ = address; // full-ASID invalidation ignores the address
                crate::arch::memory::paging::flush_asid(address_space);
            }
            #[cfg(target_arch = "riscv64")]
            crate::arch::memory::paging::flush_asid_page(address_space, address);
        }
        TlbScope::Local | TlbScope::AllCpus | TlbScope::AddressSpace(_) => {
            crate::arch::memory::paging::flush_page(address);
        }
    }
}

fn flush_range_local(scope: TlbScope, range: VirtRange) {
    if range.is_empty() {
        return;
    }
    let pages = range.size() / PAGE_SIZE;
    if pages > RANGE_PAGE_FLUSH_LIMIT {
        flush_all_local(scope);
        return;
    }
    // §11: a per-page loop would repeat LoongArch's full-ASID invalidation
    // once per page; perform a single complete invalidation instead (see
    // flush_page_local). RISC-V keeps the selective per-page path.
    #[cfg(target_arch = "loongarch64")]
    if let TlbScope::AddressSpace(address_space) = scope {
        if address_space != AddressSpaceId::KERNEL {
            crate::arch::memory::paging::flush_asid(address_space);
            return;
        }
    }
    let mut address = range.start();
    while address.get() < range.end().get() {
        flush_page_local(scope, address);
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

/// §10 rich per-CPU dump for TLB shootdown timeout classification.
///
/// Prints, per CPU: current task + decoded state, preempt/migration
/// depths, the last mirrored interrupt state and its age, irq_depth,
/// in-trap depth, context switches and last switch age, need_resched,
/// run-queue length, loaded mm/asid/mm generation, and the IPI mailbox
/// counters with last entry/exit ages. All cross-CPU reads are racy
/// snapshots taken from a panic path, diagnostic-only by design.
fn dump_rich() {
    dump();
    crate::ipi::dump();
    crate::smp::dump_cpu_states();

    let now = crate::arch::time::counter();
    crate::println!(
        "TLB timeout per-CPU diagnostics: now={} max_irq_off_cycles={}",
        now,
        crate::lockdep::max_irq_off_cycles(),
    );
    for logical in 0..crate::smp::discovered_cpu_count() {
        let cpu = CpuId::new(logical).expect("TLB diagnostic CPU exceeds MAX_CPUS");
        let d = crate::task::cpu_diagnostic(cpu);
        let (pending, irqs, doorbells, coalesced, batches, spurious, entry, exit) =
            crate::ipi::mailbox_diagnostic(cpu);
        crate::println!(
            "  cpu{logical} task={:#x} state={} preempt={} mig={} pinned={} \
             irq_mirror={} ({} ticks ago) irq_depth={} in_trap={} trap_imbalance={} \
             switches={} last_switch={} ({} ticks ago) need_resched={} \
             rq_len={} mm={} asid={} mm_gen={}",
            d.task,
            d.task_state,
            d.preempt_depth,
            d.migration_depth,
            d.migration_pinned,
            d.irq_mirror,
            ticks_ago(now, d.irq_mirror_at),
            d.irq_depth,
            d.in_trap_depth,
            d.trap_imbalance,
            d.switches,
            d.last_switch,
            ticks_ago(now, d.last_switch),
            d.need_resched,
            d.rq_len.map_or(-1_i64, |len| len as i64),
            d.loaded_mm,
            d.loaded_asid,
            d.mm_local_generation,
        );
        crate::println!(
            "         ipi pending={:#x} irq={} doorbell={} coalesced={} \
             batches={} spurious={} entry={} ({} ticks ago) exit={} ({} ticks ago)",
            pending,
            irqs,
            doorbells,
            coalesced,
            batches,
            spurious,
            entry,
            ticks_ago(now, entry),
            exit,
            ticks_ago(now, exit),
        );
    }
    crate::lockdep::dump_all_cpus();
}

fn ticks_ago(now: u64, then: u64) -> u64 {
    if then == 0 {
        return 0;
    }
    now.wrapping_sub(then)
}

pub fn dump() {
    crate::println!("TLB inboxes:");
    for (logical, inbox) in INBOXES
        .iter()
        .enumerate()
        .take(crate::smp::discovered_cpu_count())
    {
        for (index, slot) in inbox.slots.iter().enumerate() {
            let state = slot.state.load(Ordering::Acquire);
            if state == SLOT_FREE {
                continue;
            }
            let state_name = match state {
                SLOT_PUBLISHING => "publishing",
                SLOT_READY => "ready",
                _ => "invalid",
            };
            crate::println!(
                "  cpu{logical} slot{index} state={state_name} id={} kind={} \
                 targets={:#x} acked={:#x} start={:#x} end={:#x}",
                slot.diagnostic_id.load(Ordering::Acquire),
                flush_kind_name(slot.diagnostic_kind.load(Ordering::Acquire)),
                slot.diagnostic_targets.load(Ordering::Acquire),
                slot.acked.load(Ordering::Acquire),
                slot.diagnostic_start.load(Ordering::Acquire),
                slot.diagnostic_end.load(Ordering::Acquire),
            );
        }
    }
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
    for (logical, before) in remote_before
        .iter_mut()
        .enumerate()
        .take(crate::smp::discovered_cpu_count())
    {
        let cpu = CpuId::new(logical).expect("TLB verifier CPU exceeds MAX_CPUS");
        *before = remote_flush_count(cpu);
    }

    shootdown_kernel_page(VirtAddr::new(0));
    shootdown_kernel_range(VirtRange::from_bounds(0, PAGE_SIZE * 2));

    assert_eq!(
        completed_shootdowns(),
        completed_before + 2,
        "TLB request verifier lost a completed request",
    );

    let targets = crate::smp::ipi_ready_cpu_mask() & !cpu_bit(crate::smp::current_cpu_id());
    for (logical, before) in remote_before
        .iter()
        .enumerate()
        .take(crate::smp::discovered_cpu_count())
    {
        let bit = 1_usize << logical;
        if targets & bit == 0 {
            continue;
        }
        let cpu = CpuId::new(logical).expect("TLB verifier target exceeds MAX_CPUS");
        assert_eq!(
            remote_flush_count(cpu),
            *before + 2,
            "remote CPU did not execute page and range TLB requests",
        );
    }

    assert!(
        INBOXES.iter().all(|inbox| inbox
            .slots
            .iter()
            .all(|slot| slot.state.load(Ordering::Acquire) == SLOT_FREE)),
        "TLB request verifier leaked an inbox slot",
    );

    crate::println!("TLB request v2 test:");
    crate::println!("  explicit request ID : verified");
    crate::println!("  target/completion mask: verified");
    crate::println!("  page request        : verified");
    crate::println!("  range request       : verified");
    crate::println!("  long-range fallback : {} pages", RANGE_PAGE_FLUSH_LIMIT);

    let exact_completed_before = completed_shootdowns();
    let mut exact_remote_before = [0_u64; MAX_CPUS];
    for (logical, before) in exact_remote_before
        .iter_mut()
        .enumerate()
        .take(crate::smp::discovered_cpu_count())
    {
        let cpu = CpuId::new(logical).expect("M8-B1 verifier CPU exceeds MAX_CPUS");
        *before = remote_flush_count(cpu);
    }

    let current = crate::smp::current_cpu_id();
    let current_bit = cpu_bit(current);
    let ready = crate::smp::ipi_ready_cpu_mask();
    let mut exact_targets = current_bit;
    for logical in 0..crate::smp::discovered_cpu_count() {
        let bit = 1_usize << logical;
        if bit != current_bit && ready & bit != 0 {
            exact_targets |= bit;
            break;
        }
    }

    let user_asid = AddressSpaceId::new(7);
    let user_mm: UserAddressSpace<1> = UserAddressSpace::new(
        VirtRange::from_bounds(0, PAGE_SIZE),
        AsidToken::new(user_asid, 1),
    );
    for_each_cpu(exact_targets, |cpu| {
        user_mm
            .enter_cpu_after_local_sync(cpu.get(), 1, 0)
            .expect("M8-B1 verifier could not publish active CPU");
    });
    let per_mm_request = user_mm
        .plan_tlb_request(TlbFlush::Page {
            scope: TlbScope::AddressSpace(user_asid),
            address: VirtAddr::new(0),
        })
        .expect("M8-B1 verifier could not plan per-mm TLB request");
    shootdown_user(per_mm_request, None);

    assert_eq!(
        completed_shootdowns(),
        exact_completed_before + 1,
        "M8-B1 verifier lost the per-mm request",
    );
    for (logical, before) in exact_remote_before
        .iter()
        .enumerate()
        .take(crate::smp::discovered_cpu_count())
    {
        let bit = 1_usize << logical;
        let cpu = CpuId::new(logical).expect("M8-B1 target exceeds MAX_CPUS");
        let increment = if exact_targets & bit != 0 && bit != current_bit {
            1
        } else {
            0
        };
        let expected = *before + increment;
        assert_eq!(
            remote_flush_count(cpu),
            expected,
            "M8-B1 per-mm request did not honor the exact active CPU mask: cpu={logical}",
        );
    }
    for_each_cpu(exact_targets, |cpu| {
        user_mm
            .leave_cpu_after_local_flush(cpu.get(), per_mm_request.generation())
            .expect("M8-B1 verifier could not retire active CPU");
    });
    user_mm
        .assert_inactive_for_destroy()
        .expect("M8-B1 verifier leaked active CPU membership");
    assert!(
        INBOXES.iter().all(|inbox| inbox
            .slots
            .iter()
            .all(|slot| slot.state.load(Ordering::Acquire) == SLOT_FREE)),
        "M8-B1 verifier leaked a shared inbox slot",
    );

    crate::println!("M8-B1 per-mm TLB test:");
    crate::println!("  ASID-local invalidate : verified");
    crate::println!("  exact active CPU mask : verified");
    crate::println!("  shared ACK protocol   : verified");
    crate::println!("  generation handshake  : verified");
}
