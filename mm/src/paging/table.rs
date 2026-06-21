use core::mem::{align_of, size_of};

use crate::PAGE_SIZE;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageTableAccessError {
    IndexOutOfRange { index: usize, entries: usize },
}

/// 一个只保存原始 64 位表项的页表页面。
///
/// 架构层负责解释每个 u64 的位含义。
///
/// 不实现 Clone/Copy，避免意外复制整张页表。
#[repr(C, align(4096))]
pub struct RawPageTable<const ENTRIES: usize> {
    entries: [u64; ENTRIES],
}

impl<const ENTRIES: usize> RawPageTable<ENTRIES> {
    pub const fn zeroed() -> Self {
        assert!(ENTRIES > 0);

        Self {
            entries: [0; ENTRIES],
        }
    }

    pub const fn len(&self) -> usize {
        ENTRIES
    }

    pub const fn is_empty(&self) -> bool {
        ENTRIES == 0
    }

    pub fn entry(&self, index: usize) -> Result<u64, PageTableAccessError> {
        self.entries
            .get(index)
            .copied()
            .ok_or(PageTableAccessError::IndexOutOfRange {
                index,
                entries: ENTRIES,
            })
    }

    pub fn set_entry(&mut self, index: usize, value: u64) -> Result<(), PageTableAccessError> {
        let entry = self
            .entries
            .get_mut(index)
            .ok_or(PageTableAccessError::IndexOutOfRange {
                index,
                entries: ENTRIES,
            })?;

        *entry = value;

        Ok(())
    }

    pub fn clear_entry(&mut self, index: usize) -> Result<(), PageTableAccessError> {
        self.set_entry(index, 0)
    }

    pub fn clear(&mut self) {
        self.entries.fill(0);
    }

    pub const fn as_ptr(&self) -> *const u64 {
        self.entries.as_ptr()
    }

    pub fn as_mut_ptr(&mut self) -> *mut u64 {
        self.entries.as_mut_ptr()
    }

    pub fn fill(&mut self, value: u64) {
        self.entries.fill(value);
    }
}

/*
 * 当前两个架构均使用：
 *
 * 512 entries × 8 bytes = 4096 bytes。
 */
const _: () = {
    assert!(size_of::<RawPageTable<512>>() == PAGE_SIZE,);

    assert!(align_of::<RawPageTable<512>>() == PAGE_SIZE,);
};
