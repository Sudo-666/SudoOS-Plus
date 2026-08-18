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
//! - 早期 `sudoos_usb_early_probe()`（只读 MMIO 观测，scheduler 就绪前
//!   安全、失败不 panic、绝不写控制寄存器——HCRESET 会清掉 U-Boot 建立的
//!   端口供电）；晚期 `sudoos_usb_host_start()`（psc 线程内调 `usb_hc_init`，
//!   复位后恢复 root 端口供电）由 post-scheduler 的专用线程驱动；
//! - `sudoos_usb_host_poll()`（= vendored `usb_ehci_interrupt`）1ms 轮询
//!   线程驱动 Port Change / Control / Bulk / MSC 完成（2K1000 无外设中断
//!   基础设施，见 ADR-001 的 poller 决策）；
//! - MSC 就绪后注册 `UsbMscBlockDevice` 为只读 `/dev/sda` + 分区 + devfs
//!   节点，置位 `USB_STORAGE_DONE` 通知 boot idle 上的主启动线程
//!   （`wait_usb_storage_ready`）；
//! - M2 起报告枚举到的 VID:PID。

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use core::time::Duration;

use crate::block::{BlockDevice, BlockError};
use crate::smp::CpuId;
use crate::task::{Completion, WaitQueue};

/// USB 大容量存储阶段完成信号：`usb_init_thread` 跑完 host 启动 + 有界 MSC
/// 等待 + `/dev/sda` 注册后置位。主启动线程经 `wait_usb_storage_ready()` 等。
static USB_STORAGE_READY: Completion = Completion::new();
/// boot idle 可读的完成位：`usb_init_thread` 结束（无论成败）置位。
///
/// `kernel_main` 跑在 boot idle task 上，不能用 `Completion::wait_timeout`
/// （内部走 WaitQueue 阻塞，`prepare_block` 断言 "idle task attempted to
/// block"），故 `wait_usb_storage_ready` 改用 `task::boot_idle_wait_until`
/// 轮询此位。
static USB_STORAGE_DONE: AtomicBool = AtomicBool::new(false);
/// 是否检测到并注册了 MSC 设备（决定 `wait_usb_storage_ready` 的返回值）。
static USB_MSC_DETECTED: AtomicBool = AtomicBool::new(false);

/// M2 首次真机 bring-up 的传输完成驱动：轮询 EHCI 中断状态。
///
/// 2K1000 当前内核无外设中断注册/分发基础设施（trap.rs 只处理 timer/IPI
/// 位，其余一律 handle_unhandled）。`usb_ehci_interrupt()` 本身无锁：读
/// USBSTS → 提交 hpworkq 底部半处理 → 清状态。用一个低开销内核线程按
/// 1 ms 周期驱动它，即可让 iocsem 完成事件触发。真实 IRQ 布线留待 M3。
fn usb_poller() {
    // 等 psc 线程在 `usb_hc_init` 内跑完低层初始化（g_ehci.exclsem 已建、
    // 控制器已复位、端口供电已恢复），再开始轮询。避免把复位前的 USBSTS
    // 残留位误当事件。
    while unsafe { sudoos_usb_hc_ready() } == 0 {
        crate::timer::sleep(Duration::from_millis(1));
    }
    loop {
        // SAFETY: `sudoos_usb_host_poll`（= vendored `usb_ehci_interrupt`）
        // 无参、可在任意线程重复调用（每次处理当前 USBSTS 挂起位并清状态）。
        // 它驱动 Port Change / Control / Bulk / MSC 传输完成。
        unsafe { sudoos_usb_host_poll() };
        crate::timer::sleep(Duration::from_millis(1));
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
        crate::println!("USB: device {vid:04x}:{pid:04x}");
    } else {
        crate::println!("USB: no device within 10s (rc={rc})");
    }
}

