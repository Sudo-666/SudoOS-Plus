//! EHCI root 集线器端口操作：PORTSC 供电/复位/连接/速度检测。
//!
//! 端口索引 0-based（`op::portsc` 已处理 0x44 + 4n）。写 PORTSC 前一律
//! 屏蔽 W1C change 位（CSC/PEC/OCC/RESUME），避免误清连接/使能/过流事件
//! （M2.8 真机教训）。

use super::regs::{HcRegs, op, port};
use crate::usb::error::UsbError;
use crate::usb::platform::ls2k1000::busy_delay_ms;

/// 端口速度（EHCI 2.0 `PORTSC[15:13]`）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortSpeed {
    Full,
    Low,
    High,
}

/// 端口供电：写 `PP`（屏蔽 W1C change 位）。
pub fn port_power(regs: &HcRegs, port_index: usize) {
    let offset = op::portsc(port_index);
    let value = (regs.read32(offset) & !port::W1C_CHANGE) | port::PP;
    regs.write32(offset, value);
}

/// 端口是否连接（`CCS`）。
pub fn port_connected(regs: &HcRegs, port_index: usize) -> bool {
    regs.read32(op::portsc(port_index)) & port::CCS != 0
}

/// 端口是否启用（`PE`）。
///
/// `dead_code`：RUSB-4 端口复位后等待 PE 用。
#[allow(dead_code)]
pub fn port_enabled(regs: &HcRegs, port_index: usize) -> bool {
    regs.read32(op::portsc(port_index)) & port::PE != 0
}

/// 端口速度（EHCI 2.0 `PORTSC[15:13]`）。
///
/// `dead_code`：RUSB-4 枚举选 ep0 maxpacket 用。
#[allow(dead_code)]
pub fn port_speed(regs: &HcRegs, port_index: usize) -> Result<PortSpeed, UsbError> {
    let value = regs.read32(op::portsc(port_index));
    match (value & port::PORTSPD_MASK) >> port::PORTSPD_SHIFT {
        0 => Ok(PortSpeed::Full),
        1 => Ok(PortSpeed::Low),
        2 => Ok(PortSpeed::High),
        _ => Err(UsbError::InvalidState),
    }
}

/// 端口复位：挂起先 resume → 置 `RESET` 50ms → 清 `RESET` → 等 `PE`
/// （EHCI §4.2.3 端口复位；C 驱动 `usbh_reset_port` S1/S3 序列）。
///
/// `dead_code`：RUSB-4 枚举对已连接端口调用。
#[allow(dead_code)]
pub fn reset_port(regs: &HcRegs, port_index: usize) -> Result<(), UsbError> {
    let offset = op::portsc(port_index);
    let mut value = regs.read32(offset) & !port::W1C_CHANGE;

    // 复位前若挂起（U-Boot 停止控制器时的状态），先 resume 再复位。
    if value & port::SUSPEND != 0 {
        let mut resume = value & !port::W1C_CHANGE;
        resume |= port::RESUME;
        regs.write32(offset, resume);
        busy_delay_ms(20);
        let mut done = regs.read32(offset) & !port::W1C_CHANGE;
        done &= !port::RESUME;
        regs.write32(offset, done);
        busy_delay_ms(5);
        value = regs.read32(offset) & !port::W1C_CHANGE;
    }

    // 置端口复位。
    value &= !port::PE;
    value |= port::RESET;
    regs.write32(offset, value);
    busy_delay_ms(50);
    let mut clear = regs.read32(offset) & !port::W1C_CHANGE;
    clear &= !port::RESET;
    regs.write32(offset, clear);

    // 等端口重新使能（复位完成后 PE=1）。
    let start = crate::time::now();
    let wait = core::time::Duration::from_millis(250);
    loop {
        if regs.read32(offset) & port::PE != 0 {
            return Ok(());
        }
        if crate::time::now().duration_since(start) >= wait {
            return Err(UsbError::Timeout);
        }
        busy_delay_ms(2);
    }
}
