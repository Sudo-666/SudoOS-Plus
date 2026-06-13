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
pub fn flush_all() {
    // SAFETY: SFENCE.VMA with zero operands invalidates only the current
    // hart's translation caches.
    unsafe {
        core::arch::asm!("sfence.vma zero, zero", options(nostack));
    }
}
