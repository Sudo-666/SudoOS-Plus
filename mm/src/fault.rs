use crate::{AddressSpace, VirtAddr, VmAreaFlags, VmAreaKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultAccess {
    Read,
    Write,
    Execute,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultSource {
    User,
    Kernel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageFault {
    address: VirtAddr,
    access: FaultAccess,
    source: FaultSource,
    present: bool,
}

impl PageFault {
    pub const fn new(
        address: VirtAddr,
        access: FaultAccess,
        source: FaultSource,
        present: bool,
    ) -> Self {
        Self {
            address,
            access,
            source,
            present,
        }
    }

    pub const fn address(self) -> VirtAddr {
        self.address
    }

    pub const fn access(self) -> FaultAccess {
        self.access
    }

    pub const fn source(self) -> FaultSource {
        self.source
    }

    pub const fn is_present(self) -> bool {
        self.present
    }

    pub fn resolve<const VMA_CAPACITY: usize>(
        self,
        address_space: &AddressSpace<VMA_CAPACITY>,
    ) -> FaultOutcome {
        let Some(area) = address_space.find_area(self.address) else {
            return match self.source {
                FaultSource::User => FaultOutcome::SegmentationViolation,
                FaultSource::Kernel => FaultOutcome::KernelBug,
            };
        };

        if self.present && self.access == FaultAccess::Write && area.flags().is_copy_on_write() {
            return FaultOutcome::CopyOnWrite { area };
        }

        if !access_allowed(area.flags(), self.access) {
            return FaultOutcome::ProtectionViolation { area };
        }

        if self.present {
            return FaultOutcome::Spurious { area };
        }

        match area.kind() {
            VmAreaKind::Anonymous | VmAreaKind::Heap | VmAreaKind::Stack => {
                FaultOutcome::MapAnonymous { area }
            }
            VmAreaKind::FileBacked { object, offset } => {
                let in_area = self.address.get() - area.range().start().get();
                FaultOutcome::LoadFile {
                    area,
                    object,
                    offset: offset + in_area as u64,
                }
            }
            VmAreaKind::Device { physical } | VmAreaKind::IoRemap { physical } => {
                FaultOutcome::MapDevice { area, physical }
            }
            VmAreaKind::Kernel | VmAreaKind::Vmalloc => FaultOutcome::KernelBug,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultOutcome {
    MapAnonymous {
        area: crate::VmArea,
    },
    CopyOnWrite {
        area: crate::VmArea,
    },
    LoadFile {
        area: crate::VmArea,
        object: u64,
        offset: u64,
    },
    MapDevice {
        area: crate::VmArea,
        physical: crate::PhysAddr,
    },
    ProtectionViolation {
        area: crate::VmArea,
    },
    SegmentationViolation,
    Spurious {
        area: crate::VmArea,
    },
    KernelBug,
}

fn access_allowed(flags: VmAreaFlags, access: FaultAccess) -> bool {
    match access {
        FaultAccess::Read => flags.is_readable(),
        FaultAccess::Write => flags.is_writable() || flags.is_copy_on_write(),
        FaultAccess::Execute => flags.is_executable(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AddressSpace, VirtRange, VmArea, VmAreaKind};

    #[test]
    fn resolves_anonymous_fault() {
        let mut space: AddressSpace<4> =
            AddressSpace::new(VirtRange::from_bounds(0x1000, 0x1_0000));

        let area = VmArea::new(
            VirtRange::from_bounds(0x4000, 0x5000),
            VmAreaFlags::user_rw(),
            VmAreaKind::Anonymous,
        );

        space.map_area(area).unwrap();

        let fault = PageFault::new(
            VirtAddr::new(0x4123),
            FaultAccess::Read,
            FaultSource::User,
            false,
        );

        assert_eq!(fault.resolve(&space), FaultOutcome::MapAnonymous { area });
    }

    #[test]
    fn resolves_cow_fault() {
        let mut space: AddressSpace<4> =
            AddressSpace::new(VirtRange::from_bounds(0x1000, 0x1_0000));

        let flags = VmAreaFlags::READ
            .union(VmAreaFlags::USER)
            .union(VmAreaFlags::PRIVATE)
            .union(VmAreaFlags::COPY_ON_WRITE);

        let area = VmArea::new(
            VirtRange::from_bounds(0x4000, 0x5000),
            flags,
            VmAreaKind::Anonymous,
        );

        space.map_area(area).unwrap();

        let fault = PageFault::new(
            VirtAddr::new(0x4123),
            FaultAccess::Write,
            FaultSource::User,
            true,
        );

        assert_eq!(fault.resolve(&space), FaultOutcome::CopyOnWrite { area });
    }
}
