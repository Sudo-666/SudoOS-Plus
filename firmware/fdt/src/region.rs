/// 由设备树描述的一段物理内存。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryRegion {
    start: usize,
    size: usize,
}

impl MemoryRegion {
    pub const fn new(start: usize, size: usize) -> Self {
        Self { start, size }
    }

    pub const fn start(self) -> usize {
        self.start
    }

    pub const fn size(self) -> usize {
        self.size
    }

    pub const fn end(self) -> Option<usize> {
        self.start.checked_add(self.size)
    }

    pub const fn is_empty(self) -> bool {
        self.size == 0
    }
}

/// 一个通过 MMIO 暴露的 VirtIO transport。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VirtioMmioRegion<'a> {
    name: &'a str,
    base: usize,
    size: usize,
}

impl<'a> VirtioMmioRegion<'a> {
    pub const fn new(name: &'a str, base: usize, size: usize) -> Self {
        Self { name, base, size }
    }

    pub const fn name(self) -> &'a str {
        self.name
    }

    pub const fn base(self) -> usize {
        self.base
    }

    pub const fn size(self) -> usize {
        self.size
    }

    pub const fn end(self) -> Option<usize> {
        self.base.checked_add(self.size)
    }
}
