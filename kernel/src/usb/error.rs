//! 纯 Rust USB2 Host 驱动的统一错误类型。
//!
//! 从 EHCI 事务错误 / 控制器状态 / SCSI 返回码映射到单一 `UsbError`，让
//! 上层（枚举、MSC、块设备）只处理一种错误语义。所有错误都是可恢复的——
//! 驱动绝不 panic，超时/STALL/掉线都向上返回错误。

use core::fmt;

/// USB 驱动错误。
///
/// `dead_code`：本提交（RUSB-1）只构造 `OutOfMemory`/`InvalidState`；其余
/// 变体由后续提交（EHCI 传输/枚举/MSC）按序投入使用，先声明完整语义。
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsbError {
    /// 控制器或设备不存在 / 未就绪。
    NotPresent,
    /// 传输超时（控制器未在预算内完成）。
    Timeout,
    /// 端点返回 STALL。
    Stall,
    /// EHCI 事务错误（XactErr）。
    TransactionError,
    /// 设备发送超过请求长度的数据（Babble）。
    Babble,
    /// 数据缓冲错误（DataBufferErr）。
    DataBufferError,
    /// CRC 错误。
    CrcError,
    /// 端点/控制器 halted。
    Halted,
    /// DMA / 内存分配失败。
    OutOfMemory,
    /// 无效状态或调用顺序错误。
    InvalidState,
    /// USB 描述符解析失败（长度越界 / 结构非法）。
    DescriptorError,
    /// 通用传输失败。
    TransferFailed,
    /// 设备已拔出 / 离线。
    DeviceOffline,
    /// 设备未配置 / 未找到所需端点。
    NotConfigured,
}

impl fmt::Display for UsbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::NotPresent => "usb device not present",
            Self::Timeout => "usb transfer timeout",
            Self::Stall => "usb endpoint stall",
            Self::TransactionError => "usb transaction error",
            Self::Babble => "usb babble",
            Self::DataBufferError => "usb data buffer error",
            Self::CrcError => "usb crc error",
            Self::Halted => "usb endpoint halted",
            Self::OutOfMemory => "usb out of memory",
            Self::InvalidState => "usb invalid state",
            Self::DescriptorError => "usb descriptor error",
            Self::TransferFailed => "usb transfer failed",
            Self::DeviceOffline => "usb device offline",
            Self::NotConfigured => "usb device not configured",
        };
        f.write_str(text)
    }
}
