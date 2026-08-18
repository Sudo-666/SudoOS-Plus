//! EHCI 硬件队列头 QH / 队列元素传输描述符 qTD，与 DMA 链接编码。
//!
//! 硬件字段布局与 vendored `usb_ehci.h` 的 `ehci_qh_s`/`ehci_qtd_s` 相同
//! （QH=48B、qTD=32B）。EHCI 要求 QH/qTD 物理 32B 对齐——M2.11 真机证实
//! 非 32 对齐会在控制传输提交后卡死 IOC——因此槽位分配器按 32B 对齐下发
//! 偏移，QH（48B）的下一个槽位自然跳到 64B 边界。
//!
//! 软件元数据与硬件结构分离：硬件结构只经 uncached 窗口写（`dma`）；
//! 软件元数据（QtdMeta）放普通缓存堆。本提交（RUSB-2）尚无传输，QtdMeta
//! 由 RUSB-3 引入。

use core::mem::{align_of, size_of};

use super::super::{dma::DmaRegion, error::UsbError, platform::ls2k1000::uncached_to_phys};

/// 水平链接指针类型：下一个节点是 QH。
pub const LINK_TYP_QH: u32 = 1 << 1;

/// 队列元素传输描述符 qTD（32B，32 对齐）。
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Qtd {
    /// 0x00 下一个 qTD 指针（T=1 终止）。
    pub nqp: u32,
    /// 0x04 备用下一个 qTD 指针。
    pub alt: u32,
    /// 0x08 令牌。
    pub token: u32,
    /// 0x0c 缓冲页指针表。
    pub bpl: [u32; 5],
}

impl Default for Qtd {
    fn default() -> Self {
        Self {
            nqp: 0,
            alt: 0,
            token: 0,
            bpl: [0; 5],
        }
    }
}

/// 队列头 QH（48B，32 对齐）。
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Qh {
    /// 0x00 水平链接指针。
    pub hlp: u32,
    /// 0x04 端点特性。
    pub epchar: u32,
    /// 0x08 端点能力。
    pub epcaps: u32,
    /// 0x0c 当前 qTD 指针。
    pub cqp: u32,
    /// 0x10 传输覆盖（与 qTD 同布局）。
    pub overlay: Qtd,
}

impl Default for Qh {
    fn default() -> Self {
        Self {
            hlp: 0,
            epchar: 0,
            epcaps: 0,
            cqp: 0,
            overlay: Qtd::default(),
        }
    }
}

// 布局编译期断言：尺寸/对齐必须匹配 EHCI 硬件要求（M2.11）。
const _: () = {
    assert!(size_of::<Qtd>() == 32, "EHCI qTD must be 32 bytes");
    assert!(size_of::<Qh>() == 48, "EHCI QH must be 48 bytes");
    assert!(
        align_of::<Qtd>() <= 32,
        "qTD natural alignment fits 32B slots"
    );
    assert!(
        align_of::<Qh>() <= 32,
        "QH natural alignment fits 32B slots"
    );
};

/// qTD 令牌位字段。
///
/// `dead_code`：RUSB-3 传输提交/完成解码时整组投入使用。
#[allow(dead_code)]
pub mod token {
    pub const ACTIVE: u32 = 1 << 7;
    pub const HALTED: u32 = 1 << 6;
    pub const DBERR: u32 = 1 << 5;
    pub const BABBLE: u32 = 1 << 4;
    pub const XACTERR: u32 = 1 << 3;
    pub const PID_SHIFT: u32 = 8;
    pub const PID_OUT: u32 = 0 << 8;
    pub const PID_IN: u32 = 1 << 8;
    pub const PID_SETUP: u32 = 2 << 8;
    pub const CERR_SHIFT: u32 = 10;
    pub const CERR_MASK: u32 = 3 << 10;
    pub const IOC: u32 = 1 << 15;
    pub const NBYTES_SHIFT: u32 = 16;
    pub const NBYTES_MASK: u32 = 0x7fff << 16;
    pub const TOGGLE: u32 = 1 << 31;
}

/// QH 端点特性（DWord 1）位字段。
///
/// `dead_code`：RUSB-3 建 control/bulk QH 时整组投入使用。
#[allow(dead_code)]
pub mod epchar {
    pub const DEVADDR_SHIFT: u32 = 0;
    pub const ENDPT_SHIFT: u32 = 8;
    pub const EPS_SHIFT: u32 = 12;
    pub const EPS_FULL: u32 = 0 << 12;
    pub const EPS_LOW: u32 = 1 << 12;
    pub const EPS_HIGH: u32 = 2 << 12;
    pub const DTC: u32 = 1 << 14;
    pub const H: u32 = 1 << 15;
    pub const MAXPKT_SHIFT: u32 = 16;
    pub const MAXPKT_MASK: u32 = 0x7ff << 16;
    pub const C: u32 = 1 << 27;
    pub const RL_SHIFT: u32 = 28;
    pub const RL_MASK: u32 = 0xf << 28;
}

