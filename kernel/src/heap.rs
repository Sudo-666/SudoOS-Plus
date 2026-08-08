use core::{
    alloc::{GlobalAlloc, Layout},
    ptr::{NonNull, null_mut},
};

use myos_mm::{HeapAllocator, HeapError, HeapStats, PageAllocation, PageProvider, SlabError};

use crate::{
    irq_lock::IrqSpinLock,
    lockdep::{LockClass, LockRank},
};

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
            heap: IrqSpinLock::new_with_class(
                None,
                LockClass::new("kernel_heap", LockRank::Heap, 1),
            ),
        }
    }

    pub fn install(&self) -> Result<(), HeapInstallError> {
        let mut heap = self.heap.lock();

        if heap.is_some() {
            return Err(HeapInstallError::AlreadyInitialized);
        }

        *heap = Some(HeapAllocator::new(KernelPageProvider));

        /*
         * LS2K1000 真机调试：记录安装完成标志并打印全局分配器与堆句柄的
         * 静态地址，供后续 HEAP-STATE/HEAP-NONE-ALLOC 对照。仅 ls2k1000 编译。
         */
        #[cfg(feature = "platform-ls2k1000")]
        {
            LS2K_HEAP_INSTALLED.store(true, core::sync::atomic::Ordering::Release);
            crate::println!(
                "HEAP-INSTALLED global={:#x} field={:#x}",
                &GLOBAL_HEAP as *const _ as usize,
                &GLOBAL_HEAP.heap as *const _ as usize,
            );
        }

        Ok(())
    }

    #[cfg(target_arch = "riscv64")]
    pub unsafe fn install_boot(&self) -> Result<(), HeapInstallError> {
        let heap = unsafe { self.heap.get_mut_unchecked() };
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

        #[cfg(feature = "platform-ls2k1000")]
        let caller: usize = {
            let mut value: usize;
            // SAFETY: 读取当前调用方返回地址，不改动任何机器状态。
            unsafe {
                core::arch::asm!("or {}, $ra, $zero", out(reg) value, options(nomem, nostack));
            }
            value
        };

        #[cfg(feature = "platform-ls2k1000")]
        LS2K_ALLOC_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);

        let mut slot = self.heap.lock();

        let Some(heap) = slot.as_mut() else {
            /*
             * LS2K1000 真机调试：堆 Option 为 None——要么从未安装，要么
             * GLOBAL_HEAP 静态被写坏。打印调用方返回地址（可 addr2line 解析）、
             * 堆句柄地址、安装标志以及原始内存字，区分“未安装”与“损坏”。
             */
            #[cfg(feature = "platform-ls2k1000")]
            {
                let field = &GLOBAL_HEAP.heap as *const _ as usize;
                let words =
                    unsafe { core::slice::from_raw_parts(field as *const usize, 12) };
                crate::println!(
                    "HEAP-NONE-ALLOC n={} size={} align={} zeroed={} caller={:#x} field={:#x}",
                    LS2K_ALLOC_COUNT.load(core::sync::atomic::Ordering::Relaxed),
                    layout.size(),
                    layout.align(),
                    zeroed,
                    caller,
                    field,
                );
                crate::println!(
                    "HEAP-NONE installed={} words {:#x} {:#x} {:#x} {:#x} {:#x} {:#x} {:#x} {:#x} {:#x} {:#x} {:#x} {:#x}",
                    LS2K_HEAP_INSTALLED.load(core::sync::atomic::Ordering::Relaxed),
                    words[0], words[1], words[2], words[3], words[4], words[5],
                    words[6], words[7], words[8], words[9], words[10], words[11],
                );
            }
            return null_mut();
        };

        match heap.allocate(layout, zeroed) {
            Ok(pointer) => pointer.as_ptr(),
            Err(error) => {
                /*
                 * LS2K1000 真机调试：分配失败时打印具体原因与分配器状态。
                 * 错误路径不会再次分配，打印也不会分配，因此可安全执行；
                 * 其他平台此代码不参与编译。
                 */
                #[cfg(feature = "platform-ls2k1000")]
                {
                    crate::println!(
                        "HEAP-ALLOC-FAIL n={} size={} align={} zeroed={} caller={:#x} error={:?}",
                        LS2K_ALLOC_COUNT.load(core::sync::atomic::Ordering::Relaxed),
                        layout.size(),
                        layout.align(),
                        zeroed,
                        caller,
                        error,
                    );
                    crate::println!(
                        "HEAP-ALLOC-FAIL free-pages={:?}",
                        crate::page_alloc::total_free_pages(),
                    );
                }
                null_mut()
            }
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
        #[cfg(target_arch = "riscv64")]
        let caller: usize;
        #[cfg(target_arch = "riscv64")]
        // SAFETY: reading ra does not alter machine state.
        unsafe {
            core::arch::asm!("mv {}, ra", out(reg) caller, options(nomem, nostack, preserves_flags));
        }
        #[cfg(target_arch = "loongarch64")]
        let caller: usize;
        #[cfg(target_arch = "loongarch64")]
        // SAFETY: reading the return-address register does not alter machine state.
        unsafe {
            core::arch::asm!("or {}, $ra, $zero", out(reg) caller, options(nomem, nostack));
        }
        #[cfg(not(any(target_arch = "riscv64", target_arch = "loongarch64")))]
        let caller = 0_usize;
        let Some(pointer) = NonNull::new(pointer) else {
            fatal_heap_corruption();
        };

        let error = {
            let mut slot = self.heap.lock();

            let Some(heap) = slot.as_mut() else {
                drop(slot);
                fatal_heap_corruption();
            };

            // SAFETY: GlobalAlloc::dealloc 的调用者保证 pointer/layout 来自此前分配。
            unsafe { heap.deallocate(pointer, layout) }.err()
        };

        if let Some(error) = error {
            fatal_heap_deallocation(error, pointer, layout, caller);
        }
    }
}

