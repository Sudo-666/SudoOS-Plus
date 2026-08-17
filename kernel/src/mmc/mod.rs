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

/// 选择 TF 卡主机。显式优先别名 `mmc1`（VisionFive 2 的 SD/TF 槽，与
/// `sudoos.contest.dev=mmcblk1` / `register_mmcblk1` 的契约一致），而不是
/// "遍历序第一个可移除主机"——后者在 DT 未标 `non-removable` 或别名遍历
/// 顺序变化时会误选 `mmc0`（板载 eMMC）。无 `mmc1` 别名时回退到第一个
/// 可移除主机。纯函数便于单测。
pub fn select_tf_host(hosts: &[myos_fdt::MmcHostConfig]) -> Option<myos_fdt::MmcHostConfig> {
    hosts
        .iter()
        .find(|host| host.alias_index() == Some(1))
        .or_else(|| hosts.iter().find(|host| !host.non_removable()))
        .copied()
}

/// 返回 TF 卡主机（VisionFive 2 的 `mmc1` 槽）；无匹配时返回 `None`。
pub fn removable_host() -> Option<myos_fdt::MmcHostConfig> {
    select_tf_host(&DISCOVERED_HOSTS.lock())
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
        "VF2-TF00 host=mmc{} base={:#018x} bus-width={}",
        host.alias_index().unwrap_or(u8::MAX),
        host.base(),
        host.bus_width(),
    );
    // K3.2：Sv39 最终页表启用后 0x16010000 不再恒等映射，MMIO 必须经
    // `vm::ioremap` 建立内核映射（virtio 同款路径）。
    let mapping = match crate::vm::ioremap(myos_mm::PhysAddr::new(host.base()), host.size()) {
        Ok(mapping) => mapping,
        Err(error) => {
            crate::println!("VF2-TF00 ioremap-failed={error:?} no-card");
            return;
        }
    };
    let io_base = mapping.virtual_address().get();
    // 映射生命周期须与内核一致：显式泄漏 guard，不回收。
    core::mem::forget(mapping);
    // SAFETY: io_base 来自 vm::ioremap，内核生命周期内保持映射。
    let io = unsafe { dw_mmc::MmioRegisterIo::new(io_base) };
    let ciu = host.ciu_frequency_hz().unwrap_or(25_000_000);
    let fifo_depth = host.fifo_depth().unwrap_or(32);
    let mut controller = dw_mmc::DwMmcController::new(io, ciu, fifo_depth);
    match controller.power_on() {
        Ok(()) => {}
        Err(error) => {
            crate::println!("VF2-TF01 power-failed={error:?} no-card");
            return;
        }
    }
    match controller.reset() {
        Ok(()) => {}
        Err(error) => {
            crate::println!("VF2-TF01 reset-failed={error:?} no-card");
            return;
        }
    }
    controller.disable_interrupts();
    if let Err(error) = controller.set_clock(400_000) {
        crate::println!("VF2-TF01 init-clock-failed={error:?} no-card");
        return;
    }
    let info = match sd::initialize_card(&mut controller) {
        Ok(info) => info,
        Err(error) => {
            crate::println!("VF2-TF01 init-failed={error:?} no-card");
            return;
        }
    };
    let block_count = info.block_count;
    crate::println!(
        "VF2-TF01 card rca={:#x} sdhc={} blocks={} bus-width={}",
        info.rca,
        info.is_sdhc,
        info.block_count,
        info.bus_width,
    );
    match block::register_mmcblk1(controller, info) {
        Ok(()) => {
            crate::println!("VF2-TF02 registered=/dev/mmcblk1");
            crate::println!(
                "VF2-TF03 block-size={} blocks={} read-only",
                block::sd_block_size(),
                block_count,
            );
        }
        Err(error) => crate::println!("VF2-TF02 register-failed={error:?}"),
    }
}

#[cfg(debug_assertions)]
pub fn verify() {
    dw_mmc::verify();
    sd::verify();
    block::verify();

    // TF 主机选择：别名 mmc1 必须优先于"第一个可移除主机"。
    let emmc = myos_fdt::MmcHostConfig::new(
        Some(0),
        0x1601_0000,
        0x1_0000,
        74,
        8,
        Some(32),
        None,
        None,
        false, // 即使 DT 漏标 non-removable，也必须选 mmc1 而非 mmc0
    );
    let tf = myos_fdt::MmcHostConfig::new(
        Some(1),
        0x1602_0000,
        0x1_0000,
        75,
        4,
        Some(32),
        None,
        None,
        false,
    );
    assert_eq!(
        select_tf_host(&[emmc, tf]),
        Some(tf),
        "alias mmc1 must win over first-removable (mmc0)"
    );
    // 无别名：回退第一个可移除主机。
    let anonymous = myos_fdt::MmcHostConfig::new(
        None,
        0x1602_0000,
        0x1_0000,
        75,
        4,
        Some(32),
        None,
        None,
        false,
    );
    assert_eq!(select_tf_host(&[anonymous]), Some(anonymous));
    // 只有可移除的 mmc0：无 mmc1 别名时回退选中它（尽力而为）。
    assert_eq!(select_tf_host(&[emmc]), Some(emmc));
    // 全部不可移除且无别名 → None。
    let fixed = myos_fdt::MmcHostConfig::new(
        None,
        0x1602_0000,
        0x1_0000,
        75,
        4,
        Some(32),
        None,
        None,
        true,
    );
    assert_eq!(select_tf_host(&[fixed]), None);
    // 不可移除的 mmc0 + 可移除的 mmc1 → 别名 mmc1 胜出。
    let emmc_fixed = myos_fdt::MmcHostConfig::new(
        Some(0),
        0x1601_0000,
        0x1_0000,
        74,
        8,
        Some(32),
        None,
        None,
        true,
    );
    assert_eq!(select_tf_host(&[emmc_fixed, tf]), Some(tf));

    crate::println!("C6.1 host selection    : verified");
}