/// 早期轮询探针：boot 路径、scheduler 就绪前调用。
///
/// 只做 MMIO 有界轮询探测（M0–M9）与 DMA 池初始化，绝不触碰 task/信号量/
/// 工作队列/`spawn_kernel_thread`。失败只打日志，绝不 panic。线程化初始化
/// 见 `late_start()`——必须先修掉 `USB-glue M0` 后撞 scheduler 未初始化的
/// panic（真机日志 task/mod.rs:2645 是 usbh_initialize 内部 spawn 线程所致）。
pub fn early_probe() {
    probe_build_path();
    dma_pool_init();
    // SAFETY: `sudoos_usb_early_probe` 无参返回 rc；纯 MMIO 有界轮询，
    // 不依赖 scheduler。失败返回负值，不会 panic。
    let rc = unsafe { sudoos_usb_early_probe() };
    crate::println!("USB: early probe rc={rc}");
}

/// 晚期线程化初始化：须在 `task::start_boot_scheduler()` 之后调用。
///
/// 真正的 CherryUSB 宿主栈（psc/hpworkq/lpworkq 线程 + 枚举 + MSC）在此
/// 启动。spawn 一个专用线程执行，避免阻塞 boot 路径；失败只打日志并继续
/// 启动（USB 探测失败可接受，不能把内核挡在 /init 之外）。
pub fn late_start() {
    // 固定 CPU0 作为 system thread（不计入 live_kernel_threads，见
    // TaskKind::is_counted_kernel_thread）。
    crate::task::spawn_system_thread_on(usb_init_thread, CpuId::BOOT);
}

/// 等待 USB 大容量存储阶段完成，返回是否检测到并注册了 MSC 设备。
///
/// 由 boot idle task 上的 `kernel_main` 调用（LS2K1000 竞赛存储路径），
/// 故必须用 `task::boot_idle_wait_until` 轮询而非 `Completion::wait_timeout`
/// 阻塞。`usb_init_thread` 异步完成 host 启动 + 有界 MSC 等待 + `/dev/sda`
/// 注册后置位 `USB_STORAGE_DONE`。超时窗口（12s）大于线程内部 10s MSC 等待，
/// 保证拿到终态。
pub fn wait_usb_storage_ready() -> bool {
    let deadline = crate::time::deadline_after(Duration::from_secs(12));
    let done =
        crate::task::boot_idle_wait_until(deadline, || USB_STORAGE_DONE.load(Ordering::Acquire));
    if !done {
        crate::println!("USB: storage stage not done within 12s");
    }
    USB_MSC_DETECTED.load(Ordering::Acquire)
}

/// 在 psc/hpworkq/lpworkq 线程创建前先建好线程上下文（scheduler 已就绪）。
fn usb_init_thread() {
    // SAFETY: `sudoos_usb_host_start` 无参。此刻 scheduler active、中断
    // 使能，usbh_initialize 内部 spawn 的线程可正常调度。
    let rc = unsafe { sudoos_usb_host_start() };
    crate::println!("USB: cherryusb host start rc={rc}");
    if rc == 0 {
        // 传输完成由轮询线程驱动（无真实 EHCI IRQ）。poller/monitor 均为
        // 常驻线程：固定 CPU0 且为 system thread，不计入竞赛测试计数的
        // live_kernel_threads，也不会在多核下与 CherryUSB 的
        // “关本地中断作为临界区”并发冲突。
        crate::task::spawn_system_thread_on(usb_poller, CpuId::BOOT);
        crate::task::spawn_system_thread_on(usb_monitor, CpuId::BOOT);
        register_msc_storage_if_ready();
    } else {
        crate::println!("USB: host start failed — continuing boot without USB storage");
    }
    // 无论成败都发布“USB 存储阶段完成”。boot idle 的 wait_usb_storage_ready
    // 轮询 USB_STORAGE_DONE；complete_all 保留以兼容任何未来 Completion 等待者。
    USB_STORAGE_DONE.store(true, Ordering::Release);
    USB_STORAGE_READY.complete_all();
}

