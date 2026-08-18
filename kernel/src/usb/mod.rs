//! 纯 Rust USB2 Host 驱动（LS2K1000 EHCI + MSC）。
//!
//! 取代 C/CherryUSB 路径（`cusb` 模块 + `vendor/cherryusb` +
//! `kernel/csrc/usb`）。第一版：只读、单 High-Speed 设备、根集线器直连、
//! 轮询。实现依据 EHCI 1.0/2.0、USB 2.0、MSC Bulk-Only、SCSI 标准，不
//! 复制 Linux / CherryUSB / ArceOS 代码。
//!
//! 本模块只在 `platform-ls2k1000` 特性下编译。RUSB-1（本提交）交付：
//! DMA uncached 区域 + 模块骨架 + A/B 分发。C 驱动仍是默认
//! （`sudoos.usb.driver=c`），Rust 路径经 `sudoos.usb.driver=rust` 逐步
//! 接管，最终（RUSB-7）翻转为默认并删除 C 路径。

mod dma;
mod error;

use core::sync::atomic::{AtomicU8, Ordering};

/// USB 驱动实现选择（A/B 分发）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsbDriverMode {
    /// 现有 C/CherryUSB 路径（默认，直至 RUSB-7）。
    C,
    /// 纯 Rust USB2 Host 驱动（本模块）。
    Rust,
}

/// 全局 driver 模式。`0`=C，`1`=Rust。
static USB_DRIVER_MODE: AtomicU8 = AtomicU8::new(0);

impl UsbDriverMode {
    const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Rust,
            _ => Self::C,
        }
    }

    const fn to_u8(self) -> u8 {
        match self {
            Self::C => 0,
            Self::Rust => 1,
        }
    }
}

/// 从 bootargs 解析 `sudoos.usb.driver=c|rust`。缺失/未知 → `C`（保守，
/// 保持现有行为直到 RUSB-7 翻转默认）。
pub fn parse_driver_mode(args: Option<&str>) -> UsbDriverMode {
    let Some(args) = args else {
        return UsbDriverMode::C;
    };
    for word in args.split_whitespace() {
        if let Some(value) = word.strip_prefix("sudoos.usb.driver=") {
            if value.eq_ignore_ascii_case("rust") || value.eq_ignore_ascii_case("rs") {
                return UsbDriverMode::Rust;
            }
            return UsbDriverMode::C;
        }
    }
    UsbDriverMode::C
}

/// 设置全局 driver 模式。main 在 FDT bootargs 解析后调用一次。
pub fn set_driver_mode(mode: UsbDriverMode) {
    USB_DRIVER_MODE.store(mode.to_u8(), Ordering::Release);
}

fn driver_mode() -> UsbDriverMode {
    UsbDriverMode::from_u8(USB_DRIVER_MODE.load(Ordering::Acquire))
}

/// 早期轮询探针：boot 路径、scheduler 就绪前调用。
///
/// C 路径委托 `cusb::early_probe`；Rust 路径初始化 DMA 池并跑 RUSB-DMA
/// 门禁（非破坏：只切 uncached 区域 + 回环自检，不触碰 EHCI 寄存器，也不
/// spawn 线程）。失败只打日志，绝不 panic。
pub fn early_probe() {
    match driver_mode() {
        UsbDriverMode::C => crate::cusb::early_probe(),
        UsbDriverMode::Rust => rust_early_probe(),
    }
}

fn rust_early_probe() {
    crate::println!("USB-RUST: early_probe driver=rust");
    match dma::dma_gate() {
        Ok(()) => {}
        Err(error) => crate::println!("USB-RUST: DMA gate FAIL: {error:?}"),
    }
}

/// 晚期线程化初始化：须在 `task::start_boot_scheduler()` 之后调用。
///
/// C 路径委托 `cusb::late_start`。Rust 路径到 RUSB-6 才 spawn
/// init/worker 线程（当前无事可做，只打日志）。
pub fn late_start() {
    match driver_mode() {
        UsbDriverMode::C => crate::cusb::late_start(),
        UsbDriverMode::Rust => {
            // RUSB-6 前 Rust 路径不 spawn 线程；wait_usb_storage_ready
            // 返回 false，竞赛存储路径表现为无设备。
            crate::println!("USB-RUST: late_start driver=rust (pending RUSB-6)");
        }
    }
}

/// 等待 USB 大容量存储阶段完成，返回是否检测到并注册了 MSC 设备。
///
/// 由 boot idle task 上的 `kernel_main` 调用（LS2K1000 竞赛存储路径），
/// 故必须用 `task::boot_idle_wait_until` 轮询而非 WaitQueue 阻塞
/// （`prepare_block` 在 idle 任务上断言）。
pub fn wait_usb_storage_ready() -> bool {
    match driver_mode() {
        UsbDriverMode::C => crate::cusb::wait_usb_storage_ready(),
        UsbDriverMode::Rust => {
            // RUSB-6 前 Rust 路径尚未实现设备枚举/注册。
            false
        }
    }
}
