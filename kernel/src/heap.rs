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
        /*
         * LS2K1000 真机调试：调用方返回地址捕获 + 计数 + 环形缓冲记录必须放在
         * 本函数最顶部、任何提前返回之前。上一轮真机 OOM size=176 未进环形
         * （count=89 = ring=89），说明失败分配命中了 `layout.size()==0` 的
         * 提前返回路径（该路径在旧代码里位于记录之前）——这是 LoongArch
         * Layout/Alignment ABI 在 GlobalAlloc 边界把真实 size 读成 0 的表现。
         * 把记录移到最顶，下次 OOM 时环形“最后一条”即失败分配的调用方 $ra。
         * 仅 ls2k1000 平台编译。
         */
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

        /*
         * LS2K1000 真机调试：把本次分配的调用方返回地址与请求大小写入环形
         * 缓冲（volatile 写 + 固定静态数组，不分配、不加锁）。即使 size==0
         * 的提前返回路径也会在此记录，OOM-HANDLER 里“最后一条”即失败分配。
         */
        #[cfg(feature = "platform-ls2k1000")]
        {
            let ring_pos = ALLOC_RING_POS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            let slot = ring_pos % ALLOC_RING_CAP;
            // SAFETY: slot 落在 [0, ALLOC_RING_CAP)，静态数组固定大小。
            unsafe {
                core::ptr::write_volatile(
                    ALLOC_RING_RA.as_ptr().add(slot) as *mut usize,
                    caller,
                );
                core::ptr::write_volatile(
                    ALLOC_RING_SIZE.as_ptr().add(slot) as *mut usize,
                    layout.size(),
                );
            }
        }

        /*
         * LS2K1000 真机调试：防御性清洗对齐。LoongArch 上 alloc crate 的
         * 2 字结构体跨函数传参会把 align 读成内核 static 地址（如
         * lockdep::MAX_IRQ_OFF_CYCLES=0x905aa9d0），导致分配被错误拒绝。
         * 非法 align 钳到 8；合法对齐原样通过。仅 ls2k1000 平台编译，
         * qemu_virt/riscv64 保持原语义。
         */
        #[cfg(feature = "platform-ls2k1000")]
        let layout = {
            let align = layout.align();
            if align.is_power_of_two() && align <= (1usize << 22) {
                layout
            } else {
                // SAFETY: (size, 8) 总是合法 Layout；对齐损坏时钳到 8。
                unsafe { Layout::from_size_align_unchecked(layout.size(), 8) }
            }
        };

        if layout.size() == 0 {
            return null_mut();
        }

        let mut slot = self.heap.lock();

        let Some(heap) = slot.as_mut() else {
            /*
             * LS2K1000 真机调试：堆 Option 为 None——要么从未安装，要么
             * GLOBAL_HEAP 静态被写坏。此时不再返回 null 走 handle_alloc_error
             * （其 println 可能被控制台锁吞掉），而是裸写 UART 输出致命信息后停机。
             * 其他平台保持原语义（返回 null）。
             */
            #[cfg(feature = "platform-ls2k1000")]
            return Self::ls2k_fatal_heap_none(layout, zeroed, caller);
            #[cfg(not(feature = "platform-ls2k1000"))]
            return null_mut();
        };

        match heap.allocate(layout, zeroed) {
            Ok(pointer) => pointer.as_ptr(),
            Err(error) => {
                /*
                 * LS2K1000 真机调试：分配失败时裸写 UART 输出具体原因并停机，
                 * 不再返回 null。错误路径不分配、不使用 println（控制台锁可能
                 * 在持锁上下文中被重入），信息直接进入串口、无法被截断吞掉。
                 * 其他平台保持原语义（返回 null → handle_alloc_error panic）。
                 */
                #[cfg(feature = "platform-ls2k1000")]
                return Self::ls2k_fatal_alloc_error(layout, zeroed, caller, error);
                #[cfg(not(feature = "platform-ls2k1000"))]
                null_mut()
            }
        }
    }

    #[cfg(feature = "platform-ls2k1000")]
    fn ls2k_fatal_alloc_error(
        layout: Layout,
        zeroed: bool,
        caller: usize,
        error: HeapError<GlobalPageAllocatorError>,
    ) -> ! {
        /*
         * 裸串口致命输出：绕过 println / CONSOLE_WRITE_LOCK，确保 error 值
         * 落在 panic 流之外且物理上不可被串口截断。先关中断屏蔽中断源，
         * 防止输出期间定时器/IPI 再次分配造成递归。
         */
        // 首个动作裸写一个哨兵标记：即使后续 SizeClass::for_layout /
        // error Debug 格式化 / 中断屏蔽本身在板上崩溃，也能确认走到了这里。
        crate::console::raw::puts("HEAP_FATAL-START alloc-error\n");
        crate::arch::interrupt::disable();
        crate::arch::interrupt::mask_all_sources();

        use core::fmt::Write;
        let mut writer = crate::console::raw::Writer;

        // 176B/align≤256 → slab class 256；超出 2048 上限 → large 路径(class=0)。
        let class = myos_mm::SizeClass::for_layout(layout)
            .map(|size_class| size_class.size())
            .unwrap_or(0);

        let _ = write!(
            &mut writer,
            "HEAP_FATAL n={} size={} align={} zeroed={} caller={:#x} class={} error={:?}",
            LS2K_ALLOC_COUNT.load(core::sync::atomic::Ordering::Relaxed),
            layout.size(),
            layout.align(),
            zeroed,
            caller,
            class,
            error,
        );
        let _ = write!(
            &mut writer,
            " free_pages={:?}\n",
            crate::page_alloc::total_free_pages(),
        );

        loop {
            core::hint::spin_loop();
        }
    }

    #[cfg(feature = "platform-ls2k1000")]
    fn ls2k_fatal_heap_none(layout: Layout, zeroed: bool, caller: usize) -> ! {
        // 哨兵标记：见 ls2k_fatal_alloc_error 注释。
        crate::console::raw::puts("HEAP_FATAL-START heap-none\n");
        crate::arch::interrupt::disable();
        crate::arch::interrupt::mask_all_sources();

        use core::fmt::Write;
        let mut writer = crate::console::raw::Writer;

        let field = &GLOBAL_HEAP.heap as *const _ as usize;
        let words = unsafe { core::slice::from_raw_parts(field as *const usize, 12) };

        let _ = write!(
            &mut writer,
            "HEAP_FATAL-NONE n={} size={} align={} zeroed={} caller={:#x} field={:#x} installed={}",
            LS2K_ALLOC_COUNT.load(core::sync::atomic::Ordering::Relaxed),
            layout.size(),
            layout.align(),
            zeroed,
            caller,
            field,
            LS2K_HEAP_INSTALLED.load(core::sync::atomic::Ordering::Relaxed),
        );
        let _ = write!(
            &mut writer,
            " words {:#x} {:#x} {:#x} {:#x} {:#x} {:#x} {:#x} {:#x} {:#x} {:#x} {:#x} {:#x}\n",
            words[0], words[1], words[2], words[3], words[4], words[5],
            words[6], words[7], words[8], words[9], words[10], words[11],
        );

        loop {
            core::hint::spin_loop();
        }
    }
}

