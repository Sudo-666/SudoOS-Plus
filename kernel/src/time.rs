use core::sync::atomic::{AtomicU64, Ordering};

static TIMER_TICKS: AtomicU64 = AtomicU64::new(0);

pub fn initialize() {
    crate::println!("time subsystem:");
    crate::println!("  monotonic ticks  : ready");
    crate::println!("  current ticks    : {}", timer_ticks());
    crate::println!("  periodic timer   : not armed");
}

pub fn record_timer_tick() -> u64 {
    TIMER_TICKS.fetch_add(1, Ordering::Relaxed) + 1
}

pub fn timer_ticks() -> u64 {
    TIMER_TICKS.load(Ordering::Relaxed)
}
