use core::str;

use fdt_parser::{
    Fdt,
    helpers::UnalignedInfallibleNode,
    parsing::{Panic, unaligned::UnalignedParser},
    properties::Compatible,
};

use crate::{FdtBlob, FdtError, MemoryRegion, PciHostBridge, VirtioMmioRegion};

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
        self.cpu_hardware_ids().count()
    }

    /// Hardware CPU/thread identifiers from available `/cpus/cpu@*` nodes.
    ///
    /// CPUs marked `disabled` are retained because the operating system may
    /// start them through an architecture-specific enable mechanism. Nodes
    /// marked `fail` are never exposed to the SMP layer.
    pub fn cpu_hardware_ids(&self) -> impl Iterator<Item = usize> + '_ {
        self.inner.root().cpus().iter().filter_map(|cpu| {
            if cpu.status().is_some_and(|status| status.is_failed()) {
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
    pub fn memory_regions(&self) -> impl Iterator<Item = MemoryRegion> + '_ {
        self.inner
            .root()
            .memory()
            .reg()
            .iter::<u64, u64>()
            .filter_map(|entry| {
                let entry = entry.ok()?;

                let start = usize::try_from(entry.address).ok()?;

                let size = usize::try_from(entry.len).ok()?;

                Some(MemoryRegion::new(start, size))
            })
            .filter(|region| !region.is_empty())
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
    use super::property_u64;

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
}
