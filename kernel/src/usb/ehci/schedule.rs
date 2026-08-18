//! EHCI 固定异步队列：自环 async head QH + frame list。
//!
//! async head 自环（`hlp`=自身低物理地址 | QH 类型），`H`=1（reclamation
//! list 头），overlay halted/终止。无真实端点时控制器空转——标准 EHCI 空闲
//! 队列；RUSB-3 把 control/bulk QH 插到 head 之后（自环从 head 断开）。

use super::super::{dma::DmaRegion, error::UsbError, platform::ls2k1000::uncached_to_phys};
use super::descriptor::{DescPool, Qh, link_qh, link_terminate, token};

/// Frame List 表项数（EHCI 1024 × 4B = 4 KiB，`DmaPool.frame_list`）。
pub const FRAME_LIST_ENTRIES: usize = 1024;

/// 异步调度：frame list + 自环 async head。
pub struct AsyncSchedule {
    /// async head QH（自环；RUSB-3 插入真实端点 QH）。
    head: DmaRegion,
}

impl AsyncSchedule {
    /// 建立 frame list（全 T=1 终止）与自环 async head QH。
    ///
    /// # Safety
    /// - `frame_list` 长度 ≥ `FRAME_LIST_ENTRIES * 4`（`DmaPool::new` 保证）
    /// - `desc_pool` 有足够一个 QH 槽位（`DESCRIPTOR_POOL_SIZE` 保证）
    pub fn init(frame_list: DmaRegion, desc_pool: &mut DescPool) -> Result<Self, UsbError> {
        // 周期列表：全部表项写 T=1（控制器跳过）。
        for i in 0..FRAME_LIST_ENTRIES {
            // SAFETY: frame_list 长度 ≥ 1024×4（见函数 Safety 文档）。
            unsafe { frame_list.write_volatile(i * 4, 1u32) };
        }

        let head = desc_pool.alloc_qh()?;
        let head_pa = uncached_to_phys(head.as_usize());
        let mut qh = Qh::default();
        qh.hlp = link_qh(head_pa); // 自环
        qh.epchar = super::descriptor::epchar::H; // reclamation list 头
        qh.overlay.nqp = link_terminate();
        qh.overlay.alt = link_terminate();
        qh.overlay.token = token::HALTED; // 空闲保护：overlay 永不激活
        // SAFETY: head 是 32B 对齐的 QH 槽位（`DescPool` 保证），长度 48B。
        unsafe { head.write_volatile(0, qh) };

        Ok(Self { head })
    }

    /// async head 的低 32 位物理地址（写 `ASYNCLISTADDR` 用）。
    pub const fn head_physical(&self) -> u32 {
        uncached_to_phys(self.head.as_usize())
    }
}
