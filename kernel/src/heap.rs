use core::{
    alloc::{GlobalAlloc, Layout},
    ptr::{NonNull, null_mut},
};

use myos_mm::{HeapAllocator, HeapStats, PageAllocation, PageProvider};

use crate::irq_lock::IrqSpinLock;

use crate::page_alloc::{self, GlobalPageAllocatorError, PageAllocationOptions};

struct KernelPageProvider;

impl PageProvider for KernelPageProvider {
    type Error = GlobalPageAllocatorError;

    fn allocate_pages(&mut self, order: usize) -> Result<PageAllocation, Self::Error> {
        page_alloc::allocate(order, PageAllocationOptions::kernel())
    }

    fn free_pages(&mut self, allocation: PageAllocation) -> Result<(), Self::Error> {
        page_alloc::free(allocation)
    }

    fn allocation_pointer(&self, allocation: &PageAllocation) -> Result<NonNull<u8>, Self::Error> {
        let pointer =
            crate::arch::memory::phys_access::ram_mut_ptr::<u8>(allocation.range().start())
                .map_err(|_| GlobalPageAllocatorError::PhysicalMemoryNotAccessible)?;

        NonNull::new(pointer).ok_or(GlobalPageAllocatorError::PhysicalMemoryNotAccessible)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeapInstallError {
    AlreadyInitialized,
}

pub struct KernelGlobalAllocator {
    heap: IrqSpinLock<Option<HeapAllocator<KernelPageProvider>>>,
}

impl KernelGlobalAllocator {
    pub const fn new() -> Self {
        Self {
            heap: IrqSpinLock::new(None),
        }
    }

    pub fn install(&self) -> Result<(), HeapInstallError> {
        let mut heap = self.heap.lock();

        if heap.is_some() {
            return Err(HeapInstallError::AlreadyInitialized);
        }

        *heap = Some(HeapAllocator::new(KernelPageProvider));

        Ok(())
    }

    pub fn is_initialized(&self) -> bool {
        self.heap.lock().is_some()
    }

    pub fn shrink(&self) {
        let failed = {
            let mut heap = self.heap.lock();

            match heap.as_mut() {
                Some(heap) => heap.shrink().is_err(),
                None => true,
            }
        };

        if failed {
            fatal_heap_corruption();
        }
    }

    pub fn stats(&self) -> Option<HeapStats> {
        self.heap.lock().as_ref().map(HeapAllocator::stats)
    }

    fn allocate(&self, layout: Layout, zeroed: bool) -> *mut u8 {
        if layout.size() == 0 {
            return null_mut();
        }

        let mut slot = self.heap.lock();

        let Some(heap) = slot.as_mut() else {
            return null_mut();
        };

        match heap.allocate(layout, zeroed) {
            Ok(pointer) => pointer.as_ptr(),
            Err(_) => null_mut(),
        }
    }
}

// SAFETY: 所有 heap 状态都被 IrqSpinLock 串行化，返回指针遵守 GlobalAlloc 契约。
unsafe impl GlobalAlloc for KernelGlobalAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.allocate(layout, false)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        self.allocate(layout, true)
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        let Some(pointer) = NonNull::new(pointer) else {
            fatal_heap_corruption();
        };

        let failed = {
            let mut slot = self.heap.lock();

            let Some(heap) = slot.as_mut() else {
                drop(slot);
                fatal_heap_corruption();
            };

            // SAFETY: GlobalAlloc::dealloc 的调用者保证 pointer/layout 来自此前分配。
            unsafe { heap.deallocate(pointer, layout) }.is_err()
        };

        if failed {
            fatal_heap_corruption();
        }
    }
}

#[global_allocator]
static GLOBAL_HEAP: KernelGlobalAllocator = KernelGlobalAllocator::new();

pub fn shrink() {
    GLOBAL_HEAP.shrink();
}

