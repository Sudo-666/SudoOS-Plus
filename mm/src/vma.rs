use crate::{MappingOptions, PAGE_SIZE, PhysAddr, VirtAddr, VirtRange};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct VmAreaFlags(u16);

impl VmAreaFlags {
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const EXECUTE: Self = Self(1 << 2);
    pub const USER: Self = Self(1 << 3);
    pub const SHARED: Self = Self(1 << 4);
    pub const PRIVATE: Self = Self(1 << 5);
    pub const COPY_ON_WRITE: Self = Self(1 << 6);
    pub const GROW_DOWN: Self = Self(1 << 7);
    pub const LOCKED: Self = Self(1 << 8);
    pub const DEVICE: Self = Self(1 << 9);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn kernel_rw() -> Self {
        Self(Self::READ.0 | Self::WRITE.0)
    }

    pub const fn user_rw() -> Self {
        Self(Self::READ.0 | Self::WRITE.0 | Self::USER.0 | Self::PRIVATE.0)
    }

    pub const fn user_rx() -> Self {
        Self(Self::READ.0 | Self::EXECUTE.0 | Self::USER.0 | Self::PRIVATE.0)
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    pub const fn access_only(self) -> Self {
        Self(self.0 & (Self::READ.0 | Self::WRITE.0 | Self::EXECUTE.0))
    }

    pub const fn with_access(self, access: Self) -> Self {
        self.without(Self::READ.union(Self::WRITE).union(Self::EXECUTE))
            .union(access.access_only())
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn is_readable(self) -> bool {
        self.contains(Self::READ)
    }

    pub const fn is_writable(self) -> bool {
        self.contains(Self::WRITE)
    }

    pub const fn is_executable(self) -> bool {
        self.contains(Self::EXECUTE)
    }

    pub const fn is_user(self) -> bool {
        self.contains(Self::USER)
    }

    pub const fn is_copy_on_write(self) -> bool {
        self.contains(Self::COPY_ON_WRITE)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VmAreaKind {
    Anonymous,
    Heap,
    Stack,
    FileBacked { object: u64, offset: u64 },
    Device { physical: PhysAddr },
    Kernel,
    Vmalloc,
    IoRemap { physical: PhysAddr },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VmArea {
    range: VirtRange,
    flags: VmAreaFlags,
    kind: VmAreaKind,
}

impl VmArea {
    pub const fn new(range: VirtRange, flags: VmAreaFlags, kind: VmAreaKind) -> Self {
        Self { range, flags, kind }
    }

    pub const fn range(self) -> VirtRange {
        self.range
    }

    pub const fn flags(self) -> VmAreaFlags {
        self.flags
    }

    pub const fn kind(self) -> VmAreaKind {
        self.kind
    }

    pub const fn contains(self, address: VirtAddr) -> bool {
        self.range.contains(address)
    }

    pub fn mapping_options(self) -> MappingOptions {
        let permissions = match (
            self.flags.is_readable(),
            self.flags.is_writable(),
            self.flags.is_executable(),
        ) {
            (true, true, true) => crate::PagePermissions::read_write_execute(),
            (true, true, false) => crate::PagePermissions::read_write(),
            (true, false, true) => crate::PagePermissions::read_execute(),
            (true, false, false) => crate::PagePermissions::read_only(),
            _ => crate::PagePermissions::empty(),
        };

        MappingOptions::new(permissions)
            .with_user(self.flags.is_user())
            .with_memory_type(if self.flags.contains(VmAreaFlags::DEVICE) {
                crate::MemoryType::Device
            } else {
                crate::MemoryType::Normal
            })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VmAreaError {
    EmptyRange,
    UnalignedRange,
    InvalidFlags,
    CapacityExceeded,
    Overlap,
    NotFound,
    AddressOverflow,
}

#[derive(Clone)]
pub struct VmAreaSet<const CAPACITY: usize> {
    areas: [Option<VmArea>; CAPACITY],
    len: usize,
}

impl<const CAPACITY: usize> VmAreaSet<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            areas: [None; CAPACITY],
            len: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn capacity(&self) -> usize {
        CAPACITY
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn area_at(&self, index: usize) -> Option<VmArea> {
        if index < self.len {
            self.areas[index]
        } else {
            None
        }
    }

    pub fn insert(&mut self, area: VmArea) -> Result<(), VmAreaError> {
        validate_area(area)?;

        if self.len == CAPACITY {
            return Err(VmAreaError::CapacityExceeded);
        }

        let index = self.insertion_index(area.range().start());

        if index > 0 {
            let previous = self.areas[index - 1].expect("VMA slot below len is empty");

            if previous.range().overlaps(area.range()) {
                return Err(VmAreaError::Overlap);
            }
        }

        if index < self.len {
            let next = self.areas[index].expect("VMA slot below len is empty");

            if next.range().overlaps(area.range()) {
                return Err(VmAreaError::Overlap);
            }
        }

        for slot in (index..self.len).rev() {
            self.areas[slot + 1] = self.areas[slot];
        }

        self.areas[index] = Some(area);
        self.len += 1;

        Ok(())
    }

    pub fn remove_exact(&mut self, range: VirtRange) -> Result<VmArea, VmAreaError> {
        let index = self.find_exact_index(range).ok_or(VmAreaError::NotFound)?;

        let removed = self.areas[index]
            .take()
            .expect("VMA slot below len is empty");

        for slot in index..self.len - 1 {
            self.areas[slot] = self.areas[slot + 1];
        }

        self.len -= 1;
        self.areas[self.len] = None;

        Ok(removed)
    }

    /// Removes every part of every VMA intersecting `range`.
    ///
    /// Linux `munmap()` accepts holes, so a valid range that intersects no VMA
    /// succeeds and reports zero affected areas. The update is transactional:
    /// capacity and all replacement fragments are validated before publication.
    pub fn remove_range(&mut self, range: VirtRange) -> Result<usize, VmAreaError> {
        validate_operation_range(range)?;

        let mut rebuilt = [None; CAPACITY];
        let mut rebuilt_len = 0;
        let mut affected = 0;

        for index in 0..self.len {
            let area = self.areas[index].expect("VMA slot below len is empty");
            if !area.range().overlaps(range) {
                append_area(&mut rebuilt, &mut rebuilt_len, area)?;
                continue;
            }

            affected += 1;
            if area.range().start() < range.start() {
                let left = VirtRange::new(area.range().start(), range.start())
                    .ok_or(VmAreaError::AddressOverflow)?;
                append_area(
                    &mut rebuilt,
                    &mut rebuilt_len,
                    VmArea::new(left, area.flags(), area.kind()),
                )?;
            }
            if range.end() < area.range().end() {
                let right = VirtRange::new(range.end(), area.range().end())
                    .ok_or(VmAreaError::AddressOverflow)?;
                append_area(
                    &mut rebuilt,
                    &mut rebuilt_len,
                    VmArea::new(right, area.flags(), area.kind()),
                )?;
            }
        }

        self.areas = rebuilt;
        self.len = rebuilt_len;
        Ok(affected)
    }

    /// Replaces access bits over a fully mapped range, splitting VMAs as needed.
    /// Gaps are rejected and the original topology remains unchanged on failure.
    pub fn protect_range(
        &mut self,
        range: VirtRange,
        access: VmAreaFlags,
    ) -> Result<usize, VmAreaError> {
        validate_operation_range(range)?;
        if access != access.access_only() {
            return Err(VmAreaError::InvalidFlags);
        }

        let mut cursor = range.start();
        let mut affected = 0;
        for index in 0..self.len {
            let area = self.areas[index].expect("VMA slot below len is empty");
            if area.range().end() <= cursor || area.range().start() >= range.end() {
                continue;
            }
            if area.range().start() > cursor {
                return Err(VmAreaError::NotFound);
            }
            cursor = core::cmp::min(area.range().end(), range.end());
            affected += 1;
            if cursor == range.end() {
                break;
            }
        }
        if cursor != range.end() {
            return Err(VmAreaError::NotFound);
        }

        let mut rebuilt = [None; CAPACITY];
        let mut rebuilt_len = 0;
        for index in 0..self.len {
            let area = self.areas[index].expect("VMA slot below len is empty");
            if !area.range().overlaps(range) {
                append_area(&mut rebuilt, &mut rebuilt_len, area)?;
                continue;
            }

            if area.range().start() < range.start() {
                let left = VirtRange::new(area.range().start(), range.start())
                    .ok_or(VmAreaError::AddressOverflow)?;
                append_area(
                    &mut rebuilt,
                    &mut rebuilt_len,
                    VmArea::new(left, area.flags(), area.kind()),
                )?;
            }

            let protected_start = core::cmp::max(area.range().start(), range.start());
            let protected_end = core::cmp::min(area.range().end(), range.end());
            let protected = VirtRange::new(protected_start, protected_end)
                .ok_or(VmAreaError::AddressOverflow)?;
            append_area(
                &mut rebuilt,
                &mut rebuilt_len,
                VmArea::new(protected, area.flags().with_access(access), area.kind()),
            )?;

            if range.end() < area.range().end() {
                let right = VirtRange::new(range.end(), area.range().end())
                    .ok_or(VmAreaError::AddressOverflow)?;
                append_area(
                    &mut rebuilt,
                    &mut rebuilt_len,
                    VmArea::new(right, area.flags(), area.kind()),
                )?;
            }
        }

        self.areas = rebuilt;
        self.len = rebuilt_len;
        Ok(affected)
    }

    pub fn remove_kind(&mut self, kind: VmAreaKind) -> Result<usize, VmAreaError> {
        let mut rebuilt = [None; CAPACITY];
        let mut rebuilt_len = 0;
        let mut removed = 0;
        for index in 0..self.len {
            let area = self.areas[index].expect("VMA slot below len is empty");
            if area.kind() == kind {
                removed += 1;
            } else {
                append_area(&mut rebuilt, &mut rebuilt_len, area)?;
            }
        }
        self.areas = rebuilt;
        self.len = rebuilt_len;
        Ok(removed)
    }

    pub fn find(&self, address: VirtAddr) -> Option<VmArea> {
        let mut left = 0;
        let mut right = self.len;

        while left < right {
            let mid = left + (right - left) / 2;
            let area = self.areas[mid].expect("VMA slot below len is empty");

            if area.range().contains(address) {
                return Some(area);
            }

            if address < area.range().start() {
                right = mid;
            } else {
                left = mid + 1;
            }
        }

        None
    }

    pub fn find_free_range(
        &self,
        search: VirtRange,
        size: usize,
        alignment: usize,
    ) -> Result<VirtRange, VmAreaError> {
        if size == 0 {
            return Err(VmAreaError::EmptyRange);
        }

        if alignment < PAGE_SIZE || !alignment.is_power_of_two() {
            return Err(VmAreaError::UnalignedRange);
        }

        let size = align_up(size, PAGE_SIZE).ok_or(VmAreaError::AddressOverflow)?;
        let mut candidate = search
            .start()
            .align_up(alignment)
            .ok_or(VmAreaError::AddressOverflow)?;

        for index in 0..self.len {
            let area = self.areas[index].expect("VMA slot below len is empty");

            if area.range().end() <= candidate {
                continue;
            }

            let Some(end) = candidate.checked_add(size) else {
                return Err(VmAreaError::AddressOverflow);
            };

            let gap = VirtRange::new(candidate, end).ok_or(VmAreaError::AddressOverflow)?;

            if search.contains_range(gap) && !gap.overlaps(area.range()) {
                return Ok(gap);
            }

            candidate = area
                .range()
                .end()
                .align_up(alignment)
                .ok_or(VmAreaError::AddressOverflow)?;
        }

        let end = candidate
            .checked_add(size)
            .ok_or(VmAreaError::AddressOverflow)?;

        let gap = VirtRange::new(candidate, end).ok_or(VmAreaError::AddressOverflow)?;

        if search.contains_range(gap) {
            Ok(gap)
        } else {
            Err(VmAreaError::CapacityExceeded)
        }
    }

    fn insertion_index(&self, start: VirtAddr) -> usize {
        let mut left = 0;
        let mut right = self.len;

        while left < right {
            let mid = left + (right - left) / 2;
            let area = self.areas[mid].expect("VMA slot below len is empty");

            if start < area.range().start() {
                right = mid;
            } else {
                left = mid + 1;
            }
        }

        left
    }

    fn find_exact_index(&self, range: VirtRange) -> Option<usize> {
        (0..self.len).find(|index| {
            self.areas[*index]
                .map(|area| area.range() == range)
                .unwrap_or(false)
        })
    }
}

impl<const CAPACITY: usize> Default for VmAreaSet<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

fn append_area<const CAPACITY: usize>(
    areas: &mut [Option<VmArea>; CAPACITY],
    len: &mut usize,
    area: VmArea,
) -> Result<(), VmAreaError> {
    validate_area(area)?;
    if *len > 0 {
        let previous = areas[*len - 1].expect("rebuilt VMA slot below len is empty");
        if previous.range().overlaps(area.range())
            || previous.range().start() > area.range().start()
        {
            return Err(VmAreaError::Overlap);
        }
        if previous.range().end() == area.range().start()
            && previous.flags() == area.flags()
            && previous.kind() == area.kind()
        {
            let merged = VirtRange::new(previous.range().start(), area.range().end())
                .ok_or(VmAreaError::AddressOverflow)?;
            areas[*len - 1] = Some(VmArea::new(merged, area.flags(), area.kind()));
            return Ok(());
        }
    }
    if *len == CAPACITY {
        return Err(VmAreaError::CapacityExceeded);
    }
    areas[*len] = Some(area);
    *len += 1;
    Ok(())
}

fn validate_operation_range(range: VirtRange) -> Result<(), VmAreaError> {
    if range.is_empty() {
        return Err(VmAreaError::EmptyRange);
    }
    if !range.is_page_aligned() {
        return Err(VmAreaError::UnalignedRange);
    }
    Ok(())
}

fn validate_area(area: VmArea) -> Result<(), VmAreaError> {
    if area.range().is_empty() {
        return Err(VmAreaError::EmptyRange);
    }

    if !area.range().is_page_aligned() {
        return Err(VmAreaError::UnalignedRange);
    }

    let flags = area.flags();

    if !flags.is_readable() && !flags.is_writable() && !flags.is_executable() {
        return Err(VmAreaError::InvalidFlags);
    }

    if flags.is_writable() && !flags.is_readable() {
        return Err(VmAreaError::InvalidFlags);
    }

    if flags.contains(VmAreaFlags::SHARED) && flags.contains(VmAreaFlags::PRIVATE) {
        return Err(VmAreaError::InvalidFlags);
    }

    if flags.is_copy_on_write() && !flags.contains(VmAreaFlags::PRIVATE) {
        return Err(VmAreaError::InvalidFlags);
    }

    Ok(())
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    if alignment == 0 || !alignment.is_power_of_two() {
        return None;
    }

    value
        .checked_add(alignment - 1)
        .map(|v| v & !(alignment - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: usize, end: usize) -> VirtRange {
        VirtRange::from_bounds(start, end)
    }

    #[test]
    fn inserts_sorted_and_finds_area() {
        let mut set: VmAreaSet<4> = VmAreaSet::new();

        set.insert(VmArea::new(
            range(0x4000, 0x5000),
            VmAreaFlags::user_rw(),
            VmAreaKind::Anonymous,
        ))
        .unwrap();

        set.insert(VmArea::new(
            range(0x1000, 0x2000),
            VmAreaFlags::user_rx(),
            VmAreaKind::Anonymous,
        ))
        .unwrap();

        assert_eq!(set.area_at(0).unwrap().range(), range(0x1000, 0x2000));
        assert_eq!(
            set.find(VirtAddr::new(0x4800)).unwrap().range(),
            range(0x4000, 0x5000)
        );
    }

    #[test]
    fn rejects_overlapping_area() {
        let mut set: VmAreaSet<2> = VmAreaSet::new();

        set.insert(VmArea::new(
            range(0x1000, 0x3000),
            VmAreaFlags::user_rw(),
            VmAreaKind::Anonymous,
        ))
        .unwrap();

        assert_eq!(
            set.insert(VmArea::new(
                range(0x2000, 0x4000),
                VmAreaFlags::user_rw(),
                VmAreaKind::Anonymous,
            )),
            Err(VmAreaError::Overlap),
        );
    }

    #[test]
    fn finds_gap_between_areas() {
        let mut set: VmAreaSet<4> = VmAreaSet::new();

        set.insert(VmArea::new(
            range(0x1000, 0x2000),
            VmAreaFlags::user_rw(),
            VmAreaKind::Anonymous,
        ))
        .unwrap();

        set.insert(VmArea::new(
            range(0x4000, 0x5000),
            VmAreaFlags::user_rw(),
            VmAreaKind::Anonymous,
        ))
        .unwrap();

        assert_eq!(
            set.find_free_range(range(0x1000, 0x8000), PAGE_SIZE, PAGE_SIZE)
                .unwrap(),
            range(0x2000, 0x3000),
        );
    }

    #[test]
    fn munmap_splits_and_accepts_holes_transactionally() {
        let mut set: VmAreaSet<6> = VmAreaSet::new();
        set.insert(VmArea::new(
            range(0x1000, 0x5000),
            VmAreaFlags::user_rw(),
            VmAreaKind::Anonymous,
        ))
        .unwrap();

        assert_eq!(set.remove_range(range(0x2000, 0x4000)).unwrap(), 1);
        assert_eq!(set.area_at(0).unwrap().range(), range(0x1000, 0x2000));
        assert_eq!(set.area_at(1).unwrap().range(), range(0x4000, 0x5000));
        assert_eq!(set.remove_range(range(0x8000, 0x9000)).unwrap(), 0);
    }

    #[test]
    fn rebuild_coalesces_adjacent_equivalent_areas() {
        let mut set: VmAreaSet<8> = VmAreaSet::new();
        set.insert(VmArea::new(
            range(0x1000, 0x5000),
            VmAreaFlags::user_rw(),
            VmAreaKind::Anonymous,
        ))
        .unwrap();

        set.protect_range(range(0x2000, 0x4000), VmAreaFlags::READ)
            .unwrap();
        assert_eq!(set.len(), 3);
        set.protect_range(
            range(0x2000, 0x4000),
            VmAreaFlags::READ.union(VmAreaFlags::WRITE),
        )
        .unwrap();
        assert_eq!(set.len(), 1);
        assert_eq!(set.area_at(0).unwrap().range(), range(0x1000, 0x5000));
    }

    #[test]
    fn mprotect_splits_full_coverage_and_rejects_gaps() {
        let mut set: VmAreaSet<8> = VmAreaSet::new();
        set.insert(VmArea::new(
            range(0x1000, 0x5000),
            VmAreaFlags::user_rw(),
            VmAreaKind::Anonymous,
        ))
        .unwrap();

        assert_eq!(
            set.protect_range(range(0x2000, 0x4000), VmAreaFlags::READ)
                .unwrap(),
            1,
        );
        assert!(
            set.find(VirtAddr::new(0x1800))
                .unwrap()
                .flags()
                .is_writable()
        );
        assert!(
            !set.find(VirtAddr::new(0x2800))
                .unwrap()
                .flags()
                .is_writable()
        );
        assert!(
            set.find(VirtAddr::new(0x4800))
                .unwrap()
                .flags()
                .is_writable()
        );
        assert_eq!(
            set.protect_range(range(0x5000, 0x6000), VmAreaFlags::READ),
            Err(VmAreaError::NotFound),
        );
    }
}
