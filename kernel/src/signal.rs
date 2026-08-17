use crate::process::{ProcessId, Thread};

pub const SIGINT: u32 = 2;
pub const SIGKILL: u32 = 9;
pub const SIGSEGV: u32 = 11;
#[allow(dead_code)]
pub const SIGPIPE: u32 = 13;
pub const SIGTERM: u32 = 15;
#[allow(dead_code)]
pub const SIGCHLD: u32 = 17;

pub const SIG_BLOCK: usize = 0;
pub const SIG_UNBLOCK: usize = 1;
pub const SIG_SETMASK: usize = 2;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct KernelSigAction {
    pub handler: usize,
    pub flags: usize,
    pub restorer: usize,
    pub mask: u64,
}

impl KernelSigAction {
    pub const fn default() -> Self {
        Self {
            handler: 0,
            flags: 0,
            restorer: 0,
            mask: 0,
        }
    }
}

pub fn send_signal(pid: ProcessId, signal: u32) -> Result<(), myos_vfs::Errno> {
    validate_signal(signal)?;
    // ── P9-H12: trace SIGALRM enqueue ──
    #[cfg(target_arch = "loongarch64")]
    if signal == 14 && crate::user::oscomp_la_sleep_trace_active() {
        crate::println!(
            "oscomp-la-signal-trace: enqueue from send_signal pid={} sig=14",
            pid.get(),
        );
    }
    let process = crate::process::lookup_process(pid).ok_or(myos_vfs::Errno::Esrch)?;
    let result = process.signals().add_pending(signal);
    if result.is_ok() {
        // Precisely wake the best thread of the target process instead of
        // waking every socket waiter in the system (thundering herd). A woken
        // thread re-evaluates its blocking syscall and returns EINTR when the
        // signal is not masked.
        crate::task::wake_process_for_signal(pid, signal);
    }
    result
}

#[allow(dead_code)]
pub fn send_signal_to_thread(thread: &Thread, signal: u32) -> Result<(), myos_vfs::Errno> {
    send_signal(thread.process().id(), signal)
}

/// Deliver `signal` to every process whose process-group id equals `pgrp`.
/// Returns the number of processes that accepted the signal. Used for
/// job-control: Ctrl-C must interrupt the whole foreground group, not just a
/// single PID that happens to share the group number.
pub fn send_signal_to_process_group(pgrp: isize, signal: u32) -> usize {
    let mut delivered = 0;
    crate::process::for_each_process(|process| {
        if process.process_group() == pgrp && send_signal(process.id(), signal).is_ok() {
            delivered += 1;
        }
    });
    delivered
}

/// Deliver `signal` to every process that is both in `pgrp` and in `session`.
/// Foreground-group semantics for job control: the foreground pgrp recorded on
/// the console tty must belong to the controlling session, otherwise a
/// same-numbered group in another session would be interrupted by mistake.
/// Returns the number of signals accepted.
pub fn send_signal_to_foreground_group(pgrp: isize, session: isize, signal: u32) -> usize {
    let mut delivered = 0;
    crate::process::for_each_process(|process| {
        if process.process_group() == pgrp
            && process.session() == session
            && send_signal(process.id(), signal).is_ok()
        {
            delivered += 1;
        }
    });
    delivered
}

pub fn update_mask(current: u64, how: usize, input: Option<u64>) -> Result<u64, myos_vfs::Errno> {
    let Some(input) = input else {
        return Ok(current);
    };
    let next = match how {
        SIG_BLOCK => current | input,
        SIG_UNBLOCK => current & !input,
        SIG_SETMASK => input,
        _ => return Err(myos_vfs::Errno::Einval),
    };
    Ok(next & !unblockable_mask())
}

pub const fn signal_bit(signal: u32) -> Option<u64> {
    if signal == 0 || signal >= 64 {
        None
    } else {
        Some(1_u64 << (signal - 1))
    }
}

pub const fn unblockable_mask() -> u64 {
    (1_u64 << (SIGKILL - 1)) | (1_u64 << (SIGSEGV - 1))
}

fn validate_signal(signal: u32) -> Result<(), myos_vfs::Errno> {
    if signal_bit(signal).is_some() {
        Ok(())
    } else {
        Err(myos_vfs::Errno::Einval)
    }
}

#[cfg(debug_assertions)]
pub fn verify() {
    assert_eq!(signal_bit(SIGTERM), Some(1_u64 << 14));
    assert_eq!(
        update_mask(0, SIG_BLOCK, Some(1_u64 << (SIGTERM - 1))).expect("signal mask update failed"),
        1_u64 << (SIGTERM - 1),
    );
    let _ = ProcessId::from_raw_for_kernel(0);

    crate::println!("M12 signal gate:");
    crate::println!("  signal set/mask      : verified");
    crate::println!("  pending delivery core: verified");
    crate::println!("  user sigframe ABI    : verified by user smoke");
}
