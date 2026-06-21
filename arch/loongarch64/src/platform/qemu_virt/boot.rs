use core::{
    mem::{align_of, size_of},
    ptr::read_volatile,
};

use myos_mm::PhysAddr;

use crate::boot::BootContext;

/// EFI system table 的标准签名："IBI SYST" 的小端表示。
const EFI_SYSTEM_TABLE_SIGNATURE: u64 = 0x5453_5953_2049_4249;

/// 防止损坏的启动表导致无限或超大范围遍历。
const MAX_CONFIGURATION_TABLES: usize = 64;

/// Flattened Device Tree header 的大端 magic。
const FDT_MAGIC: [u8; 4] = [0xd0, 0x0d, 0xfe, 0xed];

/// EFI Device Tree configuration table GUID。
///
/// 原始 GUID：
///
/// b1b621d5-f19c-41a5-830b-d9152c69aae0
///
/// EFI GUID 的前三部分在内存中使用小端排列。
const DEVICE_TREE_GUID: EfiGuid = EfiGuid {
    bytes: [
        0xd5, 0x21, 0xb6, 0xb1, 0x9c, 0xf1, 0xa5, 0x41, 0x83, 0x0b, 0xd9, 0x15, 0x2c, 0x69, 0xaa,
        0xe0,
    ],
};

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(C, align(8))]
struct EfiGuid {
    bytes: [u8; 16],
}

#[derive(Clone, Copy)]
#[repr(C)]
struct EfiTableHeader {
    signature: u64,
    revision: u32,
    header_size: u32,
    crc32: u32,
    reserved: u32,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct EfiSystemTable {
    header: EfiTableHeader,

    firmware_vendor: u64,
    firmware_revision: u32,

    // repr(C) 在这里自动插入 4 字节 padding。
    console_in_handle: u64,
    console_in: u64,

    console_out_handle: u64,
    console_out: u64,

    standard_error_handle: u64,
    standard_error: u64,

    runtime_services: u64,
    boot_services: u64,

    number_of_table_entries: u64,
    configuration_table: u64,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct EfiConfigurationTable {
    guid: EfiGuid,
    table: u64,
}

/*
 * 与 QEMU 中对应 C 结构保持一致。
 *
 * 编译期检查可以防止未来修改字段后悄悄破坏启动协议。
 */
const _: () = {
    assert!(size_of::<EfiGuid>() == 16);
    assert!(align_of::<EfiGuid>() == 8);

    assert!(size_of::<EfiTableHeader>() == 24);
    assert!(size_of::<EfiConfigurationTable>() == 24);
    assert!(size_of::<EfiSystemTable>() == 120);
};

pub(crate) fn boot_context(arg0: usize, arg1: usize, arg2: usize) -> BootContext {
    /*
     * QEMU LoongArch direct boot：
     *
     * a0 = boot parameter count/revision，目前为 1
     * a1 = command-line buffer 的物理地址
     * a2 = EFI-style system table 的物理地址
     *
     * command line 可以位于物理地址零，因此不能使用
     * “非零才有效”的判断。
     */
    let mut context = BootContext::new([arg0, arg1, arg2]).with_command_line(arg1);

    if arg2 == 0 {
        return context;
    }

    context = context.with_system_table(arg2);

    if let Some(device_tree) = find_device_tree(arg2) {
        context = context.with_device_tree(device_tree);
    }

    context
}

/// 从 QEMU 创建的 EFI-style system table 中查找 FDT。
///
/// 这不是完整 EFI 实现，只解析 direct boot 所需的最小字段。
fn find_device_tree(system_table_address: usize) -> Option<usize> {
    let system_table = read_value::<EfiSystemTable>(system_table_address)?;

    if system_table.header.signature != EFI_SYSTEM_TABLE_SIGNATURE {
        return None;
    }

    if system_table.number_of_table_entries > MAX_CONFIGURATION_TABLES as u64 {
        return None;
    }

    let table_count = system_table.number_of_table_entries as usize;

    let configuration_table_address = usize::try_from(system_table.configuration_table).ok()?;

    if table_count != 0 && configuration_table_address == 0 {
        return None;
    }

    for index in 0..table_count {
        let offset = index.checked_mul(size_of::<EfiConfigurationTable>())?;

        let entry_address = configuration_table_address.checked_add(offset)?;

        let entry = read_value::<EfiConfigurationTable>(entry_address)?;

        if entry.guid != DEVICE_TREE_GUID {
            continue;
        }

        let device_tree_address = usize::try_from(entry.table).ok()?;

        if has_fdt_magic(device_tree_address) {
            return Some(device_tree_address);
        }
    }

    None
}

/// 从启动阶段物理地址读取一个 C-layout 值。
///
/// 当前尚未开启分页，因此 QEMU 传入的物理地址可以直接作为
/// CPU 地址使用。
fn read_value<T: Copy>(address: usize) -> Option<T> {
    let pointer = crate::memory::phys_access::ram_ptr::<T>(PhysAddr::new(address)).ok()?;

    // SAFETY:
    // 地址来自 QEMU direct-boot 启动表，并已转换为 cached DMW。
    Some(unsafe { read_volatile(pointer) })
}

fn has_fdt_magic(address: usize) -> bool {
    if address == 0 {
        return false;
    }

    for (offset, expected) in FDT_MAGIC.iter().copied().enumerate() {
        let Some(byte_address) = address.checked_add(offset) else {
            return false;
        };

        let pointer = crate::memory::phys_access::ram_ptr::<u8>(PhysAddr::new(byte_address)).ok();

        let Some(pointer) = pointer else {
            return false;
        };

        // SAFETY:
        // 地址来自 QEMU system table 中的 DEVICE_TREE_GUID 项。
        let actual = unsafe { read_volatile(pointer) };

        if actual != expected {
            return false;
        }
    }

    true
}
