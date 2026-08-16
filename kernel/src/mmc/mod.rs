//! 多功能控制器（MMC/SD）主机。
//!
//! - C6：从设备树收集 DesignWare MMC 主机（`snps,dw-mshc`，JH7110）并把
//!   配置存入静态区；
//! - C7：轮询 DW-MMC 主控（`dw_mmc`）；
//! - C8：SD 卡初始化 + 只读块（`sd`）。
//!
//! VisionFive 2 上 `mmc0` 是板载 eMMC、`mmc1` 是 TF 卡槽。

pub mod block;
pub mod dw_mmc;
pub mod registers;
pub mod sd;

#[cfg(debug_assertions)]
mod mock;

use alloc::vec::Vec;

use crate::{
    irq_lock::IrqSpinLock,
    lockdep::{LockClass, LockRank},
};

const MMC_DISCOVERY_LOCK: LockClass = LockClass::new("mmc.discovery", LockRank::Vfs, 5);

static DISCOVERED_HOSTS: IrqSpinLock<Vec<myos_fdt::MmcHostConfig>> =
    IrqSpinLock::new_with_class(Vec::new(), MMC_DISCOVERY_LOCK);

/// 从设备树收集 DW-MMC 主机并记录，打印发现日志。
pub fn discover_hosts(tree: &myos_fdt::DeviceTree) {
    let mut hosts = Vec::new();
    tree.for_each_mmc_host(|host| hosts.push(host))
        .unwrap_or_else(|error| {
            crate::println!("mmc: host discovery failed: {error}");
        });

    crate::println!("mmc:");
    crate::println!("  hosts discovered : {}", hosts.len());
    for host in &hosts {
        crate::println!(
            "  mmc{}             : base={:#018x} size={:#x} bus-width={} irq={} fifo={:?} max={:?} ciu={:?} removable={}",
            host.alias_index().unwrap_or(u8::MAX),
            host.base(),
            host.size(),
            host.bus_width(),
            host.irq(),
            host.fifo_depth(),
            host.max_frequency_hz(),
            host.ciu_frequency_hz(),
            !host.non_removable(),
        );
    }
    *DISCOVERED_HOSTS.lock() = hosts;
}

/// 返回已发现的主机配置快照。
pub fn discovered_hosts() -> Vec<myos_fdt::MmcHostConfig> {
    DISCOVERED_HOSTS.lock().clone()
}

/// 返回首个可移除主机（`non_removable == false`）。VisionFive 2 上即
/// `mmc1` TF 卡槽；无匹配时返回 `None`。
pub fn removable_host() -> Option<myos_fdt::MmcHostConfig> {
    DISCOVERED_HOSTS
        .lock()
        .iter()
        .find(|host| !host.non_removable())
        .copied()
}

/// 初始化可移除主机的 SD 卡并注册 `/dev/mmcblk1`（无卡/失败不 panic）。
///
/// 在 `fs::initialize()` 之前调用，使块设备在 devfs 建立时可见。仅当设备
/// 树发现可移除主机（VisionFive 2 的 `mmc1` TF 槽）时执行。
pub fn initialize_storage() {
    let Some(host) = removable_host() else {
        return;
    };
    crate::println!(
        "mmc: probing removable host mmc{} base={:#018x}",
        host.alias_index().unwrap_or(u8::MAX),
        host.base(),
    );
    // SAFETY: host.base 来自设备树校验过的 MMIO 区域，内核生命周期内有效。
    let io = unsafe { dw_mmc::MmioRegisterIo::new(host.base()) };
    let ciu = host.ciu_frequency_hz().unwrap_or(25_000_000);
    let mut controller = dw_mmc::DwMmcController::new(io, ciu);
    match controller.reset() {
        Ok(()) => {}
        Err(error) => {
            crate::println!("mmc: controller reset failed ({error:?}) — no card");
            return;
        }
    }
    controller.disable_interrupts();
    if let Err(error) = controller.set_clock(400_000) {
        crate::println!("mmc: init clock setup failed ({error:?}) — no card");
        return;
    }
    let info = match sd::initialize_card(&mut controller) {
        Ok(info) => info,
        Err(error) => {
            crate::println!("mmc: no SD card on removable host ({error:?})");
            return;
        }
    };
    match block::register_mmcblk1(controller, info) {
        Ok(()) => crate::println!("mmc: registered=/dev/mmcblk1"),
        Err(error) => crate::println!("mmc: register mmcblk1 failed ({error:?})"),
    }
}

#[cfg(debug_assertions)]
pub fn verify() {
    dw_mmc::verify();
    sd::verify();
}
