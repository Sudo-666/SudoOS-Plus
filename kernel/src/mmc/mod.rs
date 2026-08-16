//! 多功能控制器（MMC/SD）主机发现。
//!
//! C6：从设备树收集 DesignWare MMC 主机（`snps,dw-mshc`，JH7110）并把
//! 配置存入静态区，供后续驱动（C7 DW-MMC 主控、C8 SD 协议）消费。
//! VisionFive 2 上 `mmc0` 是板载 eMMC、`mmc1` 是 TF 卡槽。

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
