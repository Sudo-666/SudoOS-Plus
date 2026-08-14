mod activate;
mod boot;
mod entry;
mod geometry;
mod map;

pub use boot::{BootPageTable, BootPageTableError};

pub use entry::{PageTableEntry, PageTableEntryError};

pub use geometry::{ENTRIES_PER_TABLE, LEVELS, VIRTUAL_ADDRESS_BITS, indices};

pub type PageTable = myos_mm::RawPageTable<ENTRIES_PER_TABLE>;

pub use map::MapPageError;

pub fn validate() {
    geometry::validate();
    entry::validate();

    assert_eq!(core::mem::size_of::<PageTable>(), myos_mm::PAGE_SIZE,);
}

pub use activate::{
    ActivateError, current_mode, current_satp, switch_sv39_root, translation_is_enabled,
};

const SATP_MODE_SHIFT: usize = 60;
const SATP_MODE_MASK: usize = 0xf << SATP_MODE_SHIFT;
const SATP_MODE_SV39: usize = 8;
const SATP_ASID_SHIFT: usize = 44;
const SATP_ASID_BITS: usize = 16;
const SATP_ASID_MASK: usize = ((1usize << SATP_ASID_BITS) - 1) << SATP_ASID_SHIFT;
const SATP_PPN_MASK: usize = (1usize << SATP_ASID_SHIFT) - 1;

/// Returns the implemented Sv39 ASID width as an inclusive maximum ID.
pub fn maximum_address_space_id() -> u16 {
    let original = current_satp();
    assert_eq!(
        (original & SATP_MODE_MASK) >> SATP_MODE_SHIFT,
        SATP_MODE_SV39,
        "ASID probing requires an active Sv39 root",
    );
    let probe = (original & !SATP_ASID_MASK) | SATP_ASID_MASK;

    // SAFETY: the root and mode are unchanged. The WARL ASID field is restored
    // immediately and a full local fence discards translations made while the
    // probe value was visible.
    let observed = unsafe {
        core::arch::asm!("csrw satp, {value}", value = in(reg) probe, options(nostack));
        let value = current_satp();
        core::arch::asm!(
            "csrw satp, {value}",
            "sfence.vma zero, zero",
            value = in(reg) original,
            options(nostack),
        );
        value
    };

    let maximum = ((observed & SATP_ASID_MASK) >> SATP_ASID_SHIFT) as u16;
    assert_ne!(maximum, 0, "RISC-V hart implements no usable ASID bits");
    maximum
}

/// Installs an Sv39 root and ASID without invalidating unrelated ASIDs.
///
/// # Safety
/// `root` and all reachable shared kernel tables must remain alive until the
/// caller installs another root, and this CPU's switch must be serialized.
pub unsafe fn switch_user_address_space(root: myos_mm::PhysFrame, asid: myos_mm::AddressSpaceId) {
    let root_address = root.start_address().get();
    assert_eq!(
        root_address & (myos_mm::PAGE_SIZE - 1),
        0,
        "RISC-V page-table root is not page aligned",
    );
    let ppn = root_address >> myos_mm::PAGE_SHIFT;
    assert_eq!(ppn & !SATP_PPN_MASK, 0, "RISC-V root exceeds SATP.PPN");
    let asid_value = usize::from(asid.get());
    let satp = (SATP_MODE_SV39 << SATP_MODE_SHIFT) | (asid_value << SATP_ASID_SHIFT) | ppn;

    /*
     * SATP does not order ordinary page-table stores against implicit reads,
     * so publish the table before selecting it. Do not invalidate a stable
     * ASID here: UserMm's per-CPU TLB generation performs the required fence
     * on first use, ASID reuse, or after an inactive invalidation. Retaining
     * translations across ordinary context switches is the main reason Linux
     * assigns ASIDs in the first place.
     */
    // SAFETY: the caller guarantees that `root` and every reachable page-table
    // page remain alive and that this hart's address-space switch is serialized.
    // The validated SATP value selects only that root/ASID, and the assembly
    // touches no Rust-managed memory or stack while fencing the local hart.
    unsafe {
        core::arch::asm!(
            "fence rw, rw",
            "csrw satp, {satp}",
            satp = in(reg) satp,
            options(nostack),
        );
    }
}

pub fn current_lower_root() -> myos_mm::PhysFrame {
    let satp = current_satp();
    let address = myos_mm::PhysAddr::new((satp & SATP_PPN_MASK) << myos_mm::PAGE_SHIFT);
    myos_mm::PhysFrame::from_start_address(address).expect("SATP root is not page aligned")
}

pub fn current_upper_root() -> myos_mm::PhysFrame {
    current_lower_root()
}

pub fn current_address_space_id() -> myos_mm::AddressSpaceId {
    myos_mm::AddressSpaceId::new(((current_satp() & SATP_ASID_MASK) >> SATP_ASID_SHIFT) as u16)
}

#[inline]
pub fn flush_page(address: myos_mm::VirtAddr) {
    // SAFETY: SFENCE.VMA invalidates only the current hart's translation
    // caches and does not dereference the supplied virtual address.
    unsafe {
        core::arch::asm!(
            "sfence.vma {address}, zero",
            address = in(reg) address.get(),
            options(nostack),
        );
    }
}

#[inline]
pub fn flush_asid(asid: myos_mm::AddressSpaceId) {
    assert_ne!(
        asid,
        myos_mm::AddressSpaceId::KERNEL,
        "kernel/global translations require a full local TLB flush",
    );
    let asid = usize::from(asid.get());

    // SAFETY: SFENCE.VMA with rs1=zero and an explicit ASID invalidates only
    // non-global translations for that ASID on the current hart.
    unsafe {
        core::arch::asm!(
            "sfence.vma zero, {asid}",
            asid = in(reg) asid,
            options(nostack),
        );
    }
}

#[inline]
pub fn flush_asid_page(asid: myos_mm::AddressSpaceId, address: myos_mm::VirtAddr) {
    assert_ne!(
        asid,
        myos_mm::AddressSpaceId::KERNEL,
        "kernel/global translations require the kernel page flush path",
    );
    let asid = usize::from(asid.get());

    // SAFETY: SFENCE.VMA invalidates only the current hart's non-global
    // translation matching this virtual address and ASID.
    unsafe {
        core::arch::asm!(
            "sfence.vma {address}, {asid}",
            address = in(reg) address.get(),
            asid = in(reg) asid,
            options(nostack),
        );
    }
}

pub fn flush_all() {
    // SAFETY: SFENCE.VMA with zero operands invalidates only the current
    // hart's translation caches.
    unsafe {
        core::arch::asm!("sfence.vma zero, zero", options(nostack));
    }
}
