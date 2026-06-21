pub const PAGE_SHIFT: usize = 12;
pub const PAGE_SIZE: usize = 1 << PAGE_SHIFT;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct PhysAddr(usize);

impl PhysAddr {
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

    pub const fn align_down(self, alignment: usize) -> Option<Self> {
        if !alignment.is_power_of_two() {
            return None;
        }

        Some(Self(self.0 & !(alignment - 1)))
    }

    pub const fn align_up(self, alignment: usize) -> Option<Self> {
        if !alignment.is_power_of_two() {
            return None;
        }

        let mask = alignment - 1;

        match self.0.checked_add(mask) {
            Some(address) => Some(Self(address & !mask)),

            None => None,
        }
    }
    pub const fn checked_sub(self, value: usize) -> Option<Self> {
        match self.0.checked_sub(value) {
            Some(address) => Some(Self(address)),
            None => None,
        }
    }

    pub const fn is_aligned(self, alignment: usize) -> bool {
        alignment != 0 && alignment.is_power_of_two() && self.0 & (alignment - 1) == 0
    }
}
