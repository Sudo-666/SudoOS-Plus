use core::str;

use fdt_parser::{
    Fdt,
    helpers::UnalignedInfallibleNode,
    nodes::AsNode,
    parsing::{Panic, unaligned::UnalignedParser},
    properties::Compatible,
};

use crate::{
    BootRamdiskRegion, FdtBlob, FdtError, MemoryRegion, MmcHostConfig, PciHostBridge,
    VirtioMmioRegion,
};

/// MyOS 对设备树的只读视图。
///
/// 内部使用第三方 `fdt` crate，但不向其他内核模块暴露
/// 第三方类型。
#[derive(Clone)]
pub struct DeviceTree<'a> {
    inner: Fdt<'a, (UnalignedParser<'a>, Panic)>,
}

impl<'a> DeviceTree<'a> {
    /// 从已经完成基础边界验证的 blob 构造设备树。
    pub fn from_blob(blob: &FdtBlob<'a>) -> Result<Self, FdtError> {
        let inner = Fdt::new_unaligned(blob.as_bytes()).map_err(|_| FdtError::ParserRejected)?;

        Ok(Self { inner })
    }

    pub fn total_size(&self) -> usize {
        self.inner.total_size()
    }

    /// 可选的机器/开发板型号。
    pub fn model(&self) -> Option<&'a str> {
        self.inner
            .find_node("/")
            .and_then(|root| root.raw_property("model"))
            .and_then(|property| property_string(property.value))
    }

    /// 根节点中的首个 compatible。
    pub fn first_compatible(&self) -> Option<&'a str> {
        self.inner
            .find_node("/")
            .and_then(|root| root.property::<Compatible>())
            .and_then(|compatible| compatible.all().find(|value| !value.is_empty()))
    }

    pub fn cpu_count(&self) -> usize {
        self.all_cpu_hardware_ids().count()
    }

    /// 全部 hardware CPU/thread IDs(仅排除 `fail`,保留 `disabled`)。
    ///
    /// 只用于诊断输出,不应作为可启动 CPU 的依据——启动决策应使用
    /// [`available_cpu_hardware_ids`](Self::available_cpu_hardware_ids)。
    pub fn all_cpu_hardware_ids(&self) -> impl Iterator<Item = usize> + '_ {
        self.inner.root().cpus().iter().filter_map(|cpu| {
            if cpu.status().is_some_and(|status| status.is_failed()) {
                return None;
            }

            cpu.reg::<usize>().first().ok()
        })
    }

    /// 可启动的 hardware CPU/thread IDs。
    ///
    /// 排除 `disabled`、`fail` 及其他非 available 状态节点。平台层仍可用
    /// `hardware_cpu_is_supported` 进一步过滤(例如 VisionFive 2 的 hart 0
    /// 即使被错误 DTB 标成 `okay` 也必须排除)。
    pub fn available_cpu_hardware_ids(&self) -> impl Iterator<Item = usize> + '_ {
        self.inner.root().cpus().iter().filter_map(|cpu| {
            if !node_is_available(cpu.as_node()) {
                return None;
            }

            cpu.reg::<usize>().first().ok()
        })
    }

    /// `/cpus/timebase-frequency` declared by platforms such as RISC-V.
    ///
    /// Both one-cell and two-cell encodings are accepted.  A zero value is
    /// rejected because it cannot define a usable clocksource frequency.
    pub fn timebase_frequency_hz(&self) -> Option<u64> {
        let property = self
            .inner
            .find_node("/cpus")?
            .raw_property("timebase-frequency")?;
        let frequency = property_u64(property.value)?;

        (frequency != 0).then_some(frequency)
    }

    /// `/chosen/bootargs` (panic-free)。
    pub fn bootargs(&self) -> Option<&'a str> {
        self.inner
            .find_node("/chosen")
            .and_then(|chosen| chosen.raw_property("bootargs"))
            .and_then(|property| property_string(property.value))
    }

    /// Linux-compatible external initrd range from `/chosen`.
    ///
    /// QEMU publishes `linux,initrd-start` and `linux,initrd-end` when
    /// launched with `-initrd`.  Treat a half-present pair as malformed rather
    /// than silently booting without rootfs, because that usually means the
    /// firmware handoff is corrupt.
    pub fn linux_initrd_range(&self) -> Result<Option<MemoryRegion>, FdtError> {
        let Some(chosen) = self.inner.find_node("/chosen") else {
            return Ok(None);
        };

        let start = chosen.raw_property("linux,initrd-start");
        let end = chosen.raw_property("linux,initrd-end");
        let (Some(start), Some(end)) = (start, end) else {
            if start.is_some() || end.is_some() {
                return Err(FdtError::InvalidRegLength);
            }
            return Ok(None);
        };

        let start =
            usize::try_from(read_cells(start.value)?).map_err(|_| FdtError::AddressOverflow)?;
        let end = usize::try_from(read_cells(end.value)?).map_err(|_| FdtError::AddressOverflow)?;
        if end <= start {
            return Err(FdtError::InvalidRegLength);
        }

        Ok(Some(MemoryRegion::new(start, end - start)))
    }

    /// 设备树中声明的全部可用物理内存区域。
    ///
    /// 遍历所有 `device_type = "memory"` 节点及其全部 `reg` 项,而不是只
    /// 取 `root().memory()` 返回的单个 `/memory` 节点:多 RAM bank 的板卡
    /// 可能拆成多个 memory 节点,或在一个节点里写多组 reg。跳过
    /// `disabled`/`fail` 节点,跳过空区域与起始地址/长度溢出或
    /// start+size 溢出的非法项。
    pub fn memory_regions(&self) -> impl Iterator<Item = MemoryRegion> + '_ {
        self.inner
            .all_nodes()
            .filter_map(|(_depth, node)| {
                if node_is_available(node) && node_is_memory(node) {
                    node.reg()
                } else {
                    None
                }
            })
            .flat_map(|reg| {
                reg.iter::<u64, u64>().filter_map(|entry| {
                    let entry = entry.ok()?;

                    let start = usize::try_from(entry.address).ok()?;

                    let size = usize::try_from(entry.len).ok()?;

                    let region = MemoryRegion::new(start, size);

                    // 跳过空区域(start+size 溢出时 end() 为 None)。
                    (!region.is_empty() && region.end().is_some()).then_some(region)
                })
            })
    }

    /// 查找所有启用的 `virtio,mmio` 节点。
    pub fn virtio_mmio_regions(&self) -> impl Iterator<Item = VirtioMmioRegion<'a>> + '_ {
        self.inner.all_nodes().filter_map(|(_depth, node)| {
            if !node_is_available(node) {
                return None;
            }

            if !node_is_compatible(node, "virtio,mmio") {
                return None;
            }

            let reg = node.reg()?;
            let mut regions = reg.iter::<u64, u64>();
            let region = regions.next()?.ok()?;

            let base = usize::try_from(region.address).ok()?;

            let size = usize::try_from(region.len).ok()?;

            Some(VirtioMmioRegion::new(node.name().name, base, size))
        })
    }

    /// Discover generic ECAM PCI host bridges.
    ///
    /// This follows the Linux devicetree binding used by QEMU virt machines:
    /// `compatible = "pci-host-ecam-generic"`, `reg` for the ECAM window,
    /// `bus-range` for bus numbers, and `ranges` for PCI memory windows.
    pub fn pci_host_bridges(&self) -> impl Iterator<Item = PciHostBridge<'a>> + '_ {
        let root_address_cells = self
            .inner
            .find_node("/")
            .and_then(|root| read_cell_count(root, "#address-cells"))
            .unwrap_or(2);
        let root_size_cells = self
            .inner
            .find_node("/")
            .and_then(|root| read_cell_count(root, "#size-cells"))
            .unwrap_or(1);

        self.inner.all_nodes().filter_map(move |(_depth, node)| {
            if !node_is_available(node) || !node_is_compatible(node, "pci-host-ecam-generic") {
                return None;
            }

            let address_cells = read_cell_count(node, "#address-cells")?;
            let size_cells = read_cell_count(node, "#size-cells")?;
            if address_cells != 3 || !(1..=2).contains(&size_cells) {
                return None;
            }
            if validate_cell_counts(root_address_cells, root_size_cells).is_err() {
                return None;
            }

            let ecam = parse_first_region(
                node.raw_property("reg")?.value,
                root_address_cells,
                root_size_cells,
            )
            .ok()?;
            let mem32 = parse_pci_memory_range(
                node.raw_property("ranges")?.value,
                root_address_cells,
                size_cells,
            )
            .ok()?;
            let (first_bus, last_bus) = parse_bus_range(node.raw_property("bus-range")?.value)?;
            if first_bus > last_bus {
                return None;
            }

            Some(PciHostBridge::new(
                node.name().name,
                ecam,
                mem32,
                first_bus,
                last_bus,
            ))
        })
    }

    /// 遍历 `/reserved-memory` 中静态声明的区域。
    ///
    /// 当前只处理带有 `reg` 的静态区域。
    /// 带有 `size` 的动态预留区域要等页帧分配器完成后再支持。
    pub fn for_each_reserved_memory_region(
        &self,
        mut visitor: impl FnMut(&str, MemoryRegion),
    ) -> Result<(), FdtError> {
        let Some(root) = self.inner.find_node("/") else {
            return Err(FdtError::InvalidReservedMemoryLayout);
        };

        let Some(reserved) = self.inner.find_node("/reserved-memory") else {
            return Ok(());
        };

        let root_address_cells = read_cell_count(root, "#address-cells").unwrap_or(2);

        let root_size_cells = read_cell_count(root, "#size-cells").unwrap_or(1);

        let Some(address_cells) = read_cell_count(reserved, "#address-cells") else {
            return Err(FdtError::InvalidReservedMemoryLayout);
        };

        let Some(size_cells) = read_cell_count(reserved, "#size-cells") else {
            return Err(FdtError::InvalidReservedMemoryLayout);
        };

        /*
         * /reserved-memory 应使用与根节点相同的地址和长度格式，
         * 并带有空 ranges 属性。
         */
        if address_cells != root_address_cells
            || size_cells != root_size_cells
            || reserved.raw_property("ranges").is_none()
        {
            return Err(FdtError::InvalidReservedMemoryLayout);
        }

        validate_cell_counts(address_cells, size_cells)?;

        for child in reserved.children() {
            if !node_is_available(child) {
                continue;
            }

            let name = child.name().name;

            if let Some(property) = child.raw_property("reg") {
                parse_reg_property(property.value, address_cells, size_cells, |region| {
                    visitor(name, region)
                })?;

                continue;
            }

            /*
             * 只有 size 没有 reg，表示要求 OS 动态选择位置。
             * 现在不能忽略，否则以后可能把这段内存分配给其他用途。
             */
            if child.raw_property("size").is_some() {
                return Err(FdtError::DynamicReservedMemoryUnsupported);
            }
        }

        Ok(())
    }

    /// 遍历 `/reserved-memory` 中 `compatible = "sudoos,boot-ramdisk"` 的节点。
    ///
    /// 这些节点声明固件（如 LS2K1000 U-Boot）加载竞赛镜像的只读物理区域。
    /// 区域同时被 [`Self::for_each_reserved_memory_region`] 从 free memory
    /// 中排除；这里提供 `block-size` 与 `read-only` 细节供存储层注册
    /// `/dev/ram0`。
    pub fn for_each_boot_ramdisk(
        &self,
        mut visitor: impl FnMut(BootRamdiskRegion),
    ) -> Result<(), FdtError> {
        let Some(root) = self.inner.find_node("/") else {
            return Err(FdtError::InvalidReservedMemoryLayout);
        };
        let Some(reserved) = self.inner.find_node("/reserved-memory") else {
            return Ok(());
        };

        let root_address_cells = read_cell_count(root, "#address-cells").unwrap_or(2);
        let root_size_cells = read_cell_count(root, "#size-cells").unwrap_or(1);
        let Some(address_cells) = read_cell_count(reserved, "#address-cells") else {
            return Err(FdtError::InvalidReservedMemoryLayout);
        };
        let Some(size_cells) = read_cell_count(reserved, "#size-cells") else {
            return Err(FdtError::InvalidReservedMemoryLayout);
        };
        if address_cells != root_address_cells
            || size_cells != root_size_cells
            || reserved.raw_property("ranges").is_none()
        {
            return Err(FdtError::InvalidReservedMemoryLayout);
        }
        validate_cell_counts(address_cells, size_cells)?;

        for child in reserved.children() {
            if !node_is_available(child) || !node_is_compatible(child, "sudoos,boot-ramdisk") {
                continue;
            }
            let Some(reg) = child.raw_property("reg") else {
                continue;
            };
            let region = parse_first_region(reg.value, address_cells, size_cells)?;
            let block_size = child
                .raw_property("block-size")
                .and_then(|property| read_cells(property.value).ok())
                .and_then(|value| usize::try_from(value).ok())
                .unwrap_or(512);
            if block_size == 0 {
                continue;
            }
            let read_only = child.raw_property("read-only").is_some();
            visitor(BootRamdiskRegion::new(
                region.start(),
                region.size(),
                block_size,
                read_only,
            ));
        }
        Ok(())
    }

    /// 遍历 `/aliases` 中 `mmcN` 指向的 DesignWare MMC 主机
    /// （`compatible` 匹配 `"snps,dw-mshc"` 或 `"starfive,jh7110-mmc"`——
    /// 后者是 JH7110 上游 DT 的正式兼容串，K3.1）。VisionFive 2 上 `mmc0`
    /// 是板载 eMMC、`mmc1` 是 TF 卡槽；`alias_index` 让调用方按槽位选择。
    pub fn for_each_mmc_host(
        &self,
        mut visitor: impl FnMut(MmcHostConfig),
    ) -> Result<(), FdtError> {
        let Some(aliases) = self.inner.root().aliases() else {
            return Ok(());
        };
        for (name, path) in aliases.iter() {
            let Some(index_text) = name.strip_prefix("mmc") else {
                continue;
            };
            let Ok(alias_index) = index_text.parse::<u8>() else {
                continue;
            };
            let Some(node) = self.inner.find_node(path) else {
                continue;
            };
            let node = node.as_node();
            if !node_is_available(node)
                || !(node_is_compatible(node, "snps,dw-mshc")
                    || node_is_compatible(node, "starfive,jh7110-mmc"))
            {
                continue;
            }
            let Some(reg) = node.reg() else {
                continue;
            };
            let mut regions = reg.iter::<u64, u64>();
            let Some(Ok(entry)) = regions.next() else {
                continue;
            };
            let base = match usize::try_from(entry.address) {
                Ok(base) => base,
                Err(_) => continue,
            };
            let size = match usize::try_from(entry.len) {
                Ok(size) => size,
                Err(_) => continue,
            };

            let irq = node
                .raw_property("interrupts")
                .filter(|property| property.value.len() >= 4)
                .map(|property| {
                    u32::from_be_bytes([
                        property.value[0],
                        property.value[1],
                        property.value[2],
                        property.value[3],
                    ]) as usize
                })
                .unwrap_or(0);

            let bus_width = node
                .raw_property("bus-width")
                .and_then(|property| read_u32_property(property.value))
                .unwrap_or(1)
                .clamp(1, 8) as u8;

            let fifo_depth = node
                .raw_property("fifo-depth")
                .and_then(|property| read_u32_property(property.value));

            let max_frequency_hz = node
                .raw_property("max-frequency")
                .and_then(|property| read_cells(property.value).ok());

            let ciu_frequency_hz = node
                .raw_property("clock-frequency")
                .and_then(|property| read_cells(property.value).ok())
                .or_else(|| {
                    node.raw_property("assigned-clock-rates")
                        .and_then(|property| last_cell(property.value))
                });

            let non_removable = node.raw_property("non-removable").is_some();

            visitor(MmcHostConfig::new(
                Some(alias_index),
                base,
                size,
                irq,
                bus_width,
                fifo_depth,
                max_frequency_hz,
                ciu_frequency_hz,
                non_removable,
            ));
        }
        Ok(())
    }
}