/// 有界等待 MSC 枚举（poll 线程驱动完成事件），成功后注册 `/dev/sda` +
/// 分区 + 各自 devfs 节点，使 main 的 `mount_sdcard_if_present` 能选中它。
fn register_msc_storage_if_ready() {
    let deadline = crate::time::deadline_after(Duration::from_secs(10));
    loop {
        if unsafe { sudoos_usb_msc_is_ready() } != 0 {
            match register_msc_devices() {
                Ok(()) => {
                    USB_MSC_DETECTED.store(true, Ordering::Release);
                    crate::println!("USB: mass storage registered");
                }
                Err(error) => {
                    crate::println!("USB: register sda failed: {error:?}");
                }
            }
            return;
        }
        if crate::time::deadline_reached(crate::time::now(), deadline) {
            crate::println!("USB: no MSC device within 10s");
            return;
        }
        crate::timer::sleep(Duration::from_millis(50));
    }
}

/// 从 MSC 容量构造只读块设备，注册整盘 `/dev/sda` + 分区 + 各自节点。
fn register_msc_devices() -> Result<(), BlockError> {
    let mut block_count: u64 = 0;
    let mut block_size: u32 = 0;
    if unsafe { sudoos_usb_msc_capacity(&mut block_count, &mut block_size) } != 0
        || block_size == 0
        || block_count == 0
    {
        return Err(BlockError::InvalidArgument);
    }
    let device: Arc<dyn BlockDevice> = Arc::new(UsbMscBlockDevice::new(block_size, block_count));
    crate::block::register_device("sda", Arc::clone(&device))?;
    crate::println!("storage: registered /dev/sda");
    // devfs 节点（用户态 open("/dev/sda") 需要 VFS 节点）。
    crate::fs::install_block_device_node("sda").map_err(|_| BlockError::InvalidArgument)?;
    // 单盘分区扫描：raw ext4 / GPT / MBR（命名 sda1/sda2…）。
    let partitions =
        crate::partition::register_partitions("sda", &device).map_err(|error| match error {
            crate::partition::PartitionError::Block(error) => error,
            _ => BlockError::InvalidArgument,
        })?;
    for name in partitions {
        let _ = crate::fs::install_block_device_node(&name);
        crate::println!("partition: registered /dev/{name}");
    }
    Ok(())
}

/// LS2K1000 USB 大容量存储（只读）块设备。
///
/// 读取路径：VFS → `read_block` → `.nocache_ram` uncached DMA32 bounce →
/// `sudoos_usb_msc_read_blocks`（CherryUSB BOT/SCSI Read10，同步阻塞，由
/// `usb_poller` 轮询驱动完成事件）→ 拷贝回调用方缓冲。EHCI 用 32 位 DMA，
/// 数据缓冲必须物理连续且落低 4GB——`.nocache_ram` 池满足。
pub struct UsbMscBlockDevice {
    block_size: u32,
    block_count: u64,
}

impl UsbMscBlockDevice {
    pub fn new(block_size: u32, block_count: u64) -> Self {
        Self {
            block_size,
            block_count,
        }
    }
}

impl BlockDevice for UsbMscBlockDevice {
    fn block_size(&self) -> usize {
        self.block_size as usize
    }

    fn block_count(&self) -> u64 {
        self.block_count
    }

