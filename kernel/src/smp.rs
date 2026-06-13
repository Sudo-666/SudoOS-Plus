use core::{
    hint::spin_loop,
    sync::atomic::{AtomicU8, AtomicUsize, Ordering},
};

use myos_fdt::DeviceTree;

pub const MAX_CPUS: usize = crate::arch::smp::MAX_CPUS;
const SECONDARY_START_TIMEOUT_SECONDS: u64 = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpuId(usize);

impl CpuId {
    pub const BOOT: Self = Self(0);

    pub const fn new(value: usize) -> Option<Self> {
        if value < MAX_CPUS {
            Some(Self(value))
        } else {
            None
        }
    }

    pub const fn get(self) -> usize {
        self.0
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CpuState {
    Absent = 0,
    Present = 1,
    Starting = 2,
    SchedulerRegistered = 3,
    Active = 4,
    IpiReady = 5,
    Failed = 6,
    Dying = 7,
    Dead = 8,
}

impl CpuState {
    fn from_raw(raw: u8) -> Self {
        match raw {
            0 => Self::Absent,
            1 => Self::Present,
            2 => Self::Starting,
            3 => Self::SchedulerRegistered,
            4 => Self::Active,
            5 => Self::IpiReady,
            6 => Self::Failed,
            7 => Self::Dying,
            8 => Self::Dead,
            _ => panic!("invalid CPU lifecycle state value: {raw}"),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Present => "present",
            Self::Starting => "starting",
            Self::SchedulerRegistered => "scheduler-registered",
            Self::Active => "active",
            Self::IpiReady => "ipi-ready",
            Self::Failed => "failed",
            Self::Dying => "dying",
            Self::Dead => "dead",
        }
    }

    const fn is_present(self) -> bool {
        !matches!(self, Self::Absent)
    }

    const fn is_online(self) -> bool {
        matches!(
            self,
            Self::SchedulerRegistered | Self::Active | Self::IpiReady | Self::Dying
        )
    }

    const fn is_scheduler_active(self) -> bool {
        matches!(self, Self::Active | Self::IpiReady)
    }

    const fn accepts_ipi(self) -> bool {
        matches!(self, Self::IpiReady)
    }
}

#[derive(Clone, Copy)]
struct CpuTopology {
    hardware_ids: [usize; MAX_CPUS],
    discovered: usize,
}

impl CpuTopology {
    const EMPTY: Self = Self {
        hardware_ids: [usize::MAX; MAX_CPUS],
        discovered: 0,
    };
}

// Logical CPU identities and their hardware IDs are immutable after boot-time
// discovery. Publish every entry first, then publish the count with Release.
// Readers acquire the count before loading an entry, so runtime IPI paths never
// need the CPU-lifecycle lock.
static DISCOVERED_CPU_COUNT: AtomicUsize = AtomicUsize::new(0);
static HARDWARE_IDS: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(usize::MAX) }; MAX_CPUS];
static CPU_STATES: [AtomicU8; MAX_CPUS] =
    [const { AtomicU8::new(CpuState::Absent as u8) }; MAX_CPUS];
pub fn initialize(tree: &DeviceTree<'_>, boot_hardware_id: usize) {
    crate::arch::smp::set_current_cpu_id(CpuId::BOOT.get());

    let mut topology = CpuTopology::EMPTY;
    topology.hardware_ids[0] = boot_hardware_id;
    topology.discovered = 1;

    for hardware_id in tree.cpu_hardware_ids() {
        if hardware_id == boot_hardware_id {
            continue;
        }

        assert!(
            topology.discovered < MAX_CPUS,
            "firmware reports more CPUs than the kernel supports: max={MAX_CPUS}",
        );
        assert!(
            !topology.hardware_ids[..topology.discovered].contains(&hardware_id),
            "duplicate hardware CPU ID in firmware topology: {hardware_id}",
        );

        topology.hardware_ids[topology.discovered] = hardware_id;
        topology.discovered += 1;
    }

    assert!(topology.discovered != 0);

    assert_eq!(
        DISCOVERED_CPU_COUNT.load(Ordering::Acquire),
        0,
        "CPU topology was initialized more than once",
    );
    for (logical, hardware_id) in topology.hardware_ids[..topology.discovered]
        .iter()
        .copied()
        .enumerate()
    {
        HARDWARE_IDS[logical].store(hardware_id, Ordering::Relaxed);
    }
    DISCOVERED_CPU_COUNT.store(topology.discovered, Ordering::Release);

    #[cfg(debug_assertions)]
    verify_topology_snapshot(topology);
    for logical in 0..MAX_CPUS {
        let next = if logical < topology.discovered {
            CpuState::Present
        } else {
            CpuState::Absent
        };
        let previous = CPU_STATES[logical].swap(next as u8, Ordering::AcqRel);
        assert_eq!(
            CpuState::from_raw(previous),
            CpuState::Absent,
            "CPU lifecycle was initialized more than once: cpu={logical}",
        );
    }
    crate::ipi::initialize();
}

