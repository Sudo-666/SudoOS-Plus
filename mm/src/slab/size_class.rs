use core::{alloc::Layout, cmp::max};

pub const SIZE_CLASS_COUNT: usize = 9;

pub const MIN_SLAB_OBJECT_SIZE: usize = 8;
pub const MAX_SLAB_OBJECT_SIZE: usize = 2048;

const CLASS_SIZES: [usize; SIZE_CLASS_COUNT] = [8, 16, 32, 64, 128, 256, 512, 1024, 2048];

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct SizeClass(u8);

impl SizeClass {
    pub fn for_layout(layout: Layout) -> Option<Self> {
        /*
         * GlobalAlloc 不应收到零大小请求，但 slab 核心本身
         * 对零大小 Layout 仍采取保守处理。
         */
        let required = max(max(layout.size(), 1), layout.align());

        CLASS_SIZES
            .iter()
            .position(|size| *size >= required)
            .map(|index| Self(index as u8))
    }

    pub const fn from_index(index: usize) -> Option<Self> {
        if index < SIZE_CLASS_COUNT {
            Some(Self(index as u8))
        } else {
            None
        }
    }

    pub(super) const fn from_index_unchecked(index: usize) -> Self {
        Self(index as u8)
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }

    pub const fn size(self) -> usize {
        CLASS_SIZES[self.index()]
    }
}
