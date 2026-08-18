//! C-USB 桥：LS2K1000 上 CherryUSB 宿主的 Rust 侧接口。
//!
//! C 实现位于 `kernel/csrc/usb/`，由 `kernel/build.rs` 交叉编译为
//! `libsudoos_usb.a` 链进内核（见 docs/decisions/ADR-001）。
//!
//! 本模块：
//! - 向 C OSAL 导出最小原语（内存分配 / 时钟毫秒 / 忙等延时）；
//! - 驱动 `sudoos_usb_init()` 启动 CherryUSB 宿主；
//! - M2/M3 起暴露容量 / 块读取，供块设备层包装成 `/dev/sda`。

use core::alloc::Layout;

use crate::time::MonotonicInstant;

/// M0 构建路径探针 + M1 CherryUSB 宿主初始化。
pub fn init() {
    probe_build_path();
    // SAFETY: `sudoos_usb_init` 为 kernel/csrc/usb 交叉编译的 C 函数，无参。
    let rc = unsafe { sudoos_usb_init() };
    crate::println!("USB: cherryusb host init rc={rc}");
}

/// M0 探针：打印 C 胶水返回的哨兵值（0x2a4a0001），证明 C 已链进内核。
pub fn probe_build_path() {
    // SAFETY: `sudoos_usb_glue_probe` 无参、返回普通整数，ABI（lp64s）匹配。
    let value = unsafe { sudoos_usb_glue_probe() };
    crate::println!("USB-glue M0 probe={value:#010x}");
}

// ── 导出给 C OSAL 的最小原语 ─────────────────────────────────────────

/// C 侧 `usb_osal_malloc`：内核堆分配，前 8 字节记录真实 size 便于释放。
#[unsafe(no_mangle)]
pub extern "C" fn sudoos_usb_alloc(size: usize) -> *mut u8 {
    let Some(total) = size.checked_add(8) else {
        return core::ptr::null_mut();
    };
    let Ok(layout) = Layout::from_size_align(total, 8) else {
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

/// 释放 `sudoos_usb_alloc` 返回的指针（读取头部 size 后按原布局回收）。
#[unsafe(no_mangle)]
pub extern "C" fn sudoos_usb_free(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: 前 8 字节是 sudoos_usb_alloc 写入的 size 头。
    let size = unsafe { ptr.sub(8).cast::<usize>().read() };
    let Ok(layout) = Layout::from_size_align(size + 8, 8) else {
        return;
    };
    // SAFETY: ptr.sub(8) 是 alloc 返回的原始指针，layout 与其匹配。
    unsafe { alloc::alloc::dealloc(ptr.sub(8), layout) };
}

/// C 侧 `usb_osal_get_tick`：相对时钟源的毫秒数（用于相对计时）。
#[unsafe(no_mangle)]
pub extern "C" fn sudoos_usb_get_tick_ms() -> u32 {
    let elapsed = crate::time::now().duration_since(MonotonicInstant::from_cycles(0));
    elapsed.as_millis() as u32
}

/// C 侧 `usb_osal_msleep`：忙等延时（M1 线程未接线，用轮询时钟实现）。
#[unsafe(no_mangle)]
pub extern "C" fn sudoos_usb_msleep(ms: u32) {
    let start = crate::time::now();
    let wait = core::time::Duration::from_millis(ms as u64);
    while crate::time::now().duration_since(start) < wait {
        core::hint::spin_loop();
    }
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

unsafe extern "C" {
    /// `kernel/csrc/usb/usb_glue_ls2k1000.c`：CherryUSB 宿主初始化。
    fn sudoos_usb_init() -> i32;
    /// `kernel/csrc/usb/usb_glue_ls2k1000.c`：构建路径探针。
    fn sudoos_usb_glue_probe() -> u32;
}
