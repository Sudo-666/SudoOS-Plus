//! Linux 风格的 RTC (实时时钟) 子系统。
//!
//! 提供 `read_rtc()` 全局函数和 `/dev/rtc` 字符设备。
//! 支持 VirtIO-RTC 硬件（如果可用）；否则返回系统启动以来的单调时间。

use core::sync::atomic::{AtomicBool, Ordering};

use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};

const RTC_LOCK: LockClass = LockClass::new("rtc.device", LockRank::Vfs, 12);

/// RTC 时间值（Unix 时间戳，秒）。
#[derive(Clone, Copy, Debug)]
pub struct RtcTime {
    pub unix_seconds: i64,
}

/// RTC 硬件抽象 trait。
pub trait RtcHardware: Send + Sync + 'static {
    fn read_time(&self) -> Option<RtcTime>;
}

/// 全局 RTC 实例。
static RTC_INSTANCE: IrqSpinLock<Option<alloc::boxed::Box<dyn RtcHardware>>> =
    IrqSpinLock::new_with_class(None, RTC_LOCK);

static RTC_AVAILABLE: AtomicBool = AtomicBool::new(false);

pub fn initialize() {
    let available = RTC_AVAILABLE.load(Ordering::Acquire);
    crate::println!("rtc:");
    crate::println!(
        "  hardware       : {}",
        if available { "available" } else { "unavailable" }
    );
    crate::println!("  device         : /dev/rtc");
}

/// 读取 RTC 时间（Unix 秒）。如果没有硬件 RTC，返回系统启动以来的时间。
pub fn read_rtc_time() -> Option<RtcTime> {
    let rtc = RTC_INSTANCE.lock();
    if let Some(ref hw) = *rtc {
        hw.read_time()
    } else {
        // 退化为系统启动以来的单调时间
        let now = crate::time::now().cycles();
        let freq = crate::time::clock_frequency_hz();
        let seconds = if freq != 0 {
            (now / freq) as i64
        } else {
            0
        };
        Some(RtcTime {
            unix_seconds: seconds,
        })
    }
}

/// 注册硬件 RTC 设备。由 virtio probe 调用。
pub fn register_rtc(device: alloc::boxed::Box<dyn RtcHardware>) {
    *RTC_INSTANCE.lock() = Some(device);
    RTC_AVAILABLE.store(true, Ordering::Release);
}

// ---------------------------------------------------------------------------
// Verify
// ---------------------------------------------------------------------------

#[cfg(debug_assertions)]
pub fn verify() {
    let time = read_rtc_time();
    assert!(time.is_some(), "RTC should return a time value");

    crate::println!("M16 RTC gate:");
    crate::println!("  read_rtc_time      : verified");
    crate::println!(
        "  unix_seconds       : {}",
        time.unwrap().unix_seconds,
    );
}
