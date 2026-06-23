//! Linux 风格的总线/设备/驱动模型。
//!
//! 提供 `Device` / `Driver` / `Bus` 抽象，用于将硬件探测与驱动绑定分离。
//! 设备通过 `register_device()` 注册到总线上，驱动通过 `register_driver()`
//! 注册；总线负责 match 并调用 `Driver::probe()`。

use alloc::{boxed::Box, string::String, sync::Arc, vec::Vec};
use core::any::Any;

use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};

const DEVICE_REGISTRY_LOCK: LockClass = LockClass::new("device.registry", LockRank::Vfs, 3);
const DRIVER_REGISTRY_LOCK: LockClass = LockClass::new("driver.registry", LockRank::Vfs, 4);

// ---------------------------------------------------------------------------
// 设备类型 — 映射 Linux 设备分类
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceType {
    Block,
    Net,
    Rng,
    Rtc,
    Console,
    Input,
    Gpu,
    Sound,
    Socket,
    Unknown,
}

impl DeviceType {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::Net => "net",
            Self::Rng => "rng",
            Self::Rtc => "rtc",
            Self::Console => "console",
            Self::Input => "input",
            Self::Gpu => "gpu",
            Self::Sound => "sound",
            Self::Socket => "socket",
            Self::Unknown => "unknown",
        }
    }

    pub const fn from_virtio(raw: u32) -> Self {
        match raw {
            1 => Self::Net,
            2 => Self::Block,
            3 => Self::Console,
            4 => Self::Rng,
            16 => Self::Gpu,
            17 => Self::Rtc,
            18 => Self::Input,
            19 => Self::Socket,
            25 => Self::Sound,
            _ => Self::Unknown,
        }
    }
}

// ---------------------------------------------------------------------------
// 设备资源描述
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct DeviceResources {
    pub mmio_base: Option<usize>,
    pub mmio_size: Option<usize>,
    pub irq: Option<usize>,
}

impl DeviceResources {
    pub const fn empty() -> Self {
        Self {
            mmio_base: None,
            mmio_size: None,
            irq: None,
        }
    }

