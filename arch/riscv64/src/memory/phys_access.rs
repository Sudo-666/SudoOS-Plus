use core::mem::{
    align_of,
    size_of,
};

use myos_mm::{
    PhysAddr,
    VirtAddr,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhysAccessError {
    AddressOverflow,

    AddressOutOfRange {
        address: PhysAddr,
        size: usize,
    },

    Misaligned {
        address: VirtAddr,
        alignment: usize,
    },
}

/// 将普通 RAM 物理地址转换为当前可访问的虚拟地址。
///
/// 启用 Sv39 之前使用恒等地址；启用之后使用正式 direct map。
pub fn ram_virtual_address(
    physical: PhysAddr,
    size: usize,
) -> Result<VirtAddr, PhysAccessError> {
    let end = physical
        .get()
        .checked_add(size)
        .ok_or(PhysAccessError::AddressOverflow)?;

    if crate::memory::paging::translation_is_enabled() {
        if end > crate::memory::layout::DIRECT_MAP.size() {
            return Err(
                PhysAccessError::AddressOutOfRange {
                    address: physical,
                    size,
                },
            );
        }

        crate::memory::layout::phys_to_direct(physical)
            .ok_or(
                PhysAccessError::AddressOutOfRange {
                    address: physical,
                    size,
                },
            )
    } else {
        Ok(VirtAddr::new(physical.get()))
    }
}

/// 早期 MMIO 访问。
///
/// 启用分页前，物理 MMIO 可以恒等访问；启用分页后，当前只允许
/// 已显式保留恒等映射的 early UART。
pub fn mmio_virtual_address(
    physical: PhysAddr,
    size: usize,
) -> Result<VirtAddr, PhysAccessError> {
    let end = physical
        .get()
        .checked_add(size)
        .ok_or(PhysAccessError::AddressOverflow)?;

    if !crate::memory::paging::translation_is_enabled() {
        return Ok(VirtAddr::new(physical.get()));
    }

    let uart_start =
        crate::early_console::MMIO_BASE;

    let uart_end = uart_start
        .checked_add(crate::early_console::MMIO_SIZE)
        .ok_or(PhysAccessError::AddressOverflow)?;

    if physical.get() < uart_start || end > uart_end {
        return Err(
            PhysAccessError::AddressOutOfRange {
                address: physical,
                size,
            },
        );
    }

    Ok(VirtAddr::new(physical.get()))
}

pub fn ram_ptr<T>(
    physical: PhysAddr,
) -> Result<*const T, PhysAccessError> {
    let virtual_address =
        ram_virtual_address(physical, size_of::<T>())?;

    checked_const_pointer::<T>(virtual_address)
}

pub fn ram_mut_ptr<T>(
    physical: PhysAddr,
) -> Result<*mut T, PhysAccessError> {
    let virtual_address =
        ram_virtual_address(physical, size_of::<T>())?;

    checked_mut_pointer::<T>(virtual_address)
}

pub fn mmio_ptr<T>(
    physical: PhysAddr,
) -> Result<*const T, PhysAccessError> {
    let virtual_address =
        mmio_virtual_address(physical, size_of::<T>())?;

    checked_const_pointer::<T>(virtual_address)
}

pub fn mmio_mut_ptr<T>(
    physical: PhysAddr,
) -> Result<*mut T, PhysAccessError> {
    let virtual_address =
        mmio_virtual_address(physical, size_of::<T>())?;

    checked_mut_pointer::<T>(virtual_address)
}

fn checked_const_pointer<T>(
    virtual_address: VirtAddr,
) -> Result<*const T, PhysAccessError> {
    check_alignment::<T>(virtual_address)?;

    Ok(virtual_address.get() as *const T)
}

fn checked_mut_pointer<T>(
    virtual_address: VirtAddr,
) -> Result<*mut T, PhysAccessError> {
    check_alignment::<T>(virtual_address)?;

    Ok(virtual_address.get() as *mut T)
}

fn check_alignment<T>(
    virtual_address: VirtAddr,
) -> Result<(), PhysAccessError> {
    let alignment = align_of::<T>();

    if virtual_address.get() % alignment != 0 {
        return Err(PhysAccessError::Misaligned {
            address: virtual_address,
            alignment,
        });
    }

    Ok(())
}