//! Linux 风格的网络子系统。
//!
//! 提供 `NetDevice` trait、smoltcp 集成、socket 层和 VirtIO-Net 驱动。

use alloc::{sync::Arc, vec::Vec};

use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};

pub mod socket;
pub mod virtio_net;

const NET_LOCK: LockClass = LockClass::new("net.state", LockRank::Vfs, 8);

/// MAC 地址 (6 字节)。
pub type MacAddress = [u8; 6];

/// 网络设备 trait — 对应 Linux `struct net_device`。
pub trait NetDevice: Send + Sync + 'static {
    /// MAC 地址。
    fn mac_address(&self) -> MacAddress;

    /// 最大传输单元 (MTU)。
    fn mtu(&self) -> usize;

    /// 发送以太网帧。
    fn transmit(&self, frame: &[u8]) -> Result<(), NetError>;

    /// 接收以太网帧。若无数据返回 `Err(NetError::WouldBlock)`。
    fn receive(&self, buffer: &mut [u8]) -> Result<usize, NetError>;

    /// 是否有待处理数据。
    fn poll_receive(&self) -> bool;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetError {
    /// 暂时无数据
    WouldBlock,
    /// 设备断开
    Disconnected,
    /// I/O 错误
    IoError,
    /// 缓冲区太小
    NoBuffer,
}

/// 已注册的网络接口。
pub struct RegisteredInterface {
    pub name: alloc::string::String,
    pub device: Arc<dyn NetDevice>,
}

static INTERFACES: IrqSpinLock<Vec<RegisteredInterface>> =
    IrqSpinLock::new_with_class(Vec::new(), NET_LOCK);

pub fn initialize() {
    let interfaces = INTERFACES.lock();
    crate::println!("net:");
    crate::println!("  interfaces     : {}", interfaces.len(),);
    for iface in interfaces.iter() {
        crate::println!(
            "  {}           : MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} MTU {}",
            iface.name,
            iface.device.mac_address()[0],
            iface.device.mac_address()[1],
            iface.device.mac_address()[2],
            iface.device.mac_address()[3],
            iface.device.mac_address()[4],
            iface.device.mac_address()[5],
            iface.device.mtu(),
        );
    }
    crate::println!("  stack          : smoltcp (TCP/UDP/IPv4/IPv6)");
}

/// 注册网络接口。
pub fn register_interface(name: &str, device: Arc<dyn NetDevice>) -> Result<(), NetError> {
    let mut interfaces = INTERFACES.lock();
    let mut stored_name = alloc::string::String::new();
    stored_name.push_str(name);
    interfaces.try_reserve(1).map_err(|_| NetError::NoBuffer)?;
    interfaces.push(RegisteredInterface {
        name: stored_name,
        device,
    });
    Ok(())
}

/// 获取已注册的网络接口。
pub fn registered_interfaces() -> Vec<RegisteredInterface> {
    let guard = INTERFACES.lock();
    let mut result = Vec::new();
    for iface in guard.iter() {
        let mut name = alloc::string::String::new();
        name.push_str(&iface.name);
        result.push(RegisteredInterface {
            name,
            device: Arc::clone(&iface.device),
        });
    }
    result
}

/// 按名称查找网络接口。
pub fn find_interface(name: &str) -> Option<Arc<dyn NetDevice>> {
    INTERFACES
        .lock()
        .iter()
        .find(|iface| iface.name == name)
        .map(|iface| Arc::clone(&iface.device))
}