fn read_u32_property(bytes: &[u8]) -> Option<u32> {
    (bytes.len() == 4).then(|| u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// 读取大端 32-bit cell 列表的最后一个 cell（assigned-clock-rates 中
/// 的 ciu 频率）。
fn last_cell(bytes: &[u8]) -> Option<u64> {
    if bytes.len() % 4 != 0 || bytes.is_empty() {
        return None;
    }
    let offset = bytes.len() - 4;
    Some(u64::from(u32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])))
}

fn node_is_memory(node: UnalignedInfallibleNode<'_>) -> bool {
    match node.raw_property("device_type") {
        Some(property) => property_string(property.value) == Some("memory"),
        None => false,
    }
}

fn node_is_compatible(node: UnalignedInfallibleNode<'_>, expected: &str) -> bool {
    node.property::<Compatible>()
        .is_some_and(|compatible| compatible.all().any(|value| value == expected))
}

fn node_is_available(node: UnalignedInfallibleNode<'_>) -> bool {
    match node
        .raw_property("status")
        .and_then(|property| property_string(property.value))
    {
        None | Some("ok") | Some("okay") => true,
        Some(_) => false,
    }
}

fn property_u64(bytes: &[u8]) -> Option<u64> {
    match bytes {
        [a, b, c, d] => Some(u64::from(u32::from_be_bytes([*a, *b, *c, *d]))),
        [a, b, c, d, e, f, g, h] => Some(u64::from_be_bytes([*a, *b, *c, *d, *e, *f, *g, *h])),
        _ => None,
    }
}