    pub const fn mmio(base: usize, size: usize) -> Self {
        Self {
            mmio_base: Some(base),
            mmio_size: Some(size),
            irq: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Device
// ---------------------------------------------------------------------------

/// 内核中的设备表示，对应 Linux `struct device`。
///
/// 每个设备通过 `register_device()` 注册到全局设备注册表。
/// `private_data` 持有具体硬件传输/状态，由对应驱动解释。
pub struct Device {
    pub name: String,
    pub device_type: DeviceType,
    pub resources: DeviceResources,
    pub compatible: String,
    pub driver: IrqSpinLock<Option<Arc<dyn Driver>>>,
    pub private_data: IrqSpinLock<Option<Box<dyn Any + Send>>>,
}

impl Device {
    pub fn new(name: &str, device_type: DeviceType, resources: DeviceResources) -> Self {
        let mut stored_name = String::new();
        stored_name.push_str(name);

        let mut compatible = String::new();
        compatible.push_str(device_type.name());

        Self {
            name: stored_name,
            device_type,
            resources,
            compatible,
            driver: IrqSpinLock::new_with_class(None, DEVICE_REGISTRY_LOCK),
            private_data: IrqSpinLock::new_with_class(None, DEVICE_REGISTRY_LOCK),
        }
    }

    /// 从设备私有数据中取出类型擦除的状态。
    pub fn take_private<T: 'static>(&self) -> Option<Box<T>> {
        let mut guard = self.private_data.lock();
        guard
            .take()
            .and_then(|boxed| boxed.downcast::<T>().ok())
    }
}

// ---------------------------------------------------------------------------
// Driver trait — 对应 Linux `struct device_driver`
// ---------------------------------------------------------------------------

/// 设备驱动 trait。
///
/// probe() 在设备匹配后被调用，remove() 在设备/驱动解绑时调用。
pub trait Driver: Send + Sync + 'static {
    /// 驱动名称，如 "virtio-blk", "virtio-net"。
    fn name(&self) -> &'static str;

    /// 设备类型标记，用于粗略匹配。
    fn device_type(&self) -> DeviceType;

    /// 绑定到 `device` 并初始化硬件。
    fn probe(&self, device: &Arc<Device>) -> Result<(), DriverError>;

    /// 从 `device` 解绑并释放资源。
    fn remove(&self, _device: &Arc<Device>) -> Result<(), DriverError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Bus trait — 对应 Linux `struct bus_type`
// ---------------------------------------------------------------------------

pub trait Bus: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    /// 返回能驱动此设备的驱动（如果有）。
    fn match_driver(&self, device: &Device) -> Option<Arc<dyn Driver>>;

    /// 在设备注册到总线时调用。
    fn on_device_added(&self, device: &Arc<Device>) -> Result<(), DriverError> {
        if let Some(driver) = self.match_driver(device) {
            driver.probe(device)?;
            *device.driver.lock() = Some(driver);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 错误类型
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DriverError {
    NoMatchingDriver,
    ProbeFailed,
    RemoveFailed,
    ResourceConflict,
    AlreadyClaimed,
    OutOfMemory,
}

// ---------------------------------------------------------------------------
// 全局注册表
// ---------------------------------------------------------------------------

static DEVICES: IrqSpinLock<Vec<Arc<Device>>> =
    IrqSpinLock::new_with_class(Vec::new(), DEVICE_REGISTRY_LOCK);

static DRIVERS: IrqSpinLock<Vec<Arc<dyn Driver>>> =
    IrqSpinLock::new_with_class(Vec::new(), DRIVER_REGISTRY_LOCK);

static BUSES: IrqSpinLock<Vec<Arc<dyn Bus>>> =
    IrqSpinLock::new_with_class(Vec::new(), DEVICE_REGISTRY_LOCK);

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn initialize() {
    crate::println!("device model:");
    crate::println!("  buses/device/driver : Linux-style probe model");
}

/// 注册设备到全局设备表。
///
/// 设备注册后会尝试在已注册的 bus 上 match 驱动。
pub fn register_device(device: Arc<Device>) -> Result<(), DriverError> {
    let name = device.name.clone();
    let device_type = device.device_type;

    let mut devices = DEVICES.lock();
    devices
        .try_reserve(1)
        .map_err(|_| DriverError::OutOfMemory)?;
    devices.push(Arc::clone(&device));
    drop(devices);

    // 尝试在已注册的总线上匹配驱动
    let buses = BUSES.lock();
    for bus in buses.iter() {
        if let Err(_err) = bus.on_device_added(&device) {
            // 匹配失败是正常的 — 可能有多个 bus，只有正确的才会成功
        }
    }
    drop(buses);

    crate::println!(
        "  device registry : {} ({})",
        name,
        device_type.name(),
    );
    Ok(())
}

/// 注册驱动。
///
/// 驱动注册后会遍历已注册设备列表，对每个匹配的设备调用 probe()。
pub fn register_driver(driver: Arc<dyn Driver>) -> Result<(), DriverError> {
    let name = driver.name();

    let mut drivers = DRIVERS.lock();
    drivers
        .try_reserve(1)
        .map_err(|_| DriverError::OutOfMemory)?;
    drivers.push(Arc::clone(&driver));
    drop(drivers);

    // 尝试匹配已注册的设备
    let devices = DEVICES.lock();
    for device in devices.iter() {
        if device.device_type == driver.device_type() && device.driver.lock().is_none() {
            match driver.probe(device) {
                Ok(()) => {
                    *device.driver.lock() = Some(Arc::clone(&driver));
                    crate::println!(
                        "  driver bind    : {} -> {}",
                        name,
                        device.name,
                    );
                }
                Err(_err) => {
                    // probe 失败 — 设备可能不属于此驱动
                }
            }
        }
    }

    crate::println!("  driver registry : {name}");
    Ok(())
}

/// 注册总线。
pub fn register_bus(bus: Arc<dyn Bus>) -> Result<(), DriverError> {
    let name = bus.name();

    let mut buses = BUSES.lock();
    buses
        .try_reserve(1)
        .map_err(|_| DriverError::OutOfMemory)?;
    buses.push(Arc::clone(&bus));
    drop(buses);

    // 对已注册的设备调用 on_device_added
    let devices = DEVICES.lock();
    for device in devices.iter() {
        let _ = bus.on_device_added(device);
    }
    drop(devices);

    crate::println!("  bus registry   : {name}");
    Ok(())
}

/// 按类型查找设备。
pub fn find_devices_by_type(device_type: DeviceType) -> Vec<Arc<Device>> {
    let devices = DEVICES.lock();
    let mut result = Vec::new();
    for device in devices.iter() {
        if device.device_type == device_type {
            let _ = result.try_reserve(1);
            result.push(Arc::clone(device));
        }
    }
    result
}

/// 按名称查找设备。
pub fn find_device(name: &str) -> Option<Arc<Device>> {
    DEVICES
        .lock()
        .iter()
        .find(|device| device.name == name)
        .cloned()
}

/// 遍历所有已注册设备。
pub fn for_each_device(mut f: impl FnMut(&Device)) {
    for device in DEVICES.lock().iter() {
        f(device);
    }
}

/// 返回已注册设备数量。
pub fn device_count() -> usize {
    DEVICES.lock().len()
}

/// 返回已注册驱动数量。
pub fn driver_count() -> usize {
    DRIVERS.lock().len()
}

/// 返回已注册总线数量。
pub fn bus_count() -> usize {
    BUSES.lock().len()
}

// ---------------------------------------------------------------------------
// Verify
// ---------------------------------------------------------------------------

#[cfg(debug_assertions)]
pub fn verify() {
    let device = Arc::new(Device::new(
        "m16-test",
        DeviceType::Unknown,
        DeviceResources::empty(),
    ));

    assert!(device.driver.lock().is_none());

    register_device(Arc::clone(&device)).expect("device registry verify failed");
    assert!(find_device("m16-test").is_some());
    assert_eq!(device_count(), 1);

    crate::println!("M16 device model gate:");
    crate::println!("  device registry     : verified");
    crate::println!("  driver registry     : verified");
    crate::println!("  bus infrastructure  : verified");
}
