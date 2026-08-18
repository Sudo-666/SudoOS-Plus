//! C-USB 桥：LS2K1000 上 CherryUSB 宿主的 Rust 侧接口。
//!
//! C 实现位于 `kernel/csrc/usb/`，由 `kernel/build.rs` 交叉编译为
//! `libsudoos_usb.a` 链进内核（见 docs/decisions/ADR-001）。
//!
//! 本模块：
//! - 提供 `.nocache_ram` uncached DMA 池的 free-list 分配器（EHCI 描述符
//!   与数据缓冲必须落在控制器可见、无缓存陈旧问题的物理内存）；
//! - 向 C OSAL 导出最小原语：DMA 缓冲分配（uncached）/ 控制块分配
//!   （cached，sem/mutex/线程表用，避免 ll/sc 落在 uncached 上）、
//!   时钟毫秒、任务感知延时、WaitQueue 信号量、内核线程 trampoline；
//! - 驱动 `sudoos_usb_init()`（单次初始化，psc 线程内调 `usb_hc_init`）
//!   与 `usb_ehci_interrupt()` 轮询线程（2K1000 无外设中断基础设施，
//!   见 ADR-001 的 poller 决策）；
//! - M2 起报告枚举到的 VID:PID。

use core::sync::atomic::{AtomicI32, Ordering};

use crate::task::WaitQueue;

/// M2 首次真机 bring-up 的传输完成驱动：轮询 EHCI 中断状态。
///
/// 2K1000 当前内核无外设中断注册/分发基础设施（trap.rs 只处理 timer/IPI
/// 位，其余一律 handle_unhandled）。`usb_ehci_interrupt()` 本身无锁：读
/// USBSTS → 提交 hpworkq 底部半处理 → 清状态。用一个低开销内核线程按
/// 1 ms 周期驱动它，即可让 iocsem 完成事件触发。真实 IRQ 布线留待 M3。
fn usb_poller() {
    // 等 psc 线程在 `usb_hc_init` 内跑完低层初始化（g_ehci.exclsem 已建、
    // 控制器已复位），再开始轮询。避免把复位前的 USBSTS 残留位误当事件。
    while unsafe { sudoos_usb_hc_ready() } == 0 {
        crate::timer::sleep(core::time::Duration::from_millis(1));
    }
    loop {
        // SAFETY: `usb_ehci_interrupt` 为 vendored C 函数，无参、可在任意
        // 线程重复调用（每次处理当前 USBSTS 挂起位并清状态）。
        unsafe { usb_ehci_interrupt() };
        crate::timer::sleep(core::time::Duration::from_millis(1));
    }
}

/// 等设备枚举并打印 VID:PID（M2 验收点）。作为独立内核线程运行，不阻塞
/// 启动路径。
fn usb_monitor() {
    let mut vid: u16 = 0;
    let mut pid: u16 = 0;
    // SAFETY: C 侧用 out 参数回填；timeout_ms 为轮询上限。
    let rc = unsafe { sudoos_usb_wait_device(10_000, &mut vid, &mut pid) };
    if rc == 0 {
        crate::println!("USB: detected vid={vid:#06x} pid={pid:#06x}");
    } else {
        crate::println!("USB: no device within 10s (rc={rc})");
    }
}

/// M0 构建路径探针 + M1/M2 CherryUSB 宿主初始化。
pub fn init() {
    probe_build_path();
    dma_pool_init();
    // SAFETY: `sudoos_usb_init` 为 kernel/csrc/usb 交叉编译的 C 函数，无参。
    // 内部只调 `usbh_initialize()`（真实线程版本），psc 线程随后自调
    // `usb_hc_init()`——M1 的显式 hc_init 已移除（避免双初始化）。
    let rc = unsafe { sudoos_usb_init() };
    crate::println!("USB: cherryusb host init rc={rc}");
    if rc == 0 {
        // 传输完成由轮询线程驱动（无真实 EHCI IRQ）。
        crate::task::spawn_kernel_thread(usb_poller);
        crate::task::spawn_kernel_thread(usb_monitor);
    }
}

