use core::{
    cell::UnsafeCell,
    hint::spin_loop,
    mem::MaybeUninit,
    sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering},
};

use crate::{
    ipi::IpiMessage,
    smp::{CpuId, MAX_CPUS},
    task::MigrationGuard,
};

const REQUEST_TIMEOUT_SECONDS: u64 = 5;

const SLOT_FREE: u8 = 0;
const SLOT_RESERVED: u8 = 1;
const SLOT_READY: u8 = 2;

pub type CallFunction = fn(usize);

#[derive(Clone, Copy)]
struct CallPayload {
    function: CallFunction,
    argument: usize,
    targets: usize,
    generation: u64,
}

struct CallRequestSlot {
    state: AtomicU8,
    payload: UnsafeCell<MaybeUninit<CallPayload>>,
    completed: AtomicUsize,
}

// SLOT_RESERVED gives one task exclusive write ownership. SLOT_READY publishes
// an immutable payload to IPI readers. The slot is not reused until every
// target has completed its callback.
unsafe impl Sync for CallRequestSlot {}

impl CallRequestSlot {
    const fn new() -> Self {
        Self {
            state: AtomicU8::new(SLOT_FREE),
            payload: UnsafeCell::new(MaybeUninit::uninit()),
            completed: AtomicUsize::new(0),
        }
    }

    fn reset(&self) {
        self.state.store(SLOT_FREE, Ordering::Release);
        self.completed.store(0, Ordering::Release);
    }

    fn try_reserve(&self) -> bool {
        self.state
            .compare_exchange(
                SLOT_FREE,
                SLOT_RESERVED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    fn publish(&self, payload: CallPayload) {
        assert_eq!(
            self.state.load(Ordering::Acquire),
            SLOT_RESERVED,
            "call-function request was published without slot ownership",
        );
        self.completed.store(0, Ordering::Relaxed);

        // SAFETY: SLOT_RESERVED grants this caller exclusive write access.
        // Readers cannot access payload until the Release store of SLOT_READY.
        unsafe {
            (*self.payload.get()).write(payload);
        }
        self.state.store(SLOT_READY, Ordering::Release);
    }

    fn payload(&self) -> CallPayload {
        assert_eq!(
            self.state.load(Ordering::Acquire),
            SLOT_READY,
            "call-function handler observed an unpublished request",
        );

        // SAFETY: the Acquire state load observes the initialized immutable
        // payload. It remains valid until every target publishes completion.
        unsafe { *(*self.payload.get()).assume_init_ref() }
    }

    fn complete_for(&self, cpu: CpuId) {
        let bit = cpu_bit(cpu);
        let previous = self.completed.fetch_or(bit, Ordering::AcqRel);
        assert_eq!(
            previous & bit,
            0,
            "CPU completed one call-function request twice: cpu={}",
            cpu.get(),
        );
    }

    fn release(&self, expected_targets: usize) {
        assert_eq!(
            self.completed.load(Ordering::Acquire),
            expected_targets,
            "call-function request released before every target completed",
        );
        assert_eq!(
            self.state.swap(SLOT_FREE, Ordering::AcqRel),
            SLOT_READY,
            "call-function request slot had an invalid release state",
        );
    }
}

static REQUESTS: [CallRequestSlot; MAX_CPUS] = [const { CallRequestSlot::new() }; MAX_CPUS];

static PENDING_SLOTS: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(0) }; MAX_CPUS];

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn initialize() {
    assert!(
        MAX_CPUS <= usize::BITS as usize,
        "call-function slot mask is narrower than MAX_CPUS",
    );

    for slot in &REQUESTS {
        slot.reset();
    }
    for pending in &PENDING_SLOTS {
        pending.store(0, Ordering::Release);
    }
    NEXT_GENERATION.store(0, Ordering::Release);
}

pub fn call_single(cpu: CpuId, function: CallFunction, argument: usize) {
    call_many(cpu_bit(cpu), function, argument);
}

