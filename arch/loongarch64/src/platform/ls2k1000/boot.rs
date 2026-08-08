// arch/loongarch64/src/platform/ls2k1000/boot.rs

use crate::boot::BootContext;

/// 在 entry.S 中预留的全局变量，用于接收 U-Boot 的参数
#[unsafe(no_mangle)]
pub static mut BOOT_ARGS: [u64; 3] = [0; 3];

/// U-Boot 引导 LoongArch 的标准约定：
///   $a0 = 0（保留）
///   $a1 = FDT 物理地址
///   $a2 = 0（保留）
///
/// 这里将原始的启动参数解析为统一的 `BootContext`。
///
/// 注意：`kernel_main` 把 `device_tree()` / `command_line()` 当作**物理地址**，
/// 通过 `phys_access::ram_ptr` 的 `phys_to_cached` 转换访问。
/// 因此这里必须原样透传物理地址，不能把 `CACHED_BASE` OR 进去——
/// 否则 48 位掩码检查会拒绝该地址并在启动早期 panic。
pub(crate) fn boot_context(arg0: usize, arg1: usize, arg2: usize) -> BootContext {
    let mut context = BootContext::new([arg0, arg1, arg2]);

    // $a1 存放了 U-Boot 传入的 FDT 物理地址（DDR 内，0x9000_0000+）
    if arg1 != 0 {
        context = context.with_device_tree(arg1);
    }

    // $a0 可能包含 command line 的物理地址（部分 U-Boot 版本）
    if arg0 != 0 {
        context = context.with_command_line(arg0);
    }

    context
}

/// 副核早期入口存根。
///
/// 在 U-Boot 阶段，所有副核从 ROM wait loop 跳转到
/// `_start_secondary`（secondary.S），然后调用本函数。
///
/// 当前实现仅让副核进入低功耗等待状态。
/// 完整的 SMP 启动将在后续通过 BootSlot + IOCSR
/// 机制实现，届时本存根将被替换。
#[unsafe(no_mangle)]
pub extern "C" fn rust_main_secondary() -> ! {
    use core::arch::asm;

    // 读取硬件 CPU ID
    let cpu_id: usize;
    unsafe {
        asm!(
            "csrrd {tmp}, 0x20",
            "andi {tmp}, {tmp}, 0x3ff",
            tmp = out(reg) cpu_id,
            options(nomem, nostack),
        );
    }

    // 设置逻辑 CPU ID（r21 + KSave3）
    crate::smp::set_current_cpu_id(cpu_id);

    // TODO: 后续需要实现完整的 BootSlot + IPI 唤醒机制。
    // 当前副核暂不参与内核调度。
    loop {
        crate::cpu::wait_for_interrupt();
    }
}

/// 从 BOOT_ARGS 获取设备树地址（兼容旧接口，返回原始物理地址）
pub fn get_device_tree_addr() -> Option<usize> {
    unsafe {
        let a1 = BOOT_ARGS[1] as usize;
        if a1 == 0 {
            None
        } else {
            Some(a1)
        }
    }
}