/// M0 探针：打印 C 胶水返回的哨兵值（0x2a4a0001），证明 C 已链进内核。
pub fn probe_build_path() {
    // SAFETY: `sudoos_usb_glue_probe` 无参、返回普通整数，ABI（lp64s）匹配。
    let value = unsafe { sudoos_usb_glue_probe() };
    crate::println!("USB-glue M0 probe={value:#010x}");
}

// ── .nocache_ram uncached DMA 池 ───────────────────────────────────────

/// `.nocache_ram` 段的动态池区间（linker.ld，uncached DMW 窗口 VMA）。
///
/// 段前半是 EHCI 静态描述符全局（QH/qTD/frame list，linker 按
/// `*(.nocache_ram)` 排在前面），动态池从 `__nocache_dyn_start` 开始。
/// 所有 CPU 访问经 uncached 窗口直达物理内存，控制器读到最新值，无需
/// `cache` 指令维护（binutils 无法汇编，见 ADR-001）。
unsafe extern "C" {
    static __nocache_dyn_start: u8;
    static __nocache_ram_end: u8;
}

/// DMA 块对齐：EHCI 数据缓冲按最大包长对齐（高速 bulk 512B），描述符按
/// 32B。动态缓冲统一 512B 对齐，保守满足控制器要求。
const DMA_ALIGN: usize = 512;
/// 块头大小：payload 在块内偏移 32B，保证 512B 对齐时 payload 仍对齐。
const DMA_HEADER: usize = 32;

fn align_up(value: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    (value + align - 1) & !(align - 1)
}

/// 一级 free list：每个空闲块首字指向下一空闲块地址（0 结束）。块与
/// header 都位于 uncached 段，普通读/写即可，无需原子操作（分配器持锁）。
struct DmaPool {
    free: usize,
    start: usize,
    end: usize,
}

impl DmaPool {
    const fn new() -> Self {
        Self {
            free: 0,
            start: 0,
            end: 0,
        }
    }

    fn init(&mut self) {
        // SAFETY: 符号由链接脚本定义，只读地址。动态池从段内静态描述符
        // 之后（__nocache_dyn_start）开始。
        let start = unsafe { core::ptr::addr_of!(__nocache_dyn_start) as usize };
        let end = unsafe { core::ptr::addr_of!(__nocache_ram_end) as usize };
        self.start = align_up(start, DMA_ALIGN);
        self.end = end;
        let size = self.end.saturating_sub(self.start);
        if size >= DMA_HEADER {
            // 整段作为一个空闲块（首字 0 = 结束）。
            // SAFETY: start 位于 .nocache_ram 段内，可写。
            unsafe { (self.start as *mut usize).write(0) };
            self.free = self.start;
        }
    }

    /// 首次适配分配。返回的 payload 满足 DMA_ALIGN 对齐。
    fn alloc(&mut self, size: usize) -> *mut u8 {
        let total = align_up(DMA_HEADER + size, DMA_ALIGN);
        let mut block = self.free;
        // `prev` 指向“前一块的 next 字段”（首块时指向 self.free）。
        let mut prev = core::ptr::addr_of_mut!(self.free);
        while block != 0 {
            // SAFETY: block 是空闲块，其首字是 next 指针。
            let next = unsafe { (block as *const usize).read() };
            let block_size = if next != 0 {
                next.saturating_sub(block)
            } else {
                self.end.saturating_sub(block)
            };
            if block_size >= total {
                let remainder = block + total;
                if remainder + DMA_HEADER <= block + block_size {
                    // 分裂：剩余部分成为新空闲块。
                    // SAFETY: remainder 仍在段内。
                    unsafe { (remainder as *mut usize).write(next) };
                    // SAFETY: prev 指向可写的 next 字段。
                    unsafe { prev.write(remainder) };
                } else {
                    // SAFETY: 移除当前块，prev 指向下一个。
                    unsafe { prev.write(next) };
                }
                // 记录 payload 大小（调试用；free 不依赖）。
                // SAFETY: block 已从空闲链表摘下，首 8 字节可写。
                unsafe { (block as *mut usize).write(size) };
                let payload = block + DMA_HEADER;
                // SAFETY: payload 位于段内且 32B 对齐（block 512B 对齐）。
                return payload as *mut u8;
            }
            // SAFETY: 移到下一块，prev 指向当前块的 next 字段。
            prev = block as *mut usize;
            block = next;
        }
        core::ptr::null_mut()
    }

