use core::sync::atomic::{AtomicBool, AtomicU16, Ordering};

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

// All harts are homogeneous (the kernel assumes identical ASIDLEN across the
// topology), so one cached probe result is sufficient. A concurrent probe on
// another hart recomputes the same value and races benignly on these atomics.
static HARDWARE_ASID_PROBED: AtomicBool = AtomicBool::new(false);
static HARDWARE_ASID_MAXIMUM: AtomicU16 = AtomicU16::new(0);

/// Returns the implemented Sv39 ASID width as an inclusive maximum ID.
///
/// A value of `0` is legal: RISC-V lets `ASIDLEN = 0`, so SATP.ASID is WARL to
/// zero and the hart carries no hardware address-space tags. StarFive's JH7110
/// U74 cores report exactly this (`ASID allocator disabled (0 bits)` under
/// Linux). Callers must use [`hardware_address_space_id_available`] to select
/// the ASID-less fallback instead of treating 0 as a configuration error.
pub fn maximum_address_space_id() -> u16 {
    if HARDWARE_ASID_PROBED.load(Ordering::Acquire) {
        return HARDWARE_ASID_MAXIMUM.load(Ordering::Acquire);
    }

    if !translation_is_enabled() {
        // Early boot: satp is still BARE and no address space exists yet. The
        // probe writes SATP.ASID, which is meaningless without Sv39; return a
        // conservative "ASIDs present" answer and let the first post-Sv39
        // caller probe and cache the real width.
        return u16::MAX;
    }

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
    HARDWARE_ASID_MAXIMUM.store(maximum, Ordering::Release);
    HARDWARE_ASID_PROBED.store(true, Ordering::Release);
    maximum
}

/// Whether the hart implements non-zero hardware ASID bits.
///
/// When false, every logical address space maps to SATP.ASID == 0, so address
/// space switches and per-ASID flushes must fall back to full local TLB
/// flushes (the Linux behavior on `ASIDLEN = 0` harts).
pub fn hardware_address_space_id_available() -> bool {
    maximum_address_space_id() != 0
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
    let has_hardware_asid = hardware_address_space_id_available();
    // On an ASID-less hart SATP.ASID is WARL to zero: every logical address
    // space maps to hardware ASID 0, so the switch must discard the whole local
    // TLB rather than only the previous ASID's entries.
    let asid_value = if has_hardware_asid {
        usize::from(asid.get())
    } else {
        0
    };
    let satp = (SATP_MODE_SV39 << SATP_MODE_SHIFT) | (asid_value << SATP_ASID_SHIFT) | ppn;

    /*
     * SATP does not order ordinary page-table stores against implicit
     * page-table reads. Keep publication, root/ASID selection, and local
     * invalidation in one non-interruptible instruction sequence.
     */
    // SAFETY: the caller guarantees that `root` and every reachable page-table
    // page remain alive and that this hart's address-space switch is serialized.
    // The validated SATP value selects only that root/ASID, and the assembly
    // touches no Rust-managed memory or stack while fencing the local hart.
    unsafe {
        if has_hardware_asid {
            core::arch::asm!(
                "fence rw, rw",
                "sfence.vma zero, {asid}",
                "csrw satp, {satp}",
                "sfence.vma zero, {asid}",
                asid = in(reg) asid_value,
                satp = in(reg) satp,
                options(nostack),
            );
        } else {
            // No hardware tags to discriminate: a full local fence after the
            // switch is both necessary and sufficient.
            core::arch::asm!(
                "fence rw, rw",
                "csrw satp, {satp}",
                "sfence.vma zero, zero",
                satp = in(reg) satp,
                options(nostack),
            );
        }
    }

    /*
     * Synchronize the local instruction stream after any user-address-space
     * switch.  Exec and eager fork both install freshly-written executable
     * pages; SFENCE.VMA above only invalidates translation caches, and the
     * RISC-V I-cache is not coherent with the stores that wrote those pages.
     * Without FENCE.I the hart can fetch stale bytes (garbage execution,
     * SIGSEGV at pc=0 / near-null writes) — invisible under QEMU's TCG, real
     * on the JH7110 U74.  FENCE.I is per-hart, so it must run on the hart
     * that will actually execute the new code, which is this one.
     */
    // SAFETY: FENCE.I orders this hart's instruction fetch against prior
    // stores made visible to it; it touches no memory or stack.
    unsafe {
        core::arch::asm!("fence.i", options(nostack));
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
    if !hardware_address_space_id_available() {
        // ASID-less hart: SFENCE.VMA cannot discriminate address spaces, so
        // fall back to a full local flush (matches the Linux no-ASID path).
        flush_all();
        return;
    }
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
    if !hardware_address_space_id_available() {
        flush_all();
        return;
    }
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
    if !hardware_address_space_id_available() {
        flush_all();
        return;
    }
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