/*
 * LS2K1000 真机调试：try_* / realloc 覆写用的辅助函数。
 *
 * 真机 OOM 证据链：失败分配（size=176）从未进入本分配器的 allocate()——
 * 环形缓冲停在 count=89，而 ring 已放在 allocate() 最顶。原因是 core 的默认
 * `GlobalAlloc::try_alloc`（RawVec 增长路径走它）内部读取 `layout.align()`
 * 在 LoongArch 上得到垃圾值（真机为 0x90000000905A4AC8，一个内核 static
 * 地址），try_alloc 据此走 `alloc_slow_path` 且 align>isize::MAX 校验直接
 * Err——分配请求到不了本分配器。本组函数让 try_* 覆写先在自己代码上下文里
 * 校验 align：可信则走已验证可用的 alloc 路径（绕过 core 损坏逻辑，即修复）；
 * 不可信则一次性裸串口输出失败分配的调用方 $ra（即 176B 调用点），随后照样
 * 路由到 alloc——heap.allocate 在 SizeClass::for_layout 出错时会落到 large/
 * buddy 路径（不依赖 align），故即使 align 损坏分配仍能成功。
 * 仅 ls2k1000 平台编译。
 */
#[cfg(feature = "platform-ls2k1000")]
static LS2K_TRY_BAD_ALIGN_PRINTED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// 捕获调用方返回地址。单输出寄存器 + 非 volatile：与 allocate() 顶部 ring
/// 捕获同一写法，已被真机 89 条正确 ra 验证（多输出才会被 LLVM 合并）。
#[cfg(feature = "platform-ls2k1000")]
fn ls2k_capture_caller() -> usize {
    let mut value: usize;
    // SAFETY: 读取 $ra 不改动任何机器状态。
    unsafe {
        core::arch::asm!("or {}, $ra, $zero", out(reg) value, options(nomem, nostack));
    }
    value
}

