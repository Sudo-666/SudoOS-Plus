use crate::{VirtAddr, VirtRange};

/// 一个具有固定用途的虚拟地址区域。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VirtualRegion {
    name: &'static str,
    range: VirtRange,
}

impl VirtualRegion {
    pub const fn new(name: &'static str, range: VirtRange) -> Self {
        Self { name, range }
    }

    pub const fn name(self) -> &'static str {
        self.name
    }

    pub const fn range(self) -> VirtRange {
        self.range
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VirtualLayoutError {
    EmptyRegion {
        name: &'static str,
    },

    UnalignedRegion {
        name: &'static str,
    },

    AddressOutsideArchitecture {
        name: &'static str,
        address: VirtAddr,
    },

    OverlappingRegions {
        left: &'static str,
        right: &'static str,
    },

    AddressOutsideRegion {
        name: &'static str,
        address: VirtAddr,
        region: &'static str,
    },
}

/// 验证一组互不重叠的虚拟地址区域。
///
/// `address_is_allowed` 由架构提供，用于验证规范地址或特权范围。
pub fn validate_regions(
    regions: &[VirtualRegion],
    address_is_allowed: fn(VirtAddr) -> bool,
) -> Result<(), VirtualLayoutError> {
    for region in regions {
        let range = region.range();

        if range.is_empty() {
            return Err(VirtualLayoutError::EmptyRegion {
                name: region.name(),
            });
        }

        if !range.is_page_aligned() {
            return Err(VirtualLayoutError::UnalignedRegion {
                name: region.name(),
            });
        }

        if !address_is_allowed(range.start()) {
            return Err(VirtualLayoutError::AddressOutsideArchitecture {
                name: region.name(),
                address: range.start(),
            });
        }

        let last = range
            .last()
            .expect("non-empty region must have a last address");

        if !address_is_allowed(last) {
            return Err(VirtualLayoutError::AddressOutsideArchitecture {
                name: region.name(),
                address: last,
            });
        }
    }

    for left_index in 0..regions.len() {
        for right_index in left_index + 1..regions.len() {
            let left = regions[left_index];
            let right = regions[right_index];

            if left.range().overlaps(right.range()) {
                return Err(VirtualLayoutError::OverlappingRegions {
                    left: left.name(),
                    right: right.name(),
                });
            }
        }
    }

    Ok(())
}

pub fn require_address_in_region(
    name: &'static str,
    address: VirtAddr,
    region_name: &'static str,
    region: VirtRange,
) -> Result<(), VirtualLayoutError> {
    if region.contains(address) {
        Ok(())
    } else {
        Err(VirtualLayoutError::AddressOutsideRegion {
            name,
            address,
            region: region_name,
        })
    }
}