pub fn call_many(targets: usize, function: CallFunction, argument: usize) {
    crate::context::assert_task_context();
    crate::context::assert_interrupts_enabled();

    // Pin before sampling the caller CPU. Otherwise a timer preemption could
    // migrate this task between reading current_cpu_id() and disabling
    // migration, turning the caller into one of its own targets.
    let _migration_guard = MigrationGuard::new();
    let current = crate::smp::current_cpu_id();
    let current_bit = cpu_bit(current);
    assert_ne!(targets, 0, "call-function request has no targets");
    assert_eq!(
        targets & current_bit,
        0,
        "synchronous call-function request cannot target its caller CPU",
    );

    let discovered_mask = discovered_mask();
    assert_eq!(
        targets & !discovered_mask,
        0,
        "call-function request targets an undiscovered CPU: targets={targets:#x} \
         discovered={discovered_mask:#x}",
    );

    let ready = crate::smp::ipi_ready_cpu_mask();
    assert_eq!(
        targets & !ready,
        0,
        "call-function request targets a CPU that is not IPI-ready: \
         targets={targets:#x} ready={ready:#x}",
    );

    // Interrupts remain enabled so this CPU can execute requests from peers.
    assert_eq!(crate::smp::current_cpu_id(), current);

    let slot_index = reserve_slot();
    let slot = &REQUESTS[slot_index];
    let generation = NEXT_GENERATION
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    assert_ne!(generation, 0, "call-function generation wrapped to zero");

    slot.publish(CallPayload {
        function,
        argument,
        targets,
        generation,
    });

    let request_bit = slot_bit(slot_index);
    for_each_cpu(targets, |cpu| {
        let previous = PENDING_SLOTS[cpu.get()].fetch_or(request_bit, Ordering::Release);
        assert_eq!(
            previous & request_bit,
            0,
            "call-function request queued twice: generation={generation} \
             slot={slot_index} cpu={}",
            cpu.get(),
        );
        crate::ipi::send(cpu, IpiMessage::CallFunction);
    });

    wait_for_completion(slot_index, generation, targets);
    slot.release(targets);
}

pub fn handle_current() {
    let cpu = crate::smp::current_cpu_id();
    let mut slots = PENDING_SLOTS[cpu.get()].swap(0, Ordering::AcqRel);

    while slots != 0 {
        let index = slots.trailing_zeros() as usize;
        let bit = slot_bit(index);
        slots &= !bit;

        let slot = &REQUESTS[index];
        let payload = slot.payload();
        assert_ne!(
            payload.targets & cpu_bit(cpu),
            0,
            "CPU received an untargeted call-function request: \
             generation={} slot={} cpu={} targets={:#x}",
            payload.generation,
            index,
            cpu.get(),
            payload.targets,
        );

        (payload.function)(payload.argument);
        slot.complete_for(cpu);
    }
}

fn reserve_slot() -> usize {
    let deadline = timeout_deadline();
    loop {
        for (index, slot) in REQUESTS.iter().enumerate() {
            if slot.try_reserve() {
                return index;
            }
        }

        if deadline_reached(crate::arch::time::counter(), deadline) {
            dump();
            crate::ipi::dump();
            crate::smp::dump_cpu_states();
            panic!("timed out reserving a call-function request slot");
        }
        spin_loop();
    }
}

fn wait_for_completion(slot_index: usize, generation: u64, targets: usize) {
    let slot = &REQUESTS[slot_index];
    let deadline = timeout_deadline();

    loop {
        let completed = slot.completed.load(Ordering::Acquire);
        assert_eq!(
            completed & !targets,
            0,
            "call-function completed on an unexpected CPU: generation={generation} \
             slot={slot_index} targets={targets:#x} completed={completed:#x}",
        );
        if completed == targets {
            return;
        }

        if deadline_reached(crate::arch::time::counter(), deadline) {
            dump();
            crate::ipi::dump();
            crate::smp::dump_cpu_states();
            panic!(
                "call-function timed out: generation={generation} slot={slot_index} \
                 targets={targets:#x} completed={completed:#x} pending={:#x}",
                pending_target_mask(slot_index),
            );
        }
        spin_loop();
    }
}

fn pending_target_mask(slot_index: usize) -> usize {
    let request_bit = slot_bit(slot_index);
    let mut targets = 0_usize;
    for logical in 0..crate::smp::discovered_cpu_count() {
        if PENDING_SLOTS[logical].load(Ordering::Acquire) & request_bit != 0 {
            targets |= 1_usize << logical;
        }
    }
    targets
}

fn for_each_cpu(mask: usize, mut function: impl FnMut(CpuId)) {
    for logical in 0..crate::smp::discovered_cpu_count() {
        let bit = 1_usize << logical;
        if mask & bit == 0 {
            continue;
        }
        function(CpuId::new(logical).expect("call-function target exceeds MAX_CPUS"));
    }
}

fn discovered_mask() -> usize {
    let count = crate::smp::discovered_cpu_count();
    if count == usize::BITS as usize {
        usize::MAX
    } else {
        (1_usize << count) - 1
    }
}

