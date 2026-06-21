/// 启动阶段由固件或引导器提供的机器地址。
///
/// `BootAddress` 本身允许地址为零，因为某些平台的物理地址
/// 零是有效地址。
///
/// “是否提供该地址”由外层的 `Option<BootAddress>` 表达。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct BootAddress(usize);

impl BootAddress {
    pub const fn new(address: usize) -> Self {
        Self(address)
    }

    pub const fn get(self) -> usize {
        self.0
    }
}

impl From<BootAddress> for usize {
    fn from(address: BootAddress) -> Self {
        address.get()
    }
}
