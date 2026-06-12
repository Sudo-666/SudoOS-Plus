use crate::{VirtAddr, VirtRange};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct AddressSpaceId(u16);

impl AddressSpaceId {
    pub const KERNEL: Self = Self(0);

    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TlbScope {
    Local,
    AllCpus,
    AddressSpace(AddressSpaceId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TlbFlush {
    All { scope: TlbScope },
    Page { scope: TlbScope, address: VirtAddr },
    Range { scope: TlbScope, range: VirtRange },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TlbShootdown {
    flush: TlbFlush,
    generation: u64,
}

impl TlbShootdown {
    pub const fn new(flush: TlbFlush, generation: u64) -> Self {
        Self { flush, generation }
    }

    pub const fn flush(self) -> TlbFlush {
        self.flush
    }

    pub const fn generation(self) -> u64 {
        self.generation
    }
}