fn cpu_bit(cpu: CpuId) -> usize {
    1_usize
        .checked_shl(cpu.get() as u32)
        .expect("CPU ID exceeds call-function mask width")
}

fn slot_bit(index: usize) -> usize {
    1_usize
        .checked_shl(index as u32)
        .expect("request slot exceeds call-function pending-mask width")
}

fn timeout_deadline() -> u64 {
    let cycles = crate::time::clock_frequency_hz()
        .checked_mul(REQUEST_TIMEOUT_SECONDS)
        .expect("call-function timeout overflowed");
    crate::arch::time::counter().wrapping_add(cycles)
}

fn deadline_reached(now: u64, deadline: u64) -> bool {
    now.wrapping_sub(deadline) < (1_u64 << 63)
}

pub fn dump() {
    crate::println!("call-function requests:");
    for (index, slot) in REQUESTS.iter().enumerate() {
        let state = slot.state.load(Ordering::Acquire);
        if state == SLOT_FREE {
            continue;
        }

        let state_name = if state == SLOT_READY {
            "ready"
        } else if state == SLOT_RESERVED {
            "reserved"
        } else {
            "invalid"
        };
        // Diagnostics read only atomic metadata. Reading the payload here
        // would race with slot release/reuse by another CPU.
        crate::println!(
            "  slot{} state={} completed={:#x} pending={:#x}",
            index,
            state_name,
            slot.completed.load(Ordering::Acquire),
            pending_target_mask(index),
        );
    }
}

#[cfg(debug_assertions)]
static VERIFY_HITS: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(0) }; MAX_CPUS];

#[cfg(debug_assertions)]
static VERIFY_ARGUMENTS: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(0) }; MAX_CPUS];

#[cfg(debug_assertions)]
fn verify_callback(argument: usize) {
    assert!(
        crate::arch::interrupt::are_disabled(),
        "call-function callback did not run in interrupt context",
    );
    let cpu = crate::smp::current_cpu_id();
    VERIFY_ARGUMENTS[cpu.get()].store(argument, Ordering::Release);
    VERIFY_HITS[cpu.get()].fetch_add(1, Ordering::AcqRel);
}

#[cfg(debug_assertions)]
pub fn verify() {
    assert_eq!(crate::smp::current_cpu_id(), CpuId::BOOT);
    crate::context::assert_task_context();
    crate::context::assert_interrupts_enabled();

    for hit in &VERIFY_HITS {
        hit.store(0, Ordering::Release);
    }
    for argument in &VERIFY_ARGUMENTS {
        argument.store(0, Ordering::Release);
    }

    let targets = crate::smp::ipi_ready_cpu_mask() & !cpu_bit(CpuId::BOOT);
    if targets == 0 {
        crate::println!("call-function IPI test:");
        crate::println!("  remote request slots : single-CPU fallback");
        crate::println!("  callback completion  : single-CPU fallback");
        return;
    }

    const MANY_ARGUMENT: usize = 0x4341_4c4c;
    const SINGLE_ARGUMENT: usize = 0x5349_4e47;

    crate::smp::call_function_many(targets, verify_callback, MANY_ARGUMENT);
    for_each_cpu(targets, |cpu| {
        assert_eq!(VERIFY_HITS[cpu.get()].load(Ordering::Acquire), 1);
        assert_eq!(
            VERIFY_ARGUMENTS[cpu.get()].load(Ordering::Acquire),
            MANY_ARGUMENT,
        );
    });

    let target = CpuId::new(targets.trailing_zeros() as usize)
        .expect("call-function verifier target exceeds MAX_CPUS");
    crate::smp::call_function_single(target, verify_callback, SINGLE_ARGUMENT);
    assert_eq!(VERIFY_HITS[target.get()].load(Ordering::Acquire), 2);
    assert_eq!(
        VERIFY_ARGUMENTS[target.get()].load(Ordering::Acquire),
        SINGLE_ARGUMENT,
    );

    for (index, slot) in REQUESTS.iter().enumerate() {
        assert_eq!(
            slot.state.load(Ordering::Acquire),
            SLOT_FREE,
            "call-function verifier leaked request slot {index}",
        );
    }
    for logical in 0..crate::smp::discovered_cpu_count() {
        assert_eq!(
            PENDING_SLOTS[logical].load(Ordering::Acquire),
            0,
            "call-function verifier left pending slots on CPU {logical}",
        );
    }

    crate::println!("call-function IPI test:");
    crate::println!("  preallocated slots   : verified");
    crate::println!("  many-target callback : verified");
    crate::println!("  single-target reuse  : verified");
    crate::println!("  completion ordering  : verified");
}
