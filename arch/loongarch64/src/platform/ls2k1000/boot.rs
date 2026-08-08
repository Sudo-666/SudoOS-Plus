// arch/loongarch64/src/platform/ls2k1000/boot.rs

use crate::boot::BootContext;

/// 在 entry.S 中预留的全局变量，用于接收 U-Boot 的参数
#[unsafe(no_mangle)]
pub static mut BOOT_ARGS: [u64; 4] = [0; 4];

/// LoongArch 引导约定（两种，FDT 位置不同）：
///   - 主线上游 U-Boot bootm:    $a0=-2, $a1=FDT, $a2=0,   $a3=0
///   - 本厂商 BSP bootm:         $a0=argc, $a1=argv, $a2=bootparam, $a3=FDT
///     （`CONFIG_LOONGSON_BOOT_FIXUP` 默认启用，FDT 来自 env `fdt_addr`，
///      是 cached 窗口 VA，如 0x900000000a000000）
///   - `go` 命令:                寄存器残留值，不可信
///
/// 因此 FDT 通过 magic 识别：优先 $a1，其次 $a3；带 cached 窗口前缀的地址
/// 先剥成物理地址。
///
/// 注意：`kernel_main` 把 `device_tree()` / `command_line()` 当作**物理地址**，
/// 通过 `phys_access::ram_ptr` 的 `phys_to_cached` 转换访问。
/// 因此这里传给 `with_device_tree` 的必须是剥掉 CACHED 前缀的物理地址——
/// 否则 48 位掩码检查会拒绝该地址并在启动早期 panic。
pub(crate) fn boot_context(arg0: usize, arg1: usize, arg2: usize, arg3: usize) -> BootContext {
    let mut context = BootContext::new([arg0, arg1, arg2]);

    // FDT 地址识别：magic 校验优先（$a1 上游约定，$a3 本 BSP fixup 约定）。
    if has_fdt_magic(arg1) {
        context = context.with_device_tree(physical_address(arg1));
    } else if has_fdt_magic(arg3) {
        context = context.with_device_tree(physical_address(arg3));
    }

    // $a0 可能包含 command line 的物理地址（部分 U-Boot 版本）
    if arg0 != 0 {
        context = context.with_command_line(arg0);
    }

    context
}

/// LoongArch cached DMW 窗口前缀与 48 位物理地址掩码。
const CACHED_BASE: usize = 0x9000_0000_0000_0000;
const PHYS_MASK: usize = 0x0FFF_FFFF_FFFF_FFFF;

/// FDT header 的 magic：0xd00dfeed（大端存储，内存字节序 d0 0d fe ed）。
const FDT_MAGIC: u32 = 0xd00d_feed;

/// 把可能带 cached 窗口前缀的地址剥成物理地址。
fn physical_address(addr: usize) -> usize {
    addr & PHYS_MASK
}

/// 地址是否落在板子的两块 DDR 内（避免对垃圾指针/MMIO 做危险读）。
fn plausible_ram_addr(addr: usize) -> bool {
    let phys = physical_address(addr);
    phys < 0x1000_0000 || (0x9000_0000..0x1_0000_0000).contains(&phys)
}

/// 通过 cached DMW 窗口读地址处的 u32，判断是否为 FDT magic。
fn has_fdt_magic(addr: usize) -> bool {
    if addr == 0 || !plausible_ram_addr(addr) {
        return false;
    }
    let cached = CACHED_BASE | physical_address(addr);
    // SAFETY: 早期启动跑在 DMW cached 窗口上，物理地址可直接经 cached 别名读。
    let magic = unsafe { core::ptr::read_volatile(cached as *const u32) };
    // FDT magic 大端存储，小端机器读出的 u32 需 swap 后比对。
    magic.swap_bytes() == FDT_MAGIC
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
        let a3 = BOOT_ARGS[3] as usize;
        if a1 != 0 {
            Some(a1)
        } else if a3 != 0 {
            Some(a3)
        } else {
            None
        }
    }
}