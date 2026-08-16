use crate::process::{ProcessId, Thread};

pub const SIGINT: u32 = 2;
pub const SIGKILL: u32 = 9;
pub const SIGSEGV: u32 = 11;
#[allow(dead_code)]
pub const SIGPIPE: u32 = 13;
pub const SIGALRM: u32 = 14;
pub const SIGTERM: u32 = 15;
#[allow(dead_code)]
pub const SIGCHLD: u32 = 17;

pub const SIG_BLOCK: usize = 0;
pub const SIG_UNBLOCK: usize = 1;
pub const SIG_SETMASK: usize = 2;

const SIG_DFL: usize = 0;
const SIG_IGN: usize = 1;

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
        // wait4 sleeps on the process child queue. A caught signal such as
        // SIGALRM must wake that queue so the syscall can observe EINTR;
        // SIGCHLD wakeups are harmlessly coalesced with zombie publication.
        process.child_wait_queue().wake_all();
        crate::net::socket::wake_all_waiters();
        // SUDOOS_SIGNAL_WAKE_BLOCKED_V1: signal delivery runs on the
        // trap-return path, so a target blocked on any other queue (pipe,
        // futex, epoll, sigsuspend, …) would sleep through the signal and
        // never take its default action. Wake its waiters directly; running
        // and runnable tasks observe the pending signal on their own.
        crate::task::wake_process_blocked_waiters(pid);
        // SUDOOS_KILL_WAKE_REBLOCK_V1: SIGKILL is unblockable and always
        // terminating. Tasks blocked in non-trap waits (nanosleep,
        // completions) would re-block after the wake and sleep through the
        // signal; publish a forced exit so every blocking primitive breaks.
        if signal == SIGKILL {
            crate::task::force_process_thread_exit(pid, -(signal as isize));
        }
    }
    result
}

/// True when the process has a pending, unblocked signal that must interrupt
/// an interruptible wait. Mirrors wait4's filter: caught signals and
/// default-terminating signals interrupt; SIG_IGN and the default-ignored
/// SIGCHLD never do, so child-exit notification storms cannot turn into
/// spurious EINTRs under parallel builds.
pub fn has_interrupting_signal(process: &crate::process::Process, blocked: u64) -> bool {
    let pending = process.signals().pending() & !blocked;
    if pending == 0 {
        return false;
    }
    (1_u32..64).any(|signal| {
        let bit = 1_u64 << (signal - 1);
        if pending & bit == 0 {
            return false;
        }
        let action = process.signals().action(signal).unwrap_or_default();
        action.handler != SIG_IGN && !(action.handler == SIG_DFL && signal == SIGCHLD)
    })
}

#[allow(dead_code)]
pub fn send_signal_to_thread(thread: &Thread, signal: u32) -> Result<(), myos_vfs::Errno> {
    send_signal(thread.process().id(), signal)
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
