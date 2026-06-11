use myos_mm::{PageTableGeometry, PageTableIndex, VirtAddr};

use crate::memory::layout;

pub const LEVELS: usize = 4;
pub const VIRTUAL_ADDRESS_BITS: u8 = 48;
pub const ENTRIES_PER_TABLE: usize = 512;

const GEOMETRY: PageTableGeometry<LEVELS> =
    PageTableGeometry::new(VIRTUAL_ADDRESS_BITS, 9, [39, 30, 21, 12]);

pub fn indices(address: VirtAddr) -> Option<[PageTableIndex; LEVELS]> {
    let is_user = layout::USER_RANGE.contains(address);

    let is_page_mapped_kernel = address.get() >= layout::XKVRANGE_START;

    /*
     * XKPRANGE 中的 DMW 地址不经过页表。
     */
    if !is_user && !is_page_mapped_kernel {
        return None;
    }

    Some(GEOMETRY.indices(address))
}

pub(super) fn validate() {
    assert_eq!(GEOMETRY.entries_per_table(), ENTRIES_PER_TABLE,);

    assert_eq!(GEOMETRY.entry_span(0), Some(1 << 39),);

    assert_eq!(GEOMETRY.entry_span(1), Some(1 << 30),);

    assert_eq!(GEOMETRY.entry_span(2), Some(1 << 21),);

    assert_eq!(GEOMETRY.entry_span(3), Some(1 << 12),);

    assert!(indices(layout::VMALLOC.start()).is_some(),);

    /*
     * cached DMW 不应交给页表遍历器。
     */
    assert!(indices(layout::KERNEL_LINK_BASE).is_none(),);
}
