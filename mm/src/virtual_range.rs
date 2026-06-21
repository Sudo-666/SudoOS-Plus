use crate::{PAGE_SIZE, VirtAddr};

/// 半开虚拟地址范围 `[start, end)`。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VirtRange {
    start: VirtAddr,
    end: VirtAddr,
}

impl VirtRange {
    pub const fn new(start: VirtAddr, end: VirtAddr) -> Option<Self> {
        if start.get() <= end.get() {
            Some(Self { start, end })
        } else {
            None
        }
    }

    /// 构造静态地址空间布局使用的范围。
    ///
    /// 无效范围会在编译期常量求值或启动检查中失败。
    pub const fn from_bounds(start: usize, end: usize) -> Self {
        assert!(start <= end);

        Self {
            start: VirtAddr::new(start),
            end: VirtAddr::new(end),
        }
    }

    pub const fn from_start_size(start: VirtAddr, size: usize) -> Option<Self> {
        match start.checked_add(size) {
            Some(end) => Self::new(start, end),
            None => None,
        }
    }

    pub const fn start(self) -> VirtAddr {
        self.start
    }

    pub const fn end(self) -> VirtAddr {
        self.end
    }

    pub const fn size(self) -> usize {
        self.end.get() - self.start.get()
    }

    pub const fn is_empty(self) -> bool {
        self.start.get() == self.end.get()
    }

    pub const fn contains(self, address: VirtAddr) -> bool {
        self.start.get() <= address.get() && address.get() < self.end.get()
    }

    pub const fn contains_range(self, other: Self) -> bool {
        self.start.get() <= other.start.get() && other.end.get() <= self.end.get()
    }

    pub const fn overlaps(self, other: Self) -> bool {
        self.start.get() < other.end.get() && other.start.get() < self.end.get()
    }

    pub const fn last(self) -> Option<VirtAddr> {
        if self.is_empty() {
            None
        } else {
            Some(VirtAddr::new(self.end.get() - 1))
        }
    }

    pub const fn is_page_aligned(self) -> bool {
        self.start.is_aligned(PAGE_SIZE) && self.end.is_aligned(PAGE_SIZE)
    }
}