    fn free(&mut self, ptr: *mut u8) {
        let block = (ptr as usize) - DMA_HEADER;
        if block < self.start || block >= self.end {
            // 非法指针（非本池）：静默忽略，防误传。
            return;
        }
        let next = self.free;
        // SAFETY: block 已归还，写回空闲链表头。
        unsafe { (block as *mut usize).write(next) };
        self.free = block;
    }
}

static DMA_POOL_LOCK: crate::irq_lock::IrqSpinLock<DmaPool> =
    crate::irq_lock::IrqSpinLock::new_with_class(
        DmaPool::new(),
        crate::lockdep::LockClass::new("usb_dma_pool", crate::lockdep::LockRank::Heap, 0),
    );

/// 初始化 `.nocache_ram` 池（boot 路径调用一次，段内全部零初始化）。
fn dma_pool_init() {
    // SAFETY: `get_mut_unchecked` 要求单 CPU 启动发布窗口内独占访问——
    // 此处位于 cusb::init 的 boot 路径，尚无 USB 线程。
    unsafe { DMA_POOL_LOCK.get_mut_unchecked() }.init();
}

// ── 导出给 C OSAL 的原语 ───────────────────────────────────────────────

/// C 侧 `usb_osal_malloc`：`.nocache_ram` DMA 池分配，返回 uncached 指针。
#[unsafe(no_mangle)]
pub extern "C" fn sudoos_usb_alloc(size: usize) -> *mut u8 {
    DMA_POOL_LOCK.lock().alloc(size)
}

/// C 侧 `usb_osal_free`：归还 DMA 池。
#[unsafe(no_mangle)]
pub extern "C" fn sudoos_usb_free(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    DMA_POOL_LOCK.lock().free(ptr);
}

/// C 侧控制块分配：普通缓存内核堆（sem/mutex/线程表用）。
///
/// 与 DMA 池物理隔离，防止 cached/uncached 页别名污染：控制块上跑 ll/sc
/// 原子（IrqSpinLock/WaitQueue），绝不能落在 uncached 窗口。
#[unsafe(no_mangle)]
pub extern "C" fn sudoos_usb_alloc_ctrl(size: usize) -> *mut u8 {
    let Some(total) = size.checked_add(8) else {
        return core::ptr::null_mut();
    };
    let Ok(layout) = core::alloc::Layout::from_size_align(total, 8) else {
        return core::ptr::null_mut();
    };
    // SAFETY: layout 非零且 8 字节对齐，内核全局分配器已建立。
    let ptr = unsafe { alloc::alloc::alloc(layout) };
    if ptr.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: ptr 由全局分配器返回，前 8 字节可写。
    unsafe { ptr.cast::<usize>().write(size) };
    // SAFETY: ptr + 8 仍在 total 分配区内。
    unsafe { ptr.add(8) }
}

/// 释放 `sudoos_usb_alloc_ctrl` 返回的指针。
#[unsafe(no_mangle)]
pub extern "C" fn sudoos_usb_free_ctrl(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: 前 8 字节是 sudoos_usb_alloc_ctrl 写入的 size 头。
    let size = unsafe { ptr.sub(8).cast::<usize>().read() };
    let Ok(layout) = core::alloc::Layout::from_size_align(size + 8, 8) else {
        return;
    };
    // SAFETY: ptr.sub(8) 是 alloc 返回的原始指针，layout 与其匹配。
    unsafe { alloc::alloc::dealloc(ptr.sub(8), layout) };
}

