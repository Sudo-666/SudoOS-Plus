mod boot;
mod entry;
mod geometry;
mod hardware;
mod map;

pub use boot::{BootPageTable, BootPageTableError};

pub use entry::{LeafPageTableEntry, PageTableEntryError, TablePointerEntry};

pub use geometry::{ENTRIES_PER_TABLE, LEVELS, VIRTUAL_ADDRESS_BITS, indices};

pub use hardware::{HardwarePagingError, PagingHardwareState, activate, flush_all, flush_page};

pub type PageTable = myos_mm::RawPageTable<ENTRIES_PER_TABLE>;

pub use map::MapPageError;

pub fn validate() {
    geometry::validate();
    entry::validate();

    assert_eq!(core::mem::size_of::<PageTable>(), myos_mm::PAGE_SIZE,);
}