fn property_string(bytes: &[u8]) -> Option<&str> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());

    str::from_utf8(&bytes[..end]).ok()
}

fn read_cell_count(node: UnalignedInfallibleNode<'_>, property_name: &str) -> Option<u32> {
    let property = node.raw_property(property_name)?;

    if property.value.len() != 4 {
        return None;
    }

    Some(u32::from_be_bytes([
        property.value[0],
        property.value[1],
        property.value[2],
        property.value[3],
    ]))
}

fn validate_cell_counts(address_cells: u32, size_cells: u32) -> Result<(), FdtError> {
    /*
     * 当前目标均为 64 位，因此支持 1 或 2 个 cell。
     */
    if !(1..=2).contains(&address_cells) || !(1..=2).contains(&size_cells) {
        return Err(FdtError::UnsupportedCellCount {
            address_cells,
            size_cells,
        });
    }

    Ok(())
}

fn parse_reg_property(
    bytes: &[u8],
    address_cells: u32,
    size_cells: u32,
    mut visitor: impl FnMut(MemoryRegion),
) -> Result<(), FdtError> {
    validate_cell_counts(address_cells, size_cells)?;

    let address_cells = address_cells as usize;
    let size_cells = size_cells as usize;

    let entry_cells = address_cells
        .checked_add(size_cells)
        .ok_or(FdtError::AddressOverflow)?;

    let entry_size = entry_cells
        .checked_mul(4)
        .ok_or(FdtError::AddressOverflow)?;

    if entry_size == 0 || bytes.is_empty() || !bytes.len().is_multiple_of(entry_size) {
        return Err(FdtError::InvalidRegLength);
    }

    for entry in bytes.chunks_exact(entry_size) {
        let address_bytes = &entry[..address_cells * 4];

        let size_bytes = &entry[address_cells * 4..];

        let address = read_cells(address_bytes)?;

        let size = read_cells(size_bytes)?;

        let address = usize::try_from(address).map_err(|_| FdtError::AddressOverflow)?;

        let size = usize::try_from(size).map_err(|_| FdtError::AddressOverflow)?;

        if size == 0 {
            continue;
        }

        address.checked_add(size).ok_or(FdtError::AddressOverflow)?;

        visitor(MemoryRegion::new(address, size));
    }

    Ok(())
}