/// 链接指针编码：QH 水平链接（`T=0` + `TYP=QH`）。
pub const fn link_qh(pa: u32) -> u32 {
    pa | LINK_TYP_QH
}

/// 链接指针编码：qTD 下一个指针（`T=0`）。
///
/// `dead_code`：RUSB-3 qTD 链构建时使用。
#[allow(dead_code)]
pub const fn link_qtd(pa: u32) -> u32 {
    pa
}

/// 链接指针编码：终止（`T=1`）。未用链接必须序列化为 `1`，不是 `0`
/// （`0` 会被控制器当物理地址 0 读）。
pub const fn link_terminate() -> u32 {
    1
}

/// 从 uncached 虚拟地址构造 qTD 链接值。
///
/// `dead_code`：RUSB-3 qTD 链构建时使用。
#[allow(dead_code)]
pub fn link_qtd_from_va(va: usize) -> u32 {
    link_qtd(uncached_to_phys(va))
}

/// 从 uncached 虚拟地址构造 QH 水平链接值。
///
/// `dead_code`：RUSB-3 建端点 QH 时使用。
#[allow(dead_code)]
pub fn link_qh_from_va(va: usize) -> u32 {
    link_qh(uncached_to_phys(va))
}

/// 描述符池：从 DMA 描述符区域按 32B 对齐切 QH/qTD 槽位。
///
/// 槽位在驱动生命周期内常驻（不释放；单设备/固定队列规模足够）。bump
/// 分配：QH 48B → 下一个槽位从 64B 边界开始，天然满足 32B 对齐（M2.11）。
pub struct DescPool {
    region: DmaRegion,
    /// 区域内已用字节数。
    cursor: usize,
}

impl DescPool {
    pub fn new(region: DmaRegion) -> Self {
        Self { region, cursor: 0 }
    }

    /// 分配 `bytes` 字节、32B 对齐的槽位。
    fn alloc(&mut self, bytes: usize) -> Result<DmaRegion, UsbError> {
        let region_start = self.region.as_usize();
        let start = align_up(region_start + self.cursor, 32);
        let stop = start.checked_add(bytes).ok_or(UsbError::OutOfMemory)?;
        let region_end = region_start + self.region.len();
        if stop > region_end {
            return Err(UsbError::OutOfMemory);
        }
        self.cursor = stop - region_start;
        Ok(DmaRegion::from_parts(start, uncached_to_phys(start), bytes))
    }

    /// 分配一个 QH 槽位。
    pub fn alloc_qh(&mut self) -> Result<DmaRegion, UsbError> {
        self.alloc(size_of::<Qh>())
    }

    /// 分配一个 qTD 槽位。
    ///
    /// `dead_code`：RUSB-3 传输 qTD 链时使用。
    #[allow(dead_code)]
    pub fn alloc_qtd(&mut self) -> Result<DmaRegion, UsbError> {
        self.alloc(size_of::<Qtd>())
    }
}

fn align_up(value: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    (value + align - 1) & !(align - 1)
}

/// RUSB-2 无硬件自检：硬件布局 / 链接编码 / 槽位 32B 对齐。
#[cfg(debug_assertions)]
pub fn verify() {
    assert_eq!(size_of::<Qtd>(), 32, "qTD size");
    assert_eq!(size_of::<Qh>(), 48, "QH size");

    // 链接编码。
    assert_eq!(link_terminate(), 0x1, "terminate link is T=1");
    assert_eq!(link_qh(0x1234_5678), 0x1234_5678 | LINK_TYP_QH, "QH link");
    assert_eq!(link_qtd(0x1234_5678), 0x1234_5678, "qTD link");

    // 槽位 32B 对齐：QH（48B）连续分配每个都对齐且不重叠。
    let fake = DmaRegion::from_parts(0x8000_0000_0100_0000, 0, 1024);
    let mut pool = DescPool::new(fake);
    let qh0 = pool.alloc_qh().unwrap();
    let qh1 = pool.alloc_qh().unwrap();
    let qtd0 = pool.alloc_qtd().unwrap();
    assert_eq!(qh0.as_usize() % 32, 0, "QH0 32B aligned");
    assert_eq!(qh1.as_usize() % 32, 0, "QH1 32B aligned");
    assert_eq!(qtd0.as_usize() % 32, 0, "qTD0 32B aligned");
    assert_ne!(qh0.physical(), qh1.physical(), "QHs do not overlap");
    assert_ne!(qh1.physical(), qtd0.physical(), "QH/qTD do not overlap");
}
