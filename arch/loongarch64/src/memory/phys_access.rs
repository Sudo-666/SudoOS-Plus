use core::mem::{align_of, size_of};

use myos_mm::{PhysAddr, VirtAddr};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhysAccessError {
    AddressOverflow,

    AddressOutOfRange { address: PhysAddr, size: usize },

    Misaligned { address: VirtAddr, alignment: usize },
}

pub fn ram_virtual_address(physical: PhysAddr, size: usize) -> Result<VirtAddr, PhysAccessError> {
    validate_dmw_range(physical, size)?;

    crate::memory::layout::phys_to_cached(physical).ok_or(PhysAccessError::AddressOutOfRange {
        address: physical,
        size,
    })
}

pub fn mmio_virtual_address(physical: PhysAddr, size: usize) -> Result<VirtAddr, PhysAccessError> {
    validate_dmw_range(physical, size)?;

    crate::memory::layout::phys_to_uncached(physical).ok_or(PhysAccessError::AddressOutOfRange {
        address: physical,
        size,
    })
}

pub fn ram_ptr<T>(physical: PhysAddr) -> Result<*const T, PhysAccessError> {
    let virtual_address = ram_virtual_address(physical, size_of::<T>())?;

    checked_const_pointer::<T>(virtual_address)
}

pub fn ram_mut_ptr<T>(physical: PhysAddr) -> Result<*mut T, PhysAccessError> {
    let virtual_address = ram_virtual_address(physical, size_of::<T>())?;

    checked_mut_pointer::<T>(virtual_address)
}

pub fn mmio_ptr<T>(physical: PhysAddr) -> Result<*const T, PhysAccessError> {
    let virtual_address = mmio_virtual_address(physical, size_of::<T>())?;

    checked_const_pointer::<T>(virtual_address)
}

pub fn mmio_mut_ptr<T>(physical: PhysAddr) -> Result<*mut T, PhysAccessError> {
    let virtual_address = mmio_virtual_address(physical, size_of::<T>())?;

    checked_mut_pointer::<T>(virtual_address)
}

fn validate_dmw_range(physical: PhysAddr, size: usize) -> Result<(), PhysAccessError> {
    let end = physical
        .get()
        .checked_add(size)
        .ok_or(PhysAccessError::AddressOverflow)?;

    let limit = crate::memory::layout::DMW_PHYS_MASK
        .checked_add(1)
        .ok_or(PhysAccessError::AddressOverflow)?;

    if end > limit {
        return Err(PhysAccessError::AddressOutOfRange {
            address: physical,
            size,
        });
    }

    Ok(())
}

fn checked_const_pointer<T>(virtual_address: VirtAddr) -> Result<*const T, PhysAccessError> {
    check_alignment::<T>(virtual_address)?;

    Ok(virtual_address.get() as *const T)
}

fn checked_mut_pointer<T>(virtual_address: VirtAddr) -> Result<*mut T, PhysAccessError> {
    check_alignment::<T>(virtual_address)?;

    Ok(virtual_address.get() as *mut T)
}

fn check_alignment<T>(virtual_address: VirtAddr) -> Result<(), PhysAccessError> {
    let alignment = align_of::<T>();

    if !virtual_address.get().is_multiple_of(alignment) {
        return Err(PhysAccessError::Misaligned {
            address: virtual_address,
            alignment,
        });
    }

    Ok(())
}
