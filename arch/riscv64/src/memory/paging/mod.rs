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
    current_mode,
    current_satp,
    switch_sv39_root,
    translation_is_enabled,
    ActivateError,
};
