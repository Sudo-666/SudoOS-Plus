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

/// 固件加载、通过 `/reserved-memory` 声明的竞赛镜像区域
/// （`compatible = "sudoos,boot-ramdisk"`）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootRamdiskRegion {
    base: usize,
    size: usize,
    block_size: usize,
    read_only: bool,
}

impl BootRamdiskRegion {
    pub const fn new(base: usize, size: usize, block_size: usize, read_only: bool) -> Self {
        Self {
            base,
            size,
            block_size,
            read_only,
        }
    }

    pub const fn base(self) -> usize {
        self.base
    }

    pub const fn size(self) -> usize {
        self.size
    }

    pub const fn block_size(self) -> usize {
        self.block_size
    }

    pub const fn read_only(self) -> bool {
        self.read_only
    }

    pub const fn end(self) -> Option<usize> {
        self.base.checked_add(self.size)
    }
}

/// 一个通过 MMIO 暴露的 VirtIO transport。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VirtioMmioRegion<'a> {
    name: &'a str,
    base: usize,
    size: usize,
}

/// A PCI host bridge described by a Linux-compatible FDT node.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PciHostBridge<'a> {
    name: &'a str,
    ecam: MemoryRegion,
    mem32: MemoryRegion,
    first_bus: u8,
    last_bus: u8,
}

impl<'a> PciHostBridge<'a> {
    pub const fn new(
        name: &'a str,
        ecam: MemoryRegion,
        mem32: MemoryRegion,
        first_bus: u8,
        last_bus: u8,
    ) -> Self {
        Self {
            name,
            ecam,
            mem32,
            first_bus,
            last_bus,
        }
    }

    pub const fn name(self) -> &'a str {
        self.name
    }

    pub const fn ecam(self) -> MemoryRegion {
        self.ecam
    }

    pub const fn mem32(self) -> MemoryRegion {
        self.mem32
    }

    pub const fn first_bus(self) -> u8 {
        self.first_bus
    }

    pub const fn last_bus(self) -> u8 {
        self.last_bus
    }
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
