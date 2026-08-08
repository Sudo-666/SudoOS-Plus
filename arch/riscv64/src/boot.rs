use myos_boot::BootInfo;

#[derive(Clone, Copy)]
pub struct BootContext {
    raw_args: [usize; 3],
    device_tree: Option<usize>,
}

impl BootContext {
    pub(crate) const fn new(raw_args: [usize; 3]) -> Self {
        Self {
            raw_args,
            device_tree: None,
        }
    }

    pub(crate) const fn with_device_tree(mut self, address: usize) -> Self {
        self.device_tree = Some(address);
        self
    }

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

/// 由所选择的平台按各自启动约定解析原始寄存器参数。
pub fn from_raw(arg0: usize, arg1: usize, arg2: usize) -> BootContext {
    crate::platform::boot_context(arg0, arg1, arg2)
}
