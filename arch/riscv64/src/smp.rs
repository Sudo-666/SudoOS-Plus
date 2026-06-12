use core::{
    arch::asm,
    cell::UnsafeCell,
    sync::atomic::{Ordering, fence},
};

use myos_mm::VirtAddr;

pub const MAX_CPUS: usize = 8;
const BOOT_STACK_SIZE: usize = 64 * 1024;
const SIE_SSIE: usize = 1 << 1;
const SIP_SSIP: usize = 1 << 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmpError {
    InvalidLogicalCpu { cpu: usize },
    HsmUnavailable,
    IpiUnavailable,
    AddressTranslation,
    Firmware { code: isize },
}

impl From<crate::sbi::SbiError> for SmpError {
    fn from(error: crate::sbi::SbiError) -> Self {
        Self::Firmware { code: error.0 }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SecondaryBootData {
    satp: usize,
    stack_top: usize,
    logical_id: usize,
    high_entry: usize,
    global_pointer: usize,
}

impl SecondaryBootData {
    const EMPTY: Self = Self {
        satp: 0,
        stack_top: 0,
        logical_id: 0,
        high_entry: 0,
        global_pointer: 0,
    };
}

#[repr(align(64))]
struct BootSlot(UnsafeCell<SecondaryBootData>);

impl BootSlot {
    #[allow(clippy::declare_interior_mutable_const)]
    const EMPTY: Self = Self(UnsafeCell::new(SecondaryBootData::EMPTY));
}

// SAFETY: CPU0 writes each slot exactly once before firmware starts the target
// hart. The target only reads its slot after hart_start observes the release
// fence.
unsafe impl Sync for BootSlot {}

static BOOT_DATA: [BootSlot; MAX_CPUS] = [const { BootSlot::EMPTY }; MAX_CPUS];

unsafe extern "C" {
    static __riscv_secondary_entry_phys: usize;
    static __smp_boot_stack_bottom: u8;

    #[link_name = "__global_pointer$"]
    static RISCV_GLOBAL_POINTER: u8;
}

#[inline]
pub fn set_current_cpu_id(cpu: usize) {
    assert!(
        cpu < MAX_CPUS,
        "logical CPU ID is outside the supported range"
    );

    // SAFETY: tp is reserved as the kernel per-CPU identifier. Ordinary task
    // contexts deliberately do not save or restore it.
    unsafe {
        asm!("mv tp, {cpu}", cpu = in(reg) cpu, options(nomem, nostack));
    }
}

#[inline]
pub fn current_cpu_id() -> usize {
    let cpu: usize;

    // SAFETY: this only reads the kernel-owned tp value.
    unsafe {
        asm!("mv {cpu}, tp", cpu = out(reg) cpu, options(nomem, nostack));
    }

    cpu
}

pub fn start_secondary(
    logical_id: usize,
    hardware_id: usize,
    high_entry: usize,
) -> Result<(), SmpError> {
    if logical_id == 0 || logical_id >= MAX_CPUS {
        return Err(SmpError::InvalidLogicalCpu { cpu: logical_id });
    }
    if !crate::sbi::hsm_extension_available()? {
        return Err(SmpError::HsmUnavailable);
    }
    if !crate::sbi::ipi_extension_available()? {
        return Err(SmpError::IpiUnavailable);
    }

    let stack_bottom = core::ptr::addr_of!(__smp_boot_stack_bottom) as usize;
    let stack_top = stack_bottom
        .checked_add(logical_id * BOOT_STACK_SIZE)
        .expect("secondary bootstrap stack address overflowed");
    let global_pointer = core::ptr::addr_of!(RISCV_GLOBAL_POINTER) as usize;

    let slot = &BOOT_DATA[logical_id];
    // SAFETY: CPU0 is the only writer and the target hart is still stopped.
    unsafe {
        slot.0.get().write(SecondaryBootData {
            satp: crate::memory::paging::current_satp(),
            stack_top: stack_top & !0xf,
            logical_id,
            high_entry,
            global_pointer,
        });
    }

    let slot_virtual = VirtAddr::new(slot.0.get() as usize);
    let slot_physical = crate::memory::layout::kernel_image_physical_address(slot_virtual)
        .ok_or(SmpError::AddressTranslation)?;
    // The trampoline itself lives in the low physical address space, far outside
    // the high-half kernel's medany PC-relative range. Read its link-time address
    // from a nearby high-half literal instead of taking the function item's
    // address directly.
    //
    // SAFETY: the assembly definition emits one aligned, immutable XLEN-sized
    // object in kernel read-only memory.
    let entry_physical = unsafe { core::ptr::addr_of!(__riscv_secondary_entry_phys).read() };

    if entry_physical == 0 || entry_physical & 0x3 != 0 {
        return Err(SmpError::AddressTranslation);
    }

    fence(Ordering::Release);
    crate::sbi::hart_start(hardware_id, entry_physical, slot_physical.get())?;
    Ok(())
}

pub fn send_ipi(hardware_id: usize) -> Result<(), SmpError> {
    crate::sbi::send_ipi(hardware_id)?;
    Ok(())
}

pub fn enable_ipi_source() {
    // SAFETY: only the current hart's supervisor software-interrupt mask changes.
    unsafe {
        asm!("csrs sie, {mask}", mask = in(reg) SIE_SSIE, options(nostack));
    }
}

pub fn disable_ipi_source() {
    // SAFETY: only the current hart's supervisor software-interrupt mask changes.
    unsafe {
        asm!("csrc sie, {mask}", mask = in(reg) SIE_SSIE, options(nostack));
    }
}

pub fn acknowledge_ipi() -> usize {
    // SAFETY: supervisor software interrupt pending is writable by S-mode.
    unsafe {
        asm!("csrc sip, {mask}", mask = in(reg) SIP_SSIP, options(nostack));
    }
    1
}

/// RISC-V HSM passes boot data through a1 and has no platform mailbox.
pub fn clear_boot_mailbox() {}