    fn read_block(&self, block: u64, output: &mut [u8]) -> Result<(), BlockError> {
        let size = self.block_size as usize;
        if output.len() < size {
            return Err(BlockError::BufferTooSmall);
        }
        if block >= self.block_count {
            return Err(BlockError::OutOfRange);
        }
        // DMA32 bounce：控制器写入 uncached 低 4GB 物理缓冲，再拷回调用方。
        // SAFETY: sudoos_usb_alloc 返回 .nocache_ram 池 512B 对齐的可写块。
        let bounce = unsafe { sudoos_usb_alloc(size) };
        if bounce.is_null() {
            return Err(BlockError::MetadataOutOfMemory);
        }
        // SAFETY: bounce 是 size 字节的可写 uncached 缓冲；C 侧校验 LBA/长度
        // 并做单请求保护。
        let rc = unsafe { sudoos_usb_msc_read_blocks(block, 1, bounce.cast(), size as u32) };
        if rc == 0 {
            // bounce 在 uncached 窗口，普通拷贝即取到控制器写入的最新值。
            // SAFETY: 源/目标长度均为 size 且互不重叠（bounce 独立分配）。
            unsafe { core::ptr::copy_nonoverlapping(bounce, output.as_mut_ptr(), size) };
        }
        // SAFETY: bounce 由本函数经 sudoos_usb_alloc 分配。
        unsafe { sudoos_usb_free(bounce) };
        if rc != 0 {
            return Err(BlockError::InvalidArgument);
        }
        Ok(())
    }

    fn write_block(&self, _block: u64, _input: &[u8]) -> Result<(), BlockError> {
        Err(BlockError::DeviceReadOnly)
    }

    fn is_read_only(&self) -> bool {
        true
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
    // 此处位于 cusb::early_probe 的 boot 路径，尚无 USB 线程。
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
///
/// CherryUSB 的 psc/hpworkq/lpworkq 均为常驻线程：固定 CPU0 并作为
/// system thread（`TaskKind::SystemThread` 不计入 `live_kernel_threads`），
/// 避免它们破坏竞赛测试“quiescent counted kernel-thread set”的断言，同时
/// 让 CherryUSB “关本地中断作为临界区”只在 CPU0 上发生，杜绝多核并发访问。
#[unsafe(no_mangle)]
pub extern "C" fn sudoos_usb_thread_spawn(idx: u32) -> i32 {
    let Some(entry) = USB_TRAMPOLINES.get(idx as usize) else {
        return -1;
    };
    let entry = *entry;
    crate::task::spawn_system_thread_on(entry, CpuId::BOOT);
    0
}

unsafe extern "C" {
    /// `kernel/csrc/usb/usb_glue_ls2k1000.c`：CherryUSB 宿主启动。
    fn sudoos_usb_host_start() -> i32;
    /// `kernel/csrc/usb/usb_glue_ls2k1000.c`：构建路径探针。
    fn sudoos_usb_glue_probe() -> u32;
    /// `kernel/csrc/usb/usb_glue_ls2k1000.c`：早期只读 MMIO 探针
    /// （scheduler 就绪前可安全调用，失败返回负值、不 panic）。
    fn sudoos_usb_early_probe() -> i32;
    /// `kernel/csrc/usb/usb_glue_ls2k1000.c`：轮询等待设备枚举并回填 VID/PID。
    fn sudoos_usb_wait_device(timeout_ms: u32, vid: *mut u16, pid: *mut u16) -> i32;
    /// `kernel/csrc/usb/usb_glue_ls2k1000.c`：EHCI 低层初始化是否完成。
    fn sudoos_usb_hc_ready() -> i32;
    /// `kernel/csrc/usb/usb_glue_ls2k1000.c`：EHCI 中断轮询
    /// （= vendored `usb_ehci_interrupt`，USBH_IRQHandler 等价）。
    fn sudoos_usb_host_poll();
    /// `kernel/csrc/usb/usb_glue_ls2k1000.c`：MSC 是否已就绪。
    fn sudoos_usb_msc_is_ready() -> i32;
    /// `kernel/csrc/usb/usb_glue_ls2k1000.c`：回填 MSC 容量。
    fn sudoos_usb_msc_capacity(block_count: *mut u64, block_size: *mut u32) -> i32;
    /// `kernel/csrc/usb/usb_glue_ls2k1000.c`：BOT/SCSI Read10（buffer 必须
    /// 是物理连续的低 4GB DMA32 bounce 缓冲）。
    fn sudoos_usb_msc_read_blocks(lba: u64, count: u32, buffer: *mut u8, buffer_len: u32) -> i32;
}
