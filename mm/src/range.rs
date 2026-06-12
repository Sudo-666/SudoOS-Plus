use crate::{PAGE_SIZE, PhysAddr};

/// 半开物理地址范围 `[start, end)`。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhysRange {
    start: PhysAddr,
    end: PhysAddr,
}

impl PhysRange {
    pub const fn new(start: PhysAddr, end: PhysAddr) -> Option<Self> {
        if start.get() <= end.get() {
            Some(Self { start, end })
        } else {
            None
        }
    }

    pub const fn from_start_size(start: PhysAddr, size: usize) -> Option<Self> {
        match start.checked_add(size) {
            Some(end) => Self::new(start, end),
            None => None,
        }
    }

    pub const fn start(self) -> PhysAddr {
        self.start
    }

    pub const fn end(self) -> PhysAddr {
        self.end
    }

    pub const fn size(self) -> usize {
        self.end.get() - self.start.get()
    }

    pub const fn is_empty(self) -> bool {
        self.start.get() == self.end.get()
    }

    pub const fn contains(self, address: PhysAddr) -> bool {
        self.start.get() <= address.get() && address.get() < self.end.get()
    }

    pub const fn overlaps(self, other: Self) -> bool {
        self.start.get() < other.end.get() && other.start.get() < self.end.get()
    }

    /// 取完全位于当前范围内部的完整页面。
    pub const fn page_aligned_inside(self) -> Option<Self> {
        let Some(start) = self.start.align_up(PAGE_SIZE) else {
            return None;
        };

        let Some(end) = self.end.align_down(PAGE_SIZE) else {
            return None;
        };

        if start.get() >= end.get() {
            return None;
        }

        Self::new(start, end)
    }

    /// 扩张到覆盖当前范围的完整页面。
    pub const fn covering_pages(self) -> Option<Self> {
        let Some(start) = self.start.align_down(PAGE_SIZE) else {
            return None;
        };

        let Some(end) = self.end.align_up(PAGE_SIZE) else {
            return None;
        };

        Self::new(start, end)
    }

    /// 两个范围是否重叠或者首尾相接。
    ///
    /// 相邻范围也应该合并，例如：
    ///
    /// `[0x1000, 0x2000)` 和 `[0x2000, 0x3000)`。
    pub const fn touches_or_overlaps(self, other: Self) -> bool {
        self.start.get() <= other.end.get() && other.start.get() <= self.end.get()
    }

    /// 返回同时覆盖两个范围的最小范围。
    pub const fn span(self, other: Self) -> Self {
        let start = if self.start.get() <= other.start.get() {
            self.start
        } else {
            other.start
        };

        let end = if self.end.get() >= other.end.get() {
            self.end
        } else {
            other.end
        };

        Self { start, end }
    }

    pub const fn is_page_aligned(self) -> bool {
        self.start.get().is_multiple_of(PAGE_SIZE) && self.end.get().is_multiple_of(PAGE_SIZE)
    }

    pub const fn contains_range(self, other: Self) -> bool {
        self.start.get() <= other.start.get() && other.end.get() <= self.end.get()
    }
}
