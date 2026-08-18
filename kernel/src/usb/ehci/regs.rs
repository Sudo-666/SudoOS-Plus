//! EHCI 能力寄存器（HCCR）与操作寄存器（HCOR）的 volatile 访问。
//!
//! 偏移是标准 EHCI 1.0/2.0（与 vendored `usb_ehci.h` 同源）：
//! - 能力：`caplength@0x00`(8)、`hciversion@0x02`(16)、`hcsparams@0x04`、
//!   `hccparams@0x08`
//! - 操作：`usbcmd@0x00`、`usbsts@0x04`、`usbintr@0x08`、`frindex@0x0c`、
//!   `ctrldssegment@0x10`、`periodiclistbase@0x14`、`asynclistaddr@0x18`、
//!   `reserved[9]@0x1c-0x3f`、`configflag@0x40`、`portsc[n]@0x44 + 4n`
//!
//! LS2K1000 经 uncached `0x8000...` 窗口访问（LoongArch uncached DMW 本已
//! 强序）；写后仍执行 `dbar 0`，保证设备侧在 CPU 读回状态前观察到写（同
//! `arch/loongarch64/src/memory/paging/hardware.rs` 的用法）。

use core::ptr;

use super::super::platform::ls2k1000::EHCI_MMIO_UNCACHED;

/// 写后内存屏障（`dbar 0`）。
pub fn dbar() {
    // SAFETY: `dbar 0` 无操作数、无内存副作用，`options(nostack)` 内联。
    unsafe { core::arch::asm!("dbar 0", options(nostack)) };
}

/// 能力寄存器偏移（相对 MMIO 基址）。
pub mod cap {
    /// Core Capability Register Length（8 位）。
    pub const CAPLENGTH: usize = 0x00;
    /// Core Interface Version（16 位）。
    pub const HCIVERSION: usize = 0x02;
    /// Core Structural Parameters。
    pub const HCSPARAMS: usize = 0x04;
    /// Core Capability Parameters。
    pub const HCCPARAMS: usize = 0x08;
}

/// 操作寄存器偏移（相对 MMIO 基址）。
pub mod op {
    pub const USBCMD: usize = 0x00;
    pub const USBSTS: usize = 0x04;
    pub const USBINTR: usize = 0x08;
    pub const CTRLDSSEGMENT: usize = 0x10;
    pub const PERIODICLISTBASE: usize = 0x14;
    pub const ASYNCLISTADDR: usize = 0x18;
    pub const CONFIGFLAG: usize = 0x40;

    /// 端口状态寄存器（端口 0-based）。
    pub const fn portsc(port: usize) -> usize {
        0x44 + port * 4
    }
}

/// USBCMD 位字段。
pub mod cmd {
    pub const RUN: u32 = 1 << 0;
    pub const HCRESET: u32 = 1 << 1;
    pub const FLSIZE_MASK: u32 = 3 << 2;
    pub const FLSIZE_1024: u32 = 0 << 2;
    pub const PSEN: u32 = 1 << 4;
    pub const ASEN: u32 = 1 << 5;
    pub const IAADB: u32 = 1 << 6;
}

/// USBSTS 位字段。
pub mod sts {
    pub const INT: u32 = 1 << 0;
    pub const ERR: u32 = 1 << 1;
    pub const PORT_CHANGE: u32 = 1 << 2;
    pub const FRAME_LIST_ROLLOVER: u32 = 1 << 3;
    pub const HOST_SYS_ERR: u32 = 1 << 4;
    pub const IAA: u32 = 1 << 5;
    /// W1C 写 1 清的位（清状态时原样写回这几位）。
    pub const W1C: u32 = INT | ERR | PORT_CHANGE | FRAME_LIST_ROLLOVER | HOST_SYS_ERR | IAA;
    pub const HALTED: u32 = 1 << 12;
    pub const ASS: u32 = 1 << 15;
}

/// USBINTR 位字段（`EHCI_HANDLED_INTS`=0x37，C 驱动不使能 HSE 位）。
pub mod intr {
    pub const ALL: u32 = 0x37;
}

/// HCSPARAMS 位字段。
pub mod hcsp {
    pub const NPORTS_SHIFT: u32 = 0;
    pub const NPORTS_MASK: u32 = 0xf;
}

/// PORTSC 位字段（EHCI 2.0；端口速度在 [15:13]，M2.7 真机修正）。
pub mod port {
    pub const CCS: u32 = 1 << 0;
    pub const CSC: u32 = 1 << 1;
    pub const PE: u32 = 1 << 2;
    pub const PEC: u32 = 1 << 3;
    pub const OCC: u32 = 1 << 5;
    pub const RESUME: u32 = 1 << 6;
    pub const SUSPEND: u32 = 1 << 7;
    pub const RESET: u32 = 1 << 8;
    pub const PP: u32 = 1 << 12;
    pub const PORTSPD_SHIFT: u32 = 13;
    pub const PORTSPD_MASK: u32 = 7 << 13;
    /// 写回 PORTSC 时必须屏蔽的 W1C change 位，避免误清连接/使能/过流事件。
    pub const W1C_CHANGE: u32 = CSC | PEC | OCC | RESUME;
}

/// EHCI 寄存器文件。只经 uncached 窗口访问。
///
/// 基址存 `usize` 而非裸指针，使结构可放进 `static`（裸指针非 Sync）。
#[derive(Clone, Copy)]
pub struct HcRegs {
    /// uncached MMIO 虚拟基址。
    base: usize,
}

impl HcRegs {
    pub const fn new(uncached_base: usize) -> Self {
        Self {
            base: uncached_base,
        }
    }

    /// LS2K1000 默认 EHCI 控制器。
    pub const fn ls2k1000() -> Self {
        Self::new(EHCI_MMIO_UNCACHED)
    }

    /// 32 位读。
    ///
    /// # Safety
    /// 调用方保证 `offset` 4 对齐、落在能力/操作寄存器有效范围。
    pub fn read32(&self, offset: usize) -> u32 {
        // SAFETY: 调用方保证 offset 对齐且在 MMIO 寄存器区（见函数文档）。
        unsafe { ptr::read_volatile((self.base as *mut u32).add(offset / 4)) }
    }

    /// 32 位写（写后 `dbar`）。
    pub fn write32(&self, offset: usize, value: u32) {
        // SAFETY: 同 `read32`。
        unsafe { ptr::write_volatile((self.base as *mut u32).add(offset / 4), value) };
        dbar();
    }

    /// 16 位读（`hciversion@0x02`）。
    pub fn read16(&self, offset: usize) -> u16 {
        // SAFETY: offset 2 对齐且落在能力寄存器区（基址 0x40060000 为偶地址）。
        unsafe { ptr::read_volatile((self.base as *mut u8).add(offset).cast()) }
    }

    /// 8 位读（`caplength@0x00`）。
    pub fn read8(&self, offset: usize) -> u8 {
        // SAFETY: offset 落在能力寄存器区。
        unsafe { ptr::read_volatile((self.base as *mut u8).add(offset)) }
    }

    /// 能力寄存器长度。
    pub fn caplength(&self) -> u8 {
        self.read8(cap::CAPLENGTH)
    }

    /// 控制器接口版本。
    pub fn hciversion(&self) -> u16 {
        self.read16(cap::HCIVERSION)
    }

    /// HCSPARAMS 报告端口数。
    pub fn nports(&self) -> usize {
        ((self.read32(cap::HCSPARAMS) >> hcsp::NPORTS_SHIFT) & hcsp::NPORTS_MASK) as usize
    }
}
