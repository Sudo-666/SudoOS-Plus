//! LS2K1000 平台事实：EHCI MMIO 基址与 DMW 窗口常量。
//!
//! 控制器是 2K1000 通用 Intel EHCI，MMIO 物理基址 `0x4006_0000`；寄存器经
//! uncached 直接映射窗口 `0x8000_0000_4006_0000` 访问（C 胶水的
//! `CONFIG_USB_EHCI_HCCR_BASE`）。所有 DMA 描述符/缓冲同样只经 uncached
//! `0x8000...` 窗口（见 `dma`），区域内从不建立 cached 别名——从类型上
//! 杜绝 M2.11/M2.15/M2.16 的双窗口缓存一致性问题。

/// uncached 直接映射窗口（`0x8000_0000_0000_0000`）。
pub const UNCACHED_WINDOW: usize = 0x8000_0000_0000_0000;
/// 低 48 位物理地址掩码：cached/uncached 直接映射窗口都是 `BASE | phys`。
pub const PHYS_MASK: usize = 0x0fff_ffff_ffff_ffff;

/// EHCI 控制器 MMIO 物理基址。
pub const EHCI_MMIO_PHYS: usize = 0x4006_0000;
/// EHCI 寄存器 uncached 虚拟基址。
pub const EHCI_MMIO_UNCACHED: usize = UNCACHED_WINDOW | EHCI_MMIO_PHYS;

/// 端口数上限（HCSPARAMS 报告；本板 3 端口）。
pub const MAX_PORTS: usize = 3;

/// uncached 虚拟地址 → 低 32 位物理地址（EHCI 32 位 DMA）。
pub const fn uncached_to_phys(va: usize) -> u32 {
    (va & PHYS_MASK) as u32
}

/// 忙碌等待 `ms` 毫秒。
///
/// EHCI 端口复位/传输本就该独占控制器，boot 上下文与 worker 线程都可用
/// 忙碌等待（同 `cusb::sudoos_usb_msleep`，boot 期无调度器可让）。
pub fn busy_delay_ms(ms: u32) {
    let start = crate::time::now();
    let wait = core::time::Duration::from_millis(ms as u64);
    while crate::time::now().duration_since(start) < wait {
        core::hint::spin_loop();
    }
}
