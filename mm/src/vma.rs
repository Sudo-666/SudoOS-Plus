use alloc::vec::Vec;

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
    MetadataOutOfMemory,
    Overlap,
    NotFound,
    AddressOverflow,
}

#[derive(Clone)]
pub struct VmAreaSet<const CAPACITY: usize> {
    areas: Vec<VmArea>,
}

impl<const CAPACITY: usize> VmAreaSet<CAPACITY> {
    pub const fn new() -> Self {
        Self { areas: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.areas.len()
    }

    pub const fn capacity(&self) -> usize {
        CAPACITY
    }

    pub fn is_empty(&self) -> bool {
        self.areas.is_empty()
    }

    pub fn area_at(&self, index: usize) -> Option<VmArea> {
        self.areas.get(index).copied()
    }

    pub fn insert(&mut self, area: VmArea) -> Result<(), VmAreaError> {
        validate_area(area)?;

        let index = self.insertion_index(area.range().start());

        if index > 0 {
            let previous = self.areas[index - 1];

            if previous.range().overlaps(area.range()) {
                return Err(VmAreaError::Overlap);
            }
        }

        if index < self.len() {
            let next = self.areas[index];

            if next.range().overlaps(area.range()) {
                return Err(VmAreaError::Overlap);
            }
        }

        // Linux coalesces adjacent VMAs with identical attributes. This is
        // essential for allocators such as jemalloc, which issue many
        // contiguous MAP_FIXED arena mappings; retaining every call as a
        // separate entry exhausts bounded kernel metadata even though the
        // resulting address-space topology is simple.
        let coalescible = matches!(area.kind(), VmAreaKind::Anonymous);
        let merge_previous = coalescible && index > 0 && {
            let previous = self.areas[index - 1];
            previous.range().end() == area.range().start()
                && previous.flags() == area.flags()
                && previous.kind() == area.kind()
        };
        let merge_next = coalescible && index < self.len() && {
            let next = self.areas[index];
            area.range().end() == next.range().start()
                && next.flags() == area.flags()
                && next.kind() == area.kind()
        };

        if merge_previous && merge_next {
            let previous = self.areas[index - 1];
            let next = self.areas[index];
            let range = VirtRange::new(previous.range().start(), next.range().end())
                .ok_or(VmAreaError::AddressOverflow)?;
            self.areas[index - 1] = VmArea::new(range, area.flags(), area.kind());
            self.areas.remove(index);
            return Ok(());
        }
        if merge_previous {
            let previous = self.areas[index - 1];
            let range = VirtRange::new(previous.range().start(), area.range().end())
                .ok_or(VmAreaError::AddressOverflow)?;
            self.areas[index - 1] = VmArea::new(range, area.flags(), area.kind());
            return Ok(());
        }
        if merge_next {
            let next = self.areas[index];
            let range = VirtRange::new(area.range().start(), next.range().end())
                .ok_or(VmAreaError::AddressOverflow)?;
            self.areas[index] = VmArea::new(range, area.flags(), area.kind());
            return Ok(());
        }

        if self.len() == CAPACITY {
            return Err(VmAreaError::CapacityExceeded);
        }

        self.areas
            .try_reserve(1)
            .map_err(|_| VmAreaError::MetadataOutOfMemory)?;
        self.areas.insert(index, area);

        Ok(())
    }

    pub fn remove_exact(&mut self, range: VirtRange) -> Result<VmArea, VmAreaError> {
        let index = self.find_exact_index(range).ok_or(VmAreaError::NotFound)?;

        Ok(self.areas.remove(index))
    }

    /// Removes every part of every VMA intersecting `range`.
    ///
    /// Linux `munmap()` accepts holes, so a valid range that intersects no VMA
    /// succeeds and reports zero affected areas. The update is transactional:
    /// capacity and all replacement fragments are validated before publication.
    pub fn remove_range(&mut self, range: VirtRange) -> Result<usize, VmAreaError> {
        validate_operation_range(range)?;
        // The vector is ordered. Locate only the overlapping window instead
        // of rebuilding and reallocating the complete VMA table for every
        // munmap. Rustc performs thousands of small unmaps while retaining a
        // large address space, so the old implementation became quadratic.
        let first = self
            .areas
            .partition_point(|area| area.range().end() <= range.start());
        let mut last = first;
        while last < self.len() && self.areas[last].range().start() < range.end() {
            last += 1;
        }
        let affected = last - first;
        if affected == 0 {
            return Ok(0);
        }

        let first_area = self.areas[first];
        let last_area = self.areas[last - 1];
        let mut replacements = Vec::new();
        replacements
            .try_reserve(2)
            .map_err(|_| VmAreaError::MetadataOutOfMemory)?;
        if first_area.range().start() < range.start() {
            let left = VirtRange::new(first_area.range().start(), range.start())
                .ok_or(VmAreaError::AddressOverflow)?;
            replacements.push(VmArea::new(left, first_area.flags(), first_area.kind()));
        }
        if range.end() < last_area.range().end() {
            let right = VirtRange::new(range.end(), last_area.range().end())
                .ok_or(VmAreaError::AddressOverflow)?;
            replacements.push(VmArea::new(right, last_area.flags(), last_area.kind()));
        }
        let new_len = self
            .len()
            .checked_sub(affected)
            .and_then(|length| length.checked_add(replacements.len()))
            .ok_or(VmAreaError::AddressOverflow)?;
        if new_len > CAPACITY {
            return Err(VmAreaError::CapacityExceeded);
        }
        self.areas
            .try_reserve(replacements.len().saturating_sub(affected))
            .map_err(|_| VmAreaError::MetadataOutOfMemory)?;
        self.areas.splice(first..last, replacements);
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
        for area in self.areas.iter().copied() {
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

        let mut rebuilt = Vec::new();
        rebuilt
            .try_reserve(core::cmp::min(self.len().saturating_add(2), CAPACITY))
            .map_err(|_| VmAreaError::MetadataOutOfMemory)?;
        for area in self.areas.iter().copied() {
            if !area.range().overlaps(range) {
                append_area(&mut rebuilt, area, CAPACITY)?;
                continue;
            }

            if area.range().start() < range.start() {
                let left = VirtRange::new(area.range().start(), range.start())
                    .ok_or(VmAreaError::AddressOverflow)?;
                append_area(
                    &mut rebuilt,
                    VmArea::new(left, area.flags(), area.kind()),
                    CAPACITY,
                )?;
            }

            let protected_start = core::cmp::max(area.range().start(), range.start());
            let protected_end = core::cmp::min(area.range().end(), range.end());
            let protected = VirtRange::new(protected_start, protected_end)
                .ok_or(VmAreaError::AddressOverflow)?;
            append_area(
                &mut rebuilt,
                VmArea::new(protected, area.flags().with_access(access), area.kind()),
                CAPACITY,
            )?;

            if range.end() < area.range().end() {
                let right = VirtRange::new(range.end(), area.range().end())
                    .ok_or(VmAreaError::AddressOverflow)?;
                append_area(
                    &mut rebuilt,
                    VmArea::new(right, area.flags(), area.kind()),
                    CAPACITY,
                )?;
            }
        }

        self.areas = rebuilt;
        Ok(affected)
    }

    pub fn remove_kind(&mut self, kind: VmAreaKind) -> Result<usize, VmAreaError> {
        let mut rebuilt = Vec::new();
        rebuilt
            .try_reserve(self.len())
            .map_err(|_| VmAreaError::MetadataOutOfMemory)?;
        let mut removed = 0;
        for area in self.areas.iter().copied() {
            if area.kind() == kind {
                removed += 1;
            } else {
                append_area(&mut rebuilt, area, CAPACITY)?;
            }
        }
        self.areas = rebuilt;
        Ok(removed)
    }

    pub fn find(&self, address: VirtAddr) -> Option<VmArea> {
        let mut left = 0;
        let mut right = self.len();

        while left < right {
            let mid = left + (right - left) / 2;
            let area = self.areas[mid];

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

        for area in self.areas.iter().copied() {
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
        let mut right = self.len();

        while left < right {
            let mid = left + (right - left) / 2;
            let area = self.areas[mid];

            if start < area.range().start() {
                right = mid;
            } else {
                left = mid + 1;
            }
        }

        left
    }

    fn find_exact_index(&self, range: VirtRange) -> Option<usize> {
        self.areas.iter().position(|area| area.range() == range)
    }
}

impl<const CAPACITY: usize> Default for VmAreaSet<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

fn append_area(areas: &mut Vec<VmArea>, area: VmArea, capacity: usize) -> Result<(), VmAreaError> {
    validate_area(area)?;
    if let Some(previous) = areas.last().copied() {
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
            *areas.last_mut().expect("VMA predecessor disappeared") =
                VmArea::new(merged, area.flags(), area.kind());
            return Ok(());
        }
    }
    if areas.len() == capacity {
        return Err(VmAreaError::CapacityExceeded);
    }
    areas.push(area);
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
    fn dynamic_storage_keeps_the_declared_capacity_contract() {
        const ENTRIES: usize = 2_048;
        let mut set: VmAreaSet<ENTRIES> = VmAreaSet::new();
        for index in 0..ENTRIES {
            let start = 0x1000 + index * PAGE_SIZE * 2;
            set.insert(VmArea::new(
                range(start, start + PAGE_SIZE),
                VmAreaFlags::user_rw(),
                VmAreaKind::Anonymous,
            ))
            .unwrap();
        }
        assert_eq!(set.len(), ENTRIES);
        assert_eq!(
            set.insert(VmArea::new(
                range(
                    0x1000 + ENTRIES * PAGE_SIZE * 2,
                    0x2000 + ENTRIES * PAGE_SIZE * 2
                ),
                VmAreaFlags::user_rw(),
                VmAreaKind::Anonymous,
            )),
            Err(VmAreaError::CapacityExceeded),
        );

        let cloned = set.clone();
        assert_eq!(cloned.len(), ENTRIES);
        assert_eq!(
            cloned.area_at(ENTRIES - 1).unwrap().range(),
            range(
                0x1000 + (ENTRIES - 1) * PAGE_SIZE * 2,
                0x2000 + (ENTRIES - 1) * PAGE_SIZE * 2,
            ),
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

    #[test]
    fn mprotect_accepts_prot_none() {
        let mut set: VmAreaSet<8> = VmAreaSet::new();
        set.insert(VmArea::new(
            range(0x1000, 0x5000),
            VmAreaFlags::user_rw(),
            VmAreaKind::Anonymous,
        ))
        .unwrap();

        set.protect_range(range(0x2000, 0x3000), VmAreaFlags::empty())
            .unwrap();
        let flags = set.find(VirtAddr::new(0x2800)).unwrap().flags();
        assert!(flags.contains(VmAreaFlags::USER));
        assert_eq!(flags.access_only(), VmAreaFlags::empty());
    }
}
