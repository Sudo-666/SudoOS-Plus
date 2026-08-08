use crate::boot::BootContext;

/// FDT 大端 magic: 0xd00dfeed,在内存中按小端读为 0xedfe_0dd0。
const FDT_MAGIC_LE: u32 = 0xedfe_0dd0;

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
    if address == 0 {
        return None;
    }

    let physical = myos_mm::PhysAddr::new(address);

    // 此时 entry.S 已经开启 Sv39,物理地址必须经 direct map 别名读取。
    let virtual_address = crate::memory::layout::phys_to_direct(physical)?;

    // SAFETY: phys_to_direct 只对 128 GiB direct map 范围内的地址返回
    // 有效别名;该别名已由启动临时 direct map 建立。这里仅做 volatile 读。
    let magic = unsafe { core::ptr::read_volatile(virtual_address.get() as *const u32) };

    (magic == FDT_MAGIC_LE).then_some(address)
}