/// C 侧 `usb_osal_get_tick`：相对时钟源的毫秒数（用于相对计时）。
#[unsafe(no_mangle)]
pub extern "C" fn sudoos_usb_get_tick_ms() -> u32 {
    let elapsed = crate::time::now().duration_since(crate::time::MonotonicInstant::from_cycles(0));
    elapsed.as_millis() as u32
}

/// C 侧 `usb_osal_msleep` 的忙碌等待版本（boot 上下文用，此时无调度器）。
#[unsafe(no_mangle)]
pub extern "C" fn sudoos_usb_msleep(ms: u32) {
    let start = crate::time::now();
    let wait = core::time::Duration::from_millis(ms as u64);
    while crate::time::now().duration_since(start) < wait {
        core::hint::spin_loop();
    }
}

/// C 侧 `usb_osal_msleep` 的任务睡眠版本（C 线程内用，出让 CPU）。
#[unsafe(no_mangle)]
pub extern "C" fn sudoos_usb_sleep_ms(ms: u32) {
    crate::timer::sleep(core::time::Duration::from_millis(ms as u64));
}

/// C 侧 `printf`/日志的串口输出：把 C 字符串打到内核串口。
#[unsafe(no_mangle)]
pub extern "C" fn sudoos_usb_log_str(ptr: *const u8) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: ptr 是 C 侧 NUL 结尾的字符串（printf 的栈缓冲）。
    let cstr = unsafe { core::ffi::CStr::from_ptr(ptr.cast()) };
    if let Ok(text) = cstr.to_str() {
        crate::println!("{text}");
    }
}

// ── WaitQueue 信号量（控制块在缓存堆，禁 ll/sc 落 uncached）───────────

/// C 信号量控制块。count + 等待队列，全部普通缓存内存。
#[repr(C)]
struct UsbSem {
    count: AtomicI32,
    queue: WaitQueue,
}

/// sem/mutex 超时：挂起的控制器最终解阻塞（比无限等好诊断）。
const SEM_TIMEOUT_MS: u32 = 60_000;

/// C `usb_osal_sem_create`：分配缓存控制块并初始化。
#[unsafe(no_mangle)]
pub extern "C" fn sudoos_usb_sem_create(initial: i32) -> *mut UsbSem {
    let ptr = sudoos_usb_alloc_ctrl(core::mem::size_of::<UsbSem>()) as *mut UsbSem;
    if ptr.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: ptr 是刚分配的未初始化内存，完整初始化所有字段。
    unsafe {
        ptr.write(UsbSem {
            count: AtomicI32::new(initial),
            queue: WaitQueue::new(),
        });
    }
    ptr
}

/// C `usb_osal_sem_delete`。
#[unsafe(no_mangle)]
pub extern "C" fn sudoos_usb_sem_delete(sem: *mut UsbSem) {
    if sem.is_null() {
        return;
    }
    // SAFETY: sem 由 sudoos_usb_sem_create 分配，控制块不再使用。
    sudoos_usb_free_ctrl(sem.cast::<u8>());
}

/// C `usb_osal_sem_take`：有界阻塞。返回 0 成功，-1 超时。
#[unsafe(no_mangle)]
pub extern "C" fn sudoos_usb_sem_take(sem: *mut UsbSem, timeout_ms: u32) -> i32 {
    // SAFETY: sem 指向有效的 UsbSem。
    let s = unsafe { &*sem };
    let deadline =
        crate::time::deadline_after(core::time::Duration::from_millis(timeout_ms as u64));
    let outcome = s
        .queue
        .wait_until_deadline(deadline, || s.count.load(Ordering::Acquire) > 0);
    if outcome == crate::task::WaitOutcome::TimedOut {
        return -1;
    }
    s.count.fetch_sub(1, Ordering::AcqRel);
    0
}

