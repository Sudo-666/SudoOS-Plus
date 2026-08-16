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

/// 一个 DesignWare MMC 主机（`compatible = "snps,dw-mshc"`，JH7110）。
///
/// 由 `/aliases` 的 `mmcN` 指向。VisionFive 2 上 `mmc0` 是板载 eMMC（8-bit，
/// 不可移除），`mmc1` 是 TF 卡槽（4-bit，可移除）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MmcHostConfig {
    alias_index: Option<u8>,
    base: usize,
    size: usize,
    irq: usize,
    bus_width: u8,
    fifo_depth: Option<u32>,
    max_frequency_hz: Option<u64>,
    ciu_frequency_hz: Option<u64>,
    non_removable: bool,
}

impl MmcHostConfig {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        alias_index: Option<u8>,
        base: usize,
        size: usize,
        irq: usize,
        bus_width: u8,
        fifo_depth: Option<u32>,
        max_frequency_hz: Option<u64>,
        ciu_frequency_hz: Option<u64>,
        non_removable: bool,
    ) -> Self {
        Self {
            alias_index,
            base,
            size,
            irq,
            bus_width,
            fifo_depth,
            max_frequency_hz,
            ciu_frequency_hz,
            non_removable,
        }
    }

    pub const fn alias_index(self) -> Option<u8> {
        self.alias_index
    }

    pub const fn base(self) -> usize {
        self.base
    }

    pub const fn size(self) -> usize {
        self.size
    }

    pub const fn irq(self) -> usize {
        self.irq
    }

    pub const fn bus_width(self) -> u8 {
        self.bus_width
    }

    pub const fn fifo_depth(self) -> Option<u32> {
        self.fifo_depth
    }

    pub const fn max_frequency_hz(self) -> Option<u64> {
        self.max_frequency_hz
    }

    pub const fn ciu_frequency_hz(self) -> Option<u64> {
        self.ciu_frequency_hz
    }

    pub const fn non_removable(self) -> bool {
        self.non_removable
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
