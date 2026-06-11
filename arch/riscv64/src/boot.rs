use myos_boot::BootInfo;

#[derive(Clone, Copy)]
pub struct BootContext {
    raw_args: [usize; 3],
    device_tree: Option<usize>,
}

impl BootContext {
    pub const fn raw_args(&self) -> &[usize; 3] {
        &self.raw_args
    }

    pub const fn boot_cpu_id(&self) -> usize {
        self.raw_args[0]
    }

    pub const fn device_tree(&self) -> Option<usize> {
        self.device_tree
    }

    /// 转换成与架构无关的公共启动信息。
    pub const fn into_boot_info(self) -> BootInfo {
        let mut info = BootInfo::new(self.raw_args).with_boot_cpu_id(self.boot_cpu_id());

        if let Some(address) = self.device_tree {
            info = info.with_device_tree(address);
        }

        info
    }
}

/// OpenSBI 启动约定：
///
/// - a0：hart ID
/// - a1：FDT 地址
/// - a2：当前阶段保留
pub const fn from_raw(hart_id: usize, device_tree: usize, reserved: usize) -> BootContext {
    BootContext {
        raw_args: [hart_id, device_tree, reserved],
        device_tree: non_null_address(device_tree),
    }
}

const fn non_null_address(address: usize) -> Option<usize> {
    if address == 0 { None } else { Some(address) }
}
