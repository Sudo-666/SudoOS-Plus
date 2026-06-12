use core::arch::asm;

use myos_mm::{PAGE_SHIFT, PAGE_SIZE, PhysAddr, PhysFrame, VirtAddr};

use crate::memory::layout;

const CSR_PGDL: usize = 0x19;
const CSR_PGDH: usize = 0x1a;
const CSR_PWCL: usize = 0x1c;
const CSR_PWCH: usize = 0x1d;
const CSR_STLBPS: usize = 0x1e;
const CSR_TLBRENTRY: usize = 0x88;
const CSR_TLBREHI: usize = 0x8e;

const CPUCFG_ARCHITECTURE_WORD: usize = 1;
const CPUCFG_ARCH_MASK: usize = 0b11;
const CPUCFG_ARCH_LA64: usize = 0b10;
const CPUCFG_PGMMU: usize = 1 << 2;
const CPUCFG_PALEN_SHIFT: usize = 4;
const CPUCFG_PALEN_MASK: usize = 0xff << CPUCFG_PALEN_SHIFT;
const CPUCFG_VALEN_SHIFT: usize = 12;
const CPUCFG_VALEN_MASK: usize = 0xff << CPUCFG_VALEN_SHIFT;
const CPUCFG_READ_INHIBIT: usize = 1 << 21;
const CPUCFG_EXECUTE_INHIBIT: usize = 1 << 22;

const REQUIRED_VIRTUAL_ADDRESS_BITS: u8 = 48;
const REQUIRED_PHYSICAL_ADDRESS_BITS: u8 = 48;

const PT_BASE: usize = PAGE_SHIFT;
const PT_WIDTH: usize = 9;
const DIR1_BASE: usize = 21;
const DIR1_WIDTH: usize = 9;
const DIR2_BASE: usize = 30;
const DIR2_WIDTH: usize = 9;
const DIR3_BASE: usize = 39;
const DIR3_WIDTH: usize = 9;

/*
 * PWCL/PWCH encode a 4 KiB, four-level, 512-entry-per-level walk:
 *
 * PGD  [47:39]
 * PUD  [38:30]
 * PMD  [29:21]
 * PTE  [20:12]
 */
const PWCL_VALUE: usize = PT_BASE
    | (PT_WIDTH << 5)
    | (DIR1_BASE << 10)
    | (DIR1_WIDTH << 15)
    | (DIR2_BASE << 20)
    | (DIR2_WIDTH << 25);

const PWCH_VALUE: usize = DIR3_BASE | (DIR3_WIDTH << 6);

const TLBREHI_PAGE_SIZE_SHIFT: usize = 0;
const TLBREHI_PAGE_SIZE_MASK: usize = 0x3f << TLBREHI_PAGE_SIZE_SHIFT;

const INVTLB_ALL: usize = 0x0;
const INVTLB_GLOBAL_OR_MATCHING_ASID_AND_VA: usize = 0x6;
const TLB_PAIR_SIZE: usize = PAGE_SIZE * 2;

