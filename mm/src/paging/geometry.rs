use crate::{PAGE_SHIFT, VirtAddr};

/// 页表中的一个索引。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct PageTableIndex(u16);

impl PageTableIndex {
    pub const fn get(self) -> usize {
        self.0 as usize
    }
}

/// 多级页表几何结构。
///
/// `shifts` 按从根页表到叶页表的顺序保存。
///
/// Sv39：
///
/// `[30, 21, 12]`
///
/// LoongArch 四级页表：
///
/// `[39, 30, 21, 12]`
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageTableGeometry<const LEVELS: usize> {
    virtual_address_bits: u8,
    index_bits: u8,
    shifts: [u8; LEVELS],
}

impl<const LEVELS: usize> PageTableGeometry<LEVELS> {
    pub const fn new(virtual_address_bits: u8, index_bits: u8, shifts: [u8; LEVELS]) -> Self {
        assert!(LEVELS > 0);

        assert!(virtual_address_bits > PAGE_SHIFT as u8);
        assert!(virtual_address_bits <= usize::BITS as u8,);

        assert!(index_bits > 0);
        assert!(index_bits < usize::BITS as u8);

        let mut index = 0;

        while index < LEVELS {
            assert!(shifts[index] < usize::BITS as u8,);

            if index + 1 < LEVELS {
                assert!(shifts[index] > shifts[index + 1],);
            }

            index += 1;
        }

        assert!(shifts[LEVELS - 1] == PAGE_SHIFT as u8,);

        Self {
            virtual_address_bits,
            index_bits,
            shifts,
        }
    }

    pub const fn levels(&self) -> usize {
        LEVELS
    }

    pub const fn virtual_address_bits(&self) -> u8 {
        self.virtual_address_bits
    }

    pub const fn index_bits(&self) -> u8 {
        self.index_bits
    }

    pub const fn entries_per_table(&self) -> usize {
        1_usize << self.index_bits
    }

    pub const fn shift(&self, level: usize) -> Option<u8> {
        if level < LEVELS {
            Some(self.shifts[level])
        } else {
            None
        }
    }

    /// 一个指定层级表项所覆盖的字节数。
    pub const fn entry_span(&self, level: usize) -> Option<usize> {
        let Some(shift) = self.shift(level) else {
            return None;
        };

        1_usize.checked_shl(shift as u32)
    }

    /// 按根页表到叶页表的顺序提取索引。
    pub const fn indices(&self, address: VirtAddr) -> [PageTableIndex; LEVELS] {
        let mut result = [PageTableIndex(0); LEVELS];

        let mask = (1_usize << self.index_bits) - 1;

        let mut level = 0;

        while level < LEVELS {
            let index = (address.get() >> self.shifts[level]) & mask;

            result[level] = PageTableIndex(index as u16);

            level += 1;
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_sv39_indices() {
        let geometry = PageTableGeometry::<3>::new(39, 9, [30, 21, 12]);

        let indices = geometry.indices(VirtAddr::new(0xffff_ffff_8000_0000));

        assert_eq!(indices[0].get(), 510);
        assert_eq!(indices[1].get(), 0);
        assert_eq!(indices[2].get(), 0);
    }
}