#[global_allocator]
static GLOBAL_HEAP: KernelGlobalAllocator = KernelGlobalAllocator::new();

/*
 * LS2K1000 真机调试统计：分配次数计数与“堆已安装”标志。
 * 仅 ls2k1000 平台参与编译，qemu_virt/riscv64 不受影响。
 */
#[cfg(feature = "platform-ls2k1000")]
static LS2K_ALLOC_COUNT: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

#[cfg(feature = "platform-ls2k1000")]
static LS2K_HEAP_INSTALLED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/*
 * LS2K1000 真机调试：在启动关键点输出堆句柄的原始内存与可加锁状态，
 * 用于定位 GLOBAL_HEAP.heap 的 Option 被写坏/丢失的时刻。
 *
 * - try-lock=1 表示能取到堆锁，is-some 是 Option 的真实判别值；
 * - try-lock=0 表示锁被占用或 owner 字段损坏（自身即是损坏信号）；
 * - words 是堆句柄内存的前 96 字节指纹，用于跨检查点比较是否被改写。
 *
 * 不分配内存、不长时间持锁，可在任意任务上下文安全调用。
 */
#[cfg(feature = "platform-ls2k1000")]
pub fn dump_heap_state(tag: &'static str) {
    let field = &GLOBAL_HEAP.heap as *const _ as usize;
    let words = unsafe { core::slice::from_raw_parts(field as *const usize, 12) };
    let lock_state = GLOBAL_HEAP.heap.try_lock();
    let is_some = lock_state.as_ref().is_some_and(|guard| guard.is_some());

    crate::println!(
        "HEAP-STATE[{}] field={:#x} try-lock={} is-some={}",
        tag,
        field,
        if lock_state.is_some() { 1 } else { 0 },
        is_some,
    );
    crate::println!(
        "HEAP-STATE[{}] words {:#x} {:#x} {:#x} {:#x} {:#x} {:#x} {:#x} {:#x} {:#x} {:#x} {:#x} {:#x}",
        tag,
        words[0], words[1], words[2], words[3], words[4], words[5],
        words[6], words[7], words[8], words[9], words[10], words[11],
    );
}

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

#[cfg(target_arch = "riscv64")]
#[inline(always)]
fn riscv_heap_put(byte: u8) {
    crate::arch::early_console::write_byte(byte);
}