/// layout.align() 在本代码上下文是否可信：2 的幂且 ≤ 4 MiB。
/// Layout 契约允许的对齐上限远大于此，4 MiB 覆盖一切真实用途。
#[cfg(feature = "platform-ls2k1000")]
fn ls2k_layout_align_ok(layout: Layout) -> bool {
    let align = layout.align();
    align.is_power_of_two() && align <= (1usize << 22)
}

/// try_* 入口读到不可信 align 时的一次性报告（不停机）：裸串口输出路径名、
/// 失败分配的调用方 $ra、size/align 原始值。此后静默，避免每次分配刷屏。
#[cfg(feature = "platform-ls2k1000")]
fn ls2k_try_bad_align_report(which: &'static str, layout: Layout, caller: usize) {
    if LS2K_TRY_BAD_ALIGN_PRINTED.swap(true, core::sync::atomic::Ordering::Relaxed) {
        return;
    }
    // 只输出一次、不改中断状态：若本次调用恰在中断上下文，禁用中断会挂死。
    crate::console::raw::puts("TRY_ALLOC_BAD_ALIGN\n");
    use core::fmt::Write;
    let mut writer = crate::console::raw::Writer;
    let _ = write!(
        &mut writer,
        "TRY_ALLOC_BAD_ALIGN path={} n={} size={} align={} caller={:#x}\n",
        which,
        LS2K_ALLOC_COUNT.load(core::sync::atomic::Ordering::Relaxed),
        layout.size(),
        layout.align(),
        caller,
    );
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

    /*
     * LS2K1000 真机调试：覆写 GlobalAlloc::realloc。
     *
     * 真机 OOM 证据链：失败分配（size=176）从未进入本分配器的 allocate()——
     * 环形缓冲停在 count=89，而 ring 已放在 allocate() 最顶。该 nightly
     * （2025-01-18）GlobalAlloc 无 try_* 方法，RawVec 增长走的是稳定的
     * `realloc`；core 的默认 realloc 内部读取 `layout.align()` 在 LoongArch
     * 上损坏（真机为 0x90000000905A4AC8，一个内核 static 地址），内部分配
     * 逻辑判失败返回 null，请求到不了本分配器。本覆写：对齐不可信则一次性
     * 裸串口报告失败分配的调用方 $ra（即 176B 调用点）并把对齐钳到 8；
     * 随后走本分配器已验证的 allocate()（其 heap.allocate 在 SizeClass::
     * for_layout 出错时落到 large/buddy 路径、不依赖对齐，故即使对齐损坏
     * 分配仍能成功），成功则复制内容并释放旧块。
     * 仅 ls2k1000 平台编译；qemu_virt/riscv64 保持 core 默认。
     */
    #[cfg(feature = "platform-ls2k1000")]
    unsafe fn realloc(
        &self,
        pointer: *mut u8,
        layout: Layout,
        new_size: usize,
    ) -> *mut u8 {
        let caller = ls2k_capture_caller();
        let align = if ls2k_layout_align_ok(layout) {
            layout.align()
        } else {
            ls2k_try_bad_align_report("realloc", layout, caller);
            // 对齐损坏时钳到最小安全值，避免垃圾 align 落入错误 slab 类。
            8usize
        };
        // SAFETY: Layout 契约允许任意 (size, align) 组合；align 已校验或钳位。
        let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, align) };
        let new_ptr = self.allocate(new_layout, false);
        if new_ptr.is_null() {
            return null_mut();
        }
        let old_size = layout.size();
        let copy_len = core::cmp::min(old_size, new_size);
        // SAFETY: 新旧块都来自本分配器且新块刚分配、与旧块无重叠。
        unsafe {
            core::ptr::copy_nonoverlapping(pointer, new_ptr, copy_len);
        }
        // SAFETY: 释放旧块。
        unsafe {
            self.dealloc(pointer, layout);
        }
        new_ptr
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
 * LS2K1000 真机调试：OOM 调用栈快照。
 *
 * run 10（2026-08-09）证明 2048B 实时栈 dump 的 ~70% 是 alloc_error_handler
 * 自己打印 RING/栈时的 fmt 帧（bool/usize/LowerHex fmt），把真正触发 OOM 的
 * 调用者帧埋在了 sp+0x400 以下，addr2line 只能还原出 kernel_main /
 * reprogram_local / KernelGlobalAllocator::allocate（多为更早成功分配残留的
 * 陈旧帧），拿不到 176B 分配点。本快照在 handler 入口（任何 puts/fmt 之前）
 * 把栈顶 2048 字节拷进静态数组，打印阶段再读快照——栈被 fmt 污染不影响取证。
 * 仅 ls2k1000 平台编译，qemu_virt/riscv64 不受影响。
 */
