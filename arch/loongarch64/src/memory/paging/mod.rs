mod boot;
mod entry;
mod geometry;
mod hardware;
mod map;

pub use boot::{BootPageTable, BootPageTableError};

pub use entry::{LeafPageTableEntry, PageTableEntryError, TablePointerEntry};

pub use geometry::{ENTRIES_PER_TABLE, LEVELS, VIRTUAL_ADDRESS_BITS, indices};

pub use hardware::{
    HardwarePagingError, PagingHardwareState, activate, flush_all, flush_asid, flush_asid_page,
    flush_page,
};

pub type PageTable = myos_mm::RawPageTable<ENTRIES_PER_TABLE>;

const CSR_ASID: usize = 0x18;
const CSR_PGDL: usize = 0x19;
const CSR_PGDH: usize = 0x1a;
const ASID_MASK: usize = 0x3ff;
const ASID_BITS_SHIFT: usize = 16;
const ASID_BITS_MASK: usize = 0xff << ASID_BITS_SHIFT;

pub fn maximum_address_space_id() -> u16 {
    let bits = ((read_switch_csr::<CSR_ASID>() & ASID_BITS_MASK) >> ASID_BITS_SHIFT) as u32;
    assert!(
        (1..=10).contains(&bits),
        "unsupported LoongArch ASID width: {bits}",
    );
    ((1u16 << bits) - 1).min(ASID_MASK as u16)
}

/// Whether the CPU implements usable hardware ASID bits.
///
/// LoongArch always exposes a hardware ASID width in CSR.ASID (1..=10 bits,
/// validated by [`maximum_address_space_id`]), so every hart has usable ASIDs.
/// Mirrors the RISC-V contract used by the shared `user_mm` ASID-less
/// fallback (`assert_hardware_active`): a false return means "ASID is WARL to
/// zero, treat all logical spaces as ASID 0".
pub fn hardware_address_space_id_available() -> bool {
    maximum_address_space_id() != 0
}

/// Changes only PGDL and ASID. PGDH permanently remains the kernel root.
///
/// # Safety
/// `root` must remain alive until another lower root is installed, and the
/// caller must serialize this CPU's address-space switch.
pub unsafe fn switch_user_address_space(root: myos_mm::PhysFrame, asid: myos_mm::AddressSpaceId) {
    let asid = usize::from(asid.get());
    assert_eq!(asid & !ASID_MASK, 0, "LoongArch ASID exceeds 10 bits");
    let root_address = root.start_address().get();
    assert_eq!(
        root_address & (myos_mm::PAGE_SIZE - 1),
        0,
        "LoongArch PGDL root is not page aligned",
    );
    let current_asid = read_switch_csr::<CSR_ASID>();
    let next_asid = (current_asid & !ASID_MASK) | asid;

    // SAFETY: the contract guarantees root lifetime and switch serialization.
    unsafe {
        core::arch::asm!("dbar 0", options(nostack));
        write_switch_csr::<CSR_PGDL>(root_address);
        write_switch_csr::<CSR_ASID>(next_asid);
        core::arch::asm!("ibar 0", options(nostack));
    }
}

pub fn current_lower_root() -> myos_mm::PhysFrame {
    frame_from_root(read_switch_csr::<CSR_PGDL>())
}

pub fn current_upper_root() -> myos_mm::PhysFrame {
    frame_from_root(read_switch_csr::<CSR_PGDH>())
}

pub fn current_address_space_id() -> myos_mm::AddressSpaceId {
    myos_mm::AddressSpaceId::new((read_switch_csr::<CSR_ASID>() & ASID_MASK) as u16)
}

fn frame_from_root(address: usize) -> myos_mm::PhysFrame {
    myos_mm::PhysFrame::from_start_address(myos_mm::PhysAddr::new(address))
        .expect("LoongArch page-table root is not page aligned")
}

fn read_switch_csr<const CSR: usize>() -> usize {
    let value: usize;
    // SAFETY: callers select one of ASID/PGDL/PGDH, all readable privileged CSRs.
    unsafe {
        core::arch::asm!(
            "csrrd {value}, {csr}",
            value = out(reg) value,
            csr = const CSR,
            options(nomem, nostack),
        );
    }
    value
}

unsafe fn write_switch_csr<const CSR: usize>(value: usize) {
    let scratch = value;
    // SAFETY: callers select ASID or PGDL and uphold the switch contract.
    unsafe {
        core::arch::asm!(
            "csrwr {scratch}, {csr}",
            scratch = inout(reg) scratch => _,
            csr = const CSR,
            options(nomem, nostack),
        );
    }
}

pub use map::MapPageError;

pub fn validate() {
    geometry::validate();
    entry::validate();

    assert_eq!(core::mem::size_of::<PageTable>(), myos_mm::PAGE_SIZE,);
}
