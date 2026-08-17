use crate::boot::BootContext;

/// FDT 大端 magic: 0xd00dfeed,在内存中按小端读为 0xedfe_0dd0。
const FDT_MAGIC_LE: u32 = 0xedfe_0dd0;

/// FDT header 固定 40 字节 (magic..size_dt_struct)。
const FDT_HEADER_SIZE: usize = 40;

/// FDT 允许的最大 totalsize。
const FDT_MAX_SIZE: usize = 16 * 1024 * 1024;

/// JH7110 DDR 物理基址,以及启动临时 direct map 实际覆盖的范围。
///
/// entry.S 的 TEMP_DIRECT_MAP_GIB_PAGES = 8,从 0x4000_0000 起覆盖
/// 8 GiB,结束于 0x2_4000_0000。落在该范围之外的 FDT 地址无法通过
/// direct map 别名读取。
const DDR_PHYS_START: usize = 0x4000_0000;
const DDR_PHYS_END: usize = 0x2_4000_0000;

/// VisionFive 2 (JH7110) 启动约定：
///
/// U-Boot 以 OpenSBI 为 M-mode 固件,S-mode 下按 RISC-V Linux 启动协议
/// 用 `bootm`/`booti` 跳转内核:
///
/// - a0: hart ID (gd->arch.boot_hart)
/// - a1: FDT 物理地址 (images->ft_addr)
/// - a2: 未定义(忽略)
///
/// 校验 a1 是否真的指向 FDT,避免 `go` 等未传 FDT 的路径把
/// 垃圾寄存器值当作设备树地址。
pub(crate) fn boot_context(hart_id: usize, device_tree: usize, reserved: usize) -> BootContext {
    let mut context = BootContext::new([hart_id, device_tree, reserved]);

    if let Some(address) = valid_fdt_address(device_tree) {
        context = context.with_device_tree(address);
    }

    context
}

fn valid_fdt_address(address: usize) -> Option<usize> {
    // 非零。
    if address == 0 {
        return None;
    }

    // FDT 按规范 8 字节对齐。
    if address % 8 != 0 {
        return None;
    }

    // 位于 VF2 启动临时 direct map 实际覆盖的 DDR 范围内。
    if address < DDR_PHYS_START || address >= DDR_PHYS_END {
        return None;
    }

    // FDT header 40 字节必须整体可读。
    if address.checked_add(FDT_HEADER_SIZE)? > DDR_PHYS_END {
        return None;
    }

    let physical = myos_mm::PhysAddr::new(address);

    // 此时 entry.S 已经开启 Sv39,物理地址必须经 direct map 别名读取。
    let virtual_address = crate::memory::layout::phys_to_direct(physical)?;

    // SAFETY: phys_to_direct 只对 128 GiB direct map 范围内的地址返回
    // 有效别名;该别名已由启动临时 direct map 建立。这里仅做 volatile 读,
    // 且已在上方保证 address + 40 不越过映射的 DDR 边界。
    let magic = unsafe { core::ptr::read_volatile(virtual_address.get() as *const u32) };

    if magic != FDT_MAGIC_LE {
        return None;
    }

    // totalsize (FDT 大端 u32,header 偏移 4) 必须在合理范围内。
    let totalsize_bytes =
        unsafe { core::ptr::read_volatile((virtual_address.get() + 4) as *const u32) };
    let totalsize = u32::from_be(totalsize_bytes) as usize;

    if !(FDT_HEADER_SIZE..=FDT_MAX_SIZE).contains(&totalsize) {
        return None;
    }

    // address + totalsize 不溢出,且整个 FDT 仍在已映射 DDR 内。
    let end = address.checked_add(totalsize)?;
    if end > DDR_PHYS_END {
        return None;
    }

    Some(address)
}