// `static mut`：本 handler 每 boot 只进入一次（进入后无限停机），快照写入与
// 打印阶段均在该单线程临界区，无并发写风险；仅 ls2k1000 平台编译。
#[cfg(feature = "platform-ls2k1000")]
const OOM_STACK_SNAP_LEN: usize = 256; // 256 字 = 2048 字节
#[cfg(feature = "platform-ls2k1000")]
static mut OOM_STACK_SNAP: [usize; OOM_STACK_SNAP_LEN] = [0; OOM_STACK_SNAP_LEN];
#[cfg(feature = "platform-ls2k1000")]
static OOM_STACK_SNAP_SP: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/*
 * LS2K1000 真机调试：分配调用点环形缓冲。
 *
 * allocate() 每次进入（size>0）都把调用方返回地址 $ra 与请求大小写入固定
 * 环形缓冲（volatile 写，LTO 无法消除）。分配失败走到 OOM-HANDLER 时，
 * 环形缓冲的“最后一条”正是失败的那次分配：其 ra 经 addr2line 即可还原
 * 176 字节分配的精确调用点，绕开 OOM handler 自身帧对原始栈指纹的污染
 * （上一轮栈转储里全是 handler 自己 write! 的 core::fmt 帧）。
 * 仅 ls2k1000 平台编译。
 */
#[cfg(feature = "platform-ls2k1000")]
const ALLOC_RING_CAP: usize = 128;

#[cfg(feature = "platform-ls2k1000")]
static ALLOC_RING_RA: [usize; ALLOC_RING_CAP] = [0; ALLOC_RING_CAP];

#[cfg(feature = "platform-ls2k1000")]
static ALLOC_RING_SIZE: [usize; ALLOC_RING_CAP] = [0; ALLOC_RING_CAP];

#[cfg(feature = "platform-ls2k1000")]
static ALLOC_RING_POS: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/*
 * LS2K1000 真机调试：自定义 alloc_error_handler。
 *
 * 当全局分配器对 size>0 的请求返回 null 时，编译器生成的
 * `__rust_alloc_error_handler(size, align)` 会调用本函数（而不是默认的
 * __rdl_oom panic）。这意味着无论分配失败来自 KernelGlobalAllocator::allocate
 * 的哪一条路径（slab Err、堆 Option 为 None、甚至是 LTO 改写后直接返回 null
 * 的路径），都能在这里裸串口留下 size/align/$ra/原始调用栈指纹，不会被
 * 控制台锁、LTO 优化或串口截断吞掉。
 *
 * 上一轮真机（0xdeadbeef，176 字节 OOM）走到了默认 __rdl_oom 而非
 * HEAP_FATAL：表明 allocate() 的致命分支在该构建里未被失败路径命中。
 * 本 handler 是从编译器层面兜底：凡是 null 返回，必达此处。
 *
 * 仅 ls2k1000 平台编译（qemu_virt/riscv64 保持默认 __rdl_oom）。
 */
