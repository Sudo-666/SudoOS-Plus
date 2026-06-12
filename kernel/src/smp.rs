use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering, fence};

use myos_fdt::DeviceTree;

use crate::irq_lock::IrqSpinLock;

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

    fn hardware_id(self, cpu: CpuId) -> usize {
        assert!(cpu.get() < self.discovered, "logical CPU is not discovered");
        self.hardware_ids[cpu.get()]
    }
}

static TOPOLOGY: IrqSpinLock<CpuTopology> = IrqSpinLock::new(CpuTopology::EMPTY);
static ONLINE_MASK: AtomicUsize = AtomicUsize::new(0);
static ONLINE_COUNT: AtomicUsize = AtomicUsize::new(0);
static IPI_COUNTS: [AtomicU64; MAX_CPUS] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

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

    *TOPOLOGY.lock() = topology;
    ONLINE_MASK.store(1, Ordering::Release);
    ONLINE_COUNT.store(1, Ordering::Release);

    for counter in &IPI_COUNTS {
        counter.store(0, Ordering::Release);
    }
}

pub fn discovered_cpu_count() -> usize {
    TOPOLOGY.lock().discovered
}

pub fn online_cpu_count() -> usize {
    ONLINE_COUNT.load(Ordering::Acquire)
}

pub fn is_online(cpu: CpuId) -> bool {
    ONLINE_MASK.load(Ordering::Acquire) & (1_usize << cpu.get()) != 0
}

pub fn current_cpu_id() -> CpuId {
    CpuId::new(crate::arch::smp::current_cpu_id())
        .expect("architecture returned an invalid logical CPU ID")
}

pub fn hardware_id(cpu: CpuId) -> usize {
    hardware_id_for(cpu)
}

fn hardware_id_for(cpu: CpuId) -> usize {
    TOPOLOGY.lock().hardware_id(cpu)
}

pub fn start_secondaries() {
    assert_eq!(current_cpu_id(), CpuId::BOOT);
    assert!(crate::arch::interrupt::are_enabled());

    crate::arch::smp::enable_ipi_source();

    let discovered = discovered_cpu_count();
    let high_entry = kernel_secondary_entry as *const () as usize;

    for logical in 1..discovered {
        let cpu = CpuId::new(logical).expect("discovered CPU ID exceeded MAX_CPUS");
        let hardware = hardware_id(cpu);

        crate::arch::smp::start_secondary(logical, hardware, high_entry).unwrap_or_else(|error| {
            panic!(
                "unable to start secondary CPU: logical={logical} hardware={hardware} error={error:?}",
            );
        });
    }

    let frequency = crate::time::clock_frequency_hz();
    let timeout_cycles = frequency
        .checked_mul(SECONDARY_START_TIMEOUT_SECONDS)
        .expect("secondary startup timeout overflowed");
    let deadline = crate::arch::time::counter().wrapping_add(timeout_cycles);

    while online_cpu_count() < discovered {
        if deadline_reached(crate::arch::time::counter(), deadline) {
            panic!(
                "secondary CPU startup timed out: discovered={} online={}",
                discovered,
                online_cpu_count(),
            );
        }

        crate::arch::cpu::wait_for_interrupt();
    }

    crate::println!("SMP subsystem:");
    crate::println!("  discovered CPUs : {}", discovered);
    crate::println!("  online CPUs     : {}", online_cpu_count());
    crate::println!(
        "  boot CPU        : 0 (hardware {})",
        hardware_id(CpuId::BOOT)
    );
    if discovered > 1 {
        crate::println!("  secondary CPUs  : verified");
    } else {
        crate::println!("  secondary CPUs  : single-CPU fallback");
    }
    crate::println!("  per-CPU stacks  : verified");
    crate::println!("  per-CPU traps   : verified");
    crate::println!("  per-CPU timers  : armed");
}

pub fn send_ipi(cpu: CpuId) {
    assert!(is_online(cpu), "attempted to send an IPI to an offline CPU");

    let hardware = hardware_id(cpu);
    crate::arch::smp::send_ipi(hardware).unwrap_or_else(|error| {
        panic!(
            "unable to send IPI: logical={} hardware={} error={error:?}",
            cpu.get(),
            hardware,
        );
    });
}

pub fn broadcast_ipi_except_current() {
    let current = current_cpu_id();

    for logical in 0..discovered_cpu_count() {
        let cpu = CpuId::new(logical).expect("discovered CPU ID exceeded MAX_CPUS");

        if cpu != current && is_online(cpu) {
            send_ipi(cpu);
        }
    }
}

pub fn handle_ipi() {
    let cpu = current_cpu_id();
    let action = crate::arch::smp::acknowledge_ipi();

    assert!(
        action != 0,
        "IPI exception arrived without a pending action"
    );
    fence(Ordering::Acquire);
    IPI_COUNTS[cpu.get()].fetch_add(1, Ordering::AcqRel);
}

pub fn ipi_count(cpu: CpuId) -> u64 {
    IPI_COUNTS[cpu.get()].load(Ordering::Acquire)
}

fn mark_current_online() {
    let cpu = current_cpu_id();
    let bit = 1_usize << cpu.get();
    let previous = ONLINE_MASK.fetch_or(bit, Ordering::AcqRel);

    assert_eq!(
        previous & bit,
        0,
        "secondary CPU was marked online more than once",
    );
    ONLINE_COUNT.fetch_add(1, Ordering::AcqRel);
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

    mark_current_online();
    crate::task::enter_secondary_idle()
}

fn deadline_reached(now: u64, deadline: u64) -> bool {
    now.wrapping_sub(deadline) < (1_u64 << 63)
}
