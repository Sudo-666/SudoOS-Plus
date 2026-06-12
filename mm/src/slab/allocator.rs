use core::{alloc::Layout, ptr::NonNull};

use super::{
    PageProvider, SIZE_CLASS_COUNT, SizeClass, SlabError,
    cache::{SlabCache, SlabCacheStats},
};

pub struct SlabAllocator<P> {
    provider: P,

    caches: [SlabCache; SIZE_CLASS_COUNT],
}

impl<P> SlabAllocator<P>
where
    P: PageProvider,
{
    pub const fn new(provider: P) -> Self {
        Self {
            provider,

            caches: [
                SlabCache::new(SizeClass::from_index_unchecked(0)),
                SlabCache::new(SizeClass::from_index_unchecked(1)),
                SlabCache::new(SizeClass::from_index_unchecked(2)),
                SlabCache::new(SizeClass::from_index_unchecked(3)),
                SlabCache::new(SizeClass::from_index_unchecked(4)),
                SlabCache::new(SizeClass::from_index_unchecked(5)),
                SlabCache::new(SizeClass::from_index_unchecked(6)),
                SlabCache::new(SizeClass::from_index_unchecked(7)),
                SlabCache::new(SizeClass::from_index_unchecked(8)),
            ],
        }
    }

    /// 分配一个小对象。
    ///
    /// 大于 2048 字节或 alignment 大于 2048 的请求返回 None。
    pub fn allocate(&mut self, layout: Layout) -> Result<Option<NonNull<u8>>, SlabError<P::Error>> {
        let Some(class) = SizeClass::for_layout(layout) else {
            return Ok(None);
        };

        self.caches[class.index()]
            .allocate(&mut self.provider)
            .map(Some)
    }

    /// # Safety
    ///
    /// object 必须由当前 allocator 使用相同 Layout 分配，
    /// 且尚未释放。
    pub unsafe fn deallocate(
        &mut self,
        object: NonNull<u8>,
        layout: Layout,
    ) -> Result<bool, SlabError<P::Error>> {
        let Some(class) = SizeClass::for_layout(layout) else {
            return Ok(false);
        };

        // SAFETY: 当前函数的 safety contract 保证 object/layout 来自本 allocator。
        unsafe {
            self.caches[class.index()].deallocate(&mut self.provider, object)?;
        }

        Ok(true)
    }

    /// 将所有完全空闲的 slab 页归还 buddy。
    pub fn shrink(&mut self) -> Result<(), SlabError<P::Error>> {
        for cache in &mut self.caches {
            cache.shrink(&mut self.provider)?;
        }

        Ok(())
    }

    pub const fn stats(&self, class: SizeClass) -> SlabCacheStats {
        self.caches[class.index()].stats()
    }

    pub(crate) fn provider_mut(&mut self) -> &mut P {
        &mut self.provider
    }
}

/*
 * SlabAllocator 只有在外部独占访问时才修改 intrusive list。
 * 后续全局使用时会放在 SpinLock 内。
 */
// SAFETY: 修改需要 &mut self，跨 CPU 移动时 provider 也必须可 Send。
unsafe impl<P> Send for SlabAllocator<P> where P: PageProvider + Send {}