#[cfg(feature = "platform-ls2k1000")]
#[alloc_error_handler]
fn ls2k_alloc_error_handler(layout: Layout) -> ! {
    // 首动作捕获 $ra：必须放在任何调用之前，否则 $ra 已被 puts 等覆盖
    // （上一轮 ra=0x902e77ac 正是 puts 的返回地址，不是真实调用者）。
    // 同时原始捕获 a0/a1：编译器经过 __rust_alloc_error_handler -> __rg_oom
    // 两次 a0/a1 swap 后才进入本函数，layout.size()/align() 可能已是坏值；
    // 原始寄存器与 layout 字段对照可确认 size=176 是否可信。
    // 注意：三个输出必须强制分配到 $r12/$r13/$r14（t0/t1/t2），不能交给 LLVM
    // 任意选——上一轮它把第一个输出选到了 $a0，第二条 `or X, $a0, $zero` 文本上
    // 读 $a0 就变成了读第一条刚写出的值，最终 raw_a0=raw_a1=ra=0x902e7ed4。
    // 强制固定寄存器后，第二条读的是未被污染的原始入口参数 $a0。
    let mut ra: usize = 0;
    let mut raw_a0: usize = 0;
    let mut raw_a1: usize = 0;
    // SAFETY: 读取当前调用方返回地址与入口参数寄存器，不改动任何机器状态。
    // 固定寄存器输出时模板必须直接写寄存器名（不能用 {N} 占位符）。
    unsafe {
        core::arch::asm!(
            "or $r12, $ra, $zero",
            "or $r13, $a0, $zero",
            "or $r14, $a1, $zero",
            out("$r12") ra,
            out("$r13") raw_a0,
            out("$r14") raw_a1,
            options(nomem, nostack),
        );
    }

    // 栈快照：必须在任何 puts/write!（会推 fmt 帧）之前抓取，否则调用者栈区
    // 会被本 handler 的打印帧覆盖。抓取后即使后续打印污染栈，取证不受影响。
    let mut snap_sp: usize = 0;
    unsafe {
        core::arch::asm!("or {}, $sp, $zero", out(reg) snap_sp, options(nomem, nostack));
    }
    OOM_STACK_SNAP_SP.store(snap_sp, core::sync::atomic::Ordering::Relaxed);
    // 用 addr_of_mut! 裸指针写，避免 `&mut static mut`（static_mut_refs 为 deny）。
    // SAFETY: 本 handler 每 boot 只执行一次（进入后无限停机），无并发写者。
    let snap_ptr = core::ptr::addr_of_mut!(OOM_STACK_SNAP) as *mut usize;
    for index in 0..OOM_STACK_SNAP_LEN {
        let addr = snap_sp.wrapping_add(index * core::mem::size_of::<usize>());
        // SAFETY: 内核栈 64 KiB 且本函数位于栈顶附近，读取前 2048 字节安全。
        unsafe {
            core::ptr::write_volatile(
                snap_ptr.add(index),
                core::ptr::read_volatile(addr as *const usize),
            );
        }
    }

    // 第二动作裸串口哨兵：即使后续读取 $sp 栈指纹触发异常也能确认到达。
    crate::console::raw::puts("OOM-HANDLER\n");

    crate::arch::interrupt::disable();
    crate::arch::interrupt::mask_all_sources();

    use core::fmt::Write;
    let mut writer = crate::console::raw::Writer;

    let _ = write!(
        &mut writer,
        "OOM-HANDLER size={} align={} ra={:#x} raw_a0={:#x} raw_a1={:#x} count={} installed={}\n",
        layout.size(),
        layout.align(),
        ra,
        raw_a0,
        raw_a1,
        LS2K_ALLOC_COUNT.load(core::sync::atomic::Ordering::Relaxed),
        LS2K_HEAP_INSTALLED.load(core::sync::atomic::Ordering::Relaxed),
    );

    // 环形缓冲转储：allocate() 每次进入都记录 (ra,size)。“最后一条”即失败
    // 的那次分配（本 OOM 由它触发），其 ra 就是 176 字节分配的精确调用点。
    let ring_total = ALLOC_RING_POS.load(core::sync::atomic::Ordering::Relaxed);
    let _ = write!(
        &mut writer,
        "RING total={} cap={}\n",
        ring_total, ALLOC_RING_CAP,
    );
    let ring_start = ring_total.saturating_sub(ALLOC_RING_CAP);
    for index in ring_start..ring_total {
        let slot = index % ALLOC_RING_CAP;
        // SAFETY: slot 在 [0, CAP)，静态数组固定大小。
        let entry_ra = unsafe { core::ptr::read_volatile(ALLOC_RING_RA.as_ptr().add(slot)) };
        let entry_size =
            unsafe { core::ptr::read_volatile(ALLOC_RING_SIZE.as_ptr().add(slot)) };
        let _ = write!(
            &mut writer,
            "R[{:03}] sz={} ra={:#x}\n",
            index, entry_size, entry_ra,
        );
    }

    // 原始栈指纹：打印入口时快照的 OOM_STACK_SNAP（handler 帧 + 调用者链），
    // 用 `loongarch64-linux-gnu-addr2line -f -C -i -e kernel-ls2k1000 <addr>`
    // 逐字解析即可还原 分配点 -> __rust_alloc -> __rust_alloc_error_handler 链。
    // 快照在入口处已抓取，此处只读静态，不再读实时栈（已被 fmt 帧污染）。
    let sp = OOM_STACK_SNAP_SP.load(core::sync::atomic::Ordering::Relaxed);
    let _ = write!(&mut writer, "OOM sp={:#x} stack-snapshot:\n", sp);
    // SAFETY: 快照已在入口写入且本 handler 内无其他写入者，只读访问。
    let snap_ptr = core::ptr::addr_of!(OOM_STACK_SNAP) as *const usize;
    for offset in (0..OOM_STACK_SNAP_LEN).step_by(2) {
        // SAFETY: offset 在 [0, LEN)，数组固定大小。
        let w0 = unsafe { core::ptr::read_volatile(snap_ptr.add(offset)) };
        let w1 = unsafe { core::ptr::read_volatile(snap_ptr.add(offset + 1)) };
        let _ = write!(
            &mut writer,
            "  {:#18x}: {:#18x} {:#18x}\n",
            sp.wrapping_add(offset * core::mem::size_of::<usize>()),
            w0,
            w1,
        );
    }

    // 汇总扫描：从栈中收集所有落在 .text 段 [0x9000000090200000, 0x90000000902ee000)
    // 的字，按出现顺序打印。它们是调用链上的返回地址/函数指针，离线用
    // `loongarch64-linux-gnu-addr2line -f -C -i -e kernel-ls2k1000 <addr>`
    // 即可还原真实分配调用方。
    let text_start: usize = 0x9000000090200000;
    let text_end: usize = 0x90000000902ee000;
    let _ = write!(&mut writer, "OOM code-words:\n");
    let mut code_seen = 0usize;
    // SAFETY: 快照只在入口写入，此处只读访问。
    let snap_ptr = core::ptr::addr_of!(OOM_STACK_SNAP) as *const usize;
    for index in 0..OOM_STACK_SNAP_LEN {
        // SAFETY: index 在 [0, LEN)，数组固定大小。
        let word = unsafe { core::ptr::read_volatile(snap_ptr.add(index)) };
        if word >= text_start && word < text_end {
            let _ = write!(
                &mut writer,
                "  [{:03}] {:#18x} <- at sp+{:#x}\n",
                code_seen, word, index * core::mem::size_of::<usize>(),
            );
            code_seen += 1;
        }
    }
    let _ = write!(&mut writer, "OOM code-words total={}\n", code_seen);

    let _ = write!(
        &mut writer,
        "OOM heap-field={:#x} installed={}\n",
        &GLOBAL_HEAP.heap as *const _ as usize,
        LS2K_HEAP_INSTALLED.load(core::sync::atomic::Ordering::Relaxed),
    );

    loop {
        core::hint::spin_loop();
    }
}

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
