use core::{
    mem::size_of,
    ptr::{self, NonNull},
};

use crate::{PAGE_SIZE, PageAllocation};

use super::{MIN_SLAB_OBJECT_SIZE, PageProvider, SizeClass};

const SLAB_MAGIC: u64 = 0x534c_4142_4d59_4f53;

const DEAD_SLAB_MAGIC: u64 = 0x4445_4144_534c_4142;

pub(super) const NULL_SLAB_LINK: usize = 0;

const INVALID_SLOT: u16 = u16::MAX;

const MAX_OBJECTS_PER_SLAB: usize = PAGE_SIZE / MIN_SLAB_OBJECT_SIZE;

const BITMAP_WORDS: usize = MAX_OBJECTS_PER_SLAB.div_ceil(64);

#[cfg(debug_assertions)]
const ALLOCATED_OBJECT_POISON: u8 = 0xa7;

#[cfg(debug_assertions)]
const FREED_OBJECT_POISON: u8 = 0xd7;

#[derive(Debug)]
pub enum SlabError<E> {
    Provider(E),

    ProviderReturnedWrongOrder { order: usize },

    ProviderReturnedMisalignedPage,

    HeaderTooLarge,

    CorruptHeader,

    CorruptFreeList,

    InvalidObjectPointer,

    WrongSizeClass { expected: usize, actual: usize },

    DoubleFree,

    CounterOverflow,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct SlabFreeOutcome {
    pub(super) was_full: bool,
    pub(super) became_empty: bool,
}

#[repr(C)]
pub(super) struct SlabHeader {
    magic: u64,

    pub(super) partial_previous: usize,
    pub(super) partial_next: usize,

    allocation: PageAllocation,

    allocated_bitmap: [u64; BITMAP_WORDS],

    capacity: u16,
    free_count: u16,
    first_free: u16,

    class_index: u8,
    pub(super) listed_partial: u8,
}

impl SlabHeader {
    pub(super) unsafe fn create<P>(
        provider: &mut P,
        class: SizeClass,
    ) -> Result<NonNull<Self>, SlabError<P::Error>>
    where
        P: PageProvider,
    {
        let allocation = provider.allocate_slab_page().map_err(SlabError::Provider)?;

        if allocation.order() != 0 {
            let order = allocation.order();

            provider
                .free_slab_page(allocation)
                .map_err(SlabError::Provider)?;

            return Err(SlabError::ProviderReturnedWrongOrder { order });
        }

        let base = match provider.allocation_pointer(&allocation) {
            Ok(pointer) => pointer,

            Err(error) => match provider.free_slab_page(allocation) {
                Ok(()) => {
                    return Err(SlabError::Provider(error));
                }

                Err(free_error) => {
                    return Err(SlabError::Provider(free_error));
                }
            },
        };

        if !(base.as_ptr() as usize).is_multiple_of(PAGE_SIZE) {
            provider
                .free_slab_page(allocation)
                .map_err(SlabError::Provider)?;

            return Err(SlabError::ProviderReturnedMisalignedPage);
        }

        let object_offset = object_offset(class).ok_or(SlabError::HeaderTooLarge)?;

        if object_offset >= PAGE_SIZE {
            provider
                .free_slab_page(allocation)
                .map_err(SlabError::Provider)?;

            return Err(SlabError::HeaderTooLarge);
        }

        let capacity = (PAGE_SIZE - object_offset) / class.size();

        if capacity == 0 || capacity > MAX_OBJECTS_PER_SLAB || capacity > u16::MAX as usize {
            provider
                .free_slab_page(allocation)
                .map_err(SlabError::Provider)?;

            return Err(SlabError::HeaderTooLarge);
        }

        let header_pointer = base.cast::<Self>();

        // SAFETY: base 指向独占的 order-0 页，空间和对齐均足够。
        unsafe {
            header_pointer.as_ptr().write(Self {
                magic: SLAB_MAGIC,

                partial_previous: NULL_SLAB_LINK,
                partial_next: NULL_SLAB_LINK,

                allocation,

                allocated_bitmap: [0; BITMAP_WORDS],

                capacity: capacity as u16,
                free_count: capacity as u16,
                first_free: 0,

                class_index: class.index() as u8,

                listed_partial: 0,
            });
        }

        /*
         * 在每个空闲对象开头写入下一个空闲槽位。
         */
        // SAFETY: header_pointer 刚刚完成初始化，仍指向独占 slab 页。
        let header = unsafe { &mut *header_pointer.as_ptr() };

        for index in 0..capacity {
            let next = if index + 1 < capacity {
                (index + 1) as u16
            } else {
                INVALID_SLOT
            };

            // SAFETY: index < capacity，object_pointer 会落在当前 slab 页对象区内。
            let object = unsafe { header.object_pointer(index) };

            // SAFETY: 空闲对象槽位至少能容纳 u16 free-list 链接。
            unsafe {
                object.cast::<u16>().write(next);
            }
        }

        Ok(header_pointer)
    }

    pub(super) unsafe fn from_object<E>(
        object: NonNull<u8>,
    ) -> Result<NonNull<Self>, SlabError<E>> {
        let page_base = (object.as_ptr() as usize) & !(PAGE_SIZE - 1);

        let pointer =
            NonNull::new(page_base as *mut Self).ok_or(SlabError::InvalidObjectPointer)?;

        // SAFETY: page_base 是对象所在页基址；magic 校验会验证其是否为 slab header。
        let header = unsafe { &*pointer.as_ptr() };

        if header.magic != SLAB_MAGIC {
            return Err(SlabError::CorruptHeader);
        }

        Ok(pointer)
    }

