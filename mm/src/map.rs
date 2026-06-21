use crate::PhysRange;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryMapError {
    CapacityExceeded,
}

/// 启动阶段固定容量的可用物理内存表。
///
/// 始终维持以下不变量：
///
/// - 所有范围均按起始地址升序排列；
/// - 所有范围均按页面对齐；
/// - 不存在空范围；
/// - 不存在重叠范围；
/// - 不存在首尾相接但尚未合并的范围；
/// - 有效项紧密排列在数组前部。
#[derive(Clone, Copy, Debug)]
pub struct MemoryMap<const CAPACITY: usize> {
    free: [Option<PhysRange>; CAPACITY],
}

impl<const CAPACITY: usize> MemoryMap<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            free: [None; CAPACITY],
        }
    }

    /// 加入一段固件报告为可用的物理内存。
    ///
    /// 不完整的首尾页面会被舍弃。与现有区域重叠或相邻的
    /// 区域会被自动合并。
    pub fn add_usable(&mut self, range: PhysRange) -> Result<(), MemoryMapError> {
        let Some(range) = range.page_aligned_inside() else {
            return Ok(());
        };

        self.insert_merged(range)
    }

    /// 从可用物理内存中排除一个范围。
    ///
    /// 预留范围会向外扩张到页面边界，因为被部分占用的页面
    /// 也不能交给页帧分配器。
    ///
    /// 此操作是事务性的：发生容量不足时，原内存表保持不变。
    pub fn reserve(&mut self, range: PhysRange) -> Result<(), MemoryMapError> {
        let Some(reserved) = range.covering_pages() else {
            return Ok(());
        };

        let mut output = [None; CAPACITY];
        let mut output_len = 0;

        for current in self.iter() {
            if !current.overlaps(reserved) {
                push_entry(&mut output, &mut output_len, current)?;

                continue;
            }

            /*
             * 保留 current 左边未被覆盖的部分。
             */
            if current.start() < reserved.start() {
                let left = PhysRange::new(current.start(), reserved.start())
                    .expect("overlapping left range must be valid");

                push_entry(&mut output, &mut output_len, left)?;
            }

            /*
             * 保留 current 右边未被覆盖的部分。
             */
            if reserved.end() < current.end() {
                let right = PhysRange::new(reserved.end(), current.end())
                    .expect("overlapping right range must be valid");

                push_entry(&mut output, &mut output_len, right)?;
            }
        }

        /*
         * 所有操作成功后才提交，避免半修改状态。
         */
        self.free = output;

        Ok(())
    }

    pub fn iter(&self) -> impl Iterator<Item = PhysRange> + '_ {
        self.free.iter().flatten().copied()
    }

    pub fn len(&self) -> usize {
        self.iter().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 返回可用物理内存总字节数。
    ///
    /// 使用 Option 避免理论上的 usize 加法溢出。
    pub fn total_bytes(&self) -> Option<usize> {
        self.iter()
            .try_fold(0_usize, |total, range| total.checked_add(range.size()))
    }

    fn insert_merged(&mut self, range: PhysRange) -> Result<(), MemoryMapError> {
        let mut output = [None; CAPACITY];
        let mut output_len = 0;

        let mut pending = Some(range);

        for current in self.iter() {
            match pending {
                /*
                 * current 完全位于待插入范围之前。
                 */
                Some(candidate) if current.end() < candidate.start() => {
                    push_entry(&mut output, &mut output_len, current)?;
                }

                /*
                 * 待插入范围完全位于 current 之前。
                 */
                Some(candidate) if candidate.end() < current.start() => {
                    push_entry(&mut output, &mut output_len, candidate)?;

                    pending = None;

                    push_entry(&mut output, &mut output_len, current)?;
                }

                /*
                 * 两个范围相邻或者重叠，合并后继续比较。
                 */
                Some(candidate) => {
                    pending = Some(candidate.span(current));
                }

                /*
                 * 待插入范围已经写入，复制余下范围。
                 */
                None => {
                    push_entry(&mut output, &mut output_len, current)?;
                }
            }
        }

        if let Some(candidate) = pending {
            push_entry(&mut output, &mut output_len, candidate)?;
        }

        /*
         * 构造成功后一次性提交。
         */
        self.free = output;

        Ok(())
    }
}

impl<const CAPACITY: usize> Default for MemoryMap<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

fn push_entry<const CAPACITY: usize>(
    entries: &mut [Option<PhysRange>; CAPACITY],
    length: &mut usize,
    range: PhysRange,
) -> Result<(), MemoryMapError> {
    if range.is_empty() {
        return Ok(());
    }

    if *length >= CAPACITY {
        return Err(MemoryMapError::CapacityExceeded);
    }

    entries[*length] = Some(range);
    *length += 1;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{MemoryMap, MemoryMapError, PhysAddr, PhysRange};

    fn range(start: usize, end: usize) -> PhysRange {
        PhysRange::new(PhysAddr::new(start), PhysAddr::new(end)).unwrap()
    }

    #[test]
    fn adjacent_regions_are_merged() {
        let mut map = MemoryMap::<4>::new();

        map.add_usable(range(0x3000, 0x5000)).unwrap();

        map.add_usable(range(0x1000, 0x3000)).unwrap();

        let mut regions = map.iter();

        assert_eq!(regions.next(), Some(range(0x1000, 0x5000)),);

        assert_eq!(regions.next(), None);
    }

    #[test]
    fn overlapping_regions_are_merged() {
        let mut map = MemoryMap::<4>::new();

        map.add_usable(range(0x1000, 0x5000)).unwrap();

        map.add_usable(range(0x3000, 0x7000)).unwrap();

        assert_eq!(map.iter().next(), Some(range(0x1000, 0x7000)),);

        assert_eq!(map.len(), 1);
    }

    #[test]
    fn reserve_can_split_a_region() {
        let mut map = MemoryMap::<4>::new();

        map.add_usable(range(0x1000, 0x9000)).unwrap();

        map.reserve(range(0x3000, 0x5000)).unwrap();

        let mut regions = map.iter();

        assert_eq!(regions.next(), Some(range(0x1000, 0x3000)),);

        assert_eq!(regions.next(), Some(range(0x5000, 0x9000)),);

        assert_eq!(regions.next(), None);
    }

    #[test]
    fn failed_reserve_is_transactional() {
        let mut map = MemoryMap::<1>::new();

        map.add_usable(range(0x1000, 0x9000)).unwrap();

        let result = map.reserve(range(0x3000, 0x5000));

        assert_eq!(result, Err(MemoryMapError::CapacityExceeded),);

        /*
         * 操作失败后，原范围必须保持完整。
         */
        assert_eq!(map.iter().next(), Some(range(0x1000, 0x9000)),);

        assert_eq!(map.len(), 1);
    }

    #[test]
    fn total_size_is_checked() {
        let mut map = MemoryMap::<2>::new();

        map.add_usable(range(0x1000, 0x3000)).unwrap();

        map.add_usable(range(0x5000, 0x8000)).unwrap();

        assert_eq!(map.total_bytes(), Some(0x5000));
    }
}