pub fn discovered_cpu_count() -> usize {
    DISCOVERED_CPU_COUNT.load(Ordering::Acquire)
}

pub fn cpu_state(cpu: CpuId) -> CpuState {
    CpuState::from_raw(CPU_STATES[cpu.get()].load(Ordering::Acquire))
}

fn state_mask(mut predicate: impl FnMut(CpuState) -> bool) -> usize {
    let mut mask = 0_usize;
    for logical in 0..discovered_cpu_count() {
        let cpu = CpuId::new(logical).expect("discovered CPU exceeds MAX_CPUS");
        if predicate(cpu_state(cpu)) {
            mask |= 1_usize << logical;
        }
    }
    mask
}

pub fn present_cpu_mask() -> usize {
    state_mask(CpuState::is_present)
}

pub fn online_cpu_count() -> usize {
    online_cpu_mask().count_ones() as usize
}

pub fn online_cpu_mask() -> usize {
    state_mask(CpuState::is_online)
}

pub fn is_online(cpu: CpuId) -> bool {
    cpu_state(cpu).is_online()
}

pub fn scheduler_active_cpu_count() -> usize {
    scheduler_active_cpu_mask().count_ones() as usize
}

pub fn scheduler_active_cpu_mask() -> usize {
    state_mask(CpuState::is_scheduler_active)
}

pub fn is_scheduler_active(cpu: CpuId) -> bool {
    cpu_state(cpu).is_scheduler_active()
}

pub fn ipi_ready_cpu_mask() -> usize {
    state_mask(CpuState::accepts_ipi)
}

pub fn is_ipi_ready(cpu: CpuId) -> bool {
    cpu_state(cpu).accepts_ipi()
}

fn transition_cpu(cpu: CpuId, expected: CpuState, next: CpuState, owner: &'static str) {
    match CPU_STATES[cpu.get()].compare_exchange(
        expected as u8,
        next as u8,
        Ordering::AcqRel,
        Ordering::Acquire,
    ) {
        Ok(_) => {}
        Err(actual) => {
            let actual = CpuState::from_raw(actual);
            panic!(
                "invalid CPU lifecycle transition: cpu={} owner={} expected={} actual={} next={}",
                cpu.get(),
                owner,
                expected.name(),
                actual.name(),
                next.name(),
            );
        }
    }
}

fn mark_cpu_starting(cpu: CpuId) {
    assert_ne!(cpu, CpuId::BOOT, "boot CPU cannot enter secondary start");
    transition_cpu(cpu, CpuState::Present, CpuState::Starting, "boot/start");
}

fn mark_cpu_start_failed(cpu: CpuId) {
    transition_cpu(
        cpu,
        CpuState::Starting,
        CpuState::Failed,
        "boot/start-failure",
    );
}

pub(crate) fn mark_boot_scheduler_registered() {
    assert_eq!(current_cpu_id(), CpuId::BOOT);
    transition_cpu(
        CpuId::BOOT,
        CpuState::Present,
        CpuState::SchedulerRegistered,
        "boot/scheduler-register",
    );
}

pub(crate) fn mark_current_scheduler_registered() {
    crate::context::assert_interrupts_disabled();
    let cpu = current_cpu_id();
    assert_ne!(cpu, CpuId::BOOT);
    transition_cpu(
        cpu,
        CpuState::Starting,
        CpuState::SchedulerRegistered,
        "secondary/scheduler-register",
    );
}

pub(crate) fn mark_current_scheduler_active() {
    crate::context::assert_interrupts_enabled();
    let cpu = current_cpu_id();
    transition_cpu(
        cpu,
        CpuState::SchedulerRegistered,
        CpuState::Active,
        "scheduler/activate",
    );
}

pub(crate) fn mark_current_ipi_ready() {
    crate::context::assert_interrupts_enabled();
    let cpu = current_cpu_id();
    transition_cpu(cpu, CpuState::Active, CpuState::IpiReady, "arch/ipi-ready");
}

