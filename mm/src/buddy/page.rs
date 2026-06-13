use core::sync::atomic::AtomicU32;

use super::zone::INVALID_PFN;

pub(super) const INVALID_ORDER: u8 = u8::MAX;

pub(super) const INVALID_ZONE: u8 = u8::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PageState {
    /// 该 PFN 不对应实际 RAM。
    Absent,

    /// 存在，但被内核、固件、元数据或启动分配占用。
    Reserved,

    /// 已从 buddy 分配。
    Allocated,

    /// 正在释放，尚未重新暴露给 buddy 空闲链表。
    Freeing,

    /// buddy 空闲块的第一页。
    FreeHead,

    /// buddy 空闲块中的非首页。
    FreeTail,
}

/// 每个物理页对应的永久元数据。
///
/// 当前字段为长期扩展预留：
///
/// - state/order：buddy；
/// - reference_count：COW、页缓存和映射引用；
/// - next/previous：intrusive free list。
#[repr(C)]
pub struct Page {
    pub(super) state: PageState,
    pub(super) order: u8,
    pub(super) zone: u8,
    pub(super) _padding: u8,

    pub(super) reference_count: AtomicU32,

    pub(super) next: usize,
    pub(super) previous: usize,
}

impl Page {
    pub(super) const fn absent() -> Self {
        Self {
            state: PageState::Absent,
            order: INVALID_ORDER,
            zone: INVALID_ZONE,
            _padding: 0,

            reference_count: AtomicU32::new(0),

            next: INVALID_PFN,
            previous: INVALID_PFN,
        }
    }
}