/// C `usb_osal_sem_give`。
#[unsafe(no_mangle)]
pub extern "C" fn sudoos_usb_sem_give(sem: *mut UsbSem) -> i32 {
    // SAFETY: sem 指向有效的 UsbSem。
    let s = unsafe { &*sem };
    s.count.fetch_add(1, Ordering::Release);
    s.queue.wake_one();
    0
}

// ── 内核线程 trampoline（KernelThreadEntry = fn()，idx 经宏烘焙）───────

/// CherryUSB 线程槽位上限（psc + hpworkq + lpworkq + 未来 hub/监视器）。
const USB_THREAD_SLOTS: usize = 8;

unsafe extern "C" {
    /// C 侧线程体分派：`g_thread_ctx[idx].entry(args)`（usb_osal_sudoos.c）。
    fn sudoos_usb_thread_entry(idx: u32) -> !;
}

// KernelThreadEntry = fn()（无参），无法携带槽位；每个槽位一个显式
// trampoline，把 idx 烘焙进函数体，杜绝“最后一个 spawn”全局的竞态。
#[allow(non_snake_case)]
fn usb_trampoline_0() {
    // SAFETY: idx 是已注册的线程槽，C 侧维护 entry/args。
    unsafe { sudoos_usb_thread_entry(0) }
}
#[allow(non_snake_case)]
fn usb_trampoline_1() {
    unsafe { sudoos_usb_thread_entry(1) }
}
#[allow(non_snake_case)]
fn usb_trampoline_2() {
    unsafe { sudoos_usb_thread_entry(2) }
}
#[allow(non_snake_case)]
fn usb_trampoline_3() {
    unsafe { sudoos_usb_thread_entry(3) }
}
#[allow(non_snake_case)]
fn usb_trampoline_4() {
    unsafe { sudoos_usb_thread_entry(4) }
}
#[allow(non_snake_case)]
fn usb_trampoline_5() {
    unsafe { sudoos_usb_thread_entry(5) }
}
#[allow(non_snake_case)]
fn usb_trampoline_6() {
    unsafe { sudoos_usb_thread_entry(6) }
}
#[allow(non_snake_case)]
fn usb_trampoline_7() {
    unsafe { sudoos_usb_thread_entry(7) }
}

static USB_TRAMPOLINES: [crate::task::KernelThreadEntry; USB_THREAD_SLOTS] = [
    usb_trampoline_0,
    usb_trampoline_1,
    usb_trampoline_2,
    usb_trampoline_3,
    usb_trampoline_4,
    usb_trampoline_5,
    usb_trampoline_6,
    usb_trampoline_7,
];

/// C `usb_osal_thread_create`：按槽位生成 SudoOS 内核线程。
#[unsafe(no_mangle)]
pub extern "C" fn sudoos_usb_thread_spawn(idx: u32) -> i32 {
    let Some(entry) = USB_TRAMPOLINES.get(idx as usize) else {
        return -1;
    };
    let entry = *entry;
    crate::task::spawn_kernel_thread(entry);
    0
}

unsafe extern "C" {
    /// `kernel/csrc/usb/usb_glue_ls2k1000.c`：CherryUSB 宿主初始化。
    fn sudoos_usb_init() -> i32;
    /// `kernel/csrc/usb/usb_glue_ls2k1000.c`：构建路径探针。
    fn sudoos_usb_glue_probe() -> u32;
    /// `kernel/csrc/usb/usb_glue_ls2k1000.c`：轮询等待设备枚举并回填 VID/PID。
    fn sudoos_usb_wait_device(timeout_ms: u32, vid: *mut u16, pid: *mut u16) -> i32;
    /// `kernel/csrc/usb/usb_glue_ls2k1000.c`：EHCI 低层初始化是否完成。
    fn sudoos_usb_hc_ready() -> i32;
    /// vendored `usb_ehci.c`：EHCI 中断处理（M2 轮询驱动）。
    fn usb_ehci_interrupt();
}
