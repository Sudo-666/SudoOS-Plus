use crate::{
    PAGE_SIZE, VirtAddr, VirtRange, VmArea, VmAreaError, VmAreaFlags, VmAreaKind, VmAreaSet,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProgramBreak {
    start: VirtAddr,
    current: VirtAddr,
    limit: VirtAddr,
}

impl ProgramBreak {
    pub const fn new(start: VirtAddr, current: VirtAddr, limit: VirtAddr) -> Option<Self> {
        if start.get() <= current.get() && current.get() <= limit.get() {
            Some(Self {
                start,
                current,
                limit,
            })
        } else {
            None
        }
    }

    pub const fn start(self) -> VirtAddr {
        self.start
    }

    pub const fn current(self) -> VirtAddr {
        self.current
    }

    pub const fn limit(self) -> VirtAddr {
        self.limit
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddressSpaceError {
    Area(VmAreaError),
    RangeOutsideUser,
    InvalidProgramBreak,
    ProgramBreakNotConfigured,
}

impl From<VmAreaError> for AddressSpaceError {
    fn from(error: VmAreaError) -> Self {
        Self::Area(error)
    }
}

#[derive(Clone)]
pub struct AddressSpace<const VMA_CAPACITY: usize> {
    user_range: VirtRange,
    areas: VmAreaSet<VMA_CAPACITY>,
    program_break: Option<ProgramBreak>,
}

impl<const VMA_CAPACITY: usize> AddressSpace<VMA_CAPACITY> {
    pub const fn new(user_range: VirtRange) -> Self {
        Self {
            user_range,
            areas: VmAreaSet::new(),
            program_break: None,
        }
    }

    pub const fn user_range(&self) -> VirtRange {
        self.user_range
    }

    pub fn area_count(&self) -> usize {
        self.areas.len()
    }

    pub fn area_at(&self, index: usize) -> Option<VmArea> {
        self.areas.area_at(index)
    }

    pub fn map_area(&mut self, area: VmArea) -> Result<(), AddressSpaceError> {
        if !self.user_range.contains_range(area.range()) {
            return Err(AddressSpaceError::RangeOutsideUser);
        }

        self.areas.insert(area)?;

        Ok(())
    }

    pub fn unmap_exact(&mut self, range: VirtRange) -> Result<VmArea, AddressSpaceError> {
        Ok(self.areas.remove_exact(range)?)
    }

    pub fn unmap_range(&mut self, range: VirtRange) -> Result<usize, AddressSpaceError> {
        if !self.user_range.contains_range(range) {
            return Err(AddressSpaceError::RangeOutsideUser);
        }
        Ok(self.areas.remove_range(range)?)
    }

    pub fn protect_range(
        &mut self,
        range: VirtRange,
        access: VmAreaFlags,
    ) -> Result<usize, AddressSpaceError> {
        if !self.user_range.contains_range(range) {
            return Err(AddressSpaceError::RangeOutsideUser);
        }
        Ok(self.areas.protect_range(range, access)?)
    }

    pub fn find_area(&self, address: VirtAddr) -> Option<VmArea> {
        self.areas.find(address)
    }

    pub fn configure_program_break(
        &mut self,
        start: VirtAddr,
        limit: VirtAddr,
    ) -> Result<(), AddressSpaceError> {
        if !self.user_range.contains(start) {
            return Err(AddressSpaceError::RangeOutsideUser);
        }

        if limit < start
            || !self
                .user_range
                .contains(limit.checked_sub(1).unwrap_or(limit))
        {
            return Err(AddressSpaceError::RangeOutsideUser);
        }

        self.program_break = Some(
            ProgramBreak::new(start, start, limit).ok_or(AddressSpaceError::InvalidProgramBreak)?,
        );

        Ok(())
    }

    pub fn program_break(&self) -> Option<ProgramBreak> {
        self.program_break
    }

    pub fn set_program_break(
        &mut self,
        new_break: VirtAddr,
    ) -> Result<VirtAddr, AddressSpaceError> {
        let mut brk = self
            .program_break
            .ok_or(AddressSpaceError::ProgramBreakNotConfigured)?;

        if new_break < brk.start || new_break > brk.limit {
            return Err(AddressSpaceError::InvalidProgramBreak);
        }

        brk.current = new_break;
        self.program_break = Some(brk);

        Ok(brk.current)
    }

    /// Updates the logical program break and publishes one contiguous heap VMA.
    /// The old break and VMA topology are restored if rebuilding the heap fails.
    pub fn set_program_break_and_sync_heap(
        &mut self,
        new_break: VirtAddr,
    ) -> Result<VirtAddr, AddressSpaceError> {
        let old_break = self.program_break;
        let old_areas = self.areas.clone();

        // Capture the lowest existing heap page BEFORE removing the heap area.
        // A MAP_FIXED mapping can carve the head of the heap (the generic mmap
        // unmap_range splits the heap VMA and keeps `brk.start` at the
        // pre-carve address). Re-mapping from the raw `brk.start` on the next
        // brk()/fork would then collide with the mapping that carved it, so the
        // heap must keep its current low edge. When no heap area exists yet
        // (first brk after exec) this is `None` and brk.start is used.
        let existing_heap_start = {
            let mut lowest: Option<VirtAddr> = None;
            for index in 0..self.areas.len() {
                let area = self.areas.area_at(index).expect("index below count");
                if area.kind() == VmAreaKind::Heap {
                    let start = area.range().start();
                    if lowest.is_none_or(|current| start < current) {
                        lowest = Some(start);
                    }
                }
            }
            lowest
        };

        let result = (|| {
            let current = self.set_program_break(new_break)?;
            self.areas.remove_kind(VmAreaKind::Heap)?;
            let brk = self
                .program_break
                .ok_or(AddressSpaceError::ProgramBreakNotConfigured)?;
            let mut start = brk
                .start()
                .align_down(PAGE_SIZE)
                .ok_or(AddressSpaceError::InvalidProgramBreak)?;
            if let Some(existing) = existing_heap_start {
                start = core::cmp::max(start, existing);
            }
            let end = current
                .align_up(PAGE_SIZE)
                .ok_or(AddressSpaceError::InvalidProgramBreak)?;
            if end > start {
                self.map_area(VmArea::new(
                    VirtRange::new(start, end).ok_or(AddressSpaceError::InvalidProgramBreak)?,
                    VmAreaFlags::user_rw(),
                    VmAreaKind::Heap,
                ))?;
            }
            Ok(current)
        })();

        if result.is_err() {
            self.program_break = old_break;
            self.areas = old_areas;
        }
        result
    }

    pub fn map_anonymous(
        &mut self,
        search: VirtRange,
        size: usize,
        alignment: usize,
        flags: VmAreaFlags,
    ) -> Result<VmArea, AddressSpaceError> {
        if !self.user_range.contains_range(search) {
            return Err(AddressSpaceError::RangeOutsideUser);
        }

        let range = self.areas.find_free_range(search, size, alignment)?;

        let area = VmArea::new(range, flags, VmAreaKind::Anonymous);

        self.map_area(area)?;

        Ok(area)
    }

    pub fn map_heap_from_break(&mut self) -> Result<Option<VmArea>, AddressSpaceError> {
        let brk = self
            .program_break
            .ok_or(AddressSpaceError::ProgramBreakNotConfigured)?;

        let start = brk
            .start
            .align_down(PAGE_SIZE)
            .ok_or(AddressSpaceError::InvalidProgramBreak)?;
        let end = brk
            .current
            .align_up(PAGE_SIZE)
            .ok_or(AddressSpaceError::InvalidProgramBreak)?;

        if end <= start {
            return Ok(None);
        }

        let range = VirtRange::new(start, end).ok_or(AddressSpaceError::InvalidProgramBreak)?;

        let area = VmArea::new(range, VmAreaFlags::user_rw(), VmAreaKind::Heap);

        self.map_area(area)?;

        Ok(Some(area))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: VirtRange = VirtRange::from_bounds(0x1000, 0x1_0000);

    #[test]
    fn rejects_area_outside_user_range() {
        let mut space: AddressSpace<4> = AddressSpace::new(USER);

        assert_eq!(
            space.map_area(VmArea::new(
                VirtRange::from_bounds(0x0, 0x1000),
                VmAreaFlags::user_rw(),
                VmAreaKind::Anonymous,
            )),
            Err(AddressSpaceError::RangeOutsideUser),
        );
    }

    #[test]
    fn tracks_program_break() {
        let mut space: AddressSpace<4> = AddressSpace::new(USER);

        space
            .configure_program_break(VirtAddr::new(0x4000), VirtAddr::new(0x8000))
            .unwrap();

        assert_eq!(
            space.set_program_break(VirtAddr::new(0x6123)).unwrap(),
            VirtAddr::new(0x6123),
        );

        let area = space.map_heap_from_break().unwrap().unwrap();

        assert_eq!(area.range(), VirtRange::from_bounds(0x4000, 0x7000));
    }

    #[test]
    fn program_break_rebuild_is_transactional_and_shrinkable() {
        let mut space: AddressSpace<8> = AddressSpace::new(USER);
        space
            .configure_program_break(VirtAddr::new(0x4000), VirtAddr::new(0x9000))
            .unwrap();
        space
            .set_program_break_and_sync_heap(VirtAddr::new(0x7123))
            .unwrap();
        assert_eq!(
            space.find_area(VirtAddr::new(0x6000)).unwrap().kind(),
            VmAreaKind::Heap,
        );
        space
            .set_program_break_and_sync_heap(VirtAddr::new(0x4000))
            .unwrap();
        assert!(space.find_area(VirtAddr::new(0x4000)).is_none());
    }

    #[test]
    fn heap_rebuild_preserves_map_fixed_carve() {
        let mut space: AddressSpace<8> = AddressSpace::new(USER);
        space
            .configure_program_break(VirtAddr::new(0x4000), VirtAddr::new(0x9000))
            .unwrap();
        space
            .set_program_break_and_sync_heap(VirtAddr::new(0x7000))
            .unwrap();
        assert_eq!(
            space.find_area(VirtAddr::new(0x6000)).unwrap().kind(),
            VmAreaKind::Heap,
        );

        // A MAP_FIXED mapping carves the head of the heap VMA: the generic
        // mmap path unmaps the target range first, splitting [0x4000,0x7000)
        // into Heap [0x5000,0x7000) while `brk.start` stays at 0x4000, then
        // maps its own anonymous area over the freed page.
        space
            .unmap_range(VirtRange::from_bounds(0x4000, 0x5000))
            .unwrap();
        space
            .map_area(VmArea::new(
                VirtRange::from_bounds(0x4000, 0x5000),
                VmAreaFlags::user_rw(),
                VmAreaKind::Anonymous,
            ))
            .unwrap();

        // Re-syncing the heap at the SAME break must keep the heap low edge at
        // 0x5000 (the carved page) instead of re-mapping from the stale
        // brk.start 0x4000 and colliding with the anonymous mapping.
        space
            .set_program_break_and_sync_heap(VirtAddr::new(0x7000))
            .unwrap();
        assert_eq!(
            space.find_area(VirtAddr::new(0x6000)).unwrap().kind(),
            VmAreaKind::Heap,
        );
        assert_eq!(
            space.find_area(VirtAddr::new(0x4000)).unwrap().kind(),
            VmAreaKind::Anonymous,
        );

        // Growing the heap still preserves the carve.
        space
            .set_program_break_and_sync_heap(VirtAddr::new(0x8000))
            .unwrap();
        assert_eq!(
            space.find_area(VirtAddr::new(0x7000)).unwrap().kind(),
            VmAreaKind::Heap,
        );
        assert_eq!(
            space.find_area(VirtAddr::new(0x4500)).unwrap().kind(),
            VmAreaKind::Anonymous,
        );
    }
}
