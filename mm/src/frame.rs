use crate::{PAGE_SIZE, PhysAddr, PhysRange};

/// 一个 4 KiB 物理页帧。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct PhysFrame {
    start: PhysAddr,
}

impl PhysFrame {
    /// 从页对齐的物理地址构造页帧。
    pub const fn from_start_address(start: PhysAddr) -> Option<Self> {
        if start.is_aligned(PAGE_SIZE) {
            Some(Self { start })
        } else {
            None
        }
    }

    /// 返回包含给定地址的页帧。
    pub const fn containing_address(address: PhysAddr) -> Self {
        let start = address.get() & !(PAGE_SIZE - 1);

        Self {
            start: PhysAddr::new(start),
        }
    }

    pub const fn start_address(self) -> PhysAddr {
        self.start
    }

    pub const fn number(self) -> usize {
        self.start.get() / PAGE_SIZE
    }

    pub const fn range(self) -> PhysRange {
        let end = match self.start.checked_add(PAGE_SIZE) {
            Some(end) => end,
            None => {
                panic!("physical frame address overflows")
            }
        };

        match PhysRange::new(self.start, end) {
            Some(range) => range,
            None => {
                panic!("physical frame range is invalid")
            }
        }
    }
}

/// 一组连续物理页帧。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameBlock {
    start: PhysFrame,
    count: usize,
}

impl FrameBlock {
    pub const fn new(start: PhysFrame, count: usize) -> Option<Self> {
        if count == 0 {
            return None;
        }

        let Some(size) = count.checked_mul(PAGE_SIZE) else {
            return None;
        };

        if start.start_address().checked_add(size).is_none() {
            return None;
        }

        Some(Self { start, count })
    }

    pub const fn start(self) -> PhysFrame {
        self.start
    }

    pub const fn count(self) -> usize {
        self.count
    }

    pub const fn size(self) -> usize {
        self.count * PAGE_SIZE
    }

    pub const fn range(self) -> PhysRange {
        let start = self.start.start_address();

        let end = match start.checked_add(self.size()) {
            Some(end) => end,
            None => {
                panic!("frame block address overflows")
            }
        };

        match PhysRange::new(start, end) {
            Some(range) => range,
            None => {
                panic!("frame block range is invalid")
            }
        }
    }
}
