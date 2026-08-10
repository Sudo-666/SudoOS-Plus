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
         * LS2K1000：记录堆安装完成的标志，供 ls2k_fatal_heap_none 引用。
         * 仅 ls2k1000 平台编译。
         */
        #[cfg(feature = "platform-ls2k1000")]
        LS2K_HEAP_INSTALLED.store(true, core::sync::atomic::Ordering::Release);

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

        let mut slot = self.heap.lock();

        let Some(heap) = slot.as_mut() else {
            /*
             * LS2K1000：堆 Option 为 None——从未安装或被写坏。裸写 UART 输出
             * 致命信息后停机（println 可能被控制台锁吞掉）。其他平台保持原语义。
             */
            #[cfg(feature = "platform-ls2k1000")]
            return Self::ls2k_fatal_heap_none(layout, zeroed);
            #[cfg(not(feature = "platform-ls2k1000"))]
            return null_mut();
        };

        match heap.allocate(layout, zeroed) {
            Ok(pointer) => pointer.as_ptr(),
            Err(error) => {
                /*
                 * LS2K1000：分配失败时裸写 UART 输出具体原因并停机。错误路径
                 * 不分配、不使用 println（控制台锁可能被重入）。其他平台保持原语义。
                 */
                #[cfg(feature = "platform-ls2k1000")]
                return Self::ls2k_fatal_alloc_error(layout, zeroed, error);
                #[cfg(not(feature = "platform-ls2k1000"))]
                null_mut()
            }
        }
    }

    #[cfg(feature = "platform-ls2k1000")]
    fn ls2k_fatal_alloc_error(
        layout: Layout,
        zeroed: bool,
        error: HeapError<GlobalPageAllocatorError>,
    ) -> ! {
        /*
         * 最小化裸串口致命输出：绕过 println / CONSOLE_WRITE_LOCK，确保 error
         * 值落在 panic 流之外。先关中断屏蔽中断源，防止输出期间递归分配。
         */
        crate::console::raw::puts("HEAP_FATAL-START alloc-error\n");
        crate::arch::interrupt::disable();
        crate::arch::interrupt::mask_all_sources();

        use core::fmt::Write;
        let mut writer = crate::console::raw::Writer;

        let _ = write!(
            &mut writer,
            "HEAP_FATAL size={} align={} zeroed={} error={:?}\n",
            layout.size(),
            layout.align(),
            zeroed,
            error,
        );

        loop {
            core::hint::spin_loop();
        }
    }

    #[cfg(feature = "platform-ls2k1000")]
    fn ls2k_fatal_heap_none(layout: Layout, zeroed: bool) -> ! {
        crate::console::raw::puts("HEAP_FATAL-START heap-none\n");
        crate::arch::interrupt::disable();
        crate::arch::interrupt::mask_all_sources();

        use core::fmt::Write;
        let mut writer = crate::console::raw::Writer;

        let _ = write!(
            &mut writer,
            "HEAP_FATAL-NONE size={} align={} zeroed={} installed={}\n",
            layout.size(),
            layout.align(),
            zeroed,
            LS2K_HEAP_INSTALLED.load(core::sync::atomic::Ordering::Relaxed),
        );

        loop {
            core::hint::spin_loop();
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
 * LS2K1000：堆是否安装的标志。
 * 仅 ls2k1000 平台参与编译，qemu_virt/riscv64 不受影响。
 */
#[cfg(feature = "platform-ls2k1000")]
static LS2K_HEAP_INSTALLED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/*
 * LS2K1000：编译器层 OOM 兜底。
 *
 * 分配器对 size>0 的请求返回 null 时，编译器生成的
 * `__rust_alloc_error_handler(size, align)` 会进入本函数。最小化输出
 * size/align 后停机，避免在持锁/中断上下文中重入 println。
 * 仅 ls2k1000 平台编译（qemu_virt/riscv64 保持默认 __rdl_oom）。
 */
#[cfg(feature = "platform-ls2k1000")]
#[alloc_error_handler]
fn ls2k_alloc_error_handler(layout: Layout) -> ! {
    crate::console::raw::puts("OOM-HANDLER\n");
    crate::arch::interrupt::disable();
    crate::arch::interrupt::mask_all_sources();

    use core::fmt::Write;
    let mut writer = crate::console::raw::Writer;

    let _ = write!(
        &mut writer,
        "OOM-HANDLER size={} align={}\n",
        layout.size(),
        layout.align(),
    );

    loop {
        core::hint::spin_loop();
    }
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