fn parse_first_region(
    bytes: &[u8],
    address_cells: u32,
    size_cells: u32,
) -> Result<MemoryRegion, FdtError> {
    let mut first = None;
    parse_reg_property(bytes, address_cells, size_cells, |region| {
        if first.is_none() {
            first = Some(region);
        }
    })?;
    first.ok_or(FdtError::InvalidRegLength)
}

fn parse_pci_memory_range(
    bytes: &[u8],
    parent_address_cells: u32,
    size_cells: u32,
) -> Result<MemoryRegion, FdtError> {
    validate_cell_counts(parent_address_cells, size_cells)?;
    let parent_address_cells = parent_address_cells as usize;
    let size_cells = size_cells as usize;
    let child_address_cells = 3_usize;
    let entry_cells = child_address_cells
        .checked_add(parent_address_cells)
        .and_then(|cells| cells.checked_add(size_cells))
        .ok_or(FdtError::AddressOverflow)?;
    let entry_size = entry_cells
        .checked_mul(4)
        .ok_or(FdtError::AddressOverflow)?;
    if entry_size == 0 || bytes.is_empty() || !bytes.len().is_multiple_of(entry_size) {
        return Err(FdtError::InvalidRegLength);
    }

    let mut best = None;
    for entry in bytes.chunks_exact(entry_size) {
        let child_hi = read_cells(&entry[..4])?;
        let prefetchable = child_hi & 0x4000_0000 != 0;
        let space = (child_hi >> 24) & 0x03;
        let parent_start = read_cells(
            &entry[child_address_cells * 4..(child_address_cells + parent_address_cells) * 4],
        )?;
        let size = read_cells(&entry[(child_address_cells + parent_address_cells) * 4..])?;
        if prefetchable || !(space == 0x02 || space == 0x03) || size == 0 {
            continue;
        }

        let start = usize::try_from(parent_start).map_err(|_| FdtError::AddressOverflow)?;
        let size = usize::try_from(size).map_err(|_| FdtError::AddressOverflow)?;
        start.checked_add(size).ok_or(FdtError::AddressOverflow)?;
        let region = MemoryRegion::new(start, size);
        if best.is_none_or(|old: MemoryRegion| region.size() > old.size()) {
            best = Some(region);
        }
    }

    best.ok_or(FdtError::InvalidRegLength)
}

