//! Linux 风格的 RTC (实时时钟) 子系统。
//!
//! 提供 `read_rtc()` 全局函数和 `/dev/rtc` 字符设备。
//! 支持 VirtIO-RTC 硬件（如果可用）；否则返回系统启动以来的单调时间。

use core::sync::atomic::{AtomicBool, Ordering};

use myos_vfs::Errno;

use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};

const RTC_LOCK: LockClass = LockClass::new("rtc.device", LockRank::Vfs, 12);
pub const RTC_RD_TIME: usize = 0x8024_7009;

/// RTC 时间值（Unix 时间戳，秒）。
#[derive(Clone, Copy, Debug)]
pub struct RtcTime {
    pub unix_seconds: i64,
}

/// Linux RTC ioctl: _IOR('p', 0x09, struct rtc_time)
/// Match any encoding: check type='p', nr=0x09, accepting any size/direction.
fn is_rtc_rd_time(cmd: usize) -> bool {
    if cmd == RTC_RD_TIME {
        return true;
    }
    let typ = (cmd >> 8) & 0xff;
    let nr = cmd & 0xff;
    typ == b'p' as usize && nr == 0x09
}

/// Handle RTC ioctl commands. Returns Ok(0) on success, or Err(ENOTTY) for
/// unknown commands.
pub fn ioctl(cmd: usize, arg: usize) -> Result<usize, Errno> {
    if is_rtc_rd_time(cmd) {
        // struct rtc_time: 9 × i32 (36 bytes) in little-endian
        let time = read_rtc_time().unwrap_or(RtcTime { unix_seconds: 0 });
        let secs = time.unix_seconds;
        let day_secs = secs.rem_euclid(86400);
        let tm_sec = (day_secs % 60) as i32;
        let tm_min = ((day_secs / 60) % 60) as i32;
        let tm_hour = (day_secs / 3600) as i32;
        let days = (secs / 86400) as i32;
        let (tm_year, tm_mon, tm_mday) = civil_from_days(days);
        let tm_wday = ((days + 4) % 7) as i32;
        let tm_yday = (days - days_from_civil(tm_year, 0, 1)) as i32;
        let tm_isdst: i32 = 0;
        let mut buf = [0u8; 36];
        buf[0..4].copy_from_slice(&tm_sec.to_le_bytes());
        buf[4..8].copy_from_slice(&tm_min.to_le_bytes());
        buf[8..12].copy_from_slice(&tm_hour.to_le_bytes());
        buf[12..16].copy_from_slice(&tm_mday.to_le_bytes());
        buf[16..20].copy_from_slice(&tm_mon.to_le_bytes());
        buf[20..24].copy_from_slice(&(tm_year - 1900).to_le_bytes());
        buf[24..28].copy_from_slice(&tm_wday.to_le_bytes());
        buf[28..32].copy_from_slice(&tm_yday.to_le_bytes());
        buf[32..36].copy_from_slice(&tm_isdst.to_le_bytes());
        if crate::user::copy_to_user(arg, &buf).is_err() {
            return Err(Errno::Efault);
        }
        Ok(0)
    } else {
        Err(Errno::Enotty)
    }
}

/// Days from 0000-03-01 to the given (year, month 0-11, day 1-31).
/// Uses the proleptic Gregorian calendar.
fn days_from_civil(y: i32, m: i32, d: i32) -> i32 {
    let y = if m <= 1 { y - 1 } else { y };
    let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
    let yoe = y - era * 400;
    let doy = if m <= 1 {
        m * 31 + d - 1
    } else {
        (153 * (m - 2) + 2) / 5 + d - 1
    };
    era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy
}

fn civil_from_days(z: i32) -> (i32, i32, i32) {
    let z = z + 719468;
    let era = if z >= 0 {
        z / 146097
    } else {
        (z - 146096) / 146097
    };
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m - 1, d)
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
        if available {
            "available"
        } else {
            "unavailable"
        }
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
        let seconds = if freq != 0 { (now / freq) as i64 } else { 0 };
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
    crate::println!("  unix_seconds       : {}", time.unwrap().unix_seconds,);
}
