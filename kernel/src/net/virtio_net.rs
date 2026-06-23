//! VirtIO-Net 网络设备驱动。
//!
//! 封装 vendor `VirtIONetRaw`，实现 `NetDevice` trait。
//! 通过 `from_raw()` 工厂函数创建，支持 MMIO 和 PCI 传输。

use alloc::sync::Arc;

use virtio_drivers::{device::net::VirtIONetRaw, transport::Transport};

use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};

use super::{MacAddress, NetDevice, NetError};

pub const NET_QUEUE_SIZE: usize = 64;
const NET_LOCK: LockClass = LockClass::new("net.virtio", LockRank::Vfs, 11);
const RX_BUFFER_SIZE: usize = 2048;

/// VirtIO-Net 设备适配器。
///
/// 通过 `IrqSpinLock` 包装 `VirtIONetRaw` 并实现 `NetDevice` trait。
pub struct VirtioNetDevice<T: Transport + Send + 'static> {
    inner: IrqSpinLock<VirtIONetRaw<crate::virtio::SudoHal, T, NET_QUEUE_SIZE>>,
    mac: MacAddress,
    mtu: usize,
    rx_buffer: IrqSpinLock<[u8; RX_BUFFER_SIZE]>,
    _mapping: Option<crate::vm::KernelIoMapping>,
}

/// 从已初始化的 `VirtIONetRaw` 创建 `NetDevice`。
///
/// 调用者在外部执行 `VirtIONetRaw::new(transport)` 并将结果传入。
pub fn from_raw<T: Transport + Send + 'static>(
    driver: VirtIONetRaw<crate::virtio::SudoHal, T, NET_QUEUE_SIZE>,
    mapping: Option<crate::vm::KernelIoMapping>,
) -> Arc<dyn NetDevice> {
    let mac = driver.mac_address();
    Arc::new(VirtioNetDevice {
        inner: IrqSpinLock::new_with_class(driver, NET_LOCK),
        mac,
        mtu: 1500,
        rx_buffer: IrqSpinLock::new_with_class([0; RX_BUFFER_SIZE], NET_LOCK),
        _mapping: mapping,
    })
}

impl<T: Transport + Send + 'static> NetDevice for VirtioNetDevice<T> {
    fn mac_address(&self) -> MacAddress {
        self.mac
    }

    fn mtu(&self) -> usize {
        self.mtu
    }

    fn transmit(&self, frame: &[u8]) -> Result<(), NetError> {
        let mut driver = self.inner.lock();
        driver.send(frame).map_err(|_| NetError::IoError)
    }

    fn receive(&self, buffer: &mut [u8]) -> Result<usize, NetError> {
        let mut driver = self.inner.lock();

        if driver.poll_receive().is_none() {
            return Err(NetError::WouldBlock);
        }

        let mut rx_buf = self.rx_buffer.lock();
        match driver.receive_wait(&mut *rx_buf) {
            Ok((_header_len, packet_len)) => {
                let copy_len = packet_len.min(buffer.len());
                buffer[..copy_len].copy_from_slice(&rx_buf[..copy_len]);
                Ok(copy_len)
            }
            Err(_) => Err(NetError::IoError),
        }
    }

    fn poll_receive(&self) -> bool {
        self.inner.lock().poll_receive().is_some()
    }
}
