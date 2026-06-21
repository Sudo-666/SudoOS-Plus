use crate::BootAddress;

/// 架构启动代码转换出来的公共启动信息。
///
/// 这里仅保存固件提供的启动元数据，不解析 FDT、
/// EFI system table 或命令行内容。
#[derive(Clone, Copy, Debug)]
pub struct BootInfo {
    raw_args: [usize; 3],

    boot_cpu_id: Option<usize>,

    device_tree: Option<BootAddress>,
    command_line: Option<BootAddress>,
    system_table: Option<BootAddress>,
}

impl BootInfo {
    pub const fn new(raw_args: [usize; 3]) -> Self {
        Self {
            raw_args,

            boot_cpu_id: None,

            device_tree: None,
            command_line: None,
            system_table: None,
        }
    }

    pub const fn with_boot_cpu_id(mut self, cpu_id: usize) -> Self {
        self.boot_cpu_id = Some(cpu_id);
        self
    }

    pub const fn with_device_tree(mut self, address: usize) -> Self {
        self.device_tree = Some(BootAddress::new(address));
        self
    }

    pub const fn with_command_line(mut self, address: usize) -> Self {
        self.command_line = Some(BootAddress::new(address));
        self
    }

    pub const fn with_system_table(mut self, address: usize) -> Self {
        self.system_table = Some(BootAddress::new(address));
        self
    }

    pub const fn raw_args(&self) -> &[usize; 3] {
        &self.raw_args
    }

    pub const fn boot_cpu_id(&self) -> Option<usize> {
        self.boot_cpu_id
    }

    pub const fn device_tree(&self) -> Option<BootAddress> {
        self.device_tree
    }

    pub const fn command_line(&self) -> Option<BootAddress> {
        self.command_line
    }

    pub const fn system_table(&self) -> Option<BootAddress> {
        self.system_table
    }
}