#[cfg(target_arch = "riscv64")]
fn riscv_heap_print_installed() {
    riscv_heap_put(b'k');
    riscv_heap_put(b'e');
    riscv_heap_put(b'r');
    riscv_heap_put(b'n');
    riscv_heap_put(b'e');
    riscv_heap_put(b'l');
    riscv_heap_put(b' ');
    riscv_heap_put(b'h');
    riscv_heap_put(b'e');
    riscv_heap_put(b'a');
    riscv_heap_put(b'p');
    riscv_heap_put(b':');
    riscv_heap_put(b'\n');
    riscv_heap_put(b' ');
    riscv_heap_put(b' ');
    riscv_heap_put(b's');
    riscv_heap_put(b'm');
    riscv_heap_put(b'a');
    riscv_heap_put(b'l');
    riscv_heap_put(b'l');
    riscv_heap_put(b' ');
    riscv_heap_put(b'o');
    riscv_heap_put(b'b');
    riscv_heap_put(b'j');
    riscv_heap_put(b'e');
    riscv_heap_put(b'c');
    riscv_heap_put(b't');
    riscv_heap_put(b's');
    riscv_heap_put(b' ');
    riscv_heap_put(b':');
    riscv_heap_put(b' ');
    riscv_heap_put(b's');
    riscv_heap_put(b'l');
    riscv_heap_put(b'a');
    riscv_heap_put(b'b');
    riscv_heap_put(b'\n');
    riscv_heap_put(b' ');
    riscv_heap_put(b' ');
    riscv_heap_put(b'l');
    riscv_heap_put(b'a');
    riscv_heap_put(b'r');
    riscv_heap_put(b'g');
    riscv_heap_put(b'e');
    riscv_heap_put(b' ');
    riscv_heap_put(b'o');
    riscv_heap_put(b'b');
    riscv_heap_put(b'j');
    riscv_heap_put(b'e');
    riscv_heap_put(b'c');
    riscv_heap_put(b't');
    riscv_heap_put(b's');
    riscv_heap_put(b' ');
    riscv_heap_put(b':');
    riscv_heap_put(b' ');
    riscv_heap_put(b'b');
    riscv_heap_put(b'u');
    riscv_heap_put(b'd');
    riscv_heap_put(b'd');
    riscv_heap_put(b'y');
    riscv_heap_put(b'\n');
    riscv_heap_put(b' ');
    riscv_heap_put(b' ');
    riscv_heap_put(b'g');
    riscv_heap_put(b'l');
    riscv_heap_put(b'o');
    riscv_heap_put(b'b');
    riscv_heap_put(b'a');
    riscv_heap_put(b'l');
    riscv_heap_put(b' ');
    riscv_heap_put(b'a');
    riscv_heap_put(b'l');
    riscv_heap_put(b'l');
    riscv_heap_put(b'o');
    riscv_heap_put(b'c');
    riscv_heap_put(b'a');
    riscv_heap_put(b't');
    riscv_heap_put(b'o');
    riscv_heap_put(b'r');
    riscv_heap_put(b':');
    riscv_heap_put(b' ');
    riscv_heap_put(b'i');
    riscv_heap_put(b'n');
    riscv_heap_put(b's');
    riscv_heap_put(b't');
    riscv_heap_put(b'a');
    riscv_heap_put(b'l');
    riscv_heap_put(b'l');
    riscv_heap_put(b'e');
    riscv_heap_put(b'd');
    riscv_heap_put(b'\n');
}

#[cfg(target_arch = "riscv64")]
pub fn initialize_boot() {
    unsafe { GLOBAL_HEAP.install_boot() }.unwrap_or_else(|error| {
        panic!("unable to install kernel heap through boot path: {error:?}");
    });
    riscv_heap_print_installed();
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

fn fatal_heap_deallocation(
    error: HeapError<GlobalPageAllocatorError>,
    pointer: NonNull<u8>,
    layout: Layout,
    caller: usize,
) -> ! {
    let reason = match error {
        HeapError::Slab(SlabError::CorruptHeader) => "slab-corrupt-header",
        HeapError::Slab(SlabError::CorruptFreeList) => "slab-corrupt-free-list",
        HeapError::Slab(SlabError::InvalidObjectPointer) => "slab-invalid-pointer",
        HeapError::Slab(SlabError::WrongSizeClass { .. }) => "slab-wrong-size-class",
        HeapError::Slab(SlabError::DoubleFree) => "slab-double-free",
        HeapError::Slab(_) => "slab-other",
        HeapError::CorruptLargeAllocation => "large-corrupt-header",
        HeapError::LayoutMismatch => "large-layout-mismatch",
        HeapError::Provider(_) => "page-provider",
        HeapError::ZeroSizedLayout => "zero-sized-layout",
        HeapError::AllocationTooLarge => "allocation-too-large",
        HeapError::AddressOverflow => "address-overflow",
        HeapError::CounterOverflow => "counter-overflow",
    };
    crate::println!();
    crate::println!(
        "FATAL: kernel heap deallocation {} ptr={:#x} size={} align={} caller={:#x} cpu={}",
        reason,
        pointer.as_ptr() as usize,
        layout.size(),
        layout.align(),
        caller,
        crate::smp::current_cpu_id().get(),
    );
    let header = (pointer.as_ptr() as *const usize).wrapping_sub(4);
    // SAFETY: a large-allocation header immediately precedes the still-mapped
    // allocation supplied to dealloc; this is fatal-path diagnostics only.
    unsafe {
        crate::println!(
            "FATAL: heap header words {:#x} {:#x} {:#x} {:#x}",
            header.read(),
            header.add(1).read(),
            header.add(2).read(),
            header.add(3).read(),
        );
    }
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
