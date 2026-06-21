use core::ptr::NonNull;

use super::{
    PageProvider, SizeClass, SlabError,
    slab::{NULL_SLAB_LINK, SlabHeader},
};

#[derive(Clone, Copy, Debug)]
pub struct SlabCacheStats {
    pub class_size: usize,
    pub slab_count: usize,
    pub empty_slabs: usize,
    pub allocated_objects: usize,
}

pub(super) struct SlabCache {
    class: SizeClass,

    partial_head: usize,

    slab_count: usize,
    empty_slabs: usize,
    allocated_objects: usize,
}

impl SlabCache {
    pub(super) const fn new(class: SizeClass) -> Self {
        Self {
            class,

            partial_head: NULL_SLAB_LINK,

            slab_count: 0,
            empty_slabs: 0,
            allocated_objects: 0,
        }
    }

    pub(super) fn allocate<P>(
        &mut self,
        provider: &mut P,
    ) -> Result<NonNull<u8>, SlabError<P::Error>>
    where
        P: PageProvider,
    {
        if self.partial_head == NULL_SLAB_LINK {
            // SAFETY: provider 返回独占 order-0 页，SlabHeader::create 会完成初始化。
            let slab = unsafe { SlabHeader::create(provider, self.class)? };

            // SAFETY: 新 slab 尚未出现在任何链表中。
            unsafe {
                self.insert_partial::<P::Error>(slab)?;
            }

            self.slab_count = self
                .slab_count
                .checked_add(1)
                .ok_or(SlabError::CounterOverflow)?;

            self.empty_slabs = self
                .empty_slabs
                .checked_add(1)
                .ok_or(SlabError::CounterOverflow)?;
        }

        let slab_pointer = link_to_pointer::<P::Error>(self.partial_head)?;

        // SAFETY: partial_head 来自本 cache 维护的 partial 链表。
        let (object, was_empty, became_full) = unsafe {
            let slab = &mut *slab_pointer.as_ptr();

            let was_empty = slab.is_empty();

            let object = slab.allocate_object::<P::Error>()?;

            (object, was_empty, slab.is_full())
        };

        if was_empty {
            self.empty_slabs = self
                .empty_slabs
                .checked_sub(1)
                .ok_or(SlabError::CorruptHeader)?;
        }

        if became_full {
            // SAFETY: slab_pointer 当前仍在 partial 链表中。
            unsafe {
                self.remove_partial::<P::Error>(slab_pointer)?;
            }
        }

        self.allocated_objects = self
            .allocated_objects
            .checked_add(1)
            .ok_or(SlabError::CounterOverflow)?;

        Ok(object)
    }

    pub(super) unsafe fn deallocate<P>(
        &mut self,
        provider: &mut P,
        object: NonNull<u8>,
    ) -> Result<(), SlabError<P::Error>>
    where
        P: PageProvider,
    {
        // SAFETY: 调用者保证 object 是 slab allocator 管理的有效对象。
        let slab_pointer = unsafe { SlabHeader::from_object::<P::Error>(object)? };

        // SAFETY: slab_pointer 已由 magic 校验为有效 slab header。
        let actual_class = unsafe { (&*slab_pointer.as_ptr()).class::<P::Error>()? };

        if actual_class != self.class {
            return Err(SlabError::WrongSizeClass {
                expected: self.class.size(),
                actual: actual_class.size(),
            });
        }

        // SAFETY: object 已确认属于该 size class 的有效 slab。
        let outcome = unsafe { (&mut *slab_pointer.as_ptr()).free_object::<P::Error>(object)? };

        if outcome.was_full {
            // SAFETY: full slab 此前不在 partial 链表，释放对象后可重新加入。
            unsafe {
                self.insert_partial::<P::Error>(slab_pointer)?;
            }
        }

        self.allocated_objects = self
            .allocated_objects
            .checked_sub(1)
            .ok_or(SlabError::CorruptHeader)?;

        if outcome.became_empty {
            if self.empty_slabs == 0 {
                /*
                 * 每个 size class 保留一个空 slab，
                 * 降低反复申请和释放 buddy 页的开销。
                 */
                self.empty_slabs = 1;
            } else {
                /*
                 * 已经有一个空 slab，这个多余空 slab
                 * 立即归还 buddy。
                 */
                // SAFETY: 空 slab 当前位于 partial 链表中。
                unsafe {
                    self.remove_partial::<P::Error>(slab_pointer)?;
                }

                // SAFETY: slab 已从链表移除，且即将归还承载页。
                let allocation = unsafe { (&mut *slab_pointer.as_ptr()).take_allocation() };

                self.slab_count = self
                    .slab_count
                    .checked_sub(1)
                    .ok_or(SlabError::CorruptHeader)?;

                provider
                    .free_slab_page(allocation)
                    .map_err(SlabError::Provider)?;
            }
        }

        Ok(())
    }

