use crate::PAGE_SIZE;

/// free_area[0..MAX_ORDER]。
///
/// 最大有效 order 为 10：
///
/// 2^10 × 4 KiB = 4 MiB。
pub const MAX_ORDER: usize = 11;

pub const DMA32_LIMIT: usize = 0x1_0000_0000;

pub const DMA32_LIMIT_PFN: usize = DMA32_LIMIT / PAGE_SIZE;

pub(super) const INVALID_PFN: usize = usize::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ZoneKind {
    Dma32 = 0,
    Normal = 1,
}

impl ZoneKind {
    pub(super) const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct FreeArea {
    pub(super) head: usize,
    pub(super) block_count: usize,
}

impl FreeArea {
    const EMPTY: Self = Self {
        head: INVALID_PFN,
        block_count: 0,
    };
}

#[derive(Debug)]
pub(super) struct Zone {
    pub(super) start_pfn: usize,
    pub(super) end_pfn: usize,

    pub(super) present_pages: usize,
    pub(super) free_pages: usize,

    pub(super) free_area: [FreeArea; MAX_ORDER],
}

impl Zone {
    pub(super) const fn new(start_pfn: usize, end_pfn: usize) -> Self {
        Self {
            start_pfn,
            end_pfn,

            present_pages: 0,
            free_pages: 0,

            free_area: [FreeArea::EMPTY; MAX_ORDER],
        }
    }

    pub(super) const fn contains_pfn(&self, pfn: usize) -> bool {
        self.start_pfn <= pfn && pfn < self.end_pfn
    }

    pub(super) fn contains_block(&self, pfn: usize, order: usize) -> bool {
        let page_count = 1_usize << order;

        pfn >= self.start_pfn
            && pfn
                .checked_add(page_count)
                .is_some_and(|end| end <= self.end_pfn)
    }
}