pub fn dump_cpu_states() {
    crate::println!("CPU lifecycle:");
    for logical in 0..discovered_cpu_count() {
        let cpu = CpuId::new(logical).expect("discovered CPU exceeds MAX_CPUS");
        crate::println!(
            "  cpu{} hardware={} state={}",
            logical,
            hardware_id(cpu),
            cpu_state(cpu).name(),
        );
    }
}

pub fn assert_bringup_complete() {
    let discovered = discovered_cpu_count();
    let expected = if discovered == usize::BITS as usize {
        usize::MAX
    } else {
        (1_usize << discovered) - 1
    };

    assert_eq!(
        present_cpu_mask(),
        expected,
        "not every discovered CPU has a lifecycle entry",
    );
    assert_eq!(
        online_cpu_mask(),
        expected,
        "not every discovered CPU reached an online lifecycle state",
    );
    assert_eq!(
        scheduler_active_cpu_mask(),
        expected,
        "not every online CPU became scheduler-active",
    );
    assert_eq!(
        ipi_ready_cpu_mask(),
        expected,
        "not every scheduler-active CPU became IPI-ready",
    );

    for logical in 0..discovered {
        let cpu = CpuId::new(logical).expect("discovered CPU exceeds MAX_CPUS");
        assert_eq!(
            cpu_state(cpu),
            CpuState::IpiReady,
            "CPU did not finish bring-up: cpu={logical}",
        );
    }
}

pub fn current_cpu_id() -> CpuId {
    CpuId::new(crate::arch::smp::current_cpu_id())
        .expect("architecture returned an invalid logical CPU ID")
}

pub fn hardware_id(cpu: CpuId) -> usize {
    hardware_id_for(cpu)
}

fn hardware_id_for(cpu: CpuId) -> usize {
    let discovered = discovered_cpu_count();
    assert!(
        cpu.get() < discovered,
        "logical CPU is not discovered: cpu={} discovered={discovered}",
        cpu.get(),
    );

    // The Acquire load of DISCOVERED_CPU_COUNT above observes all hardware-ID
    // stores that preceded the boot CPU's Release publication.
    HARDWARE_IDS[cpu.get()].load(Ordering::Relaxed)
}

#[cfg(debug_assertions)]
fn verify_topology_snapshot(expected: CpuTopology) {
    assert_eq!(
        discovered_cpu_count(),
        expected.discovered,
        "published CPU topology count does not match firmware discovery",
    );

    for logical in 0..expected.discovered {
        let cpu = CpuId::new(logical).expect("published logical CPU exceeds MAX_CPUS");
        assert_eq!(
            hardware_id_for(cpu),
            expected.hardware_ids[logical],
            "published logical/hardware CPU mapping mismatch: logical={logical}",
        );
    }
}

