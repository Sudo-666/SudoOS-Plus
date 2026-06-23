//! Linux 风格的 /sys 文件系统。
//!
//! 暴露内核对象模型（设备、类别、内核参数）为文件系统层次结构。
//! 复用 procfs 的 `ProcFileGenerator` trait 实现动态文件生成。

use alloc::{format, string::String, sync::Arc, vec, vec::Vec};

use myos_vfs::Errno;

use crate::procfs::ProcFileGenerator;

// ---------------------------------------------------------------------------
// /sys/ 根目录
// ---------------------------------------------------------------------------

pub fn root_entries() -> Vec<(&'static str, Arc<dyn ProcFileGenerator>)> {
    vec![
        ("kernel", Arc::new(KernelDir)),
        ("devices", Arc::new(DevicesDir)),
        ("class", Arc::new(ClassDir)),
    ]
}

// ---------------------------------------------------------------------------
// /sys/kernel/ — 动态目录
// ---------------------------------------------------------------------------

struct KernelDir;

impl ProcFileGenerator for KernelDir {
    fn generate(&self) -> Result<Vec<u8>, Errno> {
        Ok(Vec::new()) // 目录不返回内容
    }
}

/// /sys/kernel/version
struct KernelVersionFile;

impl ProcFileGenerator for KernelVersionFile {
    fn generate(&self) -> Result<Vec<u8>, Errno> {
        Ok(b"SudoOS\n".to_vec())
    }
}

/// /sys/kernel/ostype
struct KernelOsTypeFile;

impl ProcFileGenerator for KernelOsTypeFile {
    fn generate(&self) -> Result<Vec<u8>, Errno> {
        let arch = crate::arch::ARCH_NAME;
        Ok(format!("SudoOS ({arch})\n").into_bytes())
    }
}

pub fn kernel_entries(
) -> Vec<(&'static str, Arc<dyn ProcFileGenerator>)> {
    vec![
        ("version", Arc::new(KernelVersionFile)),
        ("ostype", Arc::new(KernelOsTypeFile)),
    ]
}

// ---------------------------------------------------------------------------
// /sys/devices/ — 已注册设备列表
// ---------------------------------------------------------------------------

struct DevicesDir;

impl ProcFileGenerator for DevicesDir {
    fn generate(&self) -> Result<Vec<u8>, Errno> {
        Ok(Vec::new())
    }
}

struct DeviceListFile;

impl ProcFileGenerator for DeviceListFile {
    fn generate(&self) -> Result<Vec<u8>, Errno> {
        let mut output = String::new();
        crate::device::for_each_device(|device| {
            output.push_str(&format!(
                "{} type={} mmio={:?}\n",
                device.name,
                device.device_type.name(),
                device.resources.mmio_base,
            ));
        });
        Ok(output.into_bytes())
    }
}

pub fn devices_entries(
) -> Vec<(&'static str, Arc<dyn ProcFileGenerator>)> {
    vec![("list", Arc::new(DeviceListFile))]
}

// ---------------------------------------------------------------------------
// /sys/class/ — 设备类别
// ---------------------------------------------------------------------------

struct ClassDir;

impl ProcFileGenerator for ClassDir {
    fn generate(&self) -> Result<Vec<u8>, Errno> {
        Ok(Vec::new())
    }
}

struct BlockClassFile;

impl ProcFileGenerator for BlockClassFile {
    fn generate(&self) -> Result<Vec<u8>, Errno> {
        let mut output = String::new();
        let devices = crate::device::find_devices_by_type(crate::device::DeviceType::Block);
        for device in &devices {
            output.push_str(&format!("{}\n", device.name));
        }
        if devices.is_empty() {
            // 从 block registry 中查找
            output.push_str("vda\n");
        }
        Ok(output.into_bytes())
    }
}

struct NetClassFile;

impl ProcFileGenerator for NetClassFile {
    fn generate(&self) -> Result<Vec<u8>, Errno> {
        let mut output = String::new();
        let devices = crate::device::find_devices_by_type(crate::device::DeviceType::Net);
        for device in &devices {
            output.push_str(&format!("{}\n", device.name));
        }
        Ok(output.into_bytes())
    }
}

pub fn class_entries(
) -> Vec<(&'static str, Arc<dyn ProcFileGenerator>)> {
    vec![
        ("block", Arc::new(BlockClassFile)),
        ("net", Arc::new(NetClassFile)),
    ]
}