    pub(super) fn class<E>(&self) -> Result<SizeClass, SlabError<E>> {
        SizeClass::from_index(self.class_index as usize).ok_or(SlabError::CorruptHeader)
    }

    pub(super) fn capacity(&self) -> usize {
        self.capacity as usize
    }

    pub(super) fn is_empty(&self) -> bool {
        self.free_count == self.capacity
    }

    pub(super) fn is_full(&self) -> bool {
        self.free_count == 0
    }

    pub(super) unsafe fn allocate_object<E>(&mut self) -> Result<NonNull<u8>, SlabError<E>> {
        if self.magic != SLAB_MAGIC {
            return Err(SlabError::CorruptHeader);
        }

        if self.is_full() {
            return Err(SlabError::CorruptFreeList);
        }

        let index = self.first_free as usize;

        if index >= self.capacity() {
            return Err(SlabError::CorruptFreeList);
        }

        if self.is_allocated(index) {
            return Err(SlabError::CorruptFreeList);
        }

        // SAFETY: index 来自 first_free 且已验证小于 capacity。
        let object = unsafe { self.object_pointer(index) };

        // SAFETY: 空闲对象开头保存下一空闲槽位索引。
        let next = unsafe { object.cast::<u16>().read() };

        if next != INVALID_SLOT && next as usize >= self.capacity() {
            return Err(SlabError::CorruptFreeList);
        }

        self.first_free = next;

        self.free_count = self
            .free_count
            .checked_sub(1)
            .ok_or(SlabError::CorruptFreeList)?;

        self.set_allocated(index, true);

        #[cfg(debug_assertions)]
        // SAFETY: object 是刚分配出的独占对象槽位。
        unsafe {
            object.write_bytes(ALLOCATED_OBJECT_POISON, self.class::<E>()?.size());
        }

        NonNull::new(object).ok_or(SlabError::CorruptHeader)
    }

    pub(super) unsafe fn free_object<E>(
        &mut self,
        object: NonNull<u8>,
    ) -> Result<SlabFreeOutcome, SlabError<E>> {
        if self.magic != SLAB_MAGIC {
            return Err(SlabError::CorruptHeader);
        }

        let class = self.class::<E>()?;

        let index = self.object_index::<E>(object, class)?;

        if !self.is_allocated(index) {
            return Err(SlabError::DoubleFree);
        }

        if self.free_count >= self.capacity {
            return Err(SlabError::CorruptFreeList);
        }

        let was_full = self.is_full();

        self.set_allocated(index, false);

        let pointer = object.as_ptr();

        #[cfg(debug_assertions)]
        // SAFETY: object 已验证属于当前 slab 且处于 allocated 状态。
        unsafe {
            pointer.write_bytes(FREED_OBJECT_POISON, class.size());
        }

        // SAFETY: 已释放对象槽位用于保存下一空闲槽位索引。
        unsafe {
            pointer.cast::<u16>().write(self.first_free);
        }

        self.first_free = index as u16;

        self.free_count = self
            .free_count
            .checked_add(1)
            .ok_or(SlabError::CounterOverflow)?;

        Ok(SlabFreeOutcome {
            was_full,
            became_empty: self.is_empty(),
        })
    }

    pub(super) unsafe fn take_allocation(&mut self) -> PageAllocation {
        self.magic = DEAD_SLAB_MAGIC;

        // SAFETY: 调用者即将释放承载 header 的物理页，allocation token 只能取出一次。
        unsafe { ptr::read(ptr::addr_of!(self.allocation)) }
    }

    unsafe fn object_pointer(&self, index: usize) -> *mut u8 {
        let class = SizeClass::from_index(self.class_index as usize)
            .expect("validated slab class disappeared");

        let offset = object_offset(class).expect("validated slab offset disappeared");

        // SAFETY: 调用者保证 index 位于当前 slab capacity 内。
        unsafe { (self as *const Self as *mut u8).add(offset + index * class.size()) }
    }

    fn object_index<E>(
        &self,
        object: NonNull<u8>,
        class: SizeClass,
    ) -> Result<usize, SlabError<E>> {
        let base = self as *const Self as usize;

        let offset = object_offset(class).ok_or(SlabError::CorruptHeader)?;

        let object_start = base.checked_add(offset).ok_or(SlabError::CorruptHeader)?;

        let object_bytes = self
            .capacity()
            .checked_mul(class.size())
            .ok_or(SlabError::CorruptHeader)?;

        let object_end = object_start
            .checked_add(object_bytes)
            .ok_or(SlabError::CorruptHeader)?;

        let address = object.as_ptr() as usize;

        if address < object_start || address >= object_end {
            return Err(SlabError::InvalidObjectPointer);
        }

        let relative = address - object_start;

        if !relative.is_multiple_of(class.size()) {
            return Err(SlabError::InvalidObjectPointer);
        }

        let index = relative / class.size();

        if index >= self.capacity() {
            return Err(SlabError::InvalidObjectPointer);
        }

        Ok(index)
    }

    fn is_allocated(&self, index: usize) -> bool {
        let word = index / 64;
        let bit = index % 64;

        self.allocated_bitmap[word] & (1_u64 << bit) != 0
    }

    fn set_allocated(&mut self, index: usize, allocated: bool) {
        let word = index / 64;
        let bit = index % 64;
        let mask = 1_u64 << bit;

        if allocated {
            self.allocated_bitmap[word] |= mask;
        } else {
            self.allocated_bitmap[word] &= !mask;
        }
    }
}

fn object_offset(class: SizeClass) -> Option<usize> {
    align_up(size_of::<SlabHeader>(), class.size())
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    debug_assert!(alignment.is_power_of_two());

    let mask = alignment - 1;

    value.checked_add(mask).map(|value| value & !mask)
}