fn parse_bus_range(bytes: &[u8]) -> Option<(u8, u8)> {
    if bytes.len() != 8 {
        return None;
    }
    let first = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let last = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    Some((u8::try_from(first).ok()?, u8::try_from(last).ok()?))
}

fn read_cells(bytes: &[u8]) -> Result<u64, FdtError> {
    if bytes.len() != 4 && bytes.len() != 8 {
        return Err(FdtError::InvalidRegLength);
    }

    let mut value = 0_u64;

    for cell in bytes.chunks_exact(4) {
        let part = u32::from_be_bytes([cell[0], cell[1], cell[2], cell[3]]);

        value = value.checked_shl(32).ok_or(FdtError::AddressOverflow)? | u64::from(part);
    }

    Ok(value)
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::property_u64;
    use crate::{DeviceTree, FdtBlob, MemoryRegion};
    use std::vec::Vec;

    /// FDT 结构块 token。
    const FDT_BEGIN_NODE: u32 = 0x1;
    const FDT_END_NODE: u32 = 0x2;
    const FDT_PROP: u32 = 0x3;
    const FDT_END: u32 = 0x9;

    fn be32(value: u32) -> [u8; 4] {
        value.to_be_bytes()
    }

    fn u64_cells(value: u64) -> [u8; 8] {
        value.to_be_bytes()
    }

    /// 写入 FDT_BEGIN_NODE + 名称(NUL 结尾,4 字节对齐)。
    fn push_node(bytes: &mut Vec<u8>, name: &str) {
        bytes.extend_from_slice(&be32(FDT_BEGIN_NODE));
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0);
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
    }

    fn push_end_node(bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&be32(FDT_END_NODE));
    }

    /// 写入 FDT_PROP: 长度、字符串区偏移、数据(4 字节对齐)。
    fn push_prop(bytes: &mut Vec<u8>, strings: &mut Vec<u8>, name: &str, value: &[u8]) {
        bytes.extend_from_slice(&be32(FDT_PROP));
        bytes.extend_from_slice(&be32(value.len() as u32));

        let name_off = strings.len() as u32;
        strings.extend_from_slice(name.as_bytes());
        strings.push(0);

        bytes.extend_from_slice(&be32(name_off));
        bytes.extend_from_slice(value);
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
    }

    /// 一个 memory 节点的描述。
    ///
    /// status: None 表示不写 status 属性(即 available),Some("disabled")
    /// 表示显式禁用。reg 项直接给原始 8 字节地址/长度(2-cell 编码),
    /// 便于构造 start+size 溢出的非法项。
    struct MemoryNode {
        name: &'static str,
        address: u64,
        size: u64,
        status: Option<&'static str>,
    }

    /// 一个 `/cpus/cpu@*` 节点的描述。
    ///
    /// `/cpus` 固定 `#address-cells = 1`、`#size-cells = 0`,reg 是
    /// 单个 cell 的硬件 ID。
    struct CpuNode {
        name: &'static str,
        id: u32,
        status: Option<&'static str>,
    }

    /// 组装一个最小但结构合法的 FDT:根节点固定 `#address-cells = 2`、
    /// `#size-cells = 2`,外加若干 memory 节点与可选的 `/cpus` 节点。
    fn build_fdt(nodes: &[MemoryNode], cpus: &[CpuNode]) -> Vec<u8> {
        let mut structure = Vec::new();
        let mut strings = Vec::new();

        push_node(&mut structure, "");
        push_prop(&mut structure, &mut strings, "#address-cells", &be32(2));
        push_prop(&mut structure, &mut strings, "#size-cells", &be32(2));

        for node in nodes {
            push_node(&mut structure, node.name);
            push_prop(&mut structure, &mut strings, "device_type", b"memory\0");

            let mut reg = Vec::with_capacity(16);
            reg.extend_from_slice(&u64_cells(node.address));
            reg.extend_from_slice(&u64_cells(node.size));
            push_prop(&mut structure, &mut strings, "reg", &reg);

            if let Some(status) = node.status {
                let mut value = status.as_bytes().to_vec();
                value.push(0);
                push_prop(&mut structure, &mut strings, "status", &value);
            }
            push_end_node(&mut structure);
        }

        if !cpus.is_empty() {
            push_node(&mut structure, "cpus");
            push_prop(&mut structure, &mut strings, "#address-cells", &be32(1));
            push_prop(&mut structure, &mut strings, "#size-cells", &be32(0));

            for cpu in cpus {
                push_node(&mut structure, cpu.name);
                push_prop(&mut structure, &mut strings, "reg", &be32(cpu.id));
                if let Some(status) = cpu.status {
                    let mut value = status.as_bytes().to_vec();
                    value.push(0);
                    push_prop(&mut structure, &mut strings, "status", &value);
                }
                push_end_node(&mut structure);
            }

            push_end_node(&mut structure);
        }

        push_end_node(&mut structure);
        structure.extend_from_slice(&be32(FDT_END));

        let header_size = 40_usize;
        let struct_offset = header_size + 16; // header + 16 字节 rsvmap 终止项
        let strings_offset = struct_offset + structure.len();
        let total_size = strings_offset + strings.len();

        let mut fdt = Vec::with_capacity(total_size);
        fdt.extend_from_slice(&be32(0xd00d_feed)); // magic
        fdt.extend_from_slice(&be32(total_size as u32)); // totalsize
        fdt.extend_from_slice(&be32(struct_offset as u32)); // off_dt_struct
        fdt.extend_from_slice(&be32(strings_offset as u32)); // off_dt_strings
        fdt.extend_from_slice(&be32(header_size as u32)); // off_mem_rsvmap
        fdt.extend_from_slice(&be32(17)); // version
        fdt.extend_from_slice(&be32(16)); // last_comp_version
        fdt.extend_from_slice(&be32(0)); // boot_cpuid_phys
        fdt.extend_from_slice(&be32(strings.len() as u32)); // size_dt_strings
        fdt.extend_from_slice(&be32(structure.len() as u32)); // size_dt_struct
        fdt.extend_from_slice(&[0_u8; 16]); // mem_rsvmap 终止项
        fdt.extend_from_slice(&structure);
        fdt.extend_from_slice(&strings);

        assert_eq!(fdt.len(), total_size);
        fdt
    }

    fn build_fdt_mem(nodes: &[MemoryNode]) -> Vec<u8> {
        build_fdt(nodes, &[])
    }

    fn collect_memory_regions(bytes: &[u8]) -> Vec<MemoryRegion> {
        let blob = FdtBlob::from_bytes(bytes).expect("valid FDT blob");
        let tree = DeviceTree::from_blob(&blob).expect("parseable device tree");
        tree.memory_regions().collect()
    }

    fn collect_cpu_ids(bytes: &[u8], available: bool) -> Vec<usize> {
        let blob = FdtBlob::from_bytes(bytes).expect("valid FDT blob");
        let tree = DeviceTree::from_blob(&blob).expect("parseable device tree");
        if available {
            tree.available_cpu_hardware_ids().collect()
        } else {
            tree.all_cpu_hardware_ids().collect()
        }
    }

    fn assert_region(regions: &[MemoryRegion], index: usize, start: usize, size: usize) {
        let region = regions[index];
        assert_eq!(region.start(), start);
        assert_eq!(region.size(), size);
    }

    #[test]
    fn parses_one_cell_frequency() {
        assert_eq!(
            property_u64(&10_000_000_u32.to_be_bytes()),
            Some(10_000_000)
        );
    }

    #[test]
    fn parses_two_cell_frequency() {
        assert_eq!(
            property_u64(&4_000_000_000_u64.to_be_bytes()),
            Some(4_000_000_000),
        );
    }

    #[test]
    fn rejects_invalid_frequency_width() {
        assert_eq!(property_u64(&[0, 1]), None);
    }

    #[test]
    fn memory_regions_single_bank() {
        let fdt = build_fdt_mem(&[MemoryNode {
            name: "memory@80000000",
            address: 0x8000_0000,
            size: 0x2000_0000,
            status: None,
        }]);

        let regions = collect_memory_regions(&fdt);
        assert_eq!(regions.len(), 1);
        assert_region(&regions, 0, 0x8000_0000, 0x2000_0000);
    }

    #[test]
    fn memory_regions_two_banks() {
        // 两个独立 memory 节点。
        let fdt = build_fdt_mem(&[
            MemoryNode {
                name: "memory@40000000",
                address: 0x4000_0000,
                size: 0x1000_0000,
                status: None,
            },
            MemoryNode {
                name: "memory@80000000",
                address: 0x8000_0000,
                size: 0x2000_0000,
                status: None,
            },
        ]);

        let regions = collect_memory_regions(&fdt);
        assert_eq!(regions.len(), 2);
        assert_region(&regions, 0, 0x4000_0000, 0x1000_0000);
        assert_region(&regions, 1, 0x8000_0000, 0x2000_0000);
    }

    #[test]
    fn memory_regions_64bit_address() {
        // 地址/长度超过 4 GiB,验证 2-cell 编码。
        let fdt = build_fdt_mem(&[MemoryNode {
            name: "memory@200000000",
            address: 0x2_0000_0000,
            size: 0x1_0000_0000,
            status: None,
        }]);

        let regions = collect_memory_regions(&fdt);
        assert_eq!(regions.len(), 1);
        assert_region(&regions, 0, 0x2_0000_0000, 0x1_0000_0000);
    }

    #[test]
    fn memory_regions_skips_disabled_node() {
        let fdt = build_fdt_mem(&[
            MemoryNode {
                name: "memory@40000000",
                address: 0x4000_0000,
                size: 0x1000_0000,
                status: None,
            },
            MemoryNode {
                name: "memory@80000000",
                address: 0x8000_0000,
                size: 0x2000_0000,
                status: Some("disabled"),
            },
        ]);

        let regions = collect_memory_regions(&fdt);
        assert_eq!(regions.len(), 1);
        assert_region(&regions, 0, 0x4000_0000, 0x1000_0000);
    }

    #[test]
    fn memory_regions_skips_overflow_reg() {
        // start+size 溢出 u64 的非法项必须被跳过,不能 panic。
        let fdt = build_fdt_mem(&[MemoryNode {
            name: "memory@0",
            address: u64::MAX,
            size: 0x2,
            status: None,
        }]);

        let regions = collect_memory_regions(&fdt);
        assert!(regions.is_empty());
    }

    #[test]
    fn cpu_available_filters_disabled_and_fail() {
        let fdt = build_fdt(
            &[],
            &[
                CpuNode {
                    name: "cpu@0",
                    id: 0,
                    status: Some("okay"),
                },
                CpuNode {
                    name: "cpu@1",
                    id: 1,
                    status: Some("disabled"),
                },
                CpuNode {
                    name: "cpu@2",
                    id: 2,
                    status: Some("fail"),
                },
                CpuNode {
                    name: "cpu@3",
                    id: 3,
                    status: None,
                },
            ],
        );

        // all: 保留 disabled(1),排除 fail(2)。
        assert_eq!(collect_cpu_ids(&fdt, false), Vec::from([0, 1, 3]));
        // available: 排除 disabled(1)与 fail(2)。
        assert_eq!(collect_cpu_ids(&fdt, true), Vec::from([0, 3]));
    }

    #[test]
    fn cpu_available_treats_no_status_as_available() {
        let fdt = build_fdt(
            &[],
            &[CpuNode {
                name: "cpu@7",
                id: 7,
                status: None,
            }],
        );

        assert_eq!(collect_cpu_ids(&fdt, true), Vec::from([7]));
    }

    /// 组装一个带 `/reserved-memory/contest-disk@e0000000` 的最小 FDT。
    fn build_fdt_with_boot_ramdisk() -> Vec<u8> {
        let mut structure = Vec::new();
        let mut strings = Vec::new();

        push_node(&mut structure, "");
        push_prop(&mut structure, &mut strings, "#address-cells", &be32(2));
        push_prop(&mut structure, &mut strings, "#size-cells", &be32(2));

        push_node(&mut structure, "reserved-memory");
        push_prop(&mut structure, &mut strings, "#address-cells", &be32(2));
        push_prop(&mut structure, &mut strings, "#size-cells", &be32(2));
        push_prop(&mut structure, &mut strings, "ranges", &[]);

        push_node(&mut structure, "contest-disk@e0000000");
        push_prop(
            &mut structure,
            &mut strings,
            "compatible",
            b"sudoos,boot-ramdisk\0",
        );
        let mut reg = Vec::new();
        reg.extend_from_slice(&u64_cells(0xe000_0000));
        reg.extend_from_slice(&u64_cells(0x0200_0000));
        push_prop(&mut structure, &mut strings, "reg", &reg);
        push_prop(&mut structure, &mut strings, "block-size", &be32(512));
        push_prop(&mut structure, &mut strings, "read-only", &[]);
        push_end_node(&mut structure);

        push_end_node(&mut structure); // reserved-memory
        push_end_node(&mut structure); // root
        structure.extend_from_slice(&be32(FDT_END));

        let header_size = 40_usize;
        let struct_offset = header_size + 16;
        let strings_offset = struct_offset + structure.len();
        let total_size = strings_offset + strings.len();
        let mut fdt = Vec::with_capacity(total_size);
        fdt.extend_from_slice(&be32(0xd00d_feed));
        fdt.extend_from_slice(&be32(total_size as u32));
        fdt.extend_from_slice(&be32(struct_offset as u32));
        fdt.extend_from_slice(&be32(strings_offset as u32));
        fdt.extend_from_slice(&be32(header_size as u32));
        fdt.extend_from_slice(&be32(17));
        fdt.extend_from_slice(&be32(16));
        fdt.extend_from_slice(&be32(0));
        fdt.extend_from_slice(&be32(strings.len() as u32));
        fdt.extend_from_slice(&be32(structure.len() as u32));
        fdt.extend_from_slice(&[0_u8; 16]);
        fdt.extend_from_slice(&structure);
        fdt.extend_from_slice(&strings);
        assert_eq!(fdt.len(), total_size);
        fdt
    }

    #[test]
    fn for_each_boot_ramdisk_parses_contest_disk() {
        let fdt = build_fdt_with_boot_ramdisk();
        let blob = FdtBlob::from_bytes(&fdt).expect("valid FDT blob");
        let tree = DeviceTree::from_blob(&blob).expect("parseable device tree");

        let mut regions = Vec::new();
        tree.for_each_boot_ramdisk(|region| regions.push(region))
            .expect("boot ramdisk parse");

        assert_eq!(regions.len(), 1);
        let region = regions[0];
        assert_eq!(region.base(), 0xe000_0000);
        assert_eq!(region.size(), 0x0200_0000);
        assert_eq!(region.block_size(), 512);
        assert!(region.read_only());
        assert_eq!(region.end(), Some(0xe200_0000));
    }

    /// 组装一个带 `/aliases`(mmc0/mmc1/mmc2) + 三个主机的 FDT。
    /// `sdio0` 显式 disabled；`sdio1` 用 `snps,dw-mshc`；
    /// `sdio2` 用 JH7110 上游正式串 `starfive,jh7110-mmc`（K3.1）。
    fn build_fdt_with_mmc_hosts() -> Vec<u8> {
        let mut structure = Vec::new();
        let mut strings = Vec::new();

        push_node(&mut structure, "");
        push_prop(&mut structure, &mut strings, "#address-cells", &be32(2));
        push_prop(&mut structure, &mut strings, "#size-cells", &be32(2));

        push_node(&mut structure, "aliases");
        push_prop(
            &mut structure,
            &mut strings,
            "mmc0",
            b"/soc/sdio0@16010000\0",
        );
        push_prop(
            &mut structure,
            &mut strings,
            "mmc1",
            b"/soc/sdio1@16020000\0",
        );
        push_prop(
            &mut structure,
            &mut strings,
            "mmc2",
            b"/soc/sdio2@16030000\0",
        );
        push_end_node(&mut structure);

        push_node(&mut structure, "soc");
        push_prop(&mut structure, &mut strings, "#address-cells", &be32(2));
        push_prop(&mut structure, &mut strings, "#size-cells", &be32(2));
        push_prop(&mut structure, &mut strings, "ranges", &[]);

        push_node(&mut structure, "sdio0@16010000");
        push_prop(
            &mut structure,
            &mut strings,
            "compatible",
            b"snps,dw-mshc\0",
        );
        let mut reg0 = Vec::new();
        reg0.extend_from_slice(&u64_cells(0x1601_0000));
        reg0.extend_from_slice(&u64_cells(0x1_0000));
        push_prop(&mut structure, &mut strings, "reg", &reg0);
        push_prop(&mut structure, &mut strings, "bus-width", &be32(8));
        push_prop(&mut structure, &mut strings, "status", b"disabled\0");
        push_end_node(&mut structure);

        push_node(&mut structure, "sdio1@16020000");
        push_prop(
            &mut structure,
            &mut strings,
            "compatible",
            b"snps,dw-mshc\0",
        );
        let mut reg1 = Vec::new();
        reg1.extend_from_slice(&u64_cells(0x1602_0000));
        reg1.extend_from_slice(&u64_cells(0x1_0000));
        push_prop(&mut structure, &mut strings, "reg", &reg1);
        push_prop(&mut structure, &mut strings, "bus-width", &be32(4));
        push_prop(&mut structure, &mut strings, "fifo-depth", &be32(32));
        push_end_node(&mut structure);

        push_node(&mut structure, "sdio2@16030000");
        push_prop(
            &mut structure,
            &mut strings,
            "compatible",
            b"starfive,jh7110-mmc\0",
        );
        let mut reg2 = Vec::new();
        reg2.extend_from_slice(&u64_cells(0x1603_0000));
        reg2.extend_from_slice(&u64_cells(0x1_0000));
        push_prop(&mut structure, &mut strings, "reg", &reg2);
        push_prop(&mut structure, &mut strings, "bus-width", &be32(4));
        push_end_node(&mut structure);

        push_end_node(&mut structure); // soc
        push_end_node(&mut structure); // root
        structure.extend_from_slice(&be32(FDT_END));

        let header_size = 40_usize;
        let struct_offset = header_size + 16;
        let strings_offset = struct_offset + structure.len();
        let total_size = strings_offset + strings.len();
        let mut fdt = Vec::with_capacity(total_size);
        fdt.extend_from_slice(&be32(0xd00d_feed));
        fdt.extend_from_slice(&be32(total_size as u32));
        fdt.extend_from_slice(&be32(struct_offset as u32));
        fdt.extend_from_slice(&be32(strings_offset as u32));
        fdt.extend_from_slice(&be32(header_size as u32));
        fdt.extend_from_slice(&be32(17));
        fdt.extend_from_slice(&be32(16));
        fdt.extend_from_slice(&be32(0));
        fdt.extend_from_slice(&be32(strings.len() as u32));
        fdt.extend_from_slice(&be32(structure.len() as u32));
        fdt.extend_from_slice(&[0_u8; 16]);
        fdt.extend_from_slice(&structure);
        fdt.extend_from_slice(&strings);
        assert_eq!(fdt.len(), total_size);
        fdt
    }

    #[test]
    fn for_each_mmc_host_parses_dw_mshc_and_jh7110() {
        let fdt = build_fdt_with_mmc_hosts();
        let blob = FdtBlob::from_bytes(&fdt).expect("valid FDT blob");
        let tree = DeviceTree::from_blob(&blob).expect("parseable device tree");

        let mut hosts = Vec::new();
        tree.for_each_mmc_host(|host| hosts.push(host))
            .expect("mmc host parse");

        // sdio0 is disabled -> sdio1 (snps,dw-mshc) and sdio2
        // (starfive,jh7110-mmc) are discovered; both compatibles accepted.
        assert_eq!(hosts.len(), 2);
        let sdio1 = hosts[0];
        assert_eq!(sdio1.alias_index(), Some(1));
        assert_eq!(sdio1.base(), 0x1602_0000);
        assert_eq!(sdio1.size(), 0x1_0000);
        assert_eq!(sdio1.bus_width(), 4);
        assert_eq!(sdio1.fifo_depth(), Some(32));
        assert!(!sdio1.non_removable());

        let sdio2 = hosts[1];
        assert_eq!(sdio2.alias_index(), Some(2));
        assert_eq!(sdio2.base(), 0x1603_0000);
        assert_eq!(sdio2.bus_width(), 4);
        assert!(!sdio2.non_removable());
    }
}