    pub(super) fn shrink<P>(&mut self, provider: &mut P) -> Result<(), SlabError<P::Error>>
    where
        P: PageProvider,
    {
        let mut current = self.partial_head;

        while current != NULL_SLAB_LINK {
            let slab_pointer = link_to_pointer::<P::Error>(current)?;

            // SAFETY: current 是 partial 链表中的有效 slab 链接。
            let next = unsafe { (&*slab_pointer.as_ptr()).partial_next };

            // SAFETY: current 是 partial 链表中的有效 slab 链接。
            let empty = unsafe { (&*slab_pointer.as_ptr()).is_empty() };

            if empty {
                // SAFETY: empty slab 当前位于 partial 链表中。
                unsafe {
                    self.remove_partial::<P::Error>(slab_pointer)?;
                }

                // SAFETY: slab 已从链表移除，且即将归还承载页。
                let allocation = unsafe { (&mut *slab_pointer.as_ptr()).take_allocation() };

                self.slab_count = self
                    .slab_count
                    .checked_sub(1)
                    .ok_or(SlabError::CorruptHeader)?;

                self.empty_slabs = self
                    .empty_slabs
                    .checked_sub(1)
                    .ok_or(SlabError::CorruptHeader)?;

                provider
                    .free_slab_page(allocation)
                    .map_err(SlabError::Provider)?;
            }

            current = next;
        }

        Ok(())
    }

    pub(super) const fn stats(&self) -> SlabCacheStats {
        SlabCacheStats {
            class_size: self.class.size(),
            slab_count: self.slab_count,
            empty_slabs: self.empty_slabs,
            allocated_objects: self.allocated_objects,
        }
    }

    unsafe fn insert_partial<E>(
        &mut self,
        slab_pointer: NonNull<SlabHeader>,
    ) -> Result<(), SlabError<E>> {
        // SAFETY: 调用者保证 slab_pointer 指向有效 slab header。
        let slab = unsafe { &mut *slab_pointer.as_ptr() };

        if slab.listed_partial != 0 {
            return Err(SlabError::CorruptHeader);
        }

        slab.partial_previous = NULL_SLAB_LINK;

        slab.partial_next = self.partial_head;

        if self.partial_head != NULL_SLAB_LINK {
            let previous_head = link_to_pointer::<E>(self.partial_head)?;

            // SAFETY: previous_head 是当前 partial 链表头。
            unsafe {
                (*previous_head.as_ptr()).partial_previous = slab_pointer.as_ptr() as usize;
            }
        }

        slab.listed_partial = 1;

        self.partial_head = slab_pointer.as_ptr() as usize;

        Ok(())
    }

    unsafe fn remove_partial<E>(
        &mut self,
        slab_pointer: NonNull<SlabHeader>,
    ) -> Result<(), SlabError<E>> {
        // SAFETY: 调用者保证 slab_pointer 指向 partial 链表中的有效 slab。
        let slab = unsafe { &mut *slab_pointer.as_ptr() };

        if slab.listed_partial == 0 {
            return Err(SlabError::CorruptHeader);
        }

        let previous = slab.partial_previous;

        let next = slab.partial_next;

        if previous == NULL_SLAB_LINK {
            if self.partial_head != slab_pointer.as_ptr() as usize {
                return Err(SlabError::CorruptHeader);
            }

            self.partial_head = next;
        } else {
            let previous_pointer = link_to_pointer::<E>(previous)?;

            // SAFETY: previous 指向当前 slab 的前驱节点。
            unsafe {
                (*previous_pointer.as_ptr()).partial_next = next;
            }
        }

        if next != NULL_SLAB_LINK {
            let next_pointer = link_to_pointer::<E>(next)?;

            // SAFETY: next 指向当前 slab 的后继节点。
            unsafe {
                (*next_pointer.as_ptr()).partial_previous = previous;
            }
        }

        slab.partial_previous = NULL_SLAB_LINK;

        slab.partial_next = NULL_SLAB_LINK;

        slab.listed_partial = 0;

        Ok(())
    }
}

fn link_to_pointer<E>(link: usize) -> Result<NonNull<SlabHeader>, SlabError<E>> {
    if link == NULL_SLAB_LINK {
        return Err(SlabError::CorruptHeader);
    }

    NonNull::new(link as *mut SlabHeader).ok_or(SlabError::CorruptHeader)
}