pub fn start_secondaries() {
    assert_eq!(current_cpu_id(), CpuId::BOOT);
    crate::context::assert_interrupts_enabled();
    assert_eq!(
        cpu_state(CpuId::BOOT),
        CpuState::Active,
        "boot CPU scheduler must be active before enabling IPIs",
    );

    crate::arch::smp::enable_ipi_source();
    mark_current_ipi_ready();

    let discovered = discovered_cpu_count();
    let high_entry = kernel_secondary_entry as *const () as usize;
    for logical in 1..discovered {
        let cpu = CpuId::new(logical).expect("discovered CPU ID exceeded MAX_CPUS");
        let hardware = hardware_id(cpu);
        mark_cpu_starting(cpu);
        if let Err(error) = crate::arch::smp::start_secondary(logical, hardware, high_entry) {
            mark_cpu_start_failed(cpu);
            dump_cpu_states();
            panic!(
                "unable to start secondary CPU: logical={logical} hardware={hardware} \
                 error={error:?}",
            );
        }
    }

    let frequency = crate::time::clock_frequency_hz();
    let timeout_cycles = frequency
        .checked_mul(SECONDARY_START_TIMEOUT_SECONDS)
        .expect("secondary startup timeout overflowed");
    let deadline = crate::arch::time::counter().wrapping_add(timeout_cycles);

    while online_cpu_count() < discovered {
        if deadline_reached(crate::arch::time::counter(), deadline) {
            dump_cpu_states();
            panic!(
                "secondary CPU scheduler registration timed out: \
                 discovered={} online={} online_mask={:#x}",
                discovered,
                online_cpu_count(),
                online_cpu_mask(),
            );
        }
        spin_loop();
    }

    let expected_mask = if discovered == usize::BITS as usize {
        usize::MAX
    } else {
        (1_usize << discovered) - 1
    };
    while ipi_ready_cpu_mask() & expected_mask != expected_mask {
        if deadline_reached(crate::arch::time::counter(), deadline) {
            dump_cpu_states();
            panic!(
                "secondary CPU IPI readiness timed out: discovered={} \
                 online_mask={:#x} active_mask={:#x} ready_mask={:#x} expected={:#x}",
                discovered,
                online_cpu_mask(),
                scheduler_active_cpu_mask(),
                ipi_ready_cpu_mask(),
                expected_mask,
            );
        }
        spin_loop();
    }

    assert_bringup_complete();

    #[cfg(debug_assertions)]
    crate::ipi::verify();
    #[cfg(debug_assertions)]
    crate::call_function::verify();
    #[cfg(debug_assertions)]
    crate::tlb::verify_request_model();

    crate::println!("SMP subsystem:");
    crate::println!("  discovered CPUs : {}", discovered);
    crate::println!("  online CPUs     : {}", online_cpu_count());
    crate::println!("  active CPUs     : {}", scheduler_active_cpu_count());
    crate::println!("  IPI-ready CPUs  : {}", ipi_ready_cpu_mask().count_ones());
    crate::println!(
        "  boot CPU        : 0 (hardware {})",
        hardware_id(CpuId::BOOT),
    );
    if discovered > 1 {
        crate::println!("  secondary CPUs  : verified");
    } else {
        crate::println!("  secondary CPUs  : single-CPU fallback");
    }
    crate::println!("  lifecycle       : explicit state machine");
    crate::println!("  per-CPU stacks  : verified");
    crate::println!("  per-CPU traps   : verified");
    crate::println!("  per-CPU timers  : armed");
}

pub fn send_ipi(cpu: CpuId) {
    crate::ipi::send(cpu, crate::ipi::IpiMessage::Reschedule);
}

pub fn send_tlb_shootdown(cpu: CpuId) {
    crate::ipi::send(cpu, crate::ipi::IpiMessage::TlbShootdown);
}

pub fn call_function_single(
    cpu: CpuId,
    function: crate::call_function::CallFunction,
    argument: usize,
) {
    crate::call_function::call_single(cpu, function, argument);
}

pub fn call_function_many(
    targets: usize,
    function: crate::call_function::CallFunction,
    argument: usize,
) {
    crate::call_function::call_many(targets, function, argument);
}

pub fn broadcast_ipi_except_current() {
    let current = current_cpu_id();
    let targets = online_cpu_mask() & ipi_ready_cpu_mask() & !(1_usize << current.get());

    for logical in 0..discovered_cpu_count() {
        let bit = 1_usize << logical;
        if targets & bit == 0 {
            continue;
        }

        let cpu = CpuId::new(logical).expect("discovered CPU ID exceeded MAX_CPUS");
        send_ipi(cpu);
    }
}

pub fn handle_ipi() {
    crate::ipi::handle_current();
}

pub fn ipi_count(cpu: CpuId) -> u64 {
    crate::ipi::interrupt_count(cpu)
}

#[unsafe(no_mangle)]
extern "C" fn kernel_secondary_entry(logical_id: usize, hardware_id: usize) -> ! {
    crate::arch::smp::set_current_cpu_id(logical_id);
    let cpu = current_cpu_id();

    assert_ne!(cpu, CpuId::BOOT, "boot CPU entered the secondary path");
    assert_eq!(
        hardware_id,
        hardware_id_for(cpu),
        "secondary logical/hardware CPU mapping mismatch",
    );

    crate::arch::interrupt::disable();
    crate::arch::interrupt::mask_all_sources();
    crate::arch::trap::initialize();
    crate::vm::activate_secondary_cpu();
    crate::irq::initialize_secondary();
    crate::time::initialize_secondary();
    crate::task::register_secondary_cpu(cpu);
    crate::arch::smp::clear_boot_mailbox();
    crate::arch::smp::enable_ipi_source();
    crate::time::arm_periodic_secondary();

    crate::task::enter_secondary_idle()
}

fn deadline_reached(now: u64, deadline: u64) -> bool {
    now.wrapping_sub(deadline) < (1_u64 << 63)
}
