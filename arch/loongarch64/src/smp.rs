use core::{
    arch::asm,
    cell::UnsafeCell,
    sync::atomic::{Ordering, fence},
};

pub const MAX_CPUS: usize = 8;
const BOOT_STACK_SIZE: usize = 64 * 1024;
const HARDWARE_CPU_ID_LIMIT: usize = 1 << 10;

const CSR_CPUNUM: usize = 0x20;
const CPU_NUMBER_MASK: usize = 0x3ff;
const CSR_ECFG: usize = 0x4;
const ECFG_IPI_INTERRUPT: usize = 1 << 12;

const IOCSR_IPI_STATUS: usize = 0x1000;
const IOCSR_IPI_ENABLE: usize = 0x1004;
const IOCSR_IPI_CLEAR: usize = 0x100c;
const IOCSR_MAILBOX0: usize = 0x1020;
const IOCSR_IPI_SEND: usize = 0x1040;
const IOCSR_MAILBOX_SEND: usize = 0x1048;

const IOCSR_IPI_SEND_BLOCKING: u32 = 1 << 31;
const IOCSR_IPI_SEND_CPU_SHIFT: usize = 16;
const IOCSR_MAILBOX_SEND_BLOCKING: u64 = 1 << 31;
const IOCSR_MAILBOX_SEND_BOX_SHIFT: usize = 2;
const IOCSR_MAILBOX_SEND_CPU_SHIFT: usize = 16;
const IOCSR_MAILBOX_SEND_BUFFER_SHIFT: usize = 32;
const IOCSR_MAILBOX_SEND_HIGH_MASK: u64 = 0xffff_ffff_0000_0000;
const BOOT_IPI_ACTION: usize = 1;
const RUNTIME_IPI_ACTION: usize = 1 << 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmpError {
    InvalidLogicalCpu { cpu: usize },
    InvalidHardwareCpu { cpu: usize },
    AddressTranslation,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SecondaryBootData {
    stack_top: usize,
    logical_id: usize,
    high_entry: usize,
    ready: usize,
}

impl SecondaryBootData {
    const EMPTY: Self = Self {
        stack_top: 0,
        logical_id: 0,
        high_entry: 0,
        ready: 0,
    };
}

#[repr(align(64))]
struct BootSlot(UnsafeCell<SecondaryBootData>);

impl BootSlot {
    #[allow(clippy::declare_interior_mutable_const)]
    const EMPTY: Self = Self(UnsafeCell::new(SecondaryBootData::EMPTY));
}

// SAFETY: CPU0 publishes each target slot before sending the boot IPI.  The
// secondary CPU only reads the slot matching its immutable hardware CPU ID.
unsafe impl Sync for BootSlot {}

#[unsafe(no_mangle)]
static __loongarch_secondary_boot_data: [BootSlot; HARDWARE_CPU_ID_LIMIT] =
    [const { BootSlot::EMPTY }; HARDWARE_CPU_ID_LIMIT];

unsafe extern "C" {
    fn __loongarch_secondary_entry();
    static __smp_boot_stack_bottom: u8;
}

#[inline]
pub fn set_current_cpu_id(cpu: usize) {
    assert!(
        cpu < MAX_CPUS,
        "logical CPU ID is outside the supported range"
    );

    // SAFETY: r21/u0 is reserved as the kernel per-CPU logical identifier.
    // Ordinary task contexts deliberately do not save or restore it.
    unsafe {
        asm!("or $r21, {cpu}, $r0", cpu = in(reg) cpu, options(nomem, nostack));
    }
}

#[inline]
pub fn current_cpu_id() -> usize {
    let cpu: usize;

    // SAFETY: this only reads the kernel-owned r21/u0 value.
    unsafe {
        asm!("or {cpu}, $r21, $r0", cpu = out(reg) cpu, options(nomem, nostack));
    }

    cpu
}

#[inline]
pub fn hardware_cpu_id() -> usize {
    read_csr::<CSR_CPUNUM>() & CPU_NUMBER_MASK
}

pub fn start_secondary(
    logical_id: usize,
    hardware_id: usize,
    high_entry: usize,
) -> Result<(), SmpError> {
    if logical_id == 0 || logical_id >= MAX_CPUS {
        return Err(SmpError::InvalidLogicalCpu { cpu: logical_id });
    }
    if hardware_id >= HARDWARE_CPU_ID_LIMIT {
        return Err(SmpError::InvalidHardwareCpu { cpu: hardware_id });
    }

    let stack_bottom = core::ptr::addr_of!(__smp_boot_stack_bottom) as usize;
    let stack_top = stack_bottom
        .checked_add(logical_id * BOOT_STACK_SIZE)
        .ok_or(SmpError::AddressTranslation)?
        & !0xf;

    let slot = &__loongarch_secondary_boot_data[hardware_id];

    // SAFETY: CPU0 is the only writer and the target CPU remains in QEMU's
    // ROM wait loop until the mailbox and boot IPI are published below.
    unsafe {
        slot.0.get().write(SecondaryBootData {
            stack_top,
            logical_id,
            high_entry,
            ready: 1,
        });
    }

    let entry_virtual = __loongarch_secondary_entry as *const () as usize;
    let entry_physical =
        crate::memory::layout::cached_to_phys(myos_mm::VirtAddr::new(entry_virtual))
            .ok_or(SmpError::AddressTranslation)?
            .get();

    fence(Ordering::Release);
    send_mailbox(entry_physical, hardware_id, 0);
    send_raw_ipi(hardware_id, BOOT_IPI_ACTION);

    Ok(())
}

pub fn send_ipi(hardware_id: usize) -> Result<(), SmpError> {
    if hardware_id >= HARDWARE_CPU_ID_LIMIT {
        return Err(SmpError::InvalidHardwareCpu { cpu: hardware_id });
    }

    fence(Ordering::Release);
    send_raw_ipi(hardware_id, RUNTIME_IPI_ACTION);
    Ok(())
}

pub fn enable_ipi_source() {
    write_iocsr32(IOCSR_IPI_ENABLE, u32::MAX);
    update_csr_bits::<CSR_ECFG>(ECFG_IPI_INTERRUPT, ECFG_IPI_INTERRUPT);
}

pub fn disable_ipi_source() {
    update_csr_bits::<CSR_ECFG>(0, ECFG_IPI_INTERRUPT);
}

pub fn acknowledge_ipi() -> usize {
    let action = read_iocsr32(IOCSR_IPI_STATUS) as usize;

    if action != 0 {
        write_iocsr32(IOCSR_IPI_CLEAR, action as u32);
    }

    action
}

pub fn clear_boot_mailbox() {
    write_iocsr64(IOCSR_MAILBOX0, 0);

    let pending = read_iocsr32(IOCSR_IPI_STATUS);
    if pending != 0 {
        write_iocsr32(IOCSR_IPI_CLEAR, pending);
    }
}

fn send_raw_ipi(hardware_id: usize, action: usize) {
    let value = IOCSR_IPI_SEND_BLOCKING
        | ((hardware_id as u32) << IOCSR_IPI_SEND_CPU_SHIFT)
        | action as u32;

    write_iocsr32(IOCSR_IPI_SEND, value);
}

fn send_mailbox(data: usize, hardware_id: usize, mailbox: usize) {
    let data = data as u64;
    let high_box = ((mailbox << 1) + 1) << IOCSR_MAILBOX_SEND_BOX_SHIFT;
    let low_box = (mailbox << 1) << IOCSR_MAILBOX_SEND_BOX_SHIFT;

    let high = IOCSR_MAILBOX_SEND_BLOCKING
        | high_box as u64
        | ((hardware_id as u64) << IOCSR_MAILBOX_SEND_CPU_SHIFT)
        | (data & IOCSR_MAILBOX_SEND_HIGH_MASK);
    write_iocsr64(IOCSR_MAILBOX_SEND, high);

    let low = IOCSR_MAILBOX_SEND_BLOCKING
        | low_box as u64
        | ((hardware_id as u64) << IOCSR_MAILBOX_SEND_CPU_SHIFT)
        | (data << IOCSR_MAILBOX_SEND_BUFFER_SHIFT);
    write_iocsr64(IOCSR_MAILBOX_SEND, low);
}

fn read_csr<const CSR: usize>() -> usize {
    let value: usize;

    // SAFETY: callers instantiate this helper only with readable CSRs.
    unsafe {
        asm!(
            "csrrd {value}, {csr}",
            value = out(reg) value,
            csr = const CSR,
            options(nomem, nostack),
        );
    }

    value
}

fn update_csr_bits<const CSR: usize>(value: usize, mask: usize) {
    let scratch = value;

    // SAFETY: CSRXCHG changes only the selected CSR bits. r12 is reserved as
    // the mask operand for the duration of the instruction.
    unsafe {
        asm!(
            "csrxchg {scratch}, $r12, {csr}",
            scratch = inout(reg) scratch => _,
            in("$r12") mask,
            csr = const CSR,
            options(nomem, nostack),
        );
    }
}

fn read_iocsr32(address: usize) -> u32 {
    let value: usize;

    // SAFETY: address is one of the architecture-defined per-core IOCSR
    // registers and the operation does not alias Rust memory.
    unsafe {
        asm!(
            "iocsrrd.w {value}, {address}",
            value = out(reg) value,
            address = in(reg) address,
            options(nomem, nostack),
        );
    }

    value as u32
}

fn write_iocsr32(address: usize, value: u32) {
    let value = value as usize;
    // SAFETY: address is one of the architecture-defined IPI IOCSR registers.
    unsafe {
        asm!(
            "iocsrwr.w {value}, {address}",
            value = in(reg) value,
            address = in(reg) address,
            options(nomem, nostack),
        );
    }
}

fn write_iocsr64(address: usize, value: u64) {
    let value = value as usize;
    // SAFETY: address is one of the architecture-defined mailbox IOCSR
    // registers and the write is serialized by the blocking command bit.
    unsafe {
        asm!(
            "iocsrwr.d {value}, {address}",
            value = in(reg) value,
            address = in(reg) address,
            options(nomem, nostack),
        );
    }
}
