/// 页面的访问权限。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct PagePermissions(u8);

impl PagePermissions {
    pub const READ: Self = Self(1 << 0);

    pub const WRITE: Self = Self(1 << 1);

    pub const EXECUTE: Self = Self(1 << 2);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn read_only() -> Self {
        Self::READ
    }

    pub const fn read_write() -> Self {
        Self(Self::READ.0 | Self::WRITE.0)
    }

    pub const fn read_execute() -> Self {
        Self(Self::READ.0 | Self::EXECUTE.0)
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains(self, permission: Self) -> bool {
        self.0 & permission.0 == permission.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn is_readable(self) -> bool {
        self.contains(Self::READ)
    }

    pub const fn is_writable(self) -> bool {
        self.contains(Self::WRITE)
    }

    pub const fn is_executable(self) -> bool {
        self.contains(Self::EXECUTE)
    }
}

/// 页面的内存访问类型。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryType {
    /// 普通可缓存内存。
    Normal,

    /// 强序设备内存。
    Device,

    /// 不使用缓存的普通内存。
    Uncached,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MappingOptionsError {
    NoPermissions,

    WritableWithoutRead,

    UserGlobalMapping,

    WritableExecutableMapping,

    ExecutableDeviceMapping,
}

/// 与架构无关的映射语义。
///
/// 它不直接对应任何架构的 PTE 位。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MappingOptions {
    permissions: PagePermissions,
    memory_type: MemoryType,
    user: bool,
    global: bool,
}

impl MappingOptions {
    pub const fn new(permissions: PagePermissions) -> Self {
        Self {
            permissions,
            memory_type: MemoryType::Normal,
            user: false,
            global: false,
        }
    }

    pub const fn with_memory_type(mut self, memory_type: MemoryType) -> Self {
        self.memory_type = memory_type;
        self
    }

    pub const fn with_user(mut self, user: bool) -> Self {
        self.user = user;
        self
    }

    pub const fn with_global(mut self, global: bool) -> Self {
        self.global = global;
        self
    }

    pub const fn permissions(self) -> PagePermissions {
        self.permissions
    }

    pub const fn memory_type(self) -> MemoryType {
        self.memory_type
    }

    pub const fn is_user(self) -> bool {
        self.user
    }

    pub const fn is_global(self) -> bool {
        self.global
    }

    /// 验证公共安全策略。
    ///
    /// 当前默认执行 W^X：同一个映射不能同时可写和可执行。
    pub const fn validate(self) -> Result<(), MappingOptionsError> {
        if self.permissions.is_empty() {
            return Err(MappingOptionsError::NoPermissions);
        }

        if self.permissions.is_writable() && !self.permissions.is_readable() {
            return Err(MappingOptionsError::WritableWithoutRead);
        }

        if self.user && self.global {
            return Err(MappingOptionsError::UserGlobalMapping);
        }

        if self.permissions.is_writable() && self.permissions.is_executable() {
            return Err(MappingOptionsError::WritableExecutableMapping);
        }

        if matches!(self.memory_type, MemoryType::Device) && self.permissions.is_executable() {
            return Err(MappingOptionsError::ExecutableDeviceMapping);
        }

        Ok(())
    }

    pub const fn kernel_code() -> Self {
        Self::new(PagePermissions::read_execute()).with_global(true)
    }

    pub const fn kernel_rodata() -> Self {
        Self::new(PagePermissions::read_only()).with_global(true)
    }

    pub const fn kernel_data() -> Self {
        Self::new(PagePermissions::read_write()).with_global(true)
    }

    pub const fn kernel_device() -> Self {
        Self::new(PagePermissions::read_write())
            .with_memory_type(MemoryType::Device)
            .with_global(true)
    }

    pub const fn user_code() -> Self {
        Self::new(PagePermissions::read_execute()).with_user(true)
    }

    pub const fn user_data() -> Self {
        Self::new(PagePermissions::read_write()).with_user(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_writable_executable_mapping() {
        let permissions = PagePermissions::read_write().union(PagePermissions::EXECUTE);

        let mapping = MappingOptions::new(permissions);

        assert_eq!(
            mapping.validate(),
            Err(MappingOptionsError::WritableExecutableMapping,),
        );
    }

    #[test]
    fn accepts_kernel_code() {
        assert_eq!(MappingOptions::kernel_code().validate(), Ok(()),);
    }

    #[test]
    fn rejects_user_global_mapping() {
        let mapping = MappingOptions::user_data().with_global(true);

        assert_eq!(
            mapping.validate(),
            Err(MappingOptionsError::UserGlobalMapping,),
        );
    }
}
