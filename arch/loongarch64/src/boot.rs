use myos_boot::BootInfo;

#[derive(Clone, Copy)]
pub struct BootContext {
    raw_args: [usize; 3],

    device_tree: Option<usize>,
    command_line: Option<usize>,
    system_table: Option<usize>,
}

impl BootContext {
    pub(crate) const fn new(raw_args: [usize; 3]) -> Self {
        Self {
            raw_args,

            device_tree: None,
            command_line: None,
            system_table: None,
        }
    }

    pub(crate) const fn with_device_tree(mut self, address: usize) -> Self {
        self.device_tree = Some(address);
        self
    }

    pub(crate) const fn with_command_line(mut self, address: usize) -> Self {
        self.command_line = Some(address);
        self
    }

    pub(crate) const fn with_system_table(mut self, address: usize) -> Self {
        self.system_table = Some(address);
        self
    }

    pub const fn raw_args(&self) -> &[usize; 3] {
        &self.raw_args
    }

    pub const fn command_line_address(&self) -> Option<usize> {
        self.command_line
    }

    pub const fn system_table_address(&self) -> Option<usize> {
        self.system_table
    }

    pub const fn device_tree(&self) -> Option<usize> {
        self.device_tree
    }

    pub const fn into_boot_info(self) -> BootInfo {
        let mut info = BootInfo::new(self.raw_args);

        if let Some(address) = self.command_line {
            info = info.with_command_line(address);
        }

        if let Some(address) = self.system_table {
            info = info.with_system_table(address);
        }

        if let Some(address) = self.device_tree {
            info = info.with_device_tree(address);
        }

        info
    }
}

pub fn from_raw(arg0: usize, arg1: usize, arg2: usize) -> BootContext {
    crate::platform::boot_context(arg0, arg1, arg2)
}