pub fn initialize() {
    GLOBAL_HEAP.install().unwrap_or_else(|error| {
        panic!(
            "unable to install kernel heap: \
                 {error:?}",
        );
    });

    assert!(GLOBAL_HEAP.is_initialized(),);

    crate::println!("kernel heap:");
    crate::println!("  small objects : slab",);
    crate::println!("  large objects : buddy",);
    crate::println!("  global allocator: installed",);
}

/// allocator 损坏时不能 panic：panic 路径可能再次分配并导致递归。
fn fatal_heap_corruption() -> ! {
    /*
     * 这里只输出静态字符串，不构造任何堆对象。
     */
    crate::println!();
    crate::println!("FATAL: kernel heap corruption");

    loop {
        crate::arch::cpu::wait_for_interrupt();
    }
}

#[cfg(debug_assertions)]
pub fn verify() {
    use alloc::{
        alloc::{alloc_zeroed, dealloc},
        boxed::Box,
        string::String,
        sync::Arc,
        vec::Vec,
    };

    use core::{alloc::Layout, hint::black_box, slice};

    let before = page_alloc::total_free_pages().expect("page allocator unavailable");

    {
        /*
         * 小对象：slab。
         */
        let boxed = Box::new(0x1234_5678_u64);

        assert_eq!(*boxed, 0x1234_5678,);

        let mut text = String::from("MyOS");

        text.push_str(" robust kernel heap");

        assert!(text.starts_with("MyOS"),);

        let shared = Arc::new([0x5a_u8; 128]);

        let shared_clone = Arc::clone(&shared);

        assert_eq!(shared_clone[127], 0x5a,);

        /*
         * Vec 扩容会覆盖 slab、大对象以及默认 realloc 路径。
         */
        let mut values = Vec::<u64>::new();

        for value in 0..8192_u64 {
            values.push(value);
        }

        let sum: u64 = values.iter().copied().sum();

        assert_eq!(sum, (8191_u64 * 8192_u64) / 2,);

        black_box(&values);

        /*
         * 显式验证大对象和超页对齐。
         */
        let aligned_layout =
            Layout::from_size_align(96 * 1024, 8192).expect("invalid heap test layout");

        // SAFETY: 测试 layout 非零且有效，返回值随后检查空指针。
        let aligned_pointer = unsafe { alloc_zeroed(aligned_layout) };

        assert!(
            !aligned_pointer.is_null(),
            "large aligned allocation failed",
        );

        assert_eq!(aligned_pointer as usize % aligned_layout.align(), 0,);

        // SAFETY: aligned_pointer 是刚分配出的 aligned_layout.size() 字节区域。
        let bytes = unsafe { slice::from_raw_parts(aligned_pointer, aligned_layout.size()) };

        assert!(
            bytes.iter().all(|byte| *byte == 0),
            "alloc_zeroed returned dirty memory",
        );

        // SAFETY: aligned_pointer/aligned_layout 对应上面的 alloc_zeroed 调用。
        unsafe {
            dealloc(aligned_pointer, aligned_layout);
        }
    }

    /*
     * 每个 size class 默认保留一个空 slab；
     * shrink 后应全部归还 buddy。
     */
    GLOBAL_HEAP.shrink();

    let stats = GLOBAL_HEAP.stats().expect("kernel heap disappeared");

    assert_eq!(stats.large_allocations, 0, "large allocation leaked",);

    assert_eq!(stats.large_pages, 0, "large allocation pages leaked",);

    let after = page_alloc::total_free_pages().expect("page allocator unavailable");

    assert_eq!(before, after, "kernel heap leaked physical pages",);

    crate::println!("kernel heap test:");
    crate::println!("  Box/String/Arc : verified",);
    crate::println!("  Vec growth     : verified",);
    crate::println!("  large aligned  : 96 KiB / 8 KiB",);
    crate::println!("  alloc_zeroed   : verified",);
    crate::println!("  shrink         : all pages returned",);
}