unsafe extern "C" {
    fn __loongarch_tlb_refill_entry();
    fn __loongarch_cpucfg_word(index: usize) -> usize;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PagingHardwareState {
    root: PhysFrame,
    refill_entry: PhysAddr,
    virtual_address_bits: u8,
    physical_address_bits: u8,
}

impl PagingHardwareState {
    pub const fn root(self) -> PhysFrame {
        self.root
    }

    pub const fn refill_entry(self) -> PhysAddr {
        self.refill_entry
    }

    pub const fn virtual_address_bits(self) -> u8 {
        self.virtual_address_bits
    }

    pub const fn physical_address_bits(self) -> u8 {
        self.physical_address_bits
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HardwarePagingError {
    UnsupportedArchitecture {
        architecture: u8,
    },
    PageMappingUnitUnavailable,
    VirtualAddressBitsTooSmall {
        available: u8,
        required: u8,
    },
    PhysicalAddressBitsTooSmall {
        available: u8,
        required: u8,
    },
    ReadInhibitUnavailable,
    ExecuteInhibitUnavailable,
    RefillEntryOutsideCachedDmw {
        address: VirtAddr,
    },
    RefillEntryMisaligned {
        address: PhysAddr,
    },
    RegisterMismatch {
        register: &'static str,
        expected: usize,
        actual: usize,
    },
}

/// Install the active LoongArch page-table root and TLB refill vector.
///
/// # Safety
///
/// - `root` must remain allocated and contain a valid four-level page table for
///   as long as paging is active.
/// - every published directory entry must contain a physical child-table
///   address matching the configured PWCL/PWCH geometry.
/// - the caller must serialize this operation against page-table mutation and
///   execute it on every CPU before that CPU accesses paged virtual addresses.
pub unsafe fn activate(root: PhysFrame) -> Result<PagingHardwareState, HardwarePagingError> {
    let capabilities = read_capabilities()?;
    let refill_entry = refill_entry_physical_address()?;
    let root_address = root.start_address().get();

    // SAFETY: the caller establishes the root-table lifetime and serialization
    // requirements described by this function's contract.
    unsafe {
        write_csr::<CSR_PGDL>(root_address);
        write_csr::<CSR_PGDH>(root_address);
        write_csr::<CSR_PWCL>(PWCL_VALUE);
        write_csr::<CSR_PWCH>(PWCH_VALUE);
        write_csr::<CSR_STLBPS>(PAGE_SHIFT);

        let old_tlbrehi = read_csr::<CSR_TLBREHI>();
        let new_tlbrehi =
            (old_tlbrehi & !TLBREHI_PAGE_SIZE_MASK) | (PAGE_SHIFT << TLBREHI_PAGE_SIZE_SHIFT);
        write_csr::<CSR_TLBREHI>(new_tlbrehi);
        write_csr::<CSR_TLBRENTRY>(refill_entry.get());

        data_barrier();
        invalidate_all();
        instruction_barrier();
    }

    verify_register("PGDL", CSR_PGDL, root_address)?;
    verify_register("PGDH", CSR_PGDH, root_address)?;
    verify_register("PWCL", CSR_PWCL, PWCL_VALUE)?;
    verify_register("PWCH", CSR_PWCH, PWCH_VALUE)?;
    verify_register("STLBPS", CSR_STLBPS, PAGE_SHIFT)?;
    verify_register("TLBRENTRY", CSR_TLBRENTRY, refill_entry.get())?;

    // SAFETY: reading a paging CSR — pure register read, no side effects on memory.
    let tlbrehi = unsafe { read_csr::<CSR_TLBREHI>() } & TLBREHI_PAGE_SIZE_MASK;
    let expected_tlbrehi = PAGE_SHIFT << TLBREHI_PAGE_SIZE_SHIFT;

    if tlbrehi != expected_tlbrehi {
        return Err(HardwarePagingError::RegisterMismatch {
            register: "TLBREHI.PS",
            expected: expected_tlbrehi,
            actual: tlbrehi,
        });
    }

    Ok(PagingHardwareState {
        root,
        refill_entry,
        virtual_address_bits: capabilities.virtual_address_bits,
        physical_address_bits: capabilities.physical_address_bits,
    })
}

/// Invalidate the TLB pair containing `address` on the current CPU.
pub fn flush_page(address: VirtAddr) {
    let pair_address = address.get() & !(TLB_PAIR_SIZE - 1);

    // SAFETY: INVTLB only changes current-CPU translation state.  The caller
    // publishes PTE writes before invoking this function.
    unsafe {
        data_barrier();
        asm!(
            "invtlb {operation}, $r0, {address}",
            operation = const INVTLB_GLOBAL_OR_MATCHING_ASID_AND_VA,
            address = in(reg) pair_address,
            options(nostack),
        );
        data_barrier();
        instruction_barrier();
    }
}

/// Invalidate all current-CPU TLB entries.
pub fn flush_all() {
    // SAFETY: INVTLB operation 0 invalidates local translation state only.
    unsafe {
        data_barrier();
        invalidate_all();
        data_barrier();
        instruction_barrier();
    }
}

#[derive(Clone, Copy)]
struct CpuCapabilities {
    virtual_address_bits: u8,
    physical_address_bits: u8,
}

fn read_capabilities() -> Result<CpuCapabilities, HardwarePagingError> {
    // SAFETY: CPUCFG is a side-effect-free architecture query and the assembly
    // helper follows the LoongArch C ABI.
    let word = unsafe { __loongarch_cpucfg_word(CPUCFG_ARCHITECTURE_WORD) };
    let architecture = (word & CPUCFG_ARCH_MASK) as u8;

    if architecture != CPUCFG_ARCH_LA64 as u8 {
        return Err(HardwarePagingError::UnsupportedArchitecture { architecture });
    }

    if word & CPUCFG_PGMMU == 0 {
        return Err(HardwarePagingError::PageMappingUnitUnavailable);
    }

    let physical_address_bits = (((word & CPUCFG_PALEN_MASK) >> CPUCFG_PALEN_SHIFT) + 1) as u8;
    let virtual_address_bits = (((word & CPUCFG_VALEN_MASK) >> CPUCFG_VALEN_SHIFT) + 1) as u8;

    if virtual_address_bits < REQUIRED_VIRTUAL_ADDRESS_BITS {
        return Err(HardwarePagingError::VirtualAddressBitsTooSmall {
            available: virtual_address_bits,
            required: REQUIRED_VIRTUAL_ADDRESS_BITS,
        });
    }

    if physical_address_bits < REQUIRED_PHYSICAL_ADDRESS_BITS {
        return Err(HardwarePagingError::PhysicalAddressBitsTooSmall {
            available: physical_address_bits,
            required: REQUIRED_PHYSICAL_ADDRESS_BITS,
        });
    }

    if word & CPUCFG_READ_INHIBIT == 0 {
        return Err(HardwarePagingError::ReadInhibitUnavailable);
    }

    if word & CPUCFG_EXECUTE_INHIBIT == 0 {
        return Err(HardwarePagingError::ExecuteInhibitUnavailable);
    }

    Ok(CpuCapabilities {
        virtual_address_bits,
        physical_address_bits,
    })
}

fn refill_entry_physical_address() -> Result<PhysAddr, HardwarePagingError> {
    let virtual_address = VirtAddr::new(__loongarch_tlb_refill_entry as *const () as usize);
    let physical_address = layout::cached_to_phys(virtual_address).ok_or(
        HardwarePagingError::RefillEntryOutsideCachedDmw {
            address: virtual_address,
        },
    )?;

    if !physical_address.is_aligned(PAGE_SIZE) {
        return Err(HardwarePagingError::RefillEntryMisaligned {
            address: physical_address,
        });
    }

    Ok(physical_address)
}

fn verify_register(
    register: &'static str,
    csr: usize,
    expected: usize,
) -> Result<(), HardwarePagingError> {
    let actual = match csr {
        // SAFETY: pure CSR read — no side effects or aliasing.
        CSR_PGDL => unsafe { read_csr::<CSR_PGDL>() },
        // SAFETY: pure CSR read — no side effects or aliasing.
        CSR_PGDH => unsafe { read_csr::<CSR_PGDH>() },
        // SAFETY: pure CSR read — no side effects or aliasing.
        CSR_PWCL => unsafe { read_csr::<CSR_PWCL>() },
        // SAFETY: pure CSR read — no side effects or aliasing.
        CSR_PWCH => unsafe { read_csr::<CSR_PWCH>() },
        // SAFETY: pure CSR read — no side effects or aliasing.
        CSR_STLBPS => unsafe { read_csr::<CSR_STLBPS>() },
        // SAFETY: pure CSR read — no side effects or aliasing.
        CSR_TLBRENTRY => unsafe { read_csr::<CSR_TLBRENTRY>() },
        _ => unreachable!("unsupported CSR verification"),
    };

    if actual != expected {
        return Err(HardwarePagingError::RegisterMismatch {
            register,
            expected,
            actual,
        });
    }

    Ok(())
}

unsafe fn read_csr<const CSR: usize>() -> usize {
    let value: usize;

    // SAFETY: the caller chooses a valid privileged CSR for this CPU.
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

unsafe fn write_csr<const CSR: usize>(value: usize) {
    let scratch = value;

    // SAFETY: the caller chooses a writable privileged CSR and establishes the
    // architectural preconditions for changing it.
    unsafe {
        asm!(
            "csrwr {scratch}, {csr}",
            scratch = inout(reg) scratch => _,
            csr = const CSR,
            options(nomem, nostack),
        );
    }
}

unsafe fn invalidate_all() {
    // SAFETY: INVTLB operation 0 is defined as local all-entry invalidation.
    unsafe {
        asm!(
            "invtlb {operation}, $r0, $r0",
            operation = const INVTLB_ALL,
            options(nostack),
        );
    }
}

unsafe fn data_barrier() {
    // SAFETY: DBAR orders page-table memory writes before translation changes.
    unsafe {
        asm!("dbar 0", options(nostack));
    }
}

unsafe fn instruction_barrier() {
    // SAFETY: IBAR synchronizes subsequent instruction fetch after TLB changes.
    unsafe {
        asm!("ibar 0", options(nostack));
    }
}
