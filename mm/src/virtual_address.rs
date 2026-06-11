/// 内核使用的虚拟地址。
///
/// 该类型只表示一个机器虚拟地址，不承诺：
///
/// - 地址对当前架构是规范地址；
/// - 地址已经建立映射；
/// - 地址可以安全解引用；
/// - 地址属于用户空间或内核空间。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct VirtAddr(usize);

impl VirtAddr {
    pub const fn new(address: usize) -> Self {
        Self(address)
    }

    pub const fn get(self) -> usize {
        self.0
    }

    pub const fn checked_add(self, value: usize) -> Option<Self> {
        match self.0.checked_add(value) {
            Some(address) => Some(Self(address)),
            None => None,
        }
    }

    pub const fn checked_sub(self, value: usize) -> Option<Self> {
        match self.0.checked_sub(value) {
            Some(address) => Some(Self(address)),
            None => None,
        }
    }

    pub const fn align_down(self, alignment: usize) -> Option<Self> {
        if alignment == 0 || !alignment.is_power_of_two() {
            return None;
        }

        Some(Self(self.0 & !(alignment - 1)))
    }

    pub const fn align_up(self, alignment: usize) -> Option<Self> {
        if alignment == 0 || !alignment.is_power_of_two() {
            return None;
        }

        let mask = alignment - 1;

        match self.0.checked_add(mask) {
            Some(address) => Some(Self(address & !mask)),

            None => None,
        }
    }

    pub const fn is_aligned(self, alignment: usize) -> bool {
        alignment != 0 && alignment.is_power_of_two() && self.0 & (alignment - 1) == 0
    }
}
