use myos_mm::{PageTableGeometry, PageTableIndex, VirtAddr};

use crate::memory::layout;

pub const LEVELS: usize = 3;
pub const VIRTUAL_ADDRESS_BITS: u8 = 39;
pub const ENTRIES_PER_TABLE: usize = 512;

const GEOMETRY: PageTableGeometry<LEVELS> =
    PageTableGeometry::new(VIRTUAL_ADDRESS_BITS, 9, [30, 21, 12]);

pub fn indices(address: VirtAddr) -> Option<[PageTableIndex; LEVELS]> {
    if !layout::is_canonical(address) {
        return None;
    }

    Some(GEOMETRY.indices(address))
}

pub(super) fn validate() {
    assert_eq!(GEOMETRY.entries_per_table(), ENTRIES_PER_TABLE,);

    assert_eq!(GEOMETRY.entry_span(0), Some(1 << 30),);

    assert_eq!(GEOMETRY.entry_span(1), Some(1 << 21),);

    assert_eq!(GEOMETRY.entry_span(2), Some(1 << 12),);

    assert!(indices(layout::KERNEL_LINK_BASE).is_some(),);
}
