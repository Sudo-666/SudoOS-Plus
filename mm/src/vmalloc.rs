use crate::{
    PAGE_SIZE, PhysAddr, VirtRange, VmArea, VmAreaError, VmAreaFlags, VmAreaKind, VmAreaSet,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VmallocKind {
    Vmalloc,
    IoRemap { physical: PhysAddr },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KernelVirtualReservation {
    usable: VirtRange,
    reserved: VirtRange,
    kind: VmallocKind,
}

impl KernelVirtualReservation {
    pub const fn usable(self) -> VirtRange {
        self.usable
    }

    pub const fn reserved(self) -> VirtRange {
        self.reserved
    }

    pub const fn kind(self) -> VmallocKind {
        self.kind
    }
}

pub struct KernelVirtualAllocator<const CAPACITY: usize> {
    arena: VirtRange,
    areas: VmAreaSet<CAPACITY>,
}

impl<const CAPACITY: usize> KernelVirtualAllocator<CAPACITY> {
    pub const fn new(arena: VirtRange) -> Self {
        Self {
            arena,
            areas: VmAreaSet::new(),
        }
    }

    pub const fn arena(&self) -> VirtRange {
        self.arena
    }

    pub const fn reservation_count(&self) -> usize {
        self.areas.len()
    }

    pub fn reserve(
        &mut self,
        size: usize,
        alignment: usize,
        kind: VmallocKind,
    ) -> Result<KernelVirtualReservation, VmAreaError> {
        let size = align_up(size, PAGE_SIZE).ok_or(VmAreaError::AddressOverflow)?;
        let alignment = alignment.max(PAGE_SIZE);

        if !alignment.is_power_of_two() {
            return Err(VmAreaError::UnalignedRange);
        }

        let guarded_size = size
            .checked_add(PAGE_SIZE * 2)
            .ok_or(VmAreaError::AddressOverflow)?;

        let reserved = self
            .areas
            .find_free_range(self.arena, guarded_size, alignment)?;

        let usable_start = reserved
            .start()
            .checked_add(PAGE_SIZE)
            .ok_or(VmAreaError::AddressOverflow)?;

        let usable =
            VirtRange::from_start_size(usable_start, size).ok_or(VmAreaError::AddressOverflow)?;

        let flags = match kind {
            VmallocKind::Vmalloc => VmAreaFlags::kernel_rw(),
            VmallocKind::IoRemap { .. } => VmAreaFlags::kernel_rw().union(VmAreaFlags::DEVICE),
        };

        let area_kind = match kind {
            VmallocKind::Vmalloc => VmAreaKind::Vmalloc,
            VmallocKind::IoRemap { physical } => VmAreaKind::IoRemap { physical },
        };

        self.areas.insert(VmArea::new(reserved, flags, area_kind))?;

        Ok(KernelVirtualReservation {
            usable,
            reserved,
            kind,
        })
    }

    pub fn release(&mut self, reservation: KernelVirtualReservation) -> Result<(), VmAreaError> {
        self.areas.remove_exact(reservation.reserved)?;

        Ok(())
    }
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

    #[test]
    fn reserves_guarded_kernel_virtual_range() {
        let mut allocator: KernelVirtualAllocator<4> =
            KernelVirtualAllocator::new(VirtRange::from_bounds(0xffff_0000, 0xffff_8000));

        let reservation = allocator
            .reserve(PAGE_SIZE, PAGE_SIZE, VmallocKind::Vmalloc)
            .unwrap();

        assert_eq!(
            reservation.usable().start().get(),
            reservation.reserved().start().get() + PAGE_SIZE
        );
        assert_eq!(reservation.usable().size(), PAGE_SIZE);
        assert_eq!(reservation.reserved().size(), PAGE_SIZE * 3);

        allocator.release(reservation).unwrap();

        assert_eq!(allocator.reservation_count(), 0);
    }
}
