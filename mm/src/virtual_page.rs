use crate::{PAGE_SHIFT, PAGE_SIZE, VirtAddr, VirtRange};

/// 一个 4 KiB 虚拟页面。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct VirtPage {
    start: VirtAddr,
}

impl VirtPage {
    pub const fn from_start_address(start: VirtAddr) -> Option<Self> {
        if start.is_aligned(PAGE_SIZE) {
            Some(Self { start })
        } else {
            None
        }
    }

    pub const fn containing_address(address: VirtAddr) -> Self {
        Self {
            start: VirtAddr::new(address.get() & !(PAGE_SIZE - 1)),
        }
    }

    pub const fn start_address(self) -> VirtAddr {
        self.start
    }

    pub const fn number(self) -> usize {
        self.start.get() >> PAGE_SHIFT
    }

    pub const fn range(self) -> VirtRange {
        match VirtRange::from_start_size(self.start, PAGE_SIZE) {
            Some(range) => range,
            None => panic!("virtual page range overflows"),
        }
    }
}
