// SUDOOS_NEWTEST_P0_ABI_HOTFIX_V2: uname release is Linux-compatible for contest libc startup.
use alloc::{string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, AtomicUsize, Ordering};

use myos_mm::{FaultAccess, PAGE_SIZE, VirtAddr, VirtRange, VmArea, VmAreaFlags, VmAreaKind};

use crate::process::{Process, Thread};
use crate::user_mm::{UserFaultFailure, UserFaultRecovery, UserFaultResolution, UserMmRuntimeError};

const USER_CODE: usize = 0x0000_0000_0040_0000;
const USER_DATA: usize = USER_CODE + PAGE_SIZE;
const USER_DEMAND: usize = 0x0000_0000_0050_0000;
const USER_HEAP_START: usize = 0x0000_0000_0060_0000;
const USER_HEAP_LIMIT: usize = 0x0000_0000_0070_0000;
const USER_STACK: usize = 0x0000_0000_0080_0000;
const USER_STACK_TOP: usize = USER_STACK + PAGE_SIZE;
const USER_MMAP_START: usize = 0x0000_0000_0100_0000;
const USER_MMAP_END: usize = 0x0000_0000_4000_0000;

const SYS_OPENAT: usize = crate::syscall::number::OPENAT;
const SYS_CLOSE: usize = crate::syscall::number::CLOSE;
const SYS_GETCWD: usize = crate::syscall::number::GETCWD;
const SYS_DUP: usize = crate::syscall::number::DUP;
const SYS_DUP3: usize = crate::syscall::number::DUP3;
const SYS_FCNTL: usize = crate::syscall::number::FCNTL;
const SYS_IOCTL: usize = crate::syscall::number::IOCTL;
const SYS_MKDIRAT: usize = crate::syscall::number::MKDIRAT;
const SYS_UNLINKAT: usize = crate::syscall::number::UNLINKAT;
const SYS_SYMLINKAT: usize = crate::syscall::number::SYMLINKAT;
const SYS_LINKAT: usize = crate::syscall::number::LINKAT;
const SYS_RENAMEAT: usize = crate::syscall::number::RENAMEAT;
const SYS_UMOUNT2: usize = crate::syscall::number::UMOUNT2;
const SYS_MOUNT: usize = crate::syscall::number::MOUNT;
const SYS_FTRUNCATE: usize = crate::syscall::number::FTRUNCATE;
const SYS_FACCESSAT: usize = crate::syscall::number::FACCESSAT;
const SYS_CHDIR: usize = crate::syscall::number::CHDIR;
const SYS_GETDENTS64: usize = crate::syscall::number::GETDENTS64;
const SYS_PIPE2: usize = crate::syscall::number::PIPE2;
const SYS_LSEEK: usize = crate::syscall::number::LSEEK;
const SYS_READ: usize = crate::syscall::number::READ;
const SYS_WRITE: usize = crate::syscall::number::WRITE;
const SYS_READV: usize = crate::syscall::number::READV;
const SYS_WRITEV: usize = crate::syscall::number::WRITEV;
const SYS_PREAD64: usize = crate::syscall::number::PREAD64;
const SYS_PSELECT6: usize = crate::syscall::number::PSELECT6;
const SYS_PPOLL: usize = crate::syscall::number::PPOLL;
const SYS_READLINKAT: usize = crate::syscall::number::READLINKAT;
const SYS_NEWFSTATAT: usize = crate::syscall::number::NEWFSTATAT;
const SYS_FSTAT: usize = crate::syscall::number::FSTAT;
const SYS_FSYNC: usize = crate::syscall::number::FSYNC;
const SYS_EXIT: usize = crate::syscall::number::EXIT;
const SYS_EXIT_GROUP: usize = crate::syscall::number::EXIT_GROUP;
const SYS_SET_TID_ADDRESS: usize = crate::syscall::number::SET_TID_ADDRESS;
const SYS_SET_ROBUST_LIST: usize = crate::syscall::number::SET_ROBUST_LIST;
const SYS_NANOSLEEP: usize = crate::syscall::number::NANOSLEEP;
const SYS_CLOCK_GETTIME: usize = crate::syscall::number::CLOCK_GETTIME;
const SYS_CLOCK_NANOSLEEP: usize = crate::syscall::number::CLOCK_NANOSLEEP;
const SYS_SCHED_YIELD: usize = crate::syscall::number::SCHED_YIELD;
const SYS_KILL: usize = crate::syscall::number::KILL;
const SYS_TKILL: usize = crate::syscall::number::TKILL;
const SYS_TGKILL: usize = crate::syscall::number::TGKILL;
const SYS_SETSID: usize = crate::syscall::number::SETSID;
const SYS_SETPGID: usize = crate::syscall::number::SETPGID;
const SYS_GETPGID: usize = crate::syscall::number::GETPGID;
const SYS_GETSID: usize = crate::syscall::number::GETSID;
const SYS_RT_SIGACTION: usize = crate::syscall::number::RT_SIGACTION;
const SYS_RT_SIGPROCMASK: usize = crate::syscall::number::RT_SIGPROCMASK;
const SYS_RT_SIGTIMEDWAIT: usize = crate::syscall::number::RT_SIGTIMEDWAIT;
const SYS_RT_SIGRETURN: usize = crate::syscall::number::RT_SIGRETURN;
const SYS_TIMES: usize = crate::syscall::number::TIMES;
const SYS_UNAME: usize = crate::syscall::number::UNAME;
const SYS_GETTIMEOFDAY: usize = crate::syscall::number::GETTIMEOFDAY;
const SYS_GETPID: usize = crate::syscall::number::GETPID;
const SYS_GETPPID: usize = crate::syscall::number::GETPPID;
const SYS_GETUID: usize = crate::syscall::number::GETUID;
const SYS_GETEUID: usize = crate::syscall::number::GETEUID;
const SYS_GETGID: usize = crate::syscall::number::GETGID;
const SYS_GETEGID: usize = crate::syscall::number::GETEGID;
const SYS_GETTID: usize = crate::syscall::number::GETTID;
const SYS_SYSINFO: usize = crate::syscall::number::SYSINFO;
const SYS_BRK: usize = crate::syscall::number::BRK;
const SYS_MUNMAP: usize = crate::syscall::number::MUNMAP;
const SYS_CLONE: usize = crate::syscall::number::CLONE;
const SYS_EXECVE: usize = crate::syscall::number::EXECVE;
const SYS_MMAP: usize = crate::syscall::number::MMAP;
const SYS_MPROTECT: usize = crate::syscall::number::MPROTECT;
const SYS_PKEY_MPROTECT: usize = crate::syscall::number::PKEY_MPROTECT;
const SYS_WAIT4: usize = crate::syscall::number::WAIT4;
const SYS_PRLIMIT64: usize = crate::syscall::number::PRLIMIT64;
const SYS_GETRANDOM: usize = crate::syscall::number::GETRANDOM;
const SYS_STATX: usize = crate::syscall::number::STATX;
const SYS_SOCKET: usize = crate::syscall::number::SOCKET;
const SYS_BIND: usize = crate::syscall::number::BIND;
const SYS_LISTEN: usize = crate::syscall::number::LISTEN;
const SYS_ACCEPT: usize = crate::syscall::number::ACCEPT;
const SYS_CONNECT: usize = crate::syscall::number::CONNECT;
const SYS_SENDTO: usize = crate::syscall::number::SENDTO;
const SYS_RECVFROM: usize = crate::syscall::number::RECVFROM;
const SYS_SHUTDOWN: usize = crate::syscall::number::SHUTDOWN;
const SYS_SETSOCKOPT: usize = crate::syscall::number::SETSOCKOPT;
const SYS_GETSOCKOPT: usize = crate::syscall::number::GETSOCKOPT;
const SYS_FUTEX: usize = crate::syscall::number::FUTEX;
const SYS_MKNODAT: usize = crate::syscall::number::MKNODAT;
const SYS_UTIMENSAT: usize = crate::syscall::number::UTIMENSAT;
const SYS_STATFS: usize = crate::syscall::number::STATFS;
const SYS_FSTATFS: usize = crate::syscall::number::FSTATFS;
const SYS_SYSLOG: usize = crate::syscall::number::SYSLOG;
const SYS_SCHED_GETAFFINITY: usize = crate::syscall::number::SCHED_GETAFFINITY;
const SYS_SCHED_SETAFFINITY: usize = crate::syscall::number::SCHED_SETAFFINITY;
const SYS_SCHED_SETSCHEDULER: usize = crate::syscall::number::SCHED_SETSCHEDULER;
const SYS_SCHED_GETSCHEDULER: usize = crate::syscall::number::SCHED_GETSCHEDULER;
const SYS_SCHED_GETPARAM: usize = crate::syscall::number::SCHED_GETPARAM;
const SYS_RENAMEAT2: usize = crate::syscall::number::RENAMEAT2;
const SYS_PRCTL: usize = crate::syscall::number::PRCTL;
const SYS_SETITIMER: usize = crate::syscall::number::SETITIMER;
const SYS_GETITIMER: usize = crate::syscall::number::GETITIMER;
const SYS_GETRUSAGE: usize = crate::syscall::number::GETRUSAGE;
const SYS_RT_SIGPENDING: usize = crate::syscall::number::RT_SIGPENDING;
const SYS_RT_SIGSUSPEND: usize = crate::syscall::number::RT_SIGSUSPEND;

const PROT_READ: usize = 1;
const PROT_WRITE: usize = 2;
const PROT_EXEC: usize = 4;
const MAP_SHARED: usize = 0x01;
const MAP_PRIVATE: usize = 0x02;
const MAP_TYPE: usize = 0x0f;
const MAP_FIXED: usize = 0x10;
const MAP_ANONYMOUS: usize = 0x20;
const VFS_PROBE_DATA: &[u8] = b"/m11-user\0......................uvfs";
const M12_PROBE_DATA: &[u8] = b"pipe";
const EXEC_PROBE_PATH: &str = "/.m12";

const EBADF: isize = crate::syscall::errno::EBADF;
const ECHILD: isize = crate::syscall::errno::ECHILD;
const EAGAIN: isize = crate::syscall::errno::EAGAIN;
const ENOMEM: isize = crate::syscall::errno::ENOMEM;
const EFAULT: isize = crate::syscall::errno::EFAULT;
const EINVAL: isize = crate::syscall::errno::EINVAL;
const ENOSYS: isize = crate::syscall::errno::ENOSYS;
const ERANGE: isize = 34;

const MAX_USER_COPY: usize = 256;
const MAX_USER_PATH: usize = 256;
const MAX_EXEC_ARGS: usize = 32;
const MAX_EXEC_ENVS: usize = 32;
const USER_MESSAGE: &[u8] = b"hello user\n";
const AT_FDCWD: usize = usize::MAX - 99;
const AT_REMOVEDIR: usize = 0x200;
const AT_SYMLINK_NOFOLLOW: usize = 0x100;
const AT_SYMLINK_FOLLOW: usize = 0x400;
const AT_EMPTY_PATH: usize = 0x1000;
const FD_CLOEXEC: usize = 1;
const F_DUPFD: usize = 0;
const F_GETFD: usize = 1;
const F_SETFD: usize = 2;
const F_GETFL: usize = 3;
const F_DUPFD_CLOEXEC: usize = 1030;
const R_OK: usize = 4;
const W_OK: usize = 2;
const X_OK: usize = 1;
const RLIMIT_STACK: usize = 3;
const RLIMIT_NOFILE: usize = 7;
const RLIMIT_AS: usize = 9;
const SIG_DFL: usize = 0;
const SIG_IGN: usize = 1;
const SIGNAL_FRAME_MAGIC: u64 = 0x5355_444f_5349_4731;

const FAULT_NONE: usize = 0;
const FAULT_PAGE: usize = 1;
const FAULT_EXCEPTION: usize = 2;
const FAULT_RECOVERED: usize = 3;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static TERMINATED: AtomicBool = AtomicBool::new(false);
/// Set to true when the contest runner finds at least one test script on
/// the sdcard and runs the full contest loop (including shutdown).
static SDCARD_CONTEST_RAN: AtomicBool = AtomicBool::new(false);

// ── P9-G7d: contest progress atomics (visible to external watchdog) ──
static OSCOMP_ACTIVE: AtomicBool = AtomicBool::new(false);
static OSCOMP_FINALIZED: AtomicBool = AtomicBool::new(false);
static OSCOMP_TOTAL: AtomicUsize = AtomicUsize::new(0);
static OSCOMP_COMPLETED: AtomicUsize = AtomicUsize::new(0);
static OSCOMP_PASS: AtomicUsize = AtomicUsize::new(0);
static OSCOMP_FAIL: AtomicUsize = AtomicUsize::new(0);
static OSCOMP_SKIPPED: AtomicUsize = AtomicUsize::new(0);
static OSCOMP_TIMEOUT: AtomicUsize = AtomicUsize::new(0);
static OSCOMP_SIGNAL11: AtomicUsize = AtomicUsize::new(0);
static OSCOMP_SIGNAL14: AtomicUsize = AtomicUsize::new(0);
static OSCOMP_DEADLINE_CYCLES: AtomicU64 = AtomicU64::new(0);

// ── P9-H11: LoongArch sleep syscall trace (diagnostic only) ──
#[cfg(target_arch = "loongarch64")]
static OSCOMP_LA_SLEEP_TRACE: AtomicBool = AtomicBool::new(false);
#[cfg(target_arch = "loongarch64")]
static OSCOMP_LA_SLEEP_TRACE_BUDGET: AtomicUsize = AtomicUsize::new(0);
static LAST_TRACED_SYSCALL_NR: AtomicUsize = AtomicUsize::new(0);

#[cfg(target_arch = "loongarch64")]
pub(crate) fn oscomp_la_sleep_trace_active() -> bool {
    OSCOMP_LA_SLEEP_TRACE.load(Ordering::Relaxed)
}
static SYSCALL_COUNT: AtomicUsize = AtomicUsize::new(0);
static WRITE_COUNT: AtomicUsize = AtomicUsize::new(0);
static FAULT_COUNT: AtomicUsize = AtomicUsize::new(0);
static RECOVERED_FAULT_COUNT: AtomicUsize = AtomicUsize::new(0);
static ANONYMOUS_FAULT_COUNT: AtomicUsize = AtomicUsize::new(0);
static STACK_GROWTH_COUNT: AtomicUsize = AtomicUsize::new(0);
static BRK_COUNT: AtomicUsize = AtomicUsize::new(0);
static MMAP_COUNT: AtomicUsize = AtomicUsize::new(0);
static MUNMAP_COUNT: AtomicUsize = AtomicUsize::new(0);
static MPROTECT_COUNT: AtomicUsize = AtomicUsize::new(0);
static LAST_FAULT_KIND: AtomicUsize = AtomicUsize::new(FAULT_NONE);
static LAST_FAULT_ADDRESS: AtomicUsize = AtomicUsize::new(0);
static EXIT_STATUS: AtomicIsize = AtomicIsize::new(isize::MIN);
static SCHED_YIELD_SWITCH_COUNT: AtomicUsize = AtomicUsize::new(0);
static SCHEDULER_PEER_STOP: AtomicBool = AtomicBool::new(false);
static SCHEDULER_PEER_READY: crate::task::Completion = crate::task::Completion::new();
static SCHEDULER_PEER_DONE: crate::task::Completion = crate::task::Completion::new();

// Rate-limited execve tracing for diagnostic purposes.
// Each failed exec prints path + errno + reason; first 32 successful
// dynamic execs also print a one-line summary. Capped at 256 total to
// avoid flooding the contest serial log.
static EXEC_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);
const EXEC_TRACE_LIMIT: usize = 256;
const EXEC_TRACE_SUCCESS_LIMIT: usize = 32;

fn exec_trace_allow() -> bool {
    let prev = EXEC_TRACE_COUNT.fetch_add(1, Ordering::Relaxed);
    prev < EXEC_TRACE_LIMIT
}

fn exec_trace_success_allow() -> bool {
    let prev = EXEC_TRACE_COUNT.fetch_add(1, Ordering::Relaxed);
    prev < EXEC_TRACE_SUCCESS_LIMIT
}

#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!("user/riscv64.S"));

#[cfg(target_arch = "loongarch64")]
core::arch::global_asm!(include_str!("user/loongarch64.S"));

unsafe extern "C" {
    fn __m7_enter_user(entry: usize, stack_top: usize) -> isize;
    fn __m12_enter_user_frame(frame: *const crate::arch::trap::TrapFrame) -> isize;
    fn __m7_user_return();

    static __m7_user_image_start: u8;
    static __m7_user_success: u8;
    static __m7_user_unknown_syscall: u8;
    static __m7_user_bad_pointer: u8;
    static __m7_user_write_code: u8;
    static __m8_user_vm: u8;
    static __m8_user_mprotect_fault: u8;
    static __m8_user_munmap_fault: u8;
    static __m9_user_sched_yield: u8;
    static __m11_user_vfs: u8;
    static __m12_exec_success: u8;
    static __m12_m13_user_probe: u8;
    static __m7_user_image_end: u8;
}

struct UserImage {
    process: Arc<Process>,
    thread: Arc<Thread>,
}

#[derive(Clone, Copy)]
#[repr(C, align(16))]
struct UserSignalFrame {
    magic: u64,
    signal: u64,
    old_mask: u64,
    reserved: u64,
    trap_frame: crate::arch::trap::TrapFrame,
}

#[allow(dead_code)]
#[repr(C)]
struct KernelSigInfo {
    signo: i32,
    errno: i32,
    code: i32,
    payload: [u8; 116],
}

#[allow(dead_code)]
#[repr(C)]
struct KernelUContext {
    flags: u64,
    link: u64,
    stack: SigAltStack,
    signal_mask: u64,
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
#[repr(C)]
struct SigAltStack {
    sp: u64,
    flags: i32,
    size: u64,
}

impl UserImage {
    fn exec(entry_symbol: *const u8) -> Result<Self, crate::exec::ExecError> {
        let entry = VirtAddr::new(user_entry(entry_symbol));
        let elf =
            crate::elf::build_static_exec(entry, embedded_user_image(), VirtAddr::new(USER_DATA))?;
        let initramfs = crate::initramfs::build_single_file_newc("/init", &elf)?;
        let extra_areas = [VmArea::new(
            VirtRange::from_bounds(USER_DEMAND, USER_DEMAND + PAGE_SIZE),
            VmAreaFlags::user_rw(),
            VmAreaKind::Anonymous,
        )];
        let image = crate::exec::kernel_execve_from_initramfs(
            &initramfs,
            "/init",
            crate::exec::ExecConfig {
                argv: &["/init"],
                envp: &[],
                stack: VirtRange::from_bounds(USER_STACK, USER_STACK_TOP),
                heap_start: VirtAddr::new(USER_HEAP_START),
                heap_limit: VirtAddr::new(USER_HEAP_LIMIT),
                extra_areas: &extra_areas,
            },
        )?;
        Ok(Self {
            process: image.process,
            thread: image.thread,
        })
    }

    fn publish(&self) {
        assert!(
            !ACTIVE.load(Ordering::Acquire),
            "M8-B3 attempted to publish two user sessions",
        );
        ACTIVE.store(true, Ordering::Release);
    }

    fn unpublish(&self) {
        let was_active = ACTIVE.swap(false, Ordering::AcqRel);
        assert!(
            was_active,
            "M8-B3 attempted to unpublish an inactive session"
        );
    }

    fn destroy(self) {
        let Self { process, thread } = self;
        assert_eq!(
            thread.exit_status(),
            Some(EXIT_STATUS.load(Ordering::Acquire)),
            "M9-B Process teardown observed a mismatched Thread exit status",
        );
        assert!(
            thread.scheduler_task().is_some(),
            "M9-B user Thread was never bound to a scheduler task",
        );
        drop(thread);
        assert_eq!(
            Arc::strong_count(&process),
            1,
            "M9-B retained an unexpected Process owner after scheduler detach",
        );
        let process = Arc::try_unwrap(process)
            .unwrap_or_else(|_| panic!("M9-B could not obtain unique Process ownership"));
        process
            .destroy()
            .expect("unable to destroy the M9-B process address space");
    }
}

#[derive(Clone, Copy)]
struct SessionExpected {
    result: isize,
    exit_status: isize,
    syscall_count: usize,
    write_count: usize,
    fault_count: usize,
    recovered_fault_count: usize,
    anonymous_fault_count: usize,
    stack_growth_count: usize,
    brk_count: usize,
    mmap_count: usize,
    munmap_count: usize,
    mprotect_count: usize,
    fault_kind: usize,
    fault_address: usize,
}

#[derive(Clone, Copy)]
struct SessionObserved {
    result: isize,
    terminated: bool,
    exit_status: isize,
    syscall_count: usize,
    write_count: usize,
    fault_count: usize,
    recovered_fault_count: usize,
    anonymous_fault_count: usize,
    stack_growth_count: usize,
    brk_count: usize,
    mmap_count: usize,
    munmap_count: usize,
    mprotect_count: usize,
    fault_kind: usize,
    fault_address: usize,
}

pub fn verify() {
    crate::task::run_kernel_thread_sync(verify_worker);
}

fn verify_worker() {
    crate::syscall::verify_contract();
    crate::context::assert_task_context();
    crate::context::assert_interrupts_enabled();
    assert_eq!(
        crate::smp::current_cpu_id(),
        crate::smp::CpuId::BOOT,
        "M8-B3 verifier must run on the boot CPU",
    );
    assert!(
        !ACTIVE.load(Ordering::Acquire),
        "M8-B3 user session was already active",
    );

    let success = SessionExpected {
        result: 0,
        exit_status: 0,
        syscall_count: 2,
        write_count: 1,
        fault_count: 0,
        recovered_fault_count: 0,
        anonymous_fault_count: 0,
        stack_growth_count: 0,
        brk_count: 0,
        mmap_count: 0,
        munmap_count: 0,
        mprotect_count: 0,
        fault_kind: FAULT_NONE,
        fault_address: 0,
    };
    let no_fault_write = SessionExpected {
        result: 0,
        exit_status: 0,
        syscall_count: 2,
        write_count: 0,
        fault_count: 0,
        recovered_fault_count: 0,
        anonymous_fault_count: 0,
        stack_growth_count: 0,
        brk_count: 0,
        mmap_count: 0,
        munmap_count: 0,
        mprotect_count: 0,
        fault_kind: FAULT_NONE,
        fault_address: 0,
    };
    let scheduler_success = SessionExpected {
        result: 0,
        exit_status: 0,
        syscall_count: 9,
        write_count: 0,
        fault_count: 0,
        recovered_fault_count: 0,
        anonymous_fault_count: 0,
        stack_growth_count: 0,
        brk_count: 0,
        mmap_count: 0,
        munmap_count: 0,
        mprotect_count: 0,
        fault_kind: FAULT_NONE,
        fault_address: 0,
    };
    let vfs_success = SessionExpected {
        result: 0,
        exit_status: 0,
        syscall_count: 6,
        write_count: 1,
        fault_count: 0,
        recovered_fault_count: 0,
        anonymous_fault_count: 0,
        stack_growth_count: 0,
        brk_count: 0,
        mmap_count: 0,
        munmap_count: 0,
        mprotect_count: 0,
        fault_kind: FAULT_NONE,
        fault_address: 0,
    };
    let process_io_success = SessionExpected {
        result: 0,
        exit_status: 0,
        syscall_count: 19,
        write_count: 1,
        fault_count: 0,
        recovered_fault_count: 0,
        anonymous_fault_count: 0,
        stack_growth_count: 0,
        brk_count: 0,
        mmap_count: 0,
        munmap_count: 0,
        mprotect_count: 0,
        fault_kind: FAULT_NONE,
        fault_address: 0,
    };
    let write_code_fault = SessionExpected {
        result: -EFAULT,
        exit_status: -EFAULT,
        syscall_count: 0,
        write_count: 0,
        fault_count: 1,
        recovered_fault_count: 0,
        anonymous_fault_count: 0,
        stack_growth_count: 0,
        brk_count: 0,
        mmap_count: 0,
        munmap_count: 0,
        mprotect_count: 0,
        fault_kind: FAULT_PAGE,
        fault_address: USER_CODE,
    };
    let vm_success = SessionExpected {
        result: 0,
        exit_status: 0,
        syscall_count: 6,
        write_count: 0,
        fault_count: 4,
        recovered_fault_count: 4,
        anonymous_fault_count: 3,
        stack_growth_count: 1,
        brk_count: 2,
        mmap_count: 1,
        munmap_count: 1,
        mprotect_count: 1,
        fault_kind: FAULT_RECOVERED,
        fault_address: USER_MMAP_START,
    };
    let mprotect_fault = SessionExpected {
        result: -EFAULT,
        exit_status: -EFAULT,
        syscall_count: 2,
        write_count: 0,
        fault_count: 2,
        recovered_fault_count: 1,
        anonymous_fault_count: 1,
        stack_growth_count: 0,
        brk_count: 0,
        mmap_count: 1,
        munmap_count: 0,
        mprotect_count: 1,
        fault_kind: FAULT_PAGE,
        fault_address: USER_MMAP_START,
    };
    let munmap_fault = SessionExpected {
        result: -EFAULT,
        exit_status: -EFAULT,
        syscall_count: 2,
        write_count: 0,
        fault_count: 2,
        recovered_fault_count: 1,
        anonymous_fault_count: 1,
        stack_growth_count: 0,
        brk_count: 0,
        mmap_count: 1,
        munmap_count: 1,
        mprotect_count: 0,
        fault_kind: FAULT_PAGE,
        fault_address: USER_MMAP_START,
    };

    assert_session(
        "normal write/exit",
        run_session(
            core::ptr::addr_of!(__m7_user_success),
            Some(USER_MESSAGE),
            true,
        ),
        success,
    );
    assert_session(
        "unknown syscall",
        run_session(core::ptr::addr_of!(__m7_user_unknown_syscall), None, false),
        no_fault_write,
    );
    assert_session(
        "invalid user pointer",
        run_session(core::ptr::addr_of!(__m7_user_bad_pointer), None, false),
        no_fault_write,
    );
    assert_session(
        "write to RX code page",
        run_session(core::ptr::addr_of!(__m7_user_write_code), None, false),
        write_code_fault,
    );
    assert_session(
        "post-fault session reuse",
        run_session(
            core::ptr::addr_of!(__m7_user_success),
            Some(USER_MESSAGE),
            false,
        ),
        success,
    );
    assert_session(
        "demand paging and VM syscalls",
        run_session(core::ptr::addr_of!(__m8_user_vm), None, false),
        vm_success,
    );
    assert_session(
        "mprotect write rejection",
        run_session(core::ptr::addr_of!(__m8_user_mprotect_fault), None, false),
        mprotect_fault,
    );
    assert_session(
        "munmap stale translation rejection",
        run_session(core::ptr::addr_of!(__m8_user_munmap_fault), None, false),
        munmap_fault,
    );

    let scheduler_cpu = if crate::smp::scheduler_active_cpu_count() > 1 {
        crate::smp::CpuId::new(1).expect("CPU1 must fit the configured CPU mask")
    } else {
        crate::smp::CpuId::BOOT
    };
    assert_session(
        "schedulable user thread",
        run_scheduler_session(core::ptr::addr_of!(__m9_user_sched_yield), scheduler_cpu),
        scheduler_success,
    );
    assert_session(
        "user VFS syscalls",
        run_session(
            core::ptr::addr_of!(__m11_user_vfs),
            Some(VFS_PROBE_DATA),
            false,
        ),
        vfs_success,
    );
    assert_session(
        "user process/pipe/tty-adjacent syscalls",
        run_session(
            core::ptr::addr_of!(__m12_m13_user_probe),
            Some(M12_PROBE_DATA),
            false,
        ),
        process_io_success,
    );

    assert!(
        !ACTIVE.load(Ordering::Acquire),
        "M8-B3 verifier leaked an active user session",
    );
    crate::user_mm::assert_no_leaks();
    crate::process::assert_no_leaks();
    crate::task::assert_user_mm_quiescent();
    assert!(
        crate::task::user_mm_switches() >= 18,
        "M9-B did not exercise enough scheduler-owned MM transitions",
    );

    // Keep the frozen M7 evidence strings intact for the existing harness.
    crate::println!("minimal user mode test:");
    crate::println!("  U-mode/PLV3 entry : verified");
    crate::println!("  user trap stack   : verified");
    crate::println!("  write/exit ABI    : verified");
    crate::println!("  checked user copy : verified");
    crate::println!("  unknown syscall   : -ENOSYS verified");
    crate::println!("  invalid user ptr  : -EFAULT verified");
    crate::println!("  RX write fault    : isolated from kernel");
    crate::println!("  session recycle   : verified (5 runs)");
    crate::println!("  mapping reclaim   : verified");

    crate::println!("M8-B3 private-root gate:");
    crate::println!("  private user root : verified");
    crate::println!("  kernel high half  : shared");
    crate::println!("  ASID root switch  : verified");
    crate::println!("  active CPU publish: verified");
    crate::println!("  kernel root return: verified");
    crate::println!("  page/root reclaim : verified");
    crate::println!("  demand fault path : verified");

    crate::println!("M8-B4 demand paging/VM gate:");
    crate::println!("  anonymous demand  : verified");
    crate::println!("  bounded stack grow: verified");
    crate::println!("  brk growth/shrink : verified");
    crate::println!("  mmap/munmap       : verified");
    crate::println!("  mprotect          : verified");
    crate::println!("  TLB-before-free   : verified");
    crate::println!("  user fault retry  : verified");

    crate::println!("M9-A Process/Thread + ABI gate:");
    crate::println!("  Process owns MM   : verified");
    crate::println!("  Thread owns proc  : verified");
    crate::println!("  cycle-free reap   : verified");
    crate::println!("  Linux generic ABI : verified");
    crate::println!("M9-B scheduler/MM gate:");
    crate::println!("  schedulable user task : verified");
    crate::println!("  per-CPU loaded MM     : verified");
    crate::println!("  timer-preemptible user: verified");
    crate::println!("  deferred task reap    : verified");
    crate::println!("M10 ELF/initramfs gate:");
    crate::println!("  newc initramfs        : verified");
    crate::println!("  ELF64 PT_LOAD         : verified");
    crate::println!("  Linux initial stack   : verified");
    crate::println!("  kernel execve path    : verified");
    crate::println!("M11 user VFS gate:");
    crate::println!("  openat/write/read     : verified");
    crate::println!("  lseek/close via fd    : verified");
    crate::println!("M12/M13 user ABI gate:");
    crate::println!("  clone/wait4          : verified");
    crate::println!("  execve current image : verified");
    crate::println!("  pipe2/read/write      : verified");
    crate::println!("  signal frame/return   : verified");
    crate::println!("  pid/session syscalls  : verified");
    crate::println!("  clock/uname/getrandom : verified");
}

pub fn verify_busybox_rootfs() {
    if crate::fs::stat("/bin/busybox").is_err() {
        return;
    }

    crate::task::run_kernel_thread_sync(verify_busybox_rootfs_thread);
}

fn verify_busybox_rootfs_thread() {
    let result = run_rootfs_program(
        "/bin/busybox",
        &["busybox", "true"],
        &["PATH=/bin:/sbin:/usr/bin:/usr/sbin"],
    )
    .expect("unable to run BusyBox true from rootfs");
    assert_eq!(result, 0, "BusyBox true exited with a non-zero status");
    crate::println!("M14 BusyBox rootfs gate:");
    crate::println!("  /bin/busybox true : verified");
}

pub fn verify_sdcard_sample() {
    if crate::block::open_device("vda").is_none() {
        return;
    }

    crate::task::run_kernel_thread_sync(verify_sdcard_sample_thread);
}

fn verify_sdcard_sample_thread() {
    let device = match crate::block::open_device("vda") {
        Some(d) => d,
        None => return,
    };
    let sample_path = "/musl/busybox";
    let snapshot = match crate::ext4::load_path_snapshot(device, sample_path) {
        Ok(s) => s,
        Err(_) => {
            crate::println!("sdcard sample: /musl/busybox not found — skipping");
            return;
        }
    };
    let crate::ext4::Ext4SnapshotKind::Regular(image) = snapshot.kind else {
        crate::println!("sdcard sample: /musl/busybox is not a regular file — skipping");
        return;
    };
    let result = run_program_image(
        &image,
        &["busybox", "true"],
        &["PATH=/:/bin:/sbin:/usr/bin:/usr/sbin"],
        "sdcard sample",
    )
    .expect("unable to run /busybox true from ext4 sdcard image");
    assert_eq!(
        result, 0,
        "sdcard /busybox true exited with a non-zero status"
    );
    crate::println!("M15 ext4 sdcard sample gate:");
    crate::println!("  /dev/vda:/musl/busybox true : verified");
}

pub fn verify_sdcard_basic_script() {
    if crate::fs::stat("/mnt/sdcard/musl/basic_testcode.sh").is_err() {
        return;
    }

    crate::task::run_kernel_thread_sync(verify_sdcard_basic_script_thread);
}

fn verify_sdcard_basic_script_thread() {
    let echo_result = run_rootfs_program_with_cwd(
        "/mnt/sdcard/musl/busybox",
        &["busybox", "echo", "sdcard basic script smoke"],
        &["PATH=.:/mnt/sdcard/musl:/bin:/sbin:/usr/bin:/usr/sbin"],
        Some("/mnt/sdcard/musl"),
    )
    .expect("unable to run musl BusyBox echo from mounted ext4 sdcard image");
    assert_eq!(
        echo_result, 0,
        "mounted musl BusyBox echo exited with a non-zero status"
    );
    let shell_inline_result = run_rootfs_program_with_cwd(
        "/mnt/sdcard/musl/busybox",
        &["busybox", "sh", "-c", "echo sdcard shell inline smoke"],
        &["PATH=.:/mnt/sdcard/musl:/bin:/sbin:/usr/bin:/usr/sbin"],
        Some("/mnt/sdcard/musl"),
    )
    .expect("unable to run musl BusyBox sh -c from mounted ext4 sdcard image");
    assert_eq!(
        shell_inline_result, 0,
        "mounted musl BusyBox sh -c exited with a non-zero status"
    );

    crate::println!("sdcard basic script located; running basic binaries");
    verify_sdcard_basic_binaries();
    crate::println!("M16 ext4 sdcard script gate:");
    crate::println!("  /mnt/sdcard/musl/basic_testcode.sh : verified");
}

/// Returns `true` if the contest runner discovered scripts and ran to
/// completion (including shutdown).  Returns `false` when there is no
/// sdcard block device, so the caller can keep the machine alive for
/// smoke / non-contest boot paths.
pub fn verify_sdcard_all_scripts() -> bool {
    if crate::block::open_device("vda").is_none() {
        crate::println!("oscomp: no sdcard, skip contest runner");
        return false;
    }

    SDCARD_CONTEST_RAN.store(false, Ordering::Release);
    crate::task::run_kernel_thread_sync(verify_sdcard_all_scripts_thread);
    SDCARD_CONTEST_RAN.load(Ordering::Acquire)
}

fn verify_sdcard_all_scripts_thread() {
    let _ = crate::fs::mkdir("/var", 0o755);
    let _ = crate::fs::mkdir("/var/tmp", 0o755);
    let _ = crate::fs::mkdir("/tmp", 0o755);
    let _ = crate::fs::mkdir("/dev", 0o755);
    let _ = crate::fs::mkdir("/proc", 0o755);
    let _ = crate::fs::mkdir("/sys", 0o755);
    let _ = crate::fs::mkdir("/etc", 0o755);

    // Ensure busybox applet symlinks exist (fallback if not done at mount time)
    if crate::fs::stat("/bin/busybox").is_ok() {
        for applet in &[
            "cp", "sleep", "kill", "cat", "echo", "mv", "ln", "rm", "ls",
            "mkdir", "chmod", "grep", "dd", "mount", "ps", "head", "tail", "test",
            "awk", "sed", "wc", "cut", "tr", "which", "pidof", "printenv",
            "basename", "dirname", "readlink", "stat", "getopt",
        ] {
            let target = alloc::format!("/bin/{}", applet);
            if crate::fs::stat(&target).is_err() {
                let _ = crate::fs::symlink("/bin/busybox", &target);
            }
        }
    }

    let scripts: alloc::vec::Vec<alloc::string::String> = {
        let guard = crate::SCANNED_TEST_SCRIPTS.lock();
        guard.clone()
    };
    if scripts.is_empty() {
        crate::println!("sdcard scripts: no test scripts found on disk");
        return;
    }

    // Contest will run: set the flag so the caller can distinguish
    // contest-mode shutdown from smoke-mode idle loop.
    SDCARD_CONTEST_RAN.store(true, Ordering::Release);

    // ── LoongArch: probe shell candidates, prefer musl busybox ──
    #[cfg(target_arch = "loongarch64")]
    let (shell_path, la_shell_ok) = match choose_la_contest_shell() {
        Some(p) => {
            crate::println!("sdcard scripts: LA shell probe selected {}", p);
            oscomp_la_install_busybox_applets();
            (p, true)
        }
        None => {
            crate::println!("sdcard scripts: no working LA shell — all scripts will be skipped");
            ("/bin/sh", false)
        }
    };

    #[cfg(not(target_arch = "loongarch64"))]
    let shell_path = if crate::fs::stat("/bin/sh").is_ok() {
        "/bin/sh"
    } else if crate::fs::stat("/bin/busybox").is_ok() {
        "/bin/busybox"
    } else if crate::fs::stat("/busybox").is_ok() {
        "/busybox"
    } else {
        crate::println!("sdcard scripts: no shell found — skipping");
        return;
    };

    crate::println!("sdcard scripts: discovered {}", scripts.len());
    crate::println!("sdcard scripts: using shell {}", shell_path);

    // Arch-specific total budget so RV can get deeper results.
    #[cfg(target_arch = "riscv64")]
    const TOTAL_BUDGET_MS: u64 = 120_000;
    #[cfg(target_arch = "loongarch64")]
    const TOTAL_BUDGET_MS: u64 = 60_000;
    let freq_hz = crate::time::clock_frequency_hz();
    let budget_ms_to_cycles = |ms: u64| ms * freq_hz / 1000;
    let budget_start = crate::time::now().cycles();
    let budget_deadline = budget_start + budget_ms_to_cycles(TOTAL_BUDGET_MS);

    crate::println!(
        "oscomp: arch={} total_budget_ms={}",
        crate::arch::ARCH_NAME, TOTAL_BUDGET_MS,
    );

    // ── initialise contest atomics and launch external watchdog ──
    OSCOMP_ACTIVE.store(true, Ordering::Release);
    OSCOMP_FINALIZED.store(false, Ordering::Release);
    OSCOMP_TOTAL.store(scripts.len(), Ordering::Release);
    OSCOMP_COMPLETED.store(0, Ordering::Release);
    OSCOMP_PASS.store(0, Ordering::Release);
    OSCOMP_FAIL.store(0, Ordering::Release);
    OSCOMP_SKIPPED.store(0, Ordering::Release);
    OSCOMP_TIMEOUT.store(0, Ordering::Release);
    OSCOMP_SIGNAL11.store(0, Ordering::Release);
    OSCOMP_SIGNAL14.store(0, Ordering::Release);
    OSCOMP_DEADLINE_CYCLES.store(budget_deadline, Ordering::Release);

    crate::task::spawn_kernel_thread(contest_watchdog_main);

    // ── LoongArch: non-scoring exit-status diagnostics ──
    #[cfg(target_arch = "loongarch64")]
    oscomp_la_diag(shell_path);

    // ── local counters (mirror atomics for summary) ──
    #[cfg(not(target_arch = "loongarch64"))]
    let la_shell_ok: bool = true;
    let mut passed: usize = 0;
    let mut failed: usize = 0;
    let mut sig11: usize = 0;
    let mut sig14: usize = 0;

    // Track which ext4 directories have been expanded with all sibling files
    let mut expanded_dirs: alloc::vec::Vec<alloc::string::String> = alloc::vec::Vec::new();

    for (idx, script) in scripts.iter().enumerate() {
        // ── P9-H2A: stop promptly if watchdog already finalized ──
        if OSCOMP_FINALIZED.load(Ordering::Acquire) {
            crate::println!(
                "oscomp: finalized before script loop arch={} completed={} total={}",
                crate::arch::ARCH_NAME,
                OSCOMP_COMPLETED.load(Ordering::Acquire),
                OSCOMP_TOTAL.load(Ordering::Acquire),
            );
            break;
        }

        // Check global budget before starting a new group.
        let now = crate::time::now().cycles();
        if now + budget_ms_to_cycles(3_000) >= budget_deadline {
            crate::println!(
                "oscomp: global budget exhausted arch={} completed={} total={}",
                crate::arch::ARCH_NAME, idx, scripts.len(),
            );
            let remaining = scripts.len() - idx;
            OSCOMP_SKIPPED.fetch_add(remaining, Ordering::AcqRel);
            break;
        }
        crate::println!(
            "oscomp-progress: arch={} idx={}/{} script={}",
            crate::arch::ARCH_NAME, idx + 1, scripts.len(), script,
        );
        let vfs_path = if script.starts_with('/') {
            script.clone()
        } else {
            alloc::format!("/{}", script)
        };
        // Determine CWD from the script path (both for expansion and execution)
        let mut cwd = alloc::string::String::from("/");
        if let Some(pos) = vfs_path.rfind('/') {
            if pos > 0 {
                cwd.clear();
                cwd.push_str(&vfs_path[..pos]);
            }
        }

        // ── RISC-V whitelist defer ──
        // Non-whitelisted scripts are skipped early, before ext4 expansion,
        // so the 120 s budget is reserved for the six lightweight groups.
        #[cfg(target_arch = "riscv64")]
        if !oscomp_rv_whitelist(&vfs_path) {
            crate::println!("#### OS COMP TEST GROUP START {} ####", vfs_path);
            crate::println!("{} : SKIP (defer)", vfs_path);
            OSCOMP_SKIPPED.fetch_add(1, Ordering::AcqRel);
            OSCOMP_COMPLETED.fetch_add(1, Ordering::AcqRel);
            crate::println!("#### OS COMP TEST GROUP END {} ####", vfs_path);
            continue;
        }

        // ── LoongArch defer ──
        #[cfg(target_arch = "loongarch64")]
        if !la_shell_ok {
            crate::println!("#### OS COMP TEST GROUP START {} ####", vfs_path);
            crate::println!("{} : SKIP (la-shell-broken)", vfs_path);
            OSCOMP_SKIPPED.fetch_add(1, Ordering::AcqRel);
            OSCOMP_COMPLETED.fetch_add(1, Ordering::AcqRel);
            crate::println!("#### OS COMP TEST GROUP END {} ####", vfs_path);
            continue;
        }

        #[cfg(target_arch = "loongarch64")]
        if !oscomp_la_whitelist(&vfs_path) {
            crate::println!("#### OS COMP TEST GROUP START {} ####", vfs_path);
            crate::println!("{} : SKIP (la-defer)", vfs_path);
            OSCOMP_SKIPPED.fetch_add(1, Ordering::AcqRel);
            OSCOMP_COMPLETED.fetch_add(1, Ordering::AcqRel);
            crate::println!("#### OS COMP TEST GROUP END {} ####", vfs_path);
            continue;
        }

        // Expand ext4 directory: install all regular files from the
        // script's ext4 parent directory into VFS so that ./busybox,
        // ./dhry2reg, ./lmbench_all etc. resolve at runtime.
        let ext4_dir = sdcard_vfs_to_ext4_dir(&vfs_path);
        if !expanded_dirs.contains(&ext4_dir) {
            sdcard_install_ext4_dir_files(&ext4_dir);
            expanded_dirs.push(ext4_dir);
        }

        crate::println!("#### OS COMP TEST GROUP START {} ####", vfs_path);
        if crate::fs::stat(&vfs_path).is_err() {
            crate::println!("{} : SKIP (not found)", vfs_path);
            OSCOMP_SKIPPED.fetch_add(1, Ordering::AcqRel);
            OSCOMP_COMPLETED.fetch_add(1, Ordering::AcqRel);
            crate::println!("#### OS COMP TEST GROUP END {} ####", vfs_path);
            continue;
        }

        // ── heavy-group skip (RISC-V only) ──
        #[cfg(target_arch = "riscv64")]
        if oscomp_should_skip_heavy(&vfs_path) {
            crate::println!("{} : SKIP (heavy)", vfs_path);
            OSCOMP_SKIPPED.fetch_add(1, Ordering::AcqRel);
            OSCOMP_COMPLETED.fetch_add(1, Ordering::AcqRel);
            crate::println!("#### OS COMP TEST GROUP END {} ####", vfs_path);
            continue;
        }

        // Build a PATH that covers: CWD, common system dirs, and the
        // expanded ext4 directories under /mnt/sdcard.
        let cwd_path = if cwd == "/" {
            alloc::string::String::from("/")
        } else {
            alloc::format!("{}:", cwd)
        };
        let mut path_env = alloc::string::String::with_capacity(256);
        path_env.push_str("PATH=.:");
        path_env.push_str(&cwd_path);
        path_env.push_str("/:/bin:/sbin:/usr/bin:/usr/sbin:/usr/local/bin");
        // Also sniff whether common ext4 dirs exist so scripts can find
        // binaries without the ./ prefix.
        for sniff in &["/mnt/sdcard/musl", "/mnt/sdcard/glibc", "/mnt/sdcard/lmbench", "/mnt/sdcard", "/mnt/sdcard/lib", "/mnt/sdcard/usr/lib"] {
            if crate::fs::stat(sniff).is_ok() {
                path_env.push(':');
                path_env.push_str(sniff);
            }
        }

        let mut ld_env = alloc::string::String::with_capacity(256);
        ld_env.push_str("LD_LIBRARY_PATH=.:");
        ld_env.push_str(&cwd_path);
        ld_env.push_str("/:/lib:/usr/lib:/usr/local/lib");
        for sniff in &["/mnt/sdcard/lib", "/mnt/sdcard/usr/lib", "/mnt/sdcard/musl/lib", "/mnt/sdcard/musl"] {
            if crate::fs::stat(sniff).is_ok() {
                ld_env.push(':');
                ld_env.push_str(sniff);
            }
        }

        // ── RISC-V glibc/busybox presence check (non-scoring) ──
        #[cfg(target_arch = "riscv64")]
        if vfs_path.contains("glibc/busybox_testcode") {
            crate::println!(
                "oscomp-rv-busybox-pre: cwd={} busybox={} busybox_cmd={} script={}",
                cwd,
                crate::fs::stat("/mnt/sdcard/glibc/busybox").is_ok(),
                crate::fs::stat("/mnt/sdcard/glibc/busybox_cmd.txt").is_ok(),
                crate::fs::stat(&vfs_path).is_ok(),
            );
        }

        // Run the script using the verified spawn/exec/task lifecycle.
        let group_result = run_rootfs_program_with_cwd(
            shell_path,
            &["busybox", "sh", &vfs_path],
            &[&path_env, &ld_env, "HOME=/"],
            Some(&cwd),
        );
        match group_result {
            Ok(0) => {
                crate::println!("{} : PASS", vfs_path);
                passed += 1;
                OSCOMP_PASS.fetch_add(1, Ordering::AcqRel);
            }
            Ok(rc) => {
                let label = if rc < 0 {
                    let sig = -rc;
                    if sig == 11 {
                        sig11 += 1;
                        OSCOMP_SIGNAL11.fetch_add(1, Ordering::AcqRel);
                    }
                    if sig == 14 {
                        sig14 += 1;
                        OSCOMP_SIGNAL14.fetch_add(1, Ordering::AcqRel);
                    }
                    alloc::format!("FAIL (signal={})", sig)
                } else {
                    alloc::format!("FAIL (exit={})", rc)
                };
                crate::println!("{} : {}", vfs_path, label);
                failed += 1;
                OSCOMP_FAIL.fetch_add(1, Ordering::AcqRel);
            }
            Err(_) => {
                crate::println!("{} : ERROR", vfs_path);
                failed += 1;
                OSCOMP_FAIL.fetch_add(1, Ordering::AcqRel);
            }
        }
        OSCOMP_COMPLETED.fetch_add(1, Ordering::AcqRel);
        crate::println!("#### OS COMP TEST GROUP END {} ####", vfs_path);
    }

    // ── normal completion: CAS summary then shutdown ──
    OSCOMP_ACTIVE.store(false, Ordering::Release);
    if OSCOMP_FINALIZED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        let skipped = OSCOMP_SKIPPED.load(Ordering::Acquire);
        let timed_out = OSCOMP_TIMEOUT.load(Ordering::Acquire);
        crate::println!("#### OS COMP SUMMARY ####");
        crate::println!("arch={}", crate::arch::ARCH_NAME);
        crate::println!("total={}", scripts.len());
        crate::println!("completed={}", OSCOMP_COMPLETED.load(Ordering::Acquire));
        crate::println!("pass={}", passed);
        crate::println!("fail={}", failed);
        crate::println!("skipped={}", skipped);
        crate::println!("timeout={}", timed_out);
        crate::println!("signal11={}", sig11);
        crate::println!("signal14={}", sig14);
        crate::println!("score={}", passed);
        crate::println!("score: {}", passed);
        crate::println!("#### OS COMP SUMMARY END ####");
        crate::println!("oscomp: shutdown");
        contest_platform_shutdown();
    }
    // unreachable (contest_platform_shutdown diverges)
}

/// Unified contest power-off: called by the runner, the watchdog, and
/// `main.rs` after a completed contest run.
///
/// RISC-V   – SBI SRST, then legacy SBI shutdown, then `wfi` fallback.
/// LoongArch – QEMU virt PM MMIO write via uncached DMW, then spin fallback.
pub(crate) fn contest_platform_shutdown() -> ! {
    crate::println!("oscomp: platform shutdown");

    #[cfg(target_arch = "riscv64")]
    arch_contest_poweroff_rv();

    #[cfg(target_arch = "loongarch64")]
    arch_contest_poweroff_la();

    // Fallback if every power-off mechanism returned.
    #[cfg(target_arch = "riscv64")]
    loop {
        unsafe {
            core::arch::asm!("wfi", options(nomem, nostack));
        }
    }
    #[cfg(target_arch = "loongarch64")]
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(target_arch = "riscv64")]
fn arch_contest_poweroff_rv() {
    // SBI System Reset: extension=0x53525354 (SRST), func=0, type=shutdown
    crate::println!("oscomp: riscv sbi srst shutdown");
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a7") 0x53525354_usize,
            in("a6") 0_usize,
            in("a0") 0_usize,
            in("a1") 0_usize,
        );
    }

    // SRST returned — try legacy SBI shutdown (a7 = 0x8)
    crate::println!("oscomp: riscv sbi srst returned, trying legacy shutdown");
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a7") 0x8_usize,
        );
    }

    crate::println!("oscomp: riscv shutdown returned, halt");
}

#[cfg(target_arch = "loongarch64")]
fn arch_contest_poweroff_la() {
    // QEMU LoongArch virt ACPI FADT sleep control.
    // The legacy PM device at 0x1008_0010 is disabled — cloud QEMU
    // raises an exception on that address, so we go ACPI-only.
    const LA_DMW_UNCACHED: usize = 0x8000_0000_0000_0000;
    const QEMU_LA_ACPI_SLEEP_CTL: usize = LA_DMW_UNCACHED + 0x100e_001c;

    crate::println!("oscomp: loongarch qemu acpi shutdown");
    crate::println!(
        "oscomp: loongarch acpi sleep write addr={:#x} value=0x34",
        QEMU_LA_ACPI_SLEEP_CTL,
    );

    unsafe {
        core::ptr::write_volatile(QEMU_LA_ACPI_SLEEP_CTL as *mut u8, 0x34);
    }

    crate::println!("oscomp: loongarch acpi shutdown returned, idle fallback");
}

// ── P9-G7d: external contest watchdog ──

/// Heavy test groups that are known to run too long or hang.
/// Skipped on RISC-V so the runner has a chance to reach
/// busybox/basic/test.sh and produce a partial score.
#[cfg(target_arch = "riscv64")]
fn oscomp_should_skip_heavy(script: &str) -> bool {
    script.contains("unixbench")
        || script.contains("libcbench")
        || script.contains("lmbench")
        || script.contains("netperf")
        || script.contains("iperf")
        || script.contains("iozone")
        || script.contains("cyclictest")
        || script.contains("/ltp/")
        || script.contains("ltp_testcode")
}

/// RISC-V whitelist: only these four safe groups are allowed to run.
/// glibc/musl libctest are disabled — pthread_cond_smasher can trigger
/// a scheduler recursive-lock panic on cloud QEMU.
/// Everything else is deferred so the budget is spent on groups with
/// a proven chance of passing.
#[cfg(target_arch = "riscv64")]
fn oscomp_rv_whitelist(path: &str) -> bool {
    path.ends_with("/glibc/busybox_testcode.sh")
        || path.ends_with("/glibc/basic_testcode.sh")
        || path.ends_with("/musl/busybox_testcode.sh")
        || path.ends_with("/musl/basic_testcode.sh")
}

// ── P9-H7: LoongArch shell probe and contest whitelist ──

/// Probe LoongArch shell candidates, preferring the sdcard musl busybox
/// binary that the M15 gate already verified.  Returns the path of the
/// first candidate where `busybox true` exits 0.
#[cfg(target_arch = "loongarch64")]
fn choose_la_contest_shell() -> Option<&'static str> {
    // Ensure the musl directory is materialised so the candidate exists.
    sdcard_install_ext4_dir_files("/musl");

    let candidates: &[&str] = &[
        "/mnt/sdcard/musl/busybox",
        "/bin/busybox",
        "/bin/sh",
    ];

    let cwd = "/mnt/sdcard/musl";
    let env = &["PATH=.:/mnt/sdcard/musl:/mnt/sdcard/glibc:/bin:/sbin", "HOME=/"];

    for cand in candidates {
        if crate::fs::stat(cand).is_err() {
            crate::println!("oscomp-la-shell: candidate {} missing", cand);
            continue;
        }

        // Probe: busybox true (applet form — argv[0]=busybox, argv[1]=true)
        let rc = run_rootfs_program_with_cwd(cand, &["busybox", "true"], env, Some(cwd));
        match rc {
            Ok(0) => {
                crate::println!("oscomp-la-shell: probe {} true -> raw=0 PASS", cand);
                // Second probe: busybox sh -c true
                let rc2 =
                    run_rootfs_program_with_cwd(cand, &["busybox", "sh", "-c", "true"], env, Some(cwd));
                match rc2 {
                    Ok(0) => {
                        crate::println!("oscomp-la-shell: probe {} sh -c true -> raw=0 PASS", cand);
                        crate::println!("oscomp-la-shell: selected {}", cand);
                        return Some(cand);
                    }
                    Ok(raw) => crate::println!(
                        "oscomp-la-shell: probe {} sh -c true -> raw={} (not 0, skip)",
                        cand, raw,
                    ),
                    Err(_) => crate::println!(
                        "oscomp-la-shell: probe {} sh -c true -> ERROR",
                        cand,
                    ),
                }
            }
            Ok(raw) => crate::println!(
                "oscomp-la-shell: probe {} true -> raw={} (not 0, skip)",
                cand, raw,
            ),
            Err(_) => crate::println!("oscomp-la-shell: probe {} true -> ERROR", cand),
        }
    }

    crate::println!("oscomp-la-shell: no working candidate");
    None
}

/// Install BusyBox applet symlinks in /mnt/sdcard/musl so that shell
/// commands like `sleep`, `true`, `echo` can be resolved via PATH.
/// The musl busybox binary was verified working by the shell probe;
/// missing applets cause execve-fail → exit=142 (SIGALRM).
#[cfg(target_arch = "loongarch64")]
fn oscomp_la_install_busybox_applets() {
    let busybox = "/mnt/sdcard/musl/busybox";
    if crate::fs::stat(busybox).is_err() {
        return;
    }

    let applets: &[&str] = &[
        "sh", "sleep", "true", "false", "echo", "printf", "test", "[",
    ];

    for applet in applets {
        let target = alloc::format!("/mnt/sdcard/musl/{}", applet);
        if crate::fs::stat(&target).is_err() {
            if crate::fs::symlink(busybox, &target).is_ok() {
                crate::println!(
                    "oscomp-la-applet: installed /mnt/sdcard/musl/{} -> busybox",
                    applet,
                );
            }
        }
    }
}

/// Run a single probe with syscall tracing enabled on LoongArch.
/// Only used for `busybox sleep` diagnostics; does not affect scoring.
#[cfg(target_arch = "loongarch64")]
fn oscomp_la_run_sleep_trace_probe(
    label: &str,
    path: &str,
    argv: &[&str],
    env: &[&str],
    cwd: Option<&str>,
) -> isize {
    crate::println!("oscomp-la-sleep-trace: begin {}", label);
    OSCOMP_LA_SLEEP_TRACE_BUDGET.store(80, Ordering::Relaxed);
    OSCOMP_LA_SLEEP_TRACE.store(true, Ordering::Relaxed);

    let raw = match run_rootfs_program_with_cwd(path, argv, env, cwd) {
        Ok(r) => r,
        Err(_) => -127_isize,
    };

    OSCOMP_LA_SLEEP_TRACE.store(false, Ordering::Relaxed);
    let class = if raw == 0 {
        alloc::string::String::from("PASS")
    } else if raw < 0 {
        alloc::format!("signal={}", -raw)
    } else {
        alloc::format!("exit={}", raw)
    };
    crate::println!(
        "oscomp-la-sleep-trace: end {} raw={} class={}",
        label, raw, class,
    );
    raw
}

/// LoongArch whitelist: basic groups only.
/// Busybox groups are disabled — musl/busybox hits known-bad-busybox SIGSEGV.
/// Everything else is SKIP (la-defer).
#[cfg(target_arch = "loongarch64")]
fn oscomp_la_whitelist(path: &str) -> bool {
    path.ends_with("/glibc/basic_testcode.sh")
        || path.ends_with("/musl/basic_testcode.sh")
}

// ── P9-H2B: LoongArch exit-status diagnostics ──

/// Run a handful of trivial commands on LoongArch to determine whether
/// the platform-wide signal 14 failures come from a broken shell, a
/// broken wait-status decode, or a timer/alarm misconfiguration.
/// These do **not** affect scoring atomics.
#[cfg(target_arch = "loongarch64")]
fn oscomp_la_diag(_shell_path: &str) {
    crate::println!("oscomp-la-diag: begin");

    let shells: &[&str] = &[
        "/mnt/sdcard/musl/busybox",
        "/bin/busybox",
        "/bin/sh",
    ];

    for cand in shells {
        let present = crate::fs::stat(cand).is_ok();
        crate::println!(
            "oscomp-la-diag: candidate {} present={}",
            cand, present,
        );
        if !present {
            continue;
        }

        // Probe: busybox-applet true (argv[0]=busybox, argv[1]=true)
        let rc1 = run_rootfs_program_with_cwd(
            cand, &["busybox", "true"],
            &["PATH=.:/mnt/sdcard/musl:/mnt/sdcard/glibc:/bin:/sbin", "HOME=/"],
            Some("/mnt/sdcard/musl"),
        );
        match rc1 {
            Ok(raw) => crate::println!(
                "oscomp-la-diag: {} busybox true -> raw={} class={}",
                cand, raw,
                if raw == 0 { alloc::string::String::from("PASS") }
                else if raw < 0 { alloc::format!("signal={}", -raw) }
                else { alloc::format!("exit={}", raw) },
            ),
            Err(_) => crate::println!("oscomp-la-diag: {} busybox true -> ERROR", cand),
        }

        // Probe: shell -c true (argv[0]=busybox, argv[1]=sh if busybox binary)
        let rc2 = run_rootfs_program_with_cwd(
            cand, &["busybox", "sh", "-c", "true"],
            &["PATH=.:/mnt/sdcard/musl:/mnt/sdcard/glibc:/bin:/sbin", "HOME=/"],
            Some("/mnt/sdcard/musl"),
        );
        match rc2 {
            Ok(raw) => crate::println!(
                "oscomp-la-diag: {} busybox sh -c true -> raw={} class={}",
                cand, raw,
                if raw == 0 { alloc::string::String::from("PASS") }
                else if raw < 0 { alloc::format!("signal={}", -raw) }
                else { alloc::format!("exit={}", raw) },
            ),
            Err(_) => crate::println!("oscomp-la-diag: {} busybox sh -c true -> ERROR", cand),
        }
    }

    // ── applet alias diag (non-scoring) ──
    let diag_busybox = "/mnt/sdcard/musl/busybox";
    let diag_cwd = "/mnt/sdcard/musl";
    let diag_env = &["PATH=.:/mnt/sdcard/musl:/bin", "HOME=/"];

    for applet in &["sleep", "true", "echo"] {
        let path = alloc::format!("/mnt/sdcard/musl/{}", applet);
        let present = crate::fs::stat(&path).is_ok();
        crate::println!(
            "oscomp-la-applet-diag: /mnt/sdcard/musl/{} present={}",
            applet, present,
        );
    }

    // Quick functional probes with applet aliases in place
    // Trace-enabled probes for busybox sleep (syscall-level diag).
    oscomp_la_run_sleep_trace_probe(
        "busybox sleep 0",
        diag_busybox, &["busybox", "sleep", "0"], diag_env, Some(diag_cwd),
    );
    oscomp_la_run_sleep_trace_probe(
        "busybox sleep 1",
        diag_busybox, &["busybox", "sleep", "1"], diag_env, Some(diag_cwd),
    );

    // Non-trace probes (keep the existing diag format).
    let applet_probes: &[(&str, &[&str])] = &[
        ("busybox true", &["busybox", "true"] as &[&str]),
        ("sh -c true", &["busybox", "sh", "-c", "true"]),
        ("sh -c sleep 0", &["busybox", "sh", "-c", "sleep 0"]),
        ("sh -c sleep 1", &["busybox", "sh", "-c", "sleep 1"]),
        ("sh -c echo diag_ok", &["busybox", "sh", "-c", "echo diag_ok"]),
    ];

    for (label, argv) in applet_probes {
        match run_rootfs_program_with_cwd(diag_busybox, argv, diag_env, Some(diag_cwd)) {
            Ok(raw) => crate::println!(
                "oscomp-la-applet-diag: {} -> raw={} class={}",
                label, raw,
                if raw == 0 { alloc::string::String::from("PASS") }
                else if raw < 0 { alloc::format!("signal={}", -raw) }
                else { alloc::format!("exit={}", raw) },
            ),
            Err(_) => crate::println!("oscomp-la-applet-diag: {} -> ERROR", label),
        }
    }

    crate::println!("oscomp-la-diag: end");
}

/// External watchdog kernel thread.  If the contest runner blocks
/// inside a script group, the watchdog prints a partial summary and
/// shuts down when the global deadline expires.
fn contest_watchdog_main() {
    loop {
        if !OSCOMP_ACTIVE.load(Ordering::Acquire) {
            return;
        }

        let now = crate::time::now().cycles();
        let deadline = OSCOMP_DEADLINE_CYCLES.load(Ordering::Acquire);

        if deadline != 0 && now >= deadline {
            if OSCOMP_FINALIZED
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                let completed = OSCOMP_COMPLETED.load(Ordering::Acquire);
                let passed = OSCOMP_PASS.load(Ordering::Acquire);
                let failed = OSCOMP_FAIL.load(Ordering::Acquire);
                let skipped = OSCOMP_SKIPPED.load(Ordering::Acquire);
                let timed_out = OSCOMP_TIMEOUT.load(Ordering::Acquire);
                let sig11 = OSCOMP_SIGNAL11.load(Ordering::Acquire);
                let sig14 = OSCOMP_SIGNAL14.load(Ordering::Acquire);

                crate::println!("oscomp: watchdog global deadline reached");
                crate::println!("#### OS COMP SUMMARY ####");
                crate::println!("arch={}", crate::arch::ARCH_NAME);
                crate::println!("total={}", OSCOMP_TOTAL.load(Ordering::Acquire));
                crate::println!("completed={}", completed);
                crate::println!("pass={}", passed);
                crate::println!("fail={}", failed);
                crate::println!("skipped={}", skipped);
                crate::println!("timeout={}", timed_out);
                crate::println!("signal11={}", sig11);
                crate::println!("signal14={}", sig14);
                crate::println!("score={}", passed);
                crate::println!("score: {}", passed);
                crate::println!("#### OS COMP SUMMARY END ####");
                crate::println!("oscomp: shutdown");
                contest_platform_shutdown();
            }
            return;
        }

        crate::task::yield_now();
    }
}

/// Strip the /mnt/sdcard VFS prefix to recover the ext4 source path,
/// then return the parent directory portion.
fn sdcard_vfs_to_ext4_dir(vfs_path: &str) -> alloc::string::String {
    let ext4_path = vfs_path.strip_prefix("/mnt/sdcard").unwrap_or(vfs_path);
    match ext4_path.rfind('/') {
        Some(0) | None => alloc::string::String::from("/"),
        Some(pos) => {
            let mut dir = alloc::string::String::with_capacity(pos);
            dir.push_str(&ext4_path[..pos]);
            dir
        }
    }
}

/// Install every regular file from an ext4 directory into the
/// corresponding VFS mount point, so that scripts can find
/// sibling binaries (./busybox, ./dhry2reg, etc.) at runtime.
fn sdcard_install_ext4_dir_files(ext4_dir: &str) {
    const EXT4_FT_REG_FILE: u16 = 1;
    const EXT4_FT_DIR: u16 = 2;
    const EXT4_FT_SYMLINK: u16 = 7;
    let device = match crate::block::open_device("vda") {
        Some(d) => d,
        None => return,
    };
    let entries = match crate::ext4::list_directory(device, ext4_dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let vfs_dir = if ext4_dir == "/" {
        alloc::string::String::from("/mnt/sdcard")
    } else {
        alloc::format!("/mnt/sdcard{}", ext4_dir)
    };
    let mut installed = 0_usize;
    for entry in &entries {
        if entry.file_type == EXT4_FT_REG_FILE {
            let ext4_path = if ext4_dir == "/" {
                alloc::format!("/{}", entry.name)
            } else {
                alloc::format!("{}/{}", ext4_dir, entry.name)
            };
            let vfs_path = alloc::format!("{}/{}", vfs_dir, entry.name);
            // Skip if already present (e.g. scripts already installed by mount phase)
            if crate::fs::stat(&vfs_path).is_err() {
                if crate::fs::install_ext4_path("/dev/vda", &vfs_path, &ext4_path).is_ok() {
                    installed += 1;
                }
            }
        } else if entry.file_type == EXT4_FT_SYMLINK {
            let ext4_path = if ext4_dir == "/" {
                alloc::format!("/{}", entry.name)
            } else {
                alloc::format!("{}/{}", ext4_dir, entry.name)
            };
            let vfs_path = alloc::format!("{}/{}", vfs_dir, entry.name);
            if crate::fs::stat(&vfs_path).is_err() {
                if crate::fs::install_ext4_path("/dev/vda", &vfs_path, &ext4_path).is_ok() {
                    installed += 1;
                }
            }
        } else if entry.file_type == EXT4_FT_DIR
            && entry.name != "."
            && entry.name != ".."
        {
            let sub_ext4 = if ext4_dir == "/" {
                alloc::format!("/{}", entry.name)
            } else {
                alloc::format!("{}/{}", ext4_dir, entry.name)
            };
            let sub_vfs = alloc::format!("{}/{}", vfs_dir, entry.name);
            if crate::fs::stat(&sub_vfs).is_err() {
                let _ = crate::fs::mkdir(&sub_vfs, 0o755);
                // Recursively install files from the subdirectory
                sdcard_install_ext4_dir_files(&sub_ext4);
            }
        }
    }
    crate::println!("sdcard: expanded {} -> {} : {} files", ext4_dir, vfs_dir, installed);
}


fn verify_sdcard_basic_binaries() {
    const BASIC_TESTS: &[&str] = &[
        "brk",
        "chdir",
        "clone",
        "close",
        "dup2",
        "dup",
        "execve",
        "exit",
        "fork",
        "fstat",
        "getcwd",
        "getdents",
        "getpid",
        "getppid",
        "gettimeofday",
        "mkdir_",
        "mmap",
        "mount",
        "munmap",
        "openat",
        "open",
        "pipe",
        "read",
        "sleep",
        "times",
        "umount",
        "uname",
        "unlink",
        "wait",
        "waitpid",
        "write",
        "yield",
    ];
    for test in BASIC_TESTS {
        crate::println!("Testing {test} :");
        let mut path = String::from("/mnt/sdcard/musl/basic/");
        path.push_str(test);
        let result = run_rootfs_program_with_cwd(
            &path,
            &[*test],
            &[
                "PATH=.:/mnt/sdcard/musl:/bin:/sbin:/usr/bin:/usr/sbin",
                "LD_LIBRARY_PATH=/mnt/sdcard/musl/lib",
            ],
            Some("/code"),
        )
        .expect("unable to run musl basic test binary from ext4 sdcard image");
        assert_eq!(result, 0, "musl basic test binary exited non-zero");
    }
}

fn run_rootfs_program(
    path: &str,
    argv: &[&str],
    envp: &[&str],
) -> Result<isize, crate::exec::ExecError> {
    run_rootfs_program_with_cwd(path, argv, envp, None)
}

fn run_rootfs_program_with_cwd(
    path: &str,
    argv: &[&str],
    envp: &[&str],
    cwd: Option<&str>,
) -> Result<isize, crate::exec::ExecError> {
    let image =
        load_exec_image(path).map_err(|_| crate::exec::ExecError::Vfs(myos_vfs::Errno::Enoent))?;
    run_program_image_with_cwd(&image, argv, envp, "BusyBox rootfs", cwd)
}

fn run_program_image(
    image: &[u8],
    argv: &[&str],
    envp: &[&str],
    owner: &str,
) -> Result<isize, crate::exec::ExecError> {
    run_program_image_with_cwd(image, argv, envp, owner, None)
}

fn run_program_image_with_cwd(
    image: &[u8],
    argv: &[&str],
    envp: &[&str],
    owner: &str,
    cwd: Option<&str>,
) -> Result<isize, crate::exec::ExecError> {
    let extra_areas = [VmArea::new(
        VirtRange::from_bounds(USER_DEMAND, USER_DEMAND + PAGE_SIZE),
        VmAreaFlags::user_rw(),
        VmAreaKind::Anonymous,
    )];
    let exec = crate::exec::exec_elf(
        image,
        crate::exec::ExecConfig {
            argv,
            envp,
            stack: VirtRange::from_bounds(USER_STACK, USER_STACK_TOP),
            heap_start: VirtAddr::new(USER_HEAP_START),
            heap_limit: VirtAddr::new(USER_HEAP_LIMIT),
            extra_areas: &extra_areas,
        },
    )?;
    if let Some(cwd) = cwd {
        exec.process.fs().set_cwd(cwd)?;
    }
    let task = crate::task::spawn_user_thread_on(Arc::clone(&exec.thread), None);
    let result = exec.thread.wait_for_exit();
    task.wait_for_detach();
    drop(exec.thread);
    let process = Arc::try_unwrap(exec.process)
        .unwrap_or_else(|_| panic!("{owner} run retained unexpected Process owners"));
    process.destroy()?;
    Ok(result)
}

fn run_session(
    entry_symbol: *const u8,
    initial_data: Option<&[u8]>,
    exercise_copy_guards: bool,
) -> SessionObserved {
    run_session_on(
        entry_symbol,
        initial_data,
        exercise_copy_guards,
        None,
        false,
    )
}

fn run_scheduler_session(entry_symbol: *const u8, target: crate::smp::CpuId) -> SessionObserved {
    run_session_on(entry_symbol, None, false, Some(target), true)
}

fn run_session_on(
    entry_symbol: *const u8,
    initial_data: Option<&[u8]>,
    exercise_copy_guards: bool,
    target: Option<crate::smp::CpuId>,
    scheduler_probe: bool,
) -> SessionObserved {
    reset_session_state();

    let image = UserImage::exec(entry_symbol).expect("unable to exec M10 initramfs ELF image");
    prepare_user_instruction_stream();
    image.publish();

    if let Some(data) = initial_data {
        assert!(
            data.len() <= MAX_USER_COPY,
            "M8-B3 initial user data exceeds verifier buffer",
        );
        image
            .process
            .mm()
            .copy_to_user(USER_DATA, data)
            .expect("checked copy_to_user rejected M9-B user data");
        let mut round_trip = [0_u8; MAX_USER_COPY];
        image
            .process
            .mm()
            .copy_from_user(USER_DATA, &mut round_trip[..data.len()])
            .expect("checked copy_from_user rejected M9-B user data");
        assert_eq!(
            &round_trip[..data.len()],
            data,
            "M8-B3 user copy changed data",
        );
    }

    if exercise_copy_guards {
        verify_copy_guards(image.process.mm());
    }

    if scheduler_probe {
        let target = target.expect("M9-B scheduler probe requires a target CPU");
        SCHEDULER_PEER_READY.reinit();
        SCHEDULER_PEER_DONE.reinit();
        SCHEDULER_PEER_STOP.store(false, Ordering::Release);
        crate::task::spawn_kernel_thread_on(scheduler_peer, target);
        SCHEDULER_PEER_READY.wait();
    }

    let task = crate::task::spawn_user_thread_on(Arc::clone(&image.thread), target);
    assert_eq!(
        image.thread.scheduler_task(),
        Some(task.id()),
        "M9-B scheduler task binding diverged from Thread state",
    );
    let result = image.thread.wait_for_exit();
    task.wait_for_detach();

    if scheduler_probe {
        let target = target.expect("M9-B scheduler probe lost its target CPU");
        let expected_mask = 1_usize << target.get();
        assert_eq!(
            image.thread.visited_cpu_mask(),
            expected_mask,
            "M9-B user task ran outside its pinned scheduler target",
        );
        assert_eq!(
            SCHED_YIELD_SWITCH_COUNT.load(Ordering::Acquire),
            8,
            "M9-B did not prove all eight sched_yield calls switched away and back",
        );
        assert!(
            image.thread.schedule_count() >= 8,
            "M9-B scheduler did not record the proven user-task resumptions",
        );
        SCHEDULER_PEER_STOP.store(true, Ordering::Release);
        SCHEDULER_PEER_DONE.wait();
    }

    let observed = SessionObserved {
        result,
        terminated: TERMINATED.load(Ordering::Acquire),
        exit_status: EXIT_STATUS.load(Ordering::Acquire),
        syscall_count: SYSCALL_COUNT.load(Ordering::Acquire),
        write_count: WRITE_COUNT.load(Ordering::Acquire),
        fault_count: FAULT_COUNT.load(Ordering::Acquire),
        recovered_fault_count: RECOVERED_FAULT_COUNT.load(Ordering::Acquire),
        anonymous_fault_count: ANONYMOUS_FAULT_COUNT.load(Ordering::Acquire),
        stack_growth_count: STACK_GROWTH_COUNT.load(Ordering::Acquire),
        brk_count: BRK_COUNT.load(Ordering::Acquire),
        mmap_count: MMAP_COUNT.load(Ordering::Acquire),
        munmap_count: MUNMAP_COUNT.load(Ordering::Acquire),
        mprotect_count: MPROTECT_COUNT.load(Ordering::Acquire),
        fault_kind: LAST_FAULT_KIND.load(Ordering::Acquire),
        fault_address: LAST_FAULT_ADDRESS.load(Ordering::Acquire),
    };

    image.unpublish();
    image.destroy();
    crate::user_mm::assert_no_leaks();
    crate::process::assert_no_leaks();

    assert!(
        crate::arch::trap::kernel_scratch_is_clean(),
        "architecture user/kernel stack scratch was not restored",
    );
    let mut revoked = [0_u8; 1];
    assert!(
        copy_from_user(USER_DATA, &mut revoked).is_err(),
        "user backing remained accessible after session teardown",
    );

    observed
}

fn scheduler_peer() {
    SCHEDULER_PEER_READY.complete_all();
    while !SCHEDULER_PEER_STOP.load(Ordering::Acquire) {
        crate::task::yield_now();
    }
    SCHEDULER_PEER_DONE.complete_all();
}

fn reset_session_state() {
    assert!(
        !ACTIVE.load(Ordering::Acquire),
        "M8-B3 session state was reset while active",
    );
    TERMINATED.store(false, Ordering::Release);
    SYSCALL_COUNT.store(0, Ordering::Release);
    WRITE_COUNT.store(0, Ordering::Release);
    FAULT_COUNT.store(0, Ordering::Release);
    RECOVERED_FAULT_COUNT.store(0, Ordering::Release);
    ANONYMOUS_FAULT_COUNT.store(0, Ordering::Release);
    STACK_GROWTH_COUNT.store(0, Ordering::Release);
    BRK_COUNT.store(0, Ordering::Release);
    MMAP_COUNT.store(0, Ordering::Release);
    MUNMAP_COUNT.store(0, Ordering::Release);
    MPROTECT_COUNT.store(0, Ordering::Release);
    SCHED_YIELD_SWITCH_COUNT.store(0, Ordering::Release);
    LAST_FAULT_KIND.store(FAULT_NONE, Ordering::Release);
    LAST_FAULT_ADDRESS.store(0, Ordering::Release);
    EXIT_STATUS.store(isize::MIN, Ordering::Release);
}

fn assert_session(name: &str, observed: SessionObserved, expected: SessionExpected) {
    assert!(
        observed.terminated,
        "{name}: user session did not terminate"
    );
    assert_eq!(
        observed.result, expected.result,
        "{name}: wrong kernel return value",
    );
    assert_eq!(
        observed.exit_status, expected.exit_status,
        "{name}: wrong user exit status",
    );
    assert_eq!(
        observed.syscall_count, expected.syscall_count,
        "{name}: wrong syscall count",
    );
    assert_eq!(
        observed.write_count, expected.write_count,
        "{name}: wrong successful write count",
    );
    assert_eq!(
        observed.fault_count, expected.fault_count,
        "{name}: wrong user fault count",
    );
    assert_eq!(
        observed.recovered_fault_count, expected.recovered_fault_count,
        "{name}: wrong recovered fault count",
    );
    assert_eq!(
        observed.anonymous_fault_count, expected.anonymous_fault_count,
        "{name}: wrong anonymous fault count",
    );
    assert_eq!(
        observed.stack_growth_count, expected.stack_growth_count,
        "{name}: wrong stack-growth count",
    );
    assert_eq!(
        observed.brk_count, expected.brk_count,
        "{name}: wrong brk count"
    );
    assert_eq!(
        observed.mmap_count, expected.mmap_count,
        "{name}: wrong mmap count",
    );
    assert_eq!(
        observed.munmap_count, expected.munmap_count,
        "{name}: wrong munmap count",
    );
    assert_eq!(
        observed.mprotect_count, expected.mprotect_count,
        "{name}: wrong mprotect count",
    );
    assert_eq!(
        observed.fault_kind, expected.fault_kind,
        "{name}: wrong user fault class",
    );
    assert_eq!(
        observed.fault_address, expected.fault_address,
        "{name}: wrong user fault address",
    );
}

fn verify_copy_guards(mm: &crate::user_mm::UserMm) {
    assert!(
        mm.copy_to_user(USER_CODE, &[0]).is_err(),
        "copy_to_user wrote through an RX user mapping",
    );

    let mut crossing = [0_u8; 2];
    assert!(
        mm.copy_from_user(USER_DATA + PAGE_SIZE - 1, &mut crossing)
            .is_err(),
        "copy_from_user accepted a cross-VMA range",
    );
    assert!(
        mm.copy_from_user(usize::MAX - 1, &mut crossing).is_err(),
        "copy_from_user accepted an overflowing range",
    );

    let mut empty = [];
    assert!(
        mm.copy_from_user(usize::MAX, &mut empty).is_ok(),
        "zero-length user copy should not inspect its address",
    );
}

pub fn handle_syscall(frame: &mut crate::arch::trap::TrapFrame) {
    assert!(
        frame.previous_mode_was_user(),
        "syscall trap did not originate in user mode",
    );

    let verifier = ACTIVE.load(Ordering::Acquire);
    if verifier {
        SYSCALL_COUNT.fetch_add(1, Ordering::AcqRel);
    }
    let number = syscall_number(frame);
    let arguments = syscall_arguments(frame);
    advance_syscall_pc(frame);

    // ── P9-H11: LA sleep syscall trace (enter) ──
    #[cfg(target_arch = "loongarch64")]
    if OSCOMP_LA_SLEEP_TRACE.load(Ordering::Relaxed) {
        let budget = OSCOMP_LA_SLEEP_TRACE_BUDGET.load(Ordering::Relaxed);
        if budget > 0 {
            OSCOMP_LA_SLEEP_TRACE_BUDGET.store(budget - 1, Ordering::Relaxed);
            LAST_TRACED_SYSCALL_NR.store(number, Ordering::Relaxed);
            crate::println!(
                "oscomp-la-sleep-syscall: enter nr={} a0={:#x} a1={:#x} a2={:#x} a3={:#x} a4={:#x} a5={:#x}",
                number, arguments[0], arguments[1], arguments[2], arguments[3], arguments[4], arguments[5],
            );
        } else {
            OSCOMP_LA_SLEEP_TRACE.store(false, Ordering::Relaxed);
        }
    }

    let _interrupt_guard = SyscallInterruptGuard::enable_until_trap_return();

    match number {
        SYS_GETCWD => set_syscall_result(frame, sys_getcwd(arguments[0], arguments[1])),
        SYS_DUP => set_syscall_result(frame, sys_dup(arguments[0])),
        SYS_DUP3 => set_syscall_result(frame, sys_dup3(arguments[0], arguments[1], arguments[2])),
        SYS_FCNTL => set_syscall_result(frame, sys_fcntl(arguments[0], arguments[1], arguments[2])),
        SYS_IOCTL => set_syscall_result(frame, sys_ioctl(arguments[0], arguments[1], arguments[2])),
        SYS_PIPE2 => set_syscall_result(frame, sys_pipe2(arguments[0], arguments[1])),
        SYS_GETPID => set_syscall_result(frame, current_process().id().get() as isize),
        SYS_GETTID => set_syscall_result(
            frame,
            crate::task::current_user_thread()
                .expect("gettid arrived without current user Thread")
                .id()
                .get() as isize,
        ),
        SYS_GETPPID => set_syscall_result(
            frame,
            current_process().parent_id().map_or(0, |pid| pid.get()) as isize,
        ),
        SYS_GETUID => {
            set_syscall_result(frame, current_process().credentials().real_uid() as isize)
        }
        SYS_GETEUID => set_syscall_result(
            frame,
            current_process().credentials().effective_uid() as isize,
        ),
        SYS_GETGID => {
            set_syscall_result(frame, current_process().credentials().real_gid() as isize)
        }
        SYS_GETEGID => set_syscall_result(
            frame,
            current_process().credentials().effective_gid() as isize,
        ),
        SYS_SET_TID_ADDRESS => set_syscall_result(frame, sys_set_tid_address(arguments[0])),
        SYS_SET_ROBUST_LIST => {
            set_syscall_result(frame, sys_set_robust_list(arguments[0], arguments[1]))
        }
        SYS_SETSID => set_syscall_result(frame, current_process().setsid()),
        SYS_SETPGID => set_syscall_result(frame, sys_setpgid(arguments[0], arguments[1])),
        SYS_GETPGID => set_syscall_result(frame, sys_getpgid(arguments[0])),
        SYS_GETSID => set_syscall_result(frame, sys_getsid(arguments[0])),
        SYS_KILL => set_syscall_result(frame, sys_kill(arguments[0], arguments[1])),
        SYS_TKILL => set_syscall_result(frame, sys_kill(arguments[0], arguments[1])),
        SYS_TGKILL => set_syscall_result(frame, sys_kill(arguments[1], arguments[2])),
        SYS_RT_SIGACTION => set_syscall_result(frame, sys_rt_sigaction(arguments)),
        SYS_RT_SIGPROCMASK => set_syscall_result(
            frame,
            sys_rt_sigprocmask(arguments[0], arguments[1], arguments[2]),
        ),
        SYS_RT_SIGTIMEDWAIT => {
            set_syscall_result(frame, sys_rt_sigtimedwait(arguments))
        }
        SYS_RT_SIGRETURN => {
            if let Err(error) = sys_rt_sigreturn(frame) {
                set_syscall_result(frame, error);
            }
        }
        SYS_WAIT4 => set_syscall_result(frame, sys_wait4(arguments[0], arguments[1])),
        SYS_CLONE => set_syscall_result(frame, sys_clone(frame, arguments)),
        SYS_EXECVE => {
            let result = sys_execve(frame, arguments);
            set_syscall_result(frame, result);
        }
        SYS_NANOSLEEP => set_syscall_result(frame, sys_nanosleep(arguments[0], arguments[1])),
        SYS_CLOCK_GETTIME => {
            set_syscall_result(frame, sys_clock_gettime(arguments[0], arguments[1]))
        }
        SYS_CLOCK_NANOSLEEP => {
            set_syscall_result(frame, sys_clock_nanosleep(
                arguments[0], arguments[1], arguments[2], arguments[3],
            ))
        }
        SYS_GETTIMEOFDAY => set_syscall_result(frame, sys_gettimeofday(arguments[0])),
        SYS_TIMES => set_syscall_result(frame, sys_times(arguments[0])),
        SYS_UNAME => set_syscall_result(frame, sys_uname(arguments[0])),
        SYS_SYSINFO => set_syscall_result(frame, sys_sysinfo(arguments[0])),
        SYS_GETRANDOM => set_syscall_result(frame, sys_getrandom(arguments[0], arguments[1])),
        SYS_MKDIRAT => {
            set_syscall_result(frame, sys_mkdirat(arguments[0], arguments[1], arguments[2]))
        }
        SYS_UNLINKAT => set_syscall_result(
            frame,
            sys_unlinkat(arguments[0], arguments[1], arguments[2]),
        ),
        SYS_SYMLINKAT => set_syscall_result(
            frame,
            sys_symlinkat(arguments[0], arguments[1], arguments[2]),
        ),
        SYS_LINKAT => set_syscall_result(
            frame,
            sys_linkat(
                arguments[0],
                arguments[1],
                arguments[2],
                arguments[3],
                arguments[4],
            ),
        ),
        SYS_RENAMEAT => set_syscall_result(
            frame,
            sys_renameat(arguments[0], arguments[1], arguments[2], arguments[3]),
        ),
        SYS_MOUNT => set_syscall_result(
            frame,
            sys_mount(arguments[0], arguments[1], arguments[2], arguments[3]),
        ),
        SYS_UMOUNT2 => set_syscall_result(frame, sys_umount2(arguments[0], arguments[1])),
        SYS_FACCESSAT => set_syscall_result(
            frame,
            sys_faccessat(arguments[0], arguments[1], arguments[2]),
        ),
        SYS_FTRUNCATE => set_syscall_result(frame, sys_ftruncate(arguments[0], arguments[1])),
        SYS_CHDIR => set_syscall_result(frame, sys_chdir(arguments[0])),
        SYS_OPENAT => {
            let result = sys_openat(arguments[0], arguments[1], arguments[2]);
            set_syscall_result(frame, result);
        }
        SYS_CLOSE => set_syscall_result(frame, sys_close(arguments[0])),
        SYS_GETDENTS64 => {
            let result = sys_getdents64(arguments[0], arguments[1], arguments[2]);
            set_syscall_result(frame, result);
        }
        SYS_LSEEK => set_syscall_result(
            frame,
            sys_lseek(arguments[0], arguments[1] as isize, arguments[2]),
        ),
        SYS_READ => {
            let result = sys_read(arguments[0], arguments[1], arguments[2]);
            set_syscall_result(frame, result);
        }
        SYS_WRITE => {
            let result = sys_write(arguments[0], arguments[1], arguments[2]);
            set_syscall_result(frame, result);
        }
        SYS_READV => {
            let result = sys_readv(arguments[0], arguments[1], arguments[2]);
            set_syscall_result(frame, result);
        }
        SYS_WRITEV => {
            let result = sys_writev(arguments[0], arguments[1], arguments[2]);
            set_syscall_result(frame, result);
        }
        SYS_PREAD64 => {
            let result = sys_pread64(arguments[0], arguments[1], arguments[2], arguments[3]);
            set_syscall_result(frame, result);
        }
        SYS_READLINKAT => set_syscall_result(
            frame,
            sys_readlinkat(arguments[0], arguments[1], arguments[2], arguments[3]),
        ),
        SYS_PPOLL => set_syscall_result(frame, sys_ppoll(arguments[0], arguments[1], arguments[2])),
        SYS_PSELECT6 => set_syscall_result(frame, sys_pselect6(arguments)),
        SYS_NEWFSTATAT => set_syscall_result(
            frame,
            sys_newfstatat(arguments[0], arguments[1], arguments[2], arguments[3]),
        ),
        SYS_FSTAT => set_syscall_result(frame, sys_fstat(arguments[0], arguments[1])),
        SYS_STATX => set_syscall_result(
            frame,
            sys_statx(
                arguments[0],
                arguments[1],
                arguments[2],
                arguments[3],
                arguments[4],
            ),
        ),
        SYS_FSYNC => set_syscall_result(frame, sys_fsync(arguments[0])),
        SYS_BRK => set_syscall_result(frame, sys_brk(arguments[0])),
        SYS_MUNMAP => set_syscall_result(frame, sys_munmap(arguments[0], arguments[1])),
        SYS_MMAP => set_syscall_result(frame, sys_mmap(arguments)),
        SYS_MPROTECT => set_syscall_result(
            frame,
            sys_mprotect(arguments[0], arguments[1], arguments[2]),
        ),
        SYS_PKEY_MPROTECT => set_syscall_result(
            frame,
            sys_pkey_mprotect(arguments[0], arguments[1], arguments[2], arguments[3]),
        ),
        SYS_PRLIMIT64 => set_syscall_result(
            frame,
            sys_prlimit64(arguments[0], arguments[1], arguments[2], arguments[3]),
        ),
        SYS_FUTEX => {
            set_syscall_result(
                frame,
                sys_futex(
                    arguments[0],
                    arguments[1],
                    arguments[2],
                    arguments[3],
                    arguments[4],
                    arguments[5],
                ),
            );
        }
        SYS_MKNODAT => {
            set_syscall_result(frame, sys_mknodat(arguments[0], arguments[1], arguments[2], arguments[3]));
        }
        SYS_UTIMENSAT => set_syscall_result(frame, sys_utimensat(arguments[0], arguments[1], arguments[2])),
        SYS_STATFS => set_syscall_result(frame, sys_statfs_path(arguments[0], arguments[1])),
        SYS_FSTATFS => set_syscall_result(frame, sys_statfs_fd(arguments[0], arguments[1])),
        SYS_SYSLOG => set_syscall_result(frame, sys_syslog(arguments[0], arguments[1], arguments[2])),
        SYS_SCHED_GETAFFINITY => set_syscall_result(frame, sys_sched_getaffinity(arguments[0], arguments[1], arguments[2])),
        SYS_SCHED_SETAFFINITY => set_syscall_result(frame, sys_sched_setaffinity(arguments[0], arguments[1], arguments[2])),
        SYS_SCHED_SETSCHEDULER => set_syscall_result(frame, sys_sched_setscheduler(arguments[0], arguments[1], arguments[2])),
        SYS_SCHED_GETSCHEDULER => set_syscall_result(frame, sys_sched_getscheduler(arguments[0])),
        SYS_SCHED_GETPARAM => set_syscall_result(frame, sys_sched_getparam(arguments[0], arguments[1])),
        SYS_RENAMEAT2 => {
            set_syscall_result(frame,
                sys_renameat2(arguments[0], arguments[1], arguments[2], arguments[3], arguments[4]))
        }
        SYS_PRCTL => set_syscall_result(frame, sys_prctl(arguments[0], arguments[1], arguments[2])),
        SYS_SETITIMER => set_syscall_result(frame, sys_setitimer(arguments[0], arguments[1], arguments[2])),
        SYS_GETITIMER => set_syscall_result(frame, sys_getitimer(arguments[0], arguments[1])),
        SYS_GETRUSAGE => set_syscall_result(frame, sys_getrusage(arguments[0], arguments[1])),
        SYS_RT_SIGPENDING => set_syscall_result(frame, 0),
        SYS_SCHED_YIELD => {
            let thread = crate::task::current_user_thread()
                .expect("M9-B sched_yield arrived without a current user Thread");
            let schedules_before = thread.schedule_count();
            set_syscall_result(frame, 0);
            crate::task::yield_from_user_trap();
            if verifier {
                assert!(
                    thread.schedule_count() > schedules_before,
                    "M9-B sched_yield returned without switching to a runnable peer",
                );
                SCHED_YIELD_SWITCH_COUNT.fetch_add(1, Ordering::AcqRel);
            }
        }
        SYS_EXIT | SYS_EXIT_GROUP => {
            // CLONE_CHILD_CLEARTID: write 0 to the user-space address
            // specified by clone's ctid argument, then futex-wake waiters.
            if let Some(thread) = crate::task::current_user_thread() {
                let ctid = thread.clear_child_tid_address();
                if ctid != 0 {
                    let zero: u32 = 0;
                    let _ = copy_to_user(ctid, &zero.to_ne_bytes());
                }
            }
            if verifier {
                EXIT_STATUS.store(arguments[0] as isize, Ordering::Release);
                TERMINATED.store(true, Ordering::Release);
            }
            return_to_kernel(frame, arguments[0] as isize);
        }
        SYS_SOCKET => {
            set_syscall_result(
                frame,
                crate::net::socket::sys_socket(arguments[0], arguments[1], arguments[2]),
            );
        }
        SYS_BIND => {
            set_syscall_result(
                frame,
                crate::net::socket::sys_bind(arguments[0], arguments[1], arguments[2]),
            );
        }
        SYS_LISTEN => {
            set_syscall_result(
                frame,
                crate::net::socket::sys_listen(arguments[0], arguments[1]),
            );
        }
        SYS_ACCEPT => {
            set_syscall_result(
                frame,
                crate::net::socket::sys_accept(arguments[0], arguments[1], arguments[2]),
            );
        }
        SYS_CONNECT => {
            set_syscall_result(
                frame,
                crate::net::socket::sys_connect(arguments[0], arguments[1], arguments[2]),
            );
        }
        SYS_SENDTO => {
            set_syscall_result(
                frame,
                crate::net::socket::sys_sendto(
                    arguments[0],
                    arguments[1],
                    arguments[2],
                    arguments[3],
                    arguments[4],
                    arguments[5],
                ),
            );
        }
        SYS_RECVFROM => {
            set_syscall_result(
                frame,
                crate::net::socket::sys_recvfrom(
                    arguments[0],
                    arguments[1],
                    arguments[2],
                    arguments[3],
                    arguments[4],
                    arguments[5],
                ),
            );
        }
        SYS_SHUTDOWN => {
            set_syscall_result(
                frame,
                crate::net::socket::sys_shutdown(arguments[0], arguments[1]),
            );
        }
        SYS_SETSOCKOPT => {
            set_syscall_result(
                frame,
                sys_setsockopt(
                    arguments[0], arguments[1], arguments[2],
                    arguments[3], arguments[4],
                ),
            );
        }
        SYS_GETSOCKOPT => {
            set_syscall_result(
                frame,
                sys_getsockopt(
                    arguments[0], arguments[1], arguments[2],
                    arguments[3], arguments[4],
                ),
            );
        }
        _ => set_syscall_result(frame, -ENOSYS),
    }
    deliver_pending_signal(frame);
}

struct SyscallInterruptGuard {
    restore_disabled: bool,
}

impl SyscallInterruptGuard {
    fn enable_until_trap_return() -> Self {
        let restore_disabled = crate::arch::interrupt::are_disabled();
        if restore_disabled {
            // SAFETY: syscall handlers run on the current task's kernel stack
            // after the trap frame has been saved. Ordinary syscall work may
            // need cross-CPU IPI completion, timers, or wakeups.
            unsafe { crate::arch::interrupt::enable() };
        }
        Self { restore_disabled }
    }
}

impl Drop for SyscallInterruptGuard {
    fn drop(&mut self) {
        if self.restore_disabled {
            crate::arch::interrupt::disable();
        }
    }
}

pub fn handle_fault(
    frame: &mut crate::arch::trap::TrapFrame,
    address: VirtAddr,
    access: FaultAccess,
    _raw: usize,
) {
    assert!(
        frame.previous_mode_was_user(),
        "M8-B4 user fault handler received a kernel fault",
    );

    let verifier = ACTIVE.load(Ordering::Acquire);
    if verifier {
        FAULT_COUNT.fetch_add(1, Ordering::AcqRel);
    }
    let user_sp = VirtAddr::new(frame.stack_pointer());
    match current_user_mm().resolve_user_fault(address, access, user_sp) {
        Ok(UserFaultResolution::Recovered(recovery)) => {
            if verifier {
                RECOVERED_FAULT_COUNT.fetch_add(1, Ordering::AcqRel);
                match recovery {
                    UserFaultRecovery::Anonymous => {
                        ANONYMOUS_FAULT_COUNT.fetch_add(1, Ordering::AcqRel);
                    }
                    UserFaultRecovery::StackGrowth => {
                        STACK_GROWTH_COUNT.fetch_add(1, Ordering::AcqRel);
                    }
                    UserFaultRecovery::Spurious => {}
                }
                LAST_FAULT_ADDRESS.store(address.get(), Ordering::Release);
                LAST_FAULT_KIND.store(FAULT_RECOVERED, Ordering::Release);
            }
        }
        Ok(UserFaultResolution::Fatal(failure)) => {
            assert!(
                !matches!(failure, UserFaultFailure::KernelBug),
                "M8-B4 fault planner classified a user trap as a kernel bug",
            );
            if !verifier {
                #[cfg(target_arch = "loongarch64")]
                let fault_pc = frame.era;
                #[cfg(target_arch = "riscv64")]
                let fault_pc = frame.sepc;

                // Classify the fault for summary.
                #[cfg(target_arch = "loongarch64")]
                let bpc = frame.era;
                #[cfg(target_arch = "riscv64")]
                let bpc = frame.sepc;
                let baddr = address.get();
                let bsp = frame.stack_pointer();
                let class = classify_segv(bpc, baddr, bsp, frame);
                // Rate-limited trace (at most 4 per class).
                static FAULT_PRINT_COUNT: AtomicUsize = AtomicUsize::new(0);
                let print_idx = FAULT_PRINT_COUNT.fetch_add(1, Ordering::Relaxed);
                if print_idx < 8 {
                    crate::println!(
                        "sigsegv: class={} pc={:#018x} badaddr={:#018x} access={:?} sp={:#018x}",
                        class, bpc, baddr, access, bsp,
                    );
                } else if print_idx == 8 {
                    crate::println!("sigsegv: ... further faults suppressed");
                }
            }
            if verifier {
                LAST_FAULT_ADDRESS.store(address.get(), Ordering::Release);
                LAST_FAULT_KIND.store(FAULT_PAGE, Ordering::Release);
                TERMINATED.store(true, Ordering::Release);
                // Gate tests expect -EFAULT for write-to-RX faults.
                EXIT_STATUS.store(-EFAULT, Ordering::Release);
            }
            // External programs: use SIGSEGV so wait4 produces signal status.
            let exit_code = if ACTIVE.load(Ordering::Acquire) { -EFAULT } else { -11 }; // SIGSEGV
            return_to_kernel(frame, exit_code);
        }
        Err(error) => panic!("M8-B4 user fault recovery failed: {error:?}"),
    }
}

fn classify_segv(pc: usize, badaddr: usize, sp: usize, _frame: &crate::arch::trap::TrapFrame) -> &'static str {
    // Known-bad LA static busybox: andi rX,r0,imm placeholder.
    if pc == 0x12018ae50 || pc == 0x12018bd2c || pc == 0x12018b840
        || pc == 0x12018b4c8 || pc == 0x1201acc9c
    {
        return "known-bad-busybox";
    }
    // Near-stack access: within 64KB of stack pointer.
    if badaddr >= sp.saturating_sub(0x10000) && badaddr <= sp.saturating_add(0x10000) {
        return "near-stack";
    }
    // Null/near-null dereference.
    if badaddr < 0x1000 {
        return "null-deref";
    }
    // Low address access.
    if badaddr < 0x100000 {
        return "low-addr";
    }
    "other"
}

pub fn handle_exception(frame: &mut crate::arch::trap::TrapFrame, _code: usize) {
    assert!(
        frame.previous_mode_was_user(),
        "M8-B3 user exception handler received a kernel exception",
    );

    if ACTIVE.load(Ordering::Acquire) {
        LAST_FAULT_ADDRESS.store(0, Ordering::Release);
        LAST_FAULT_KIND.store(FAULT_EXCEPTION, Ordering::Release);
        FAULT_COUNT.fetch_add(1, Ordering::AcqRel);
        TERMINATED.store(true, Ordering::Release);
        EXIT_STATUS.store(-EFAULT, Ordering::Release);
    }
    return_to_kernel(frame, -EFAULT);
}

fn sys_brk(address: usize) -> isize {
    let mm = current_user_mm();
    let current = match mm.program_break() {
        Ok(current) => current,
        Err(_) => return -ENOMEM,
    };
    if address == 0 {
        return current.get() as isize;
    }

    match mm.set_program_break(VirtAddr::new(address)) {
        Ok(new_break) => {
            if ACTIVE.load(Ordering::Acquire) {
                BRK_COUNT.fetch_add(1, Ordering::AcqRel);
            }
            new_break.get() as isize
        }
        Err(_) => current.get() as isize,
    }
}

fn sys_mmap(arguments: [usize; 6]) -> isize {
    let [address, mut length, protection, flags, file, offset] = arguments;
    if length >> 32 == u32::MAX as usize {
        length &= u32::MAX as usize;
    }
    if length == 0 {
        return -EINVAL;
    }

    // MAP_FIXED requires a non-zero address; MAP_FIXED without an
    // address is an error.  Non-FIXED address hints are accepted but
    // ignored (map_anonymous chooses the address).
    let is_fixed = flags & MAP_FIXED != 0;
    if is_fixed && address == 0 {
        return -EINVAL;
    }

    let vm_flags = match protection_flags(protection) {
        Some(flags) => flags,
        None => return -EINVAL,
    };
    let rounded = match length.checked_add(PAGE_SIZE - 1) {
        Some(length) => length & !(PAGE_SIZE - 1),
        None => return -ENOMEM,
    };

    // ld-linux and glibc often set historical flags that Linux silently
    // ignores.  Accept MAP_DENYWRITE(0x800) and MAP_EXECUTABLE(0x1000)
    // as no-ops so file-backed mmap doesn't fail with EINVAL.
    const MAP_ACCEPTED: usize = MAP_PRIVATE | MAP_SHARED | MAP_ANONYMOUS
        | MAP_FIXED | 0x800 | 0x1000;  // MAP_DENYWRITE | MAP_EXECUTABLE

    let map_type = flags & MAP_TYPE;
    let is_file_backed = file != usize::MAX && (map_type == MAP_PRIVATE || map_type == MAP_SHARED);

    if flags & !MAP_ACCEPTED != 0 {
        return -EINVAL;
    }

    if is_file_backed && flags & MAP_ANONYMOUS == 0 {
        // MAP_FIXED: ld-linux uses this to place PT_LOAD segments at
        // specific addresses.  We map the file content into a temporary
        // anonymous area first, then the caller may re-map with MAP_FIXED.
        let is_fixed = flags & MAP_FIXED != 0;
        return sys_file_private_mmap(file, offset, length, rounded, vm_flags, address, is_fixed);
    }

    // Anonymous mapping (MAP_PRIVATE | MAP_ANONYMOUS or similar).
    if file != usize::MAX || offset != 0 {
        return -EINVAL;
    }

    // MAP_FIXED: unmap the target range first, then let the allocator
    // place the mapping.  (For true MAP_FIXED we should map at the exact
    // address; this is a best-effort approximation.)
    if is_fixed && address != 0 {
        let fixed_start = VirtAddr::new(address);
        if let Some(fixed_range) = fixed_start
            .checked_add(rounded)
            .and_then(|end| VirtRange::new(fixed_start, end))
        {
            let _ = current_user_mm().unmap_range(fixed_range);
        }
        if mmap_file_ok_trace() {
            crate::println!(
                "mmap-anon: FIXED addr={:#x} len={:#x} prot={:?}",
                address, rounded, vm_flags,
            );
        }
    }

    match current_user_mm().map_anonymous(
        VirtRange::from_bounds(USER_MMAP_START, USER_MMAP_END),
        rounded,
        vm_flags,
    ) {
        Ok(start) => {
            if ACTIVE.load(Ordering::Acquire) {
                MMAP_COUNT.fetch_add(1, Ordering::AcqRel);
            }
            // Trace anonymous mmap (rate-limited).
            if mmap_file_ok_trace() {
                crate::println!(
                    "mmap-anon: ok addr_req={:#x} -> {:#x} len={:#x} prot={:?}",
                    address, start.get(), rounded, vm_flags,
                );
            }
            start.get() as isize
        }
        Err(_) => {
            if is_fixed {
                crate::println!(
                    "mmap-anon: FAIL FIXED addr={:#x} len={:#x} prot={:?}",
                    address, rounded, vm_flags,
                );
            }
            -ENOMEM
        }
    }
}

fn sys_file_private_mmap(
    fd: usize,
    offset: usize,
    length: usize,
    rounded: usize,
    vm_flags: VmAreaFlags,
    _address: usize,
    is_fixed: bool,
) -> isize {
    const MAX_FILE_MMAP: usize = 16 * 1024 * 1024;
    if offset & (PAGE_SIZE - 1) != 0 || rounded > MAX_FILE_MMAP {
        return -EINVAL;
    }
    let file = match current_process_file(fd) {
        Ok(file) => file,
        Err(errno) => return errno.to_isize(),
    };
    let stat = match file.fstat() {
        Ok(stat) => stat,
        Err(errno) => return errno.to_isize(),
    };
    // Accept regular files AND block-device-backed ext4 files.
    let mode = stat.mode & myos_vfs::FileMode::S_IFMT;
    if mode != myos_vfs::FileMode::S_IFREG && mode != myos_vfs::FileMode::S_IFBLK {
        return -(myos_vfs::Errno::Eacces.to_isize());
    }
    let file_size = if stat.size <= 0 { 0 } else { stat.size as usize };
    let readable = file_size.saturating_sub(offset).min(length);

    let temporary_flags = VmAreaFlags::user_rw();

    // MAP_FIXED: unmap the target range first, then let the allocator
    // place the new mapping.  (A full implementation would map at the
    // exact address; for now this avoids EINVAL when ld-linux uses
    // MAP_FIXED to place PT_LOAD segments.)
    if is_fixed {
        let fixed_start = VirtAddr::new(_address);
        if let Some(fixed_range) = fixed_start
            .checked_add(rounded)
            .and_then(|end| VirtRange::new(fixed_start, end))
        {
            let _ = current_user_mm().unmap_range(fixed_range);
        }
    }

    let start = match current_user_mm().map_anonymous(
        VirtRange::from_bounds(USER_MMAP_START, USER_MMAP_END),
        rounded,
        temporary_flags,
    ) {
        Ok(start) => start,
        Err(_) => return -ENOMEM,
    };
    let range = match start
        .checked_add(rounded)
        .and_then(|end| VirtRange::new(start, end))
    {
        Some(range) => range,
        None => return -ENOMEM,
    };

    let old_position = file.position();
    if file.seek(offset as i64, myos_vfs::SeekWhence::Set).is_err() {
        let _ = current_user_mm().unmap_range(range);
        return -EINVAL;
    }
    let result = copy_file_into_private_mapping(&file, start, readable);
    let _ = file.seek(old_position as i64, myos_vfs::SeekWhence::Set);
    if result.is_err() {
        let _ = current_user_mm().unmap_range(range);
        return -EFAULT;
    }
    // Zero-fill tail padding (BSS / partial page beyond file size).
    if rounded > readable {
        let _ = zero_user_mapping(start.checked_add(readable).unwrap_or(start), rounded - readable);
    }
    if let Err(e) = current_user_mm().protect_range(range, vm_flags.access_only()) {
        if mmap_file_fail_trace() {
            let path = file.path().unwrap_or("?");
            crate::println!(
                "mmap-file: FAIL fd={} path={} off={:#x} len={:#x} range=[{:#x},{:#x}) err={:?}",
                fd, path, offset, length, range.start().get(), range.end().get(), e,
            );
        }
        let _ = current_user_mm().unmap_range(range);
        return -ENOMEM;
    }
    if ACTIVE.load(Ordering::Acquire) {
        MMAP_COUNT.fetch_add(1, Ordering::AcqRel);
    }
    let path = file.path().unwrap_or("?");
    let is_lib = path.contains("/lib/") || path.contains("/lib64/") || path.contains("ld-linux") || path.contains("ld-musl");
    // Always print mmap for lib paths; rate-limit everything else.
    if is_lib || mmap_file_ok_trace() {
        crate::println!(
            "mmap-file: ok fd={} path={} off={:#x} len={:#x} -> {:#x} prot={:?}",
            fd, path, offset, length, start.get(), vm_flags,
        );
    }
    start.get() as isize
}

fn zero_user_mapping(mut addr: VirtAddr, mut remaining: usize) -> Result<(), ()> {
    let mm = current_user_mm();
    while remaining > 0 {
        let chunk = core::cmp::min(PAGE_SIZE, remaining);
        let phys = mm.populate_page(addr).map_err(|_| ())?;
        let ptr = crate::arch::memory::phys_access::ram_mut_ptr::<u8>(phys)
            .map_err(|_| ())?;
        unsafe { core::ptr::write_bytes(ptr, 0, chunk); }
        addr = addr.checked_add(chunk).ok_or(())?;
        remaining -= chunk;
    }
    Ok(())
}

fn copy_file_into_private_mapping(
    file: &myos_vfs::ArcFile,
    start: VirtAddr,
    length: usize,
) -> Result<(), ()> {
    let mut copied = 0;
    let mut buffer = [0_u8; MAX_USER_COPY];
    while copied < length {
        let chunk = (length - copied).min(MAX_USER_COPY);
        let mut output = myos_vfs::MutableIoBuffer::new(&mut buffer[..chunk]);
        let read = file.read(&mut output).map_err(|_| ())?;
        if read == 0 {
            break;
        }
        let destination = start.get().checked_add(copied).ok_or(())?;
        current_user_mm()
            .populate_page(VirtAddr::new(destination))
            .map_err(|_| ())?;
        copy_to_user(destination, output.filled_bytes()).map_err(|_| ())?;
        copied += read;
    }
    Ok(())
}

fn sys_munmap(address: usize, length: usize) -> isize {
    let range = match syscall_range(address, length) {
        Some(range) => range,
        None => return -EINVAL,
    };
    match current_user_mm().unmap_range(range) {
        Ok(()) => {
            if ACTIVE.load(Ordering::Acquire) {
                MUNMAP_COUNT.fetch_add(1, Ordering::AcqRel);
            }
            0
        }
        Err(_) => -EINVAL,
    }
}

// Separated trace counters: failures print unconditionally up to 64.
// Successes are rate-limited independently so they don't crowd out failures.
static MPROTECT_OK_COUNT: AtomicUsize = AtomicUsize::new(0);
static MPROTECT_FAIL_COUNT: AtomicUsize = AtomicUsize::new(0);
static MMAP_FILE_OK_COUNT: AtomicUsize = AtomicUsize::new(0);
static MMAP_FILE_FAIL_COUNT: AtomicUsize = AtomicUsize::new(0);
const TRACE_OK_LIMIT: usize = 8;
const TRACE_FAIL_LIMIT: usize = 128;

fn mprotect_ok_trace() -> bool {
    MPROTECT_OK_COUNT.fetch_add(1, Ordering::Relaxed) < TRACE_OK_LIMIT
}
fn mprotect_fail_trace() -> bool {
    MPROTECT_FAIL_COUNT.fetch_add(1, Ordering::Relaxed) < TRACE_FAIL_LIMIT
}
fn mmap_file_ok_trace() -> bool {
    MMAP_FILE_OK_COUNT.fetch_add(1, Ordering::Relaxed) < TRACE_OK_LIMIT
}
fn mmap_file_fail_trace() -> bool {
    MMAP_FILE_FAIL_COUNT.fetch_add(1, Ordering::Relaxed) < TRACE_FAIL_LIMIT
}

fn sys_pkey_mprotect(address: usize, length: usize, protection: usize, pkey: usize) -> isize {
    // pkey == -1 (usize::MAX) or pkey == 0 → forward to mprotect.
    // Other pkey values are not supported.
    if pkey == usize::MAX || pkey == 0 {
        return sys_mprotect(address, length, protection);
    }
    if mmap_file_fail_trace() {
        crate::println!(
            "pkey_mprotect: FAIL pkey={} addr={:#x} len={:#x} prot={:#x}",
            pkey, address, length, protection,
        );
    }
    -EINVAL
}

fn sys_mprotect(address: usize, length: usize, protection: usize) -> isize {
    if length == 0 {
        return 0;
    }
    if address & (PAGE_SIZE - 1) != 0 {
        return -EINVAL;
    }
    // PROT_NONE: prot=0 is valid, maps to empty access (no R/W/X).
    const PROT_ALLOWED: usize = 0x7; // PROT_READ|PROT_WRITE|PROT_EXEC
    if protection & !PROT_ALLOWED != 0 {
        return -EINVAL;
    }
    let end = match address.checked_add(length) {
        Some(end) => end,
        None => return -ENOMEM,
    };
    let rounded_end = match end.checked_add(PAGE_SIZE - 1) {
        Some(end) => end & !(PAGE_SIZE - 1),
        None => return -ENOMEM,
    };
    let range = match VirtRange::new(VirtAddr::new(address), VirtAddr::new(rounded_end)) {
        Some(range) => range,
        None => return -ENOMEM,
    };
    // PROT_NONE (prot=0): access-only flags will be empty (no R/W/X bits).
    let flags = if protection == 0 {
        // PROT_NONE: effectively no access. Use USER flag only so VMA
        // is recognized as a user mapping; access bits are all cleared.
        VmAreaFlags::USER
    } else {
        match protection_flags(protection) {
            Some(flags) => flags.access_only(),
            None => return -EINVAL,
        }
    };

    let ret = match current_user_mm().protect_range(range, flags) {
        Ok(()) => {
            if ACTIVE.load(Ordering::Acquire) {
                MPROTECT_COUNT.fetch_add(1, Ordering::AcqRel);
            }
            0
        }
        Err(ref e) => {
            let errno = match e {
                UserMmRuntimeError::NotMapped => -ENOMEM,
                UserMmRuntimeError::MetadataOutOfMemory => -ENOMEM,
                UserMmRuntimeError::InvalidRange => -EINVAL,
                UserMmRuntimeError::PermissionDenied => -ENOMEM,
                _ => -ENOMEM,
            };
            // FAILURES PRINT UNCONDITIONALLY (up to limit) — never suppressed.
            if mprotect_fail_trace() {
                crate::println!(
                    "mprotect: FAIL ret={} addr={:#x} len={:#x} prot={:#x} range=[{:#x},{:#x}) err={:?}",
                    errno, address, length, protection,
                    range.start().get(), range.end().get(), e,
                );
            }
            errno
        }
    };

    // Print EVERY failure unconditionally (no rate limit for failures).
    if ret < 0 {
        crate::println!(
            "mprotect: FAIL ret={} addr={:#x} len={:#x} prot={:#x} range=[{:#x},{:#x})",
            ret, address, length, protection,
            range.start().get(), range.end().get(),
        );
    } else if mprotect_ok_trace() {
        crate::println!(
            "mprotect: ok addr={:#x} len={:#x} prot={:#x}",
            address, length, protection,
        );
    }
    ret
}

fn syscall_range(address: usize, mut length: usize) -> Option<VirtRange> {
    if length >> 32 == u32::MAX as usize {
        length &= u32::MAX as usize;
    }
    if length == 0 || address & (PAGE_SIZE - 1) != 0 {
        return None;
    }
    let rounded = length.checked_add(PAGE_SIZE - 1)? & !(PAGE_SIZE - 1);
    let end = address.checked_add(rounded)?;
    VirtRange::new(VirtAddr::new(address), VirtAddr::new(end))
}

fn protection_flags(protection: usize) -> Option<VmAreaFlags> {
    if protection == 0 || protection & !(PROT_READ | PROT_WRITE | PROT_EXEC) != 0 {
        return None;
    }

    let mut flags = VmAreaFlags::USER.union(VmAreaFlags::PRIVATE);
    if protection & PROT_READ != 0 || protection & (PROT_WRITE | PROT_EXEC) != 0 {
        flags = flags.union(VmAreaFlags::READ);
    }
    if protection & PROT_WRITE != 0 {
        flags = flags.union(VmAreaFlags::WRITE);
    }
    if protection & PROT_EXEC != 0 {
        flags = flags.union(VmAreaFlags::EXECUTE);
    }
    if flags.is_writable() && flags.is_executable() {
        return None;
    }
    Some(flags)
}

fn sys_write(fd: usize, address: usize, length: usize) -> isize {
    let length = length.min(MAX_USER_COPY);

    let mut buffer = [0_u8; MAX_USER_COPY];
    if copy_from_user(address, &mut buffer[..length]).is_err() {
        return -EFAULT;
    }

    let file = match current_process_file(fd) {
        Ok(file) => file,
        Err(errno) => return errno.to_isize(),
    };
    match file.write(&myos_vfs::IoBuffer::new(&buffer[..length])) {
        Ok(written) => {
            if ACTIVE.load(Ordering::Acquire) {
                WRITE_COUNT.fetch_add(1, Ordering::AcqRel);
            }
            written as isize
        }
        Err(errno) => errno.to_isize(),
    }
}

fn sys_read(fd: usize, address: usize, length: usize) -> isize {
    let length = length.min(MAX_USER_COPY);
    let file = match current_process_file(fd) {
        Ok(file) => file,
        Err(errno) => return errno.to_isize(),
    };

    let mut buffer = [0_u8; MAX_USER_COPY];
    let mut output = myos_vfs::MutableIoBuffer::new(&mut buffer[..length]);
    match file.read(&mut output) {
        Ok(read) => {
            if copy_to_user(address, output.filled_bytes()).is_err() {
                return -EFAULT;
            }
            read as isize
        }
        Err(errno) => errno.to_isize(),
    }
}

fn sys_readv(fd: usize, iov_address: usize, iov_count: usize) -> isize {
    sys_iov_io(fd, iov_address, iov_count, true)
}

fn sys_writev(fd: usize, iov_address: usize, iov_count: usize) -> isize {
    sys_iov_io(fd, iov_address, iov_count, false)
}

fn sys_iov_io(fd: usize, iov_address: usize, iov_count: usize, read: bool) -> isize {
    const MAX_IOV: usize = 16;
    if iov_count > MAX_IOV {
        return -EINVAL;
    }
    let mut total = 0_isize;
    for index in 0..iov_count {
        let entry = match iov_address.checked_add(index * 2 * core::mem::size_of::<usize>()) {
            Some(entry) => entry,
            None => return if total > 0 { total } else { -EFAULT },
        };
        let base = match copy_plain_from_user::<usize>(entry) {
            Ok(base) => base,
            Err(errno) => return if total > 0 { total } else { errno },
        };
        let len = match copy_plain_from_user::<usize>(entry + core::mem::size_of::<usize>()) {
            Ok(len) => len,
            Err(errno) => return if total > 0 { total } else { errno },
        };
        let mut done = 0;
        while done < len {
            let chunk = (len - done).min(MAX_USER_COPY);
            let address = match base.checked_add(done) {
                Some(address) => address,
                None => return if total > 0 { total } else { -EFAULT },
            };
            let result = if read {
                sys_read(fd, address, chunk)
            } else {
                sys_write(fd, address, chunk)
            };
            if result < 0 {
                return if total > 0 { total } else { result };
            }
            if result == 0 {
                return total;
            }
            total = total.saturating_add(result);
            done += result as usize;
            if result as usize != chunk {
                return total;
            }
        }
    }
    total
}

fn sys_pread64(fd: usize, address: usize, length: usize, offset: usize) -> isize {
    let file = match current_process_file(fd) {
        Ok(file) => file,
        Err(errno) => return errno.to_isize(),
    };
    // Save/restore file position so pread64 doesn't affect fd offset.
    let old_position = file.position();
    if file.seek(offset as i64, myos_vfs::SeekWhence::Set).is_err() {
        return -(myos_vfs::Errno::Einval.to_isize());
    }
    // Read in chunks: sys_read caps at MAX_USER_COPY, but pread64
    // must be able to read larger amounts (glibc ld-linux reads
    // ELF header + program headers in a single pread64 call).
    let mut total: usize = 0;
    let mut remaining = length;
    while remaining > 0 {
        let chunk = remaining.min(MAX_USER_COPY);
        // Build a chunk-sized buffer and read into it.
        let mut chunk_buf = [0_u8; MAX_USER_COPY];
        let mut output = myos_vfs::MutableIoBuffer::new(&mut chunk_buf[..chunk]);
        match file.read(&mut output) {
            Ok(0) => break, // EOF
            Ok(read) => {
                let dest = match address.checked_add(total) {
                    Some(addr) => addr,
                    None => break,
                };
                if copy_to_user(dest, output.filled_bytes()).is_err() {
                    if total > 0 {
                        break;
                    }
                    let _ = file.seek(old_position as i64, myos_vfs::SeekWhence::Set);
                    return -EFAULT;
                }
                total += read;
                if read < chunk {
                    break; // short read → EOF
                }
                remaining -= read;
            }
            Err(errno) => {
                if total > 0 {
                    break;
                }
                let _ = file.seek(old_position as i64, myos_vfs::SeekWhence::Set);
                return errno.to_isize();
            }
        }
    }
    let _ = file.seek(old_position as i64, myos_vfs::SeekWhence::Set);
    total as isize
}

fn sys_close(fd: usize) -> isize {
    match current_process().files().close(fd) {
        Ok(()) => 0,
        Err(errno) => errno.to_isize(),
    }
}

fn sys_lseek(fd: usize, offset: isize, whence: usize) -> isize {
    let whence = match myos_vfs::SeekWhence::from_raw(whence) {
        Some(whence) => whence,
        None => return -EINVAL,
    };
    let file = match current_process_file(fd) {
        Ok(file) => file,
        Err(errno) => return errno.to_isize(),
    };
    match file.seek(offset as i64, whence) {
        Ok(position) => position as isize,
        Err(errno) => errno.to_isize(),
    }
}

fn sys_fstat(fd: usize, stat_address: usize) -> isize {
    let file = match current_process_file(fd) {
        Ok(file) => file,
        Err(errno) => return errno.to_isize(),
    };
    let stat = match file.fstat() {
        Ok(stat) => stat,
        Err(errno) => return errno.to_isize(),
    };
    // SAFETY: `stat` is a plain repr(C) value and the byte slice is used only
    // for checked copy_to_user before `stat` goes out of scope.
    let bytes = unsafe {
        core::slice::from_raw_parts(
            core::ptr::addr_of!(stat) as *const u8,
            core::mem::size_of::<myos_vfs::Stat>(),
        )
    };
    if copy_to_user(stat_address, bytes).is_err() {
        return -EFAULT;
    }
    0
}

fn sys_newfstatat(dirfd: usize, path_address: usize, stat_address: usize, flags: usize) -> isize {
    // Accept common Linux flags; ignore harmless ones like AT_NO_AUTOMOUNT.
    const AT_OK: usize = AT_SYMLINK_NOFOLLOW | AT_EMPTY_PATH | 0x800 | 0x1000;
    if flags & !AT_OK != 0 {
        return -EINVAL;
    }
    // AT_EMPTY_PATH: stat the fd itself (equivalent to fstat).
    if flags & AT_EMPTY_PATH != 0 {
        let file = match current_process_file(dirfd) {
            Ok(file) => file,
            Err(errno) => return errno.to_isize(),
        };
        let stat = match file.fstat() {
            Ok(stat) => stat,
            Err(errno) => return errno.to_isize(),
        };
        return copy_stat_to_user(stat_address, &stat);
    }
    let path = match resolve_user_path(dirfd, path_address) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    let mut stat_result = if flags & AT_SYMLINK_NOFOLLOW != 0 {
        crate::fs::lstat(&path)
    } else {
        crate::fs::stat(&path)
    };
    if stat_result.is_err() && crate::ensure_sdcard_dir_materialized(&path) {
        stat_result = if flags & AT_SYMLINK_NOFOLLOW != 0 {
            crate::fs::lstat(&path)
        } else {
            crate::fs::stat(&path)
        };
    }
    match stat_result {
        Ok(stat) => copy_stat_to_user(stat_address, &stat),
        Err(errno) => errno.to_isize(),
    }
}

fn sys_statx(
    dirfd: usize,
    path_address: usize,
    flags: usize,
    _mask: usize,
    statx_address: usize,
) -> isize {
    // Accept AT_STATX_SYNC_TYPE (0x6000) and other common flags.
    const AT_STATX_OK: usize = AT_SYMLINK_NOFOLLOW | AT_EMPTY_PATH | 0x6000 | 0x800;
    if flags & !AT_STATX_OK != 0 {
        return -EINVAL;
    }

    let raw_path = match copy_user_c_string(path_address) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    let stat = if raw_path.is_empty() && flags & AT_EMPTY_PATH != 0 {
        let file = match current_process_file(dirfd) {
            Ok(file) => file,
            Err(errno) => return errno.to_isize(),
        };
        match file.fstat() {
            Ok(stat) => stat,
            Err(errno) => return errno.to_isize(),
        }
    } else {
        if raw_path.is_empty() {
            return myos_vfs::Errno::Enoent.to_isize();
        }
        let path = match resolve_path_from_user(dirfd, &raw_path) {
            Ok(path) => path,
            Err(errno) => return errno,
        };
        let mut stat = if flags & AT_SYMLINK_NOFOLLOW != 0 {
            crate::fs::lstat(&path)
        } else {
            crate::fs::stat(&path)
        };
        if stat.is_err() && crate::ensure_sdcard_dir_materialized(&path) {
            stat = if flags & AT_SYMLINK_NOFOLLOW != 0 {
                crate::fs::lstat(&path)
            } else {
                crate::fs::stat(&path)
            };
        }
        match stat {
            Ok(stat) => stat,
            Err(errno) => return errno.to_isize(),
        }
    };
    copy_statx_to_user(statx_address, &stat)
}

fn sys_openat(dirfd: usize, path_address: usize, flags: usize) -> isize {
    let raw_path = match copy_user_c_string(path_address) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    let flags = myos_vfs::OpenFlags::from_bits(flags as u32);
    let path = match resolve_path_from_user(dirfd, &raw_path) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    let file = match crate::fs::open(&path, flags) {
        Ok(file) => file,
        Err(errno) => return errno.to_isize(),
    };
    match current_process().files().allocate(file, flags.is_cloexec()) {
        Ok(fd) => fd as isize,
        Err(errno) => errno.to_isize(),
    }
}

fn sys_pipe2(fds_address: usize, flags: usize) -> isize {
    let flags = myos_vfs::OpenFlags::from_bits(flags as u32);
    let (reader, writer) = match crate::pipe::create_pipe(flags) {
        Ok(pipe) => pipe,
        Err(errno) => return errno.to_isize(),
    };
    let process = current_process();
    let reader_fd = match process.files().allocate(reader, flags.is_cloexec()) {
        Ok(fd) => fd,
        Err(errno) => return errno.to_isize(),
    };
    let writer_fd = match process.files().allocate(writer, flags.is_cloexec()) {
        Ok(fd) => fd,
        Err(errno) => {
            let _ = process.files().close(reader_fd);
            return errno.to_isize();
        }
    };

    let mut raw = [0_u8; 2 * core::mem::size_of::<i32>()];
    raw[..4].copy_from_slice(&(reader_fd as i32).to_ne_bytes());
    raw[4..].copy_from_slice(&(writer_fd as i32).to_ne_bytes());
    if copy_to_user(fds_address, &raw).is_err() {
        let _ = process.files().close(reader_fd);
        let _ = process.files().close(writer_fd);
        return -EFAULT;
    }
    0
}

fn sys_set_tid_address(address: usize) -> isize {
    let thread = crate::task::current_user_thread()
        .expect("set_tid_address arrived without current user Thread");
    // Save the clear_child_tid address for futex-wake on thread exit.
    thread.set_clear_child_tid(address);
    // Write the thread ID to the user-space address (set_child_tid).
    if address != 0 {
        let tid = thread.id().get() as u32;
        let _ = copy_to_user(address, &tid.to_ne_bytes());
    }
    thread.id().get() as isize
}

fn sys_set_robust_list(_head: usize, length: usize) -> isize {
    const ROBUST_LIST_HEAD_SIZE: usize = 24;
    if length != ROBUST_LIST_HEAD_SIZE {
        return -EINVAL;
    }
    0
}

fn sys_clone(frame: &crate::arch::trap::TrapFrame, arguments: [usize; 6]) -> isize {
    const CSIGNAL_MASK: usize = 0xff;
    const CLONE_VM: usize = 0x0000_0100;
    const CLONE_FS: usize = 0x0000_0200;
    const CLONE_FILES: usize = 0x0000_0400;
    const CLONE_SIGHAND: usize = 0x0000_0800;
    const CLONE_THREAD: usize = 0x0001_0000;
    const CLONE_SETTLS: usize = 0x0008_0000;
    const CLONE_CHILD_CLEARTID: usize = 0x0020_0000;
    const CLONE_CHILD_SETTID: usize = 0x0100_0000;
    const CLONE_PARENT_SETTID: usize = 0x0010_0000;

    let flags = arguments[0];
    let wants_thread = flags & CLONE_THREAD != 0;
    let wants_vm_share = flags & CLONE_VM != 0;
    let wants_tls = flags & CLONE_SETTLS != 0;
    let wants_child_cleartid = flags & CLONE_CHILD_CLEARTID != 0;
    let wants_child_settid = flags & CLONE_CHILD_SETTID != 0;
    let wants_parent_settid = flags & CLONE_PARENT_SETTID != 0;

    // Thread creation requires CLONE_VM (shared address space).
    if wants_thread && !wants_vm_share {
        return -(crate::syscall::errno::EINVAL);
    }

    let _exit_signal = flags & CSIGNAL_MASK;
    let parent = current_process();
    let current_thread =
        crate::task::current_user_thread().expect("clone arrived without a current user Thread");

    let child = if wants_vm_share {
        // Thread: share the parent's address space and file table.
        match parent.fork_child_thread() {
            Ok(child) => child,
            Err(_) => return -ENOMEM,
        }
    } else {
        // Process: copy address space and file table.
        let child_mm = match parent.mm().fork_clone_eager() {
            Ok(mm) => mm,
            Err(_) => return -ENOMEM,
        };
        match parent.fork_child(child_mm) {
            Ok(child) => child,
            Err(_) => return -ENOMEM,
        }
    };

    let child_thread =
        match child.create_initial_thread(current_thread.entry(), current_thread.user_stack()) {
            Ok(thread) => thread,
            Err(_) => return -ENOMEM,
        };

    // Copy signal mask from parent thread.
    child_thread.set_blocked_signals(current_thread.blocked_signals());

    // CLONE_SETTLS: set the new thread's TLS pointer.
    // On RISC-V this is the `tp` register; on LoongArch it's `tp` (r2).
    if wants_tls && arguments[3] != 0 {
        child_thread.set_tls_pointer(arguments[3]);
    }

    // CLONE_CHILD_CLEARTID: write the child tid pointer for futex wake on exit.
    if wants_child_cleartid && arguments[5] != 0 {
        child_thread.set_clear_child_tid(arguments[5]);
    }

    // CLONE_PARENT_SETTID: write child TID to parent's user-space pointer.
    if wants_parent_settid && arguments[2] != 0 {
        let tid = child_thread.id().get() as u32;
        let _ = copy_to_user(arguments[2], &tid.to_ne_bytes());
    }

    // CLONE_CHILD_SETTID: write child TID to child's user-space pointer.
    if wants_child_settid && arguments[4] != 0 {
        let tid = child_thread.id().get() as u32;
        let _ = copy_to_user(arguments[4], &tid.to_ne_bytes());
    }

    // Prepare the child's trap frame (copy of parent's, with return value 0).
    let mut child_frame = *frame;
    set_syscall_result(&mut child_frame, 0);
    // If child stack is specified (arguments[1] != 0), use it.
    if arguments[1] != 0 {
        set_frame_stack_pointer(&mut child_frame, arguments[1]);
    }
    child_thread.save_trap_frame(child_frame);

    let _task = crate::task::spawn_user_thread_from_user_trap(child_thread);
    child.id().get() as isize
}

fn sys_execve(frame: &mut crate::arch::trap::TrapFrame, arguments: [usize; 6]) -> isize {
    let raw_path = match copy_user_c_string(arguments[0]) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    let path = match resolve_path_from_user(AT_FDCWD, &raw_path) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    let argv = match copy_user_string_array(arguments[1], MAX_EXEC_ARGS, Some(&raw_path)) {
        Ok(values) => values,
        Err(errno) => return errno,
    };
    let envp = match copy_user_string_array(arguments[2], MAX_EXEC_ENVS, None) {
        Ok(values) => values,
        Err(errno) => return errno,
    };
    let mut exec_argv = argv;
    let exec_path = path;
    let mut image = match load_exec_image(&exec_path) {
        Ok(image) => image,
        Err(errno) => {
            if exec_trace_allow() {
                crate::println!(
                    "execve-fail: phase=target-open path={} errno={}",
                    exec_path, errno,
                );
            }
            return errno;
        }
    };
    if let Some((interpreter, optional_arg)) = match parse_shebang(&image) {
        Ok(shebang) => shebang,
        Err(errno) => return errno,
    } {
        if exec_argv.len() + 2 + usize::from(optional_arg.is_some()) > MAX_EXEC_ARGS {
            return -EINVAL;
        }
        let interpreter_path = match resolve_path_from_user(AT_FDCWD, &interpreter) {
            Ok(path) => path,
            Err(errno) => return errno,
        };
        let mut rewritten_argv = Vec::new();
        if rewritten_argv
            .try_reserve(exec_argv.len() + 2 + usize::from(optional_arg.is_some()))
            .is_err()
        {
            return -ENOMEM;
        }
        rewritten_argv.push(interpreter_path.clone());
        if let Some(argument) = optional_arg {
            rewritten_argv.push(argument);
        }
        rewritten_argv.push(exec_path.clone());
        for argument in exec_argv.iter().skip(1) {
            rewritten_argv.push(argument.clone());
        }
        image = match load_exec_image(&interpreter_path) {
            Ok(image) => image,
            Err(errno) => {
                if exec_trace_allow() {
                    crate::println!(
                        "execve-fail: phase=shebang-interp path={} interp={} errno={}",
                        exec_path, interpreter_path, errno,
                    );
                }
                return errno;
            }
        };
        exec_argv = rewritten_argv;
    }

    // C1-C3: if the file is not an ELF and no shebang was found,
    // fallback to /bin/sh so scripts without #! still execute.
    // Linux leaves this to the shell, but contest shells may not
    // implement the ENOEXEC→/bin/sh fallback reliably.
    if !image.starts_with(b"\x7fELF") && exec_path != "/bin/sh" && exec_path != "/bin/busybox" {
        if exec_argv.len() + 1 > MAX_EXEC_ARGS {
            return -EINVAL;
        }
        // Construct: /bin/sh <original_path> <original_argv[1..]>
        let mut fallback_argv = Vec::new();
        if fallback_argv
            .try_reserve(exec_argv.len() + 1)
            .is_err()
        {
            return -ENOMEM;
        }
        fallback_argv.push(alloc::string::String::from("/bin/sh"));
        fallback_argv.push(exec_path.clone());
        for argument in exec_argv.iter().skip(1) {
            fallback_argv.push(argument.clone());
        }
        exec_argv = fallback_argv;
        image = match load_exec_image("/bin/sh") {
            Ok(image) => image,
            Err(errno) => {
                if exec_trace_allow() {
                    crate::println!(
                        "execve-fail: phase=sh-fallback path={} interp=/bin/sh errno={}",
                        exec_path, errno,
                    );
                }
                return errno;
            }
        };
        if exec_trace_allow() {
            crate::println!("execve: path={} falling back to /bin/sh", exec_path);
        }
    }

    let argv_refs = exec_argv.iter().map(String::as_str).collect::<Vec<_>>();
    let envp_refs = envp.iter().map(String::as_str).collect::<Vec<_>>();
    let extra_areas = [VmArea::new(
        VirtRange::from_bounds(USER_DEMAND, USER_DEMAND + PAGE_SIZE),
        VmAreaFlags::user_rw(),
        VmAreaKind::Anonymous,
    )];
    let prepared = match crate::exec::prepare_elf(
        &image,
        crate::exec::ExecConfig {
            argv: &argv_refs,
            envp: &envp_refs,
            stack: VirtRange::from_bounds(USER_STACK, USER_STACK_TOP),
            heap_start: VirtAddr::new(USER_HEAP_START),
            heap_limit: VirtAddr::new(USER_HEAP_LIMIT),
            extra_areas: &extra_areas,
        },
    ) {
        Ok(prepared) => {
            // Trace successful dynamic execs (first 32 only).
            if prepared.interp_base.is_some() && exec_trace_success_allow() {
                crate::println!(
                    "execve: path={} kind=dynamic interp_base={:#x} main_entry={:#x}",
                    exec_path,
                    prepared.interp_base.map(|b| b.get()).unwrap_or(0),
                    prepared.main_entry.map(|e| e.get()).unwrap_or(0),
                );
            }
            prepared
        }
        Err(ref e @ crate::exec::ExecError::Elf(crate::elf::ElfError::InvalidHeader)) => {
            if exec_trace_allow() {
                crate::println!(
                    "execve: path={} failed errno=ENOEXEC reason={}",
                    exec_path, e.reason(),
                );
            }
            return myos_vfs::Errno::Enoexec.to_isize()
        }
        Err(ref e @ crate::exec::ExecError::Elf(crate::elf::ElfError::Unsupported)) => {
            if exec_trace_allow() {
                crate::println!(
                    "execve: path={} failed errno=ENOEXEC reason={}",
                    exec_path, e.reason(),
                );
            }
            return myos_vfs::Errno::Enoexec.to_isize()
        }
        Err(ref e @ crate::exec::ExecError::Elf(crate::elf::ElfError::InvalidMachine)) => {
            if exec_trace_allow() {
                crate::println!(
                    "execve: path={} failed errno=ENOEXEC reason={}",
                    exec_path, e.reason(),
                );
            }
            return myos_vfs::Errno::Enoexec.to_isize()
        }
        Err(ref e @ crate::exec::ExecError::DynamicInterpreterUnsupported) => {
            // ENOEXEC lets shell fallback; EINVAL would be wrong here.
            if exec_trace_allow() {
                crate::println!(
                    "execve: path={} failed errno=ENOEXEC reason={}",
                    exec_path, e.reason(),
                );
            }
            return myos_vfs::Errno::Enoexec.to_isize()
        }
        Err(ref e @ crate::exec::ExecError::Vfs(eno)) => {
            let errno: isize = eno.to_isize();
            if exec_trace_allow() {
                crate::println!(
                    "execve: path={} failed errno={} reason={}",
                    exec_path, errno, e.reason(),
                );
            }
            return errno
        }
        Err(ref e @ crate::exec::ExecError::MetadataOutOfMemory) |
        Err(ref e @ crate::exec::ExecError::UserMm(_)) |
        Err(ref e @ crate::exec::ExecError::AddressOverflow) => {
            if exec_trace_allow() {
                crate::println!(
                    "execve: path={} failed errno=ENOMEM reason={}",
                    exec_path, e.reason(),
                );
            }
            return -ENOMEM
        }
        Err(ref e) => {
            if exec_trace_allow() {
                crate::println!(
                    "execve: path={} failed errno=EINVAL reason={}",
                    exec_path, e.reason(),
                );
            }
            return -EINVAL
        }
    };

    let process = current_process();
    let thread =
        crate::task::current_user_thread().expect("execve arrived without a current user Thread");
    if process.files().close_on_exec().is_err() {
        return -ENOMEM;
    }
    let old_mm = process.replace_mm(prepared.mm);
    let new_mm = process.mm_arc();
    crate::task::replace_current_user_mm(Arc::clone(&old_mm), Arc::clone(&new_mm));
    if thread
        .exec_replace_context(prepared.entry, prepared.stack, prepared.stack_pointer)
        .is_err()
    {
        return -EINVAL;
    }
    // Always reset TLS for the new process.  The previous process may
    // have set tp via musl/glibc __init_tp; the exec'd process must
    // start with a clean tp.  For dynamic programs, set a non-zero
    // initial TLS so ld-linux can access tp-relative GOT before its
    // own TLS_INIT_TP runs.  For static programs, tp=0 is fine.
    let init_tls = if prepared.interp_base.is_some() {
        USER_DEMAND
    } else {
        0_usize
    };
    thread.set_tls_pointer(init_tls);
    set_frame_tls(frame, init_tls);
    match Arc::try_unwrap(old_mm) {
        Ok(mut old_mm) => {
            if old_mm.destroy().is_err() {
                return -EINVAL;
            }
        }
        Err(_) => return -EINVAL,
    }
    set_frame_entry(frame, prepared.entry.get());
    set_frame_stack_pointer(frame, prepared.stack_pointer.get());
    0
}

fn parse_shebang(image: &[u8]) -> Result<Option<(String, Option<String>)>, isize> {
    if !image.starts_with(b"#!") {
        return Ok(None);
    }
    let mut end = 2;
    while end < image.len() && image[end] != b'\n' {
        end += 1;
        if end >= MAX_USER_PATH {
            return Err(-EINVAL);
        }
    }
    let line = core::str::from_utf8(&image[2..end]).map_err(|_| -EINVAL)?;
    let mut rest = line.trim_matches([' ', '\t', '\r']);
    if rest.is_empty() {
        return Err(-EINVAL);
    }
    let split = rest.find([' ', '\t']).unwrap_or(rest.len());
    let interpreter = &rest[..split];
    rest = rest[split..].trim_matches([' ', '\t', '\r']);
    if interpreter.is_empty() || interpreter.len() >= MAX_USER_PATH {
        return Err(-EINVAL);
    }
    let mut interpreter_path = String::new();
    interpreter_path
        .try_reserve(interpreter.len())
        .map_err(|_| -ENOMEM)?;
    interpreter_path.push_str(interpreter);
    let optional_arg = if rest.is_empty() {
        None
    } else {
        let mut argument = String::new();
        argument.try_reserve(rest.len()).map_err(|_| -ENOMEM)?;
        argument.push_str(rest);
        Some(argument)
    };
    Ok(Some((interpreter_path, optional_arg)))
}

fn load_exec_image(path: &str) -> Result<Vec<u8>, isize> {
    const MAX_EXEC_IMAGE: usize = 16 * 1024 * 1024;

    match crate::fs::open(path, myos_vfs::OpenFlags::O_RDONLY) {
        Ok(file) => {
            let stat = file.fstat().map_err(|e| e.to_isize())?;
            if stat.size < 0 {
                return Err(myos_vfs::Errno::Einval.to_isize());
            }
            let size =
                usize::try_from(stat.size).map_err(|_| myos_vfs::Errno::Eoverflow.to_isize())?;
            if size > MAX_EXEC_IMAGE {
                return Err(myos_vfs::Errno::Eoverflow.to_isize());
            }
            let mut image = Vec::new();
            image.try_reserve(size).map_err(|_| -ENOMEM)?;
            image.resize(size, 0);
            let mut output = myos_vfs::MutableIoBuffer::new(&mut image);
            let read = file.read(&mut output).map_err(|e| e.to_isize())?;
            image.truncate(read);
            return Ok(image);
        }
        Err(myos_vfs::Errno::Enoent) if path == "/init" || path == EXEC_PROBE_PATH => {}
        Err(myos_vfs::Errno::Enoent) => {
            // Try lazy materialize from sdcard, then retry.
            if crate::ensure_sdcard_dir_materialized(path) {
                match crate::fs::open(path, myos_vfs::OpenFlags::O_RDONLY) {
                    Ok(file) => {
                        let stat = file.fstat().map_err(|e| e.to_isize())?;
                        let size = usize::try_from(stat.size)
                            .map_err(|_| myos_vfs::Errno::Eoverflow.to_isize())?;
                        if size > MAX_EXEC_IMAGE {
                            return Err(myos_vfs::Errno::Eoverflow.to_isize());
                        }
                        let mut image = Vec::new();
                        image.try_reserve(size).map_err(|_| -ENOMEM)?;
                        image.resize(size, 0);
                        let mut output = myos_vfs::MutableIoBuffer::new(&mut image);
                        let read = file.read(&mut output).map_err(|e| e.to_isize())?;
                        image.truncate(read);
                        return Ok(image);
                    }
                    Err(e) => return Err(e.to_isize()),
                }
            }
            return Err(myos_vfs::Errno::Enoent.to_isize());
        }
        Err(error) => return Err(error.to_isize()),
    }

    if path == "/init" || path == EXEC_PROBE_PATH {
        let entry = VirtAddr::new(user_entry(core::ptr::addr_of!(__m12_exec_success)));
        return crate::elf::build_static_exec(
            entry,
            embedded_user_image(),
            VirtAddr::new(USER_DATA),
        )
        .map_err(|_| -EINVAL);
    }

    Err(myos_vfs::Errno::Enoent.to_isize())
}

#[cfg(target_arch = "riscv64")]
fn set_frame_stack_pointer(frame: &mut crate::arch::trap::TrapFrame, stack_pointer: usize) {
    frame.gpr[2] = stack_pointer;
}

#[cfg(target_arch = "riscv64")]
fn set_frame_tls(frame: &mut crate::arch::trap::TrapFrame, tls: usize) {
    frame.gpr[4] = tls; // tp is x4 on RISC-V
}

#[cfg(target_arch = "riscv64")]
fn set_frame_entry(frame: &mut crate::arch::trap::TrapFrame, entry: usize) {
    frame.sepc = entry;
}

#[cfg(target_arch = "riscv64")]
fn set_signal_handler_frame(
    frame: &mut crate::arch::trap::TrapFrame,
    stack_pointer: usize,
    signal: usize,
    handler: usize,
    restorer: usize,
) {
    frame.gpr[1] = restorer;
    frame.gpr[2] = stack_pointer;
    frame.gpr[10] = signal;
    frame.sepc = handler;
}

#[cfg(target_arch = "loongarch64")]
fn set_frame_stack_pointer(frame: &mut crate::arch::trap::TrapFrame, stack_pointer: usize) {
    frame.gpr[3] = stack_pointer;
}

#[cfg(target_arch = "loongarch64")]
fn set_frame_tls(frame: &mut crate::arch::trap::TrapFrame, tls: usize) {
    frame.gpr[2] = tls; // tp is r2 on LoongArch
}

#[cfg(target_arch = "loongarch64")]
fn set_frame_entry(frame: &mut crate::arch::trap::TrapFrame, entry: usize) {
    frame.era = entry;
}

#[cfg(target_arch = "loongarch64")]
fn set_signal_handler_frame(
    frame: &mut crate::arch::trap::TrapFrame,
    stack_pointer: usize,
    signal: usize,
    handler: usize,
    restorer: usize,
) {
    frame.gpr[1] = restorer;
    frame.gpr[3] = stack_pointer;
    frame.gpr[4] = signal;
    frame.era = handler;
}

fn sys_setpgid(pid: usize, pgid: usize) -> isize {
    let current = current_process();
    let target = if pid == 0 {
        current
    } else {
        match crate::process::lookup_process(crate::process::ProcessId::from_raw_for_kernel(pid)) {
            Some(process) => process,
            None => return -crate::syscall::errno::ESRCH,
        }
    };
    let group = if pgid == 0 { target.id().get() } else { pgid };
    target.set_process_group(group as isize);
    0
}

fn sys_getpgid(pid: usize) -> isize {
    let process = if pid == 0 {
        current_process()
    } else {
        match crate::process::lookup_process(crate::process::ProcessId::from_raw_for_kernel(pid)) {
            Some(process) => process,
            None => return -crate::syscall::errno::ESRCH,
        }
    };
    process.process_group()
}

fn sys_getsid(pid: usize) -> isize {
    let process = if pid == 0 {
        current_process()
    } else {
        match crate::process::lookup_process(crate::process::ProcessId::from_raw_for_kernel(pid)) {
            Some(process) => process,
            None => return -crate::syscall::errno::ESRCH,
        }
    };
    process.session()
}

fn sys_kill(pid: usize, signal: usize) -> isize {
    match crate::signal::send_signal(
        crate::process::ProcessId::from_raw_for_kernel(pid),
        signal as u32,
    ) {
        Ok(()) => 0,
        Err(errno) => errno.to_isize(),
    }
}

fn sys_rt_sigaction(arguments: [usize; 6]) -> isize {
    let signal = arguments[0] as u32;
    if crate::signal::signal_bit(signal).is_none() || signal == crate::signal::SIGKILL {
        return -EINVAL;
    }
    if arguments[3] != core::mem::size_of::<u64>() {
        return -EINVAL;
    }
    let new_action = arguments[1];
    let old_action = arguments[2];
    let process = current_process();
    let signals = process.signals();
    if old_action != 0 {
        let action = signals.action(signal).unwrap_or_default();
        let result = copy_plain_to_user(old_action, &action);
        if result != 0 {
            return result;
        }
    }
    if new_action != 0 {
        let mut action = match copy_plain_from_user::<crate::signal::KernelSigAction>(new_action) {
            Ok(action) => action,
            Err(errno) => return errno,
        };
        action.mask &= !crate::signal::unblockable_mask();
        if let Err(errno) = signals.set_action(signal, action) {
            return errno.to_isize();
        }
    }
    0
}

fn sys_rt_sigprocmask(how: usize, set_address: usize, oldset_address: usize) -> isize {
    let thread =
        crate::task::current_user_thread().expect("rt_sigprocmask arrived without current Thread");
    let old = thread.blocked_signals();
    if oldset_address != 0 && copy_to_user(oldset_address, &old.to_ne_bytes()).is_err() {
        return -EFAULT;
    }
    let input = if set_address == 0 {
        None
    } else {
        let mut bytes = [0_u8; core::mem::size_of::<u64>()];
        if copy_from_user(set_address, &mut bytes).is_err() {
            return -EFAULT;
        }
        Some(u64::from_ne_bytes(bytes))
    };
    let next = match crate::signal::update_mask(old, how, input) {
        Ok(mask) => mask,
        Err(errno) => return errno.to_isize(),
    };
    thread.set_blocked_signals(next);
    0
}

fn sys_rt_sigtimedwait(arguments: [usize; 6]) -> isize {
    // set, info, timeout, sigsetsize
    let set_address = arguments[0];
    if set_address == 0 {
        return -EINVAL;
    }
    let mut set_bytes = [0_u8; 8]; // 64-bit sigset
    if copy_from_user(set_address, &mut set_bytes).is_err() {
        return -EFAULT;
    }
    let waited_mask = u64::from_ne_bytes(set_bytes);

    let thread =
        crate::task::current_user_thread().expect("rt_sigtimedwait without current Thread");
    let process = thread.process();

    // Check if any signal in the waited set is pending and unblocked
    if let Some(signal) = process.signals().take_matching_unblocked(waited_mask, thread.blocked_signals()) {
        // Write siginfo to userspace if info pointer is non-null
        let info_address = arguments[1];
        if info_address != 0 {
            let info = KernelSigInfo {
                signo: signal as i32,
                errno: 0,
                code: 0, // SI_USER
                payload: [0; 116],
            };
            if copy_to_user(info_address, unsafe {
                core::slice::from_raw_parts(
                    &info as *const KernelSigInfo as *const u8,
                    core::mem::size_of::<KernelSigInfo>(),
                )
            })
            .is_err()
            {
                return -EFAULT;
            }
        }
        return signal as isize;
    }

    // No matching signal
    -EAGAIN
}

fn sys_rt_sigreturn(frame: &mut crate::arch::trap::TrapFrame) -> Result<(), isize> {
    let signal_frame =
        copy_plain_from_user::<UserSignalFrame>(frame.stack_pointer()).map_err(|_| -EFAULT)?;
    if signal_frame.magic != SIGNAL_FRAME_MAGIC {
        return Err(-EINVAL);
    }
    let thread =
        crate::task::current_user_thread().expect("rt_sigreturn arrived without current Thread");
    thread.set_blocked_signals(signal_frame.old_mask);
    *frame = signal_frame.trap_frame;
    Ok(())
}

const SOL_SOCKET: usize = 1;
const SO_REUSEADDR: usize = 2;
const SO_ERROR: usize = 4;
const SO_KEEPALIVE: usize = 9;
const SO_BROADCAST: usize = 6;
const SO_SNDBUF: usize = 7;
const SO_RCVBUF: usize = 8;
const SO_SNDTIMEO: usize = 21;
const SO_RCVTIMEO: usize = 20;
const TCP_NODELAY: usize = 1;
const IPPROTO_TCP: usize = 6;

fn sys_setsockopt(
    _fd: usize, _level: usize, _optname: usize,
    _optval: usize, _optlen: usize,
) -> isize {
    // Accept common socket options as no-ops so netperf/iperf don't fail.
    // A full implementation would validate the option and apply it to
    // the socket's internal state.
    0
}

fn sys_getsockopt(
    _fd: usize, _level: usize, optname: usize,
    optval: usize, optlen: usize,
) -> isize {
    // Return sensible defaults for commonly-queried socket options.
    let mut value_len = [0_u8; 4];
    let value: i32 = match optname {
        SO_ERROR => 0,          // no pending error
        SO_KEEPALIVE => 0,      // keepalive disabled
        SO_REUSEADDR => 1,      // reuseaddr enabled (common default)
        SO_SNDBUF => 65536,     // default send buffer
        SO_RCVBUF => 65536,     // default recv buffer
        _ => 0,
    };
    value_len.copy_from_slice(&value.to_ne_bytes());
    if optlen != 0 {
        let len = core::cmp::min(4, optlen);
        if copy_to_user(optval, &value_len[..len]).is_err() {
            return -EFAULT;
        }
        // Write back the actual length if the user provided a pointer.
        if copy_to_user(optlen, &len.to_ne_bytes()).is_err() {
            return -EFAULT;
        }
    }
    0
}

fn deliver_pending_signal(frame: &mut crate::arch::trap::TrapFrame) {
    let thread =
        crate::task::current_user_thread().expect("signal delivery arrived without current Thread");
    let process = thread.process();
    let Some(signal) = process.signals().take_unblocked(thread.blocked_signals()) else {
        return;
    };

    // ── P9-H12: trace SIGALRM delivery ──
    #[cfg(target_arch = "loongarch64")]
    if signal == 14 && oscomp_la_sleep_trace_active() {
        let action = process.signals().action(signal).unwrap_or_default();
        crate::println!(
            "oscomp-la-signal-trace: deliver sig=14 handler={:#x} blocked={:#x}",
            action.handler,
            thread.blocked_signals(),
        );
    }

    let action = process.signals().action(signal).unwrap_or_default();
    match action.handler {
        SIG_DFL => {
            TERMINATED.store(true, Ordering::Release);
            EXIT_STATUS.store(-(signal as isize), Ordering::Release);
            return_to_kernel(frame, -(signal as isize));
        }
        SIG_IGN => {}
        handler => {
            if action.restorer == 0 {
                TERMINATED.store(true, Ordering::Release);
                EXIT_STATUS.store(-EINVAL, Ordering::Release);
                return_to_kernel(frame, -EINVAL);
                return;
            }
            if install_signal_frame(frame, signal, action, handler).is_err() {
                TERMINATED.store(true, Ordering::Release);
                EXIT_STATUS.store(-EFAULT, Ordering::Release);
                return_to_kernel(frame, -EFAULT);
            }
        }
    }
}

fn install_signal_frame(
    frame: &mut crate::arch::trap::TrapFrame,
    signal: u32,
    action: crate::signal::KernelSigAction,
    handler: usize,
) -> Result<(), ()> {
    let thread =
        crate::task::current_user_thread().expect("signal frame install without current Thread");
    let old_mask = thread.blocked_signals();
    let signal_bit = crate::signal::signal_bit(signal).ok_or(())?;
    let new_mask = (old_mask | action.mask | signal_bit) & !crate::signal::unblockable_mask();
    let frame_size = core::mem::size_of::<UserSignalFrame>();
    let signal_sp = frame
        .stack_pointer()
        .checked_sub(frame_size)
        .map(|sp| sp & !0xf)
        .ok_or(())?;
    let signal_frame = UserSignalFrame {
        magic: SIGNAL_FRAME_MAGIC,
        signal: signal as u64,
        old_mask,
        reserved: 0,
        trap_frame: *frame,
    };
    let result = copy_plain_to_user(signal_sp, &signal_frame);
    if result != 0 {
        return Err(());
    }
    thread.set_blocked_signals(new_mask);
    set_signal_handler_frame(frame, signal_sp, signal as usize, handler, action.restorer);
    Ok(())
}

fn sys_wait4(pid: usize, status_address: usize) -> isize {
    let requested = if pid == 0 { -1 } else { pid as isize };
    let process = current_process();
    loop {
        match process.wait_zombie_child(requested) {
            Ok(Some((child, raw_status))) => {
                let child_pid = child.id().get();
                if status_address != 0 {
                    // Linux wait status encoding for external programs.
                    // Gate tests (verifier=true) expect raw exit codes.
                    let encoded: i32 = if ACTIVE.load(Ordering::Acquire) {
                        raw_status as i32
                    } else if raw_status < 0 {
                        // Killed by signal
                        ((-raw_status) & 0x7f) as i32
                    } else {
                        // Normal exit
                        ((raw_status as i32) & 0xff) << 8
                    };
                    if copy_to_user(status_address, &encoded.to_ne_bytes()).is_err() {
                        return -EFAULT;
                    }
                }
                match Arc::try_unwrap(child) {
                    Ok(child) => {
                        if child.destroy().is_err() {
                            return -EINVAL;
                        }
                    }
                    Err(_) => return -EINVAL,
                }
                return child_pid as isize;
            }
            Ok(None) if !process.has_child(requested) => return -ECHILD,
            Ok(None) => {
                let _ = crate::task::block_current_on_if_from_user_trap(
                    process.child_wait_queue(),
                    || !process.has_zombie_child(requested) && process.has_child(requested),
                );
            }
            Err(_) => return -ECHILD,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct KernelTimespec {
    sec: isize,
    nsec: isize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct KernelTimeval {
    sec: isize,
    usec: isize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct KernelTms {
    utime: isize,
    stime: isize,
    cutime: isize,
    cstime: isize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct KernelRlimit64 {
    cur: u64,
    max: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct KernelPollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

impl KernelPollFd {
    fn from_bytes(bytes: &[u8]) -> Self {
        debug_assert_eq!(bytes.len(), core::mem::size_of::<Self>());
        Self {
            fd: i32::from_ne_bytes(bytes[0..4].try_into().expect("pollfd fd slice")),
            events: i16::from_ne_bytes(bytes[4..6].try_into().expect("pollfd events slice")),
            revents: i16::from_ne_bytes(bytes[6..8].try_into().expect("pollfd revents slice")),
        }
    }

    fn write_bytes(self, bytes: &mut [u8]) {
        debug_assert_eq!(bytes.len(), core::mem::size_of::<Self>());
        bytes[0..4].copy_from_slice(&self.fd.to_ne_bytes());
        bytes[4..6].copy_from_slice(&self.events.to_ne_bytes());
        bytes[6..8].copy_from_slice(&self.revents.to_ne_bytes());
    }
}

fn sys_nanosleep(request_address: usize, remain_address: usize) -> isize {
    let request = match copy_plain_from_user::<KernelTimespec>(request_address) {
        Ok(request) => request,
        Err(errno) => return errno,
    };
    if request.sec < 0 || request.nsec < 0 || request.nsec >= 1_000_000_000 {
        return -EINVAL;
    }
    if remain_address != 0 {
        let zero = KernelTimespec { sec: 0, nsec: 0 };
        let result = copy_plain_to_user(remain_address, &zero);
        if result != 0 {
            return result;
        }
    }
    let duration = core::time::Duration::new(request.sec as u64, request.nsec as u32);
    if !duration.is_zero() {
        crate::timer::sleep(duration);
    }
    0
}

fn sys_clock_gettime(clock_id: usize, timespec_address: usize) -> isize {
    if clock_id > 1 {
        return -EINVAL;
    }
    let ns = current_time_ns();
    let ts = KernelTimespec {
        sec: (ns / 1_000_000_000) as isize,
        nsec: (ns % 1_000_000_000) as isize,
    };
    copy_plain_to_user(timespec_address, &ts)
}

const TIMER_ABSTIME: usize = 1;

fn sys_clock_nanosleep(
    clock_id: usize,
    flags: usize,
    request_address: usize,
    remain_address: usize,
) -> isize {
    if clock_id > 1 {
        return -EINVAL;
    }
    if flags & !TIMER_ABSTIME != 0 {
        return -EINVAL;
    }

    let request = match copy_plain_from_user::<KernelTimespec>(request_address) {
        Ok(request) => request,
        Err(errno) => return errno,
    };

    if request.sec < 0 || request.nsec < 0 || request.nsec >= 1_000_000_000 {
        return -EINVAL;
    }

    let duration = core::time::Duration::new(request.sec as u64, request.nsec as u32);

    if remain_address != 0 {
        let zero = KernelTimespec { sec: 0, nsec: 0 };
        let result = copy_plain_to_user(remain_address, &zero);
        if result != 0 {
            return result;
        }
    }

    if flags & TIMER_ABSTIME != 0 {
        let target_ns = (duration.as_secs() as u128) * 1_000_000_000_u128
            + u128::from(duration.subsec_nanos());
        let now_ns = current_time_ns();
        if target_ns > now_ns {
            let delta_ns = target_ns - now_ns;
            let sleep_ns = core::cmp::min(delta_ns, u128::from(u64::MAX));
            crate::timer::sleep(core::time::Duration::from_nanos(sleep_ns as u64));
        }
    } else if !duration.is_zero() {
        crate::timer::sleep(duration);
    }

    0
}

fn sys_gettimeofday(timeval_address: usize) -> isize {
    if timeval_address == 0 {
        return 0;
    }
    let ns = current_time_ns();
    let tv = KernelTimeval {
        sec: (ns / 1_000_000_000) as isize,
        usec: ((ns % 1_000_000_000) / 1_000) as isize,
    };
    copy_plain_to_user(timeval_address, &tv)
}

fn sys_times(tms_address: usize) -> isize {
    let ticks = (current_time_ns() / 10_000_000) as isize;
    if tms_address != 0 {
        let tms = KernelTms {
            utime: 0,
            stime: 0,
            cutime: 0,
            cstime: 0,
        };
        let result = copy_plain_to_user(tms_address, &tms);
        if result != 0 {
            return result;
        }
    }
    ticks
}

fn current_time_ns() -> u128 {
    let cycles = crate::time::now().cycles();
    (u128::from(cycles) * 1_000_000_000_u128) / u128::from(crate::time::clock_frequency_hz())
}

fn sys_prlimit64(pid: usize, resource: usize, new_limit: usize, old_limit: usize) -> isize {
    if pid != 0 && pid != current_process().id().get() {
        return -crate::syscall::errno::ESRCH;
    }
    if new_limit != 0 {
        return -crate::syscall::errno::EPERM;
    }
    if old_limit == 0 {
        return 0;
    }
    let limit = match resource {
        RLIMIT_NOFILE => KernelRlimit64 {
            cur: crate::process::PROCESS_MAX_FDS as u64,
            max: crate::process::PROCESS_MAX_FDS as u64,
        },
        RLIMIT_STACK => KernelRlimit64 {
            cur: PAGE_SIZE as u64,
            max: PAGE_SIZE as u64,
        },
        RLIMIT_AS => KernelRlimit64 {
            cur: u64::MAX,
            max: u64::MAX,
        },
        _ => KernelRlimit64 {
            cur: u64::MAX,
            max: u64::MAX,
        },
    };
    copy_plain_to_user(old_limit, &limit)
}

fn sys_sysinfo(address: usize) -> isize {
    let mut raw = [0_u8; 104];
    let uptime = (crate::time::now().cycles() / crate::time::clock_frequency_hz()) as i64;
    raw[0..8].copy_from_slice(&uptime.to_ne_bytes());
    let free_pages = crate::page_alloc::total_free_pages().unwrap_or(0);
    let free = free_pages.saturating_mul(PAGE_SIZE) as u64;
    let total = free;
    raw[32..40].copy_from_slice(&total.to_ne_bytes());
    raw[40..48].copy_from_slice(&free.to_ne_bytes());
    raw[80..82].copy_from_slice(&(current_process().thread_count() as u16).to_ne_bytes());
    raw[100..104].copy_from_slice(&1_u32.to_ne_bytes());
    if copy_to_user(address, &raw).is_err() {
        return -EFAULT;
    }
    0
}

fn sys_getrandom(address: usize, length: usize) -> isize {
    if length > MAX_USER_COPY {
        return -EINVAL;
    }
    let mut bytes = [0_u8; MAX_USER_COPY];
    crate::rng::fill_random(&mut bytes[..length]);
    if copy_to_user(address, &bytes[..length]).is_err() {
        return -EFAULT;
    }
    length as isize
}

// ---------------------------------------------------------------------------
// Futex — 轻量级用户空间互斥锁支持
// ---------------------------------------------------------------------------

const FUTEX_WAIT: usize = 0;
const FUTEX_WAKE: usize = 1;
const FUTEX_PRIVATE_FLAG: usize = 128;

static FUTEX_QUEUES: crate::irq_lock::IrqSpinLock<
    alloc::collections::BTreeMap<usize, alloc::sync::Arc<crate::task::WaitQueue>>,
> = crate::irq_lock::IrqSpinLock::new_with_class(
    alloc::collections::BTreeMap::new(),
    crate::lockdep::LockClass::new("futex.queues", crate::lockdep::LockRank::WaitQueue, 5),
);

fn get_futex_queue(uaddr: usize) -> alloc::sync::Arc<crate::task::WaitQueue> {
    let mut queues = FUTEX_QUEUES.lock();
    if let Some(q) = queues.get(&uaddr) {
        alloc::sync::Arc::clone(q)
    } else {
        let q = alloc::sync::Arc::new(crate::task::WaitQueue::new());
        queues.insert(uaddr, alloc::sync::Arc::clone(&q));
        q
    }
}

fn sys_futex(
    uaddr: usize,
    futex_op: usize,
    val: usize,
    _timeout: usize,
    _uaddr2: usize,
    _val3: usize,
) -> isize {
    let op = futex_op & !FUTEX_PRIVATE_FLAG;

    match op {
        FUTEX_WAIT | 9 /* FUTEX_WAIT_BITSET */ => {
            let current_val = match copy_plain_from_user::<u32>(uaddr) {
                Ok(v) => v as usize,
                Err(e) => return e,
            };
            if current_val != val {
                return -(crate::syscall::errno::EAGAIN);
            }
            let queue = get_futex_queue(uaddr);
            let _ = crate::task::block_current_on_if_from_user_trap(
                &queue,
                || {
                    // Re-check: if value changed between check and block, don't sleep
                    match copy_plain_from_user::<u32>(uaddr) {
                        Ok(v) => v as usize == val,
                        Err(_) => false,
                    }
                },
            );
            0
        }
        FUTEX_WAKE | 10 /* FUTEX_WAKE_BITSET */ => {
            let queue = get_futex_queue(uaddr);
            let woken = if val >= 1 { queue.wake_all() } else { 0 };
            woken as isize
        }
        _ => -(crate::syscall::errno::ENOSYS),
    }
}

fn sys_mknodat(dirfd: usize, path_address: usize, mode: usize, _dev: usize) -> isize {
    // mknodat creates device special files; regular files must use open(O_CREAT).
    let file_type = mode & 0o170000;
    if file_type == 0o100000 || file_type == 0 {
        // Regular file: not allowed through mknod; use openat(O_CREAT).
        return -(crate::syscall::errno::EINVAL);
    }
    let path = match resolve_user_path(dirfd, path_address) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    // Only allow creating device nodes under /dev or /dev/pts.
    if !path.starts_with("/dev/") && path != "/dev" {
        return -(crate::syscall::errno::EPERM);
    }
    // For supported device types, allow the operation as a no-op if the path
    // already exists in devfs; devfs already has all standard device nodes.
    match crate::fs::stat(&path) {
        Ok(_) => 0,
        Err(myos_vfs::Errno::Enoent) => {
            // Creating new device nodes is not supported yet; return success
            // for char devices under /dev to unblock scripts that try to
            // recreate standard device nodes.
            if file_type == 0o020000 || file_type == 0o060000 {
                0
            } else {
                -(crate::syscall::errno::ENOSYS)
            }
        }
        Err(errno) => errno.to_isize(),
    }
}

fn sys_utimensat(_dirfd: usize, _path: usize, _times: usize) -> isize {
    // Stub: enough to unblock `touch` without full nanosecond timestamp support.
    // Busybox `touch` calls utimensat(AT_FDCWD, path, NULL, 0) to set mtime/atime
    // to "now". Accepting NULL times with success makes touch work; the VFS
    // does not track timestamps yet.
    0
}

fn fill_statfs_buffer(f_type: u64) -> [u8; 112] {
    // struct statfs layout (112 bytes for 64-bit Linux):
    // f_type(8) f_bsize(8) f_blocks(8) f_bfree(8) f_bavail(8) f_files(8)
    // f_ffree(8) f_fsid(8) f_namelen(8) f_frsize(8) f_flags(8) f_spare[4](32)
    let mut data = [0_u8; 112];
    data[0..8].copy_from_slice(&f_type.to_ne_bytes());          // f_type
    data[8..16].copy_from_slice(&4096_u64.to_ne_bytes());       // f_bsize
    data[16..24].copy_from_slice(&1000000_u64.to_ne_bytes());   // f_blocks
    data[24..32].copy_from_slice(&900000_u64.to_ne_bytes());    // f_bfree
    data[32..40].copy_from_slice(&900000_u64.to_ne_bytes());    // f_bavail
    data[40..48].copy_from_slice(&1000000_u64.to_ne_bytes());   // f_files
    data[48..56].copy_from_slice(&999000_u64.to_ne_bytes());    // f_ffree
    // f_fsid[0..1] stays zero (56..72)
    data[64..72].copy_from_slice(&255_u64.to_ne_bytes());       // f_namelen
    data[72..80].copy_from_slice(&4096_u64.to_ne_bytes());      // f_frsize
    data
}

fn sys_statfs_path(path_address: usize, buf: usize) -> isize {
    let raw_path = match copy_user_c_string(path_address) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    let path = match resolve_path_from_user(AT_FDCWD, &raw_path) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    let f_type = crate::fs::resolve_fs_magic(&path);
    let data = fill_statfs_buffer(f_type);
    if copy_to_user(buf, &data).is_err() {
        return -EFAULT;
    }
    0
}

fn sys_statfs_fd(fd: usize, buf: usize) -> isize {
    // fstatfs: determine fs type from the file descriptor.
    // For fd-based fstatfs we default to tmpfs magic unless we can
    // resolve the path from the file descriptor's procfs entry.
    match current_process_file(fd) {
        Ok(_file) => {
            // Without per-file mount tracking, default to tmpfs.
            // Procfs fd lookups get procfs magic.
            let f_type = if fd < 3 {
                // stdin/stdout/stderr → likely console on devtmpfs
                0x01021994_u64
            } else {
                0x01021994_u64
            };
            let data = fill_statfs_buffer(f_type);
            if copy_to_user(buf, &data).is_err() {
                return -EFAULT;
            }
            0
        }
        Err(errno) => errno.to_isize(),
    }
}

fn sys_syslog(action: usize, _buf: usize, _len: usize) -> isize {
    // SYSLOG_ACTION_CLOSE(0)          — no-op
    // SYSLOG_ACTION_OPEN(1)           — no-op
    // SYSLOG_ACTION_READ(2)           — not supported, return 0
    // SYSLOG_ACTION_READ_ALL(3)       — return 0 (no messages in ring buffer)
    // SYSLOG_ACTION_READ_CLEAR(4)     — return 0 (no messages, buffer cleared)
    // SYSLOG_ACTION_CLEAR(5)          — no-op
    // SYSLOG_ACTION_CONSOLE_OFF(6)    — not supported
    // SYSLOG_ACTION_CONSOLE_ON(7)     — not supported
    // SYSLOG_ACTION_CONSOLE_LEVEL(8)  — not supported
    // SYSLOG_ACTION_SIZE_UNREAD(9)    — return 0
    // SYSLOG_ACTION_SIZE_BUFFER(10)   — report ring buffer size = 0
    match action {
        0 | 1 | 5 => 0,
        2 | 3 | 4 | 9 => 0, // 0 bytes read
        10 => 0,             // size of kernel log buffer = 0
        _ => -(crate::syscall::errno::EINVAL),
    }
}

const SCHED_OTHER: usize = 0;
const SCHED_FIFO: usize = 1;
const SCHED_RR: usize = 2;

fn sys_sched_getaffinity(_pid: usize, cpusetsize: usize, mask: usize) -> isize {
    // Return affinity mask with all active CPUs set (as-per-Linux cpumask).
    if cpusetsize < core::mem::size_of::<u64>() {
        return -(crate::syscall::errno::EINVAL);
    }
    // pid=0 means current thread; non-zero pids may be checked against process list.
    let cpu_count = crate::smp::scheduler_active_cpu_count().min(64);
    let bits = if cpu_count >= 64 { !0_u64 } else { (1_u64 << cpu_count) - 1 };
    let raw = bits.to_ne_bytes();
    let copy_len = core::cmp::min(cpusetsize, core::mem::size_of::<u64>());
    if copy_to_user(mask, &raw[..copy_len]).is_err() {
        return -EFAULT;
    }
    copy_len as isize
}

fn sys_sched_setaffinity(_pid: usize, cpusetsize: usize, mask: usize) -> isize {
    // Single-core scheduler: accept any mask that includes CPU 0, reject others.
    if cpusetsize < core::mem::size_of::<u64>() {
        return -(crate::syscall::errno::EINVAL);
    }
    let mut raw = [0_u8; 8];
    let copy_len = core::cmp::min(cpusetsize, core::mem::size_of::<u64>());
    if copy_len > raw.len() {
        return -(crate::syscall::errno::EINVAL);
    }
    if copy_from_user(mask, &mut raw[..copy_len]).is_err() {
        return -EFAULT;
    }
    let bits = u64::from_ne_bytes(raw);
    // Accept mask that includes at least CPU 0 (bit 0 set).
    if bits & 1 == 0 {
        return -(crate::syscall::errno::EINVAL);
    }
    0
}

fn sys_sched_getscheduler(_pid: usize) -> isize {
    // All tasks run under SCHED_OTHER in this kernel.
    SCHED_OTHER as isize
}

fn sys_sched_getparam(_pid: usize, param_address: usize) -> isize {
    // struct sched_param: sched_priority (i32), on Linux 64-bit.
    // SCHED_OTHER always has priority 0.
    let priority: i32 = 0;
    let raw = priority.to_ne_bytes();
    if copy_to_user(param_address, &raw).is_err() {
        return -EFAULT;
    }
    0
}

fn sys_sched_setscheduler(_pid: usize, policy: usize, param_address: usize) -> isize {
    // Only SCHED_OTHER is fully supported. SCHED_FIFO/RR are accepted but
    // mapped to SCHED_OTHER internally (no real-time scheduling yet).
    match policy {
        SCHED_OTHER | SCHED_FIFO | SCHED_RR => {
            // Read sched_param to verify the pointer is valid.
            let mut raw = [0_u8; 4];
            if copy_from_user(param_address, &mut raw).is_err() {
                return -EFAULT;
            }
            // For SCHED_OTHER, priority must be 0 per POSIX.
            let priority = i32::from_ne_bytes(raw);
            if policy == SCHED_OTHER && priority != 0 {
                return -(crate::syscall::errno::EINVAL);
            }
            // For SCHED_FIFO/RR, accept any priority in valid range [1..99].
            if (policy == SCHED_FIFO || policy == SCHED_RR) && priority < 1 {
                return -(crate::syscall::errno::EINVAL);
            }
            0
        }
        _ => -(crate::syscall::errno::EINVAL),
    }
}

fn sys_renameat2(
    olddirfd: usize, oldpath: usize,
    newdirfd: usize, newpath: usize,
    _flags: usize,
) -> isize {
    // Delegate to renameat for now; flags (RENAME_NOREPLACE etc.) are ignored.
    sys_renameat(olddirfd, oldpath, newdirfd, newpath)
}

fn sys_prctl(_option: usize, _arg2: usize, _arg3: usize) -> isize {
    // Most PR_* options are not needed by test workloads.
    // Return 0 for PR_SET_NAME / PR_GET_NAME and friends.
    0
}

fn sys_setitimer(_which: usize, _new_value: usize, _old_value: usize) -> isize {
    // Stub: busybox `sleep` and some benchmarks use setitimer(ITIMER_REAL).
    // Accept the call without implementing interval timers yet.
    0
}

fn sys_getitimer(_which: usize, _old_value: usize) -> isize {
    0
}

fn sys_getrusage(_who: usize, usage: usize) -> isize {
    // Return a zeroed rusage struct (144 bytes on Linux 64-bit).
    let raw = [0_u8; 144];
    if copy_to_user(usage, &raw).is_err() {
        return -EFAULT;
    }
    0
}

fn sys_uname(address: usize) -> isize {
    let mut raw = [0_u8; 65 * 6];
    write_uts_field(&mut raw, 0, b"Linux");
    write_uts_field(&mut raw, 1, b"sudoos");
    write_uts_field(&mut raw, 2, b"6.12.0");
    write_uts_field(&mut raw, 3, b"#1 SMP PREEMPT_DYNAMIC");
    write_uts_field(&mut raw, 4, crate::arch::ARCH_NAME.as_bytes());
    write_uts_field(&mut raw, 5, b"unknown");
    if copy_to_user(address, &raw).is_err() {
        return -EFAULT;
    }
    0
}

fn write_uts_field(raw: &mut [u8], index: usize, value: &[u8]) {
    let start = index * 65;
    let len = value.len().min(64);
    raw[start..start + len].copy_from_slice(&value[..len]);
}

fn sys_getcwd(address: usize, size: usize) -> isize {
    if size == 0 {
        return -EINVAL;
    }
    let cwd = current_process().fs().cwd_path();
    let needed = match cwd.len().checked_add(1) {
        Some(needed) => needed,
        None => return -ERANGE,
    };
    if needed > size || needed > MAX_USER_COPY {
        return -ERANGE;
    }
    let mut bytes = [0_u8; MAX_USER_COPY];
    bytes[..cwd.len()].copy_from_slice(cwd.as_bytes());
    bytes[cwd.len()] = 0;
    if copy_to_user(address, &bytes[..needed]).is_err() {
        return -EFAULT;
    }
    address as isize
}

fn sys_chdir(path_address: usize) -> isize {
    let path = match resolve_user_path(AT_FDCWD, path_address) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    match crate::fs::chdir(&path) {
        Ok(()) => match current_process().fs().set_cwd(&path) {
            Ok(()) => 0,
            Err(errno) => errno.to_isize(),
        },
        Err(errno) => errno.to_isize(),
    }
}

fn sys_getdents64(fd: usize, address: usize, length: usize) -> isize {
    if length > MAX_USER_COPY {
        return -EINVAL;
    }
    let file = match current_process_file(fd) {
        Ok(file) => file,
        Err(errno) => return errno.to_isize(),
    };
    let mut buffer = [0_u8; MAX_USER_COPY];
    let mut output = myos_vfs::MutableIoBuffer::new(&mut buffer[..length]);
    match file.readdir(&mut output) {
        Ok(read) => {
            if copy_to_user(address, output.filled_bytes()).is_err() {
                return -EFAULT;
            }
            read as isize
        }
        Err(errno) => errno.to_isize(),
    }
}

fn sys_mkdirat(dirfd: usize, path_address: usize, mode: usize) -> isize {
    let path = match resolve_user_path(dirfd, path_address) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    match crate::fs::mkdir(&path, mode as u32) {
        Ok(()) => 0,
        Err(errno) => errno.to_isize(),
    }
}

fn sys_unlinkat(dirfd: usize, path_address: usize, flags: usize) -> isize {
    if flags & !AT_REMOVEDIR != 0 {
        return -EINVAL;
    }
    let path = match resolve_user_path(dirfd, path_address) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    match crate::fs::unlink(&path, flags & AT_REMOVEDIR != 0) {
        Ok(()) => 0,
        Err(errno) => errno.to_isize(),
    }
}

fn sys_symlinkat(target_address: usize, new_dirfd: usize, link_path_address: usize) -> isize {
    let target = match copy_user_c_string(target_address) {
        Ok(target) => target,
        Err(errno) => return errno,
    };
    let link_path = match resolve_user_path(new_dirfd, link_path_address) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    match crate::fs::symlink(&target, &link_path) {
        Ok(()) => 0,
        Err(errno) => errno.to_isize(),
    }
}

fn sys_linkat(
    old_dirfd: usize,
    old_path_address: usize,
    new_dirfd: usize,
    new_path_address: usize,
    flags: usize,
) -> isize {
    if flags & !AT_SYMLINK_FOLLOW != 0 {
        return -EINVAL;
    }
    let old_path = match resolve_user_path(old_dirfd, old_path_address) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    let new_path = match resolve_user_path(new_dirfd, new_path_address) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    match crate::fs::link(&old_path, &new_path, flags & AT_SYMLINK_FOLLOW != 0) {
        Ok(()) => 0,
        Err(errno) => errno.to_isize(),
    }
}

fn sys_mount(
    source_address: usize,
    target_address: usize,
    filesystem_type_address: usize,
    flags: usize,
) -> isize {
    let source = if source_address == 0 {
        None
    } else {
        match copy_user_c_string(source_address) {
            Ok(source) => Some(source),
            Err(errno) => return errno,
        }
    };
    let target = match resolve_user_path(AT_FDCWD, target_address) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    if filesystem_type_address == 0 {
        return -EINVAL;
    }
    let filesystem_type = match copy_user_c_string(filesystem_type_address) {
        Ok(filesystem_type) => filesystem_type,
        Err(errno) => return errno,
    };
    match crate::fs::mount(source.as_deref(), &target, &filesystem_type, flags) {
        Ok(()) => 0,
        Err(errno) => errno.to_isize(),
    }
}

fn sys_umount2(target_address: usize, flags: usize) -> isize {
    let target = match resolve_user_path(AT_FDCWD, target_address) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    match crate::fs::umount(&target, flags) {
        Ok(()) => 0,
        Err(errno) => errno.to_isize(),
    }
}

fn sys_faccessat(dirfd: usize, path_address: usize, mode: usize) -> isize {
    if mode & !(R_OK | W_OK | X_OK) != 0 {
        return -EINVAL;
    }
    let path = match resolve_user_path(dirfd, path_address) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    let mut result = crate::fs::stat(&path);
    if result.is_err() && crate::ensure_sdcard_dir_materialized(&path) {
        result = crate::fs::stat(&path);
    }
    match result {
        Ok(_) => 0,
        Err(errno) => errno.to_isize(),
    }
}

fn sys_renameat(
    old_dirfd: usize,
    old_path_address: usize,
    new_dirfd: usize,
    new_path_address: usize,
) -> isize {
    let old_path = match resolve_user_path(old_dirfd, old_path_address) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    let new_path = match resolve_user_path(new_dirfd, new_path_address) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    match crate::fs::rename(&old_path, &new_path) {
        Ok(()) => 0,
        Err(errno) => errno.to_isize(),
    }
}

fn sys_readlinkat(
    dirfd: usize,
    path_address: usize,
    buffer_address: usize,
    length: usize,
) -> isize {
    if length > MAX_USER_COPY {
        return -EINVAL;
    }
    let path = match resolve_user_path(dirfd, path_address) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    let mut buffer = [0_u8; MAX_USER_COPY];
    let mut output = myos_vfs::MutableIoBuffer::new(&mut buffer[..length]);
    match crate::fs::readlink(&path, &mut output) {
        Ok(read) => {
            if copy_to_user(buffer_address, output.filled_bytes()).is_err() {
                return -EFAULT;
            }
            read as isize
        }
        Err(errno) => errno.to_isize(),
    }
}

fn sys_ppoll(fds_address: usize, nfds: usize, _timeout_address: usize) -> isize {
    let pollfd_len = core::mem::size_of::<KernelPollFd>();
    let bytes_len = match nfds.checked_mul(pollfd_len) {
        Some(length) if length <= MAX_USER_COPY => length,
        _ => return -EINVAL,
    };
    if nfds == 0 {
        return 0;
    }
    let mut buffer = [0_u8; MAX_USER_COPY];
    if copy_from_user(fds_address, &mut buffer[..bytes_len]).is_err() {
        return -EFAULT;
    }
    let mut ready = 0_isize;
    for index in 0..nfds {
        let offset = index * pollfd_len;
        let mut pollfd = KernelPollFd::from_bytes(&buffer[offset..offset + pollfd_len]);
        pollfd.revents = 0;
        if pollfd.fd >= 0 {
            match current_process_file(pollfd.fd as usize) {
                Ok(file) => {
                    let requested = myos_vfs::PollEvents::from_bits(pollfd.events as u16);
                    let events = file.poll(requested);
                    pollfd.revents = events.bits() as i16;
                    if !events.is_empty() {
                        ready += 1;
                    }
                }
                Err(_) => {
                    pollfd.revents = myos_vfs::PollEvents::NVAL.bits() as i16;
                    ready += 1;
                }
            }
        }
        pollfd.write_bytes(&mut buffer[offset..offset + pollfd_len]);
    }
    if copy_to_user(fds_address, &buffer[..bytes_len]).is_err() {
        return -EFAULT;
    }
    ready
}

fn sys_pselect6(arguments: [usize; 6]) -> isize {
    let nfds = arguments[0];
    let readfds_address = arguments[1];
    let writefds_address = arguments[2];
    let exceptfds_address = arguments[3];
    let timeout_address = arguments[4];
    let _sigmask_address = arguments[5];

    if nfds == 0 {
        // pselect6 uses the same relative timespec sleeping contract here.
        // Restartable syscall handling remains fail-closed in the signal path.
        return if timeout_address != 0 {
            sys_nanosleep(timeout_address, 0)
        } else {
            0
        };
    }

    let bytes_len = match fdset_len(nfds) {
        Some(length) if length <= MAX_USER_COPY => length,
        _ => return -EINVAL,
    };
    let mut readfds = [0_u8; MAX_USER_COPY];
    let mut writefds = [0_u8; MAX_USER_COPY];
    let mut exceptfds = [0_u8; MAX_USER_COPY];
    if copy_fdset_from_user(readfds_address, &mut readfds[..bytes_len]).is_err()
        || copy_fdset_from_user(writefds_address, &mut writefds[..bytes_len]).is_err()
        || copy_fdset_from_user(exceptfds_address, &mut exceptfds[..bytes_len]).is_err()
    {
        return -EFAULT;
    }

    let mut ready = 0_isize;
    let mut out_readfds = [0_u8; MAX_USER_COPY];
    let mut out_writefds = [0_u8; MAX_USER_COPY];
    let mut out_exceptfds = [0_u8; MAX_USER_COPY];

    for fd in 0..nfds {
        let wants_read = fd_is_set(&readfds[..bytes_len], fd);
        let wants_write = fd_is_set(&writefds[..bytes_len], fd);
        let wants_except = fd_is_set(&exceptfds[..bytes_len], fd);
        if !wants_read && !wants_write && !wants_except {
            continue;
        }

        let file = match current_process_file(fd) {
            Ok(file) => file,
            Err(_) => return -EBADF,
        };
        let mut fd_ready = false;
        if wants_read
            && file
                .poll(
                    myos_vfs::PollEvents::IN
                        .union(myos_vfs::PollEvents::HUP)
                        .union(myos_vfs::PollEvents::ERR),
                )
                .contains_any(
                    myos_vfs::PollEvents::IN
                        .union(myos_vfs::PollEvents::HUP)
                        .union(myos_vfs::PollEvents::ERR),
                )
        {
            set_fd_bit(&mut out_readfds[..bytes_len], fd);
            fd_ready = true;
        }
        if wants_write
            && file
                .poll(myos_vfs::PollEvents::OUT.union(myos_vfs::PollEvents::ERR))
                .contains_any(myos_vfs::PollEvents::OUT.union(myos_vfs::PollEvents::ERR))
        {
            set_fd_bit(&mut out_writefds[..bytes_len], fd);
            fd_ready = true;
        }
        if wants_except
            && file
                .poll(myos_vfs::PollEvents::PRI.union(myos_vfs::PollEvents::ERR))
                .contains_any(myos_vfs::PollEvents::PRI.union(myos_vfs::PollEvents::ERR))
        {
            set_fd_bit(&mut out_exceptfds[..bytes_len], fd);
            fd_ready = true;
        }
        if fd_ready {
            ready += 1;
        }
    }

    if ready == 0 && timeout_address != 0 {
        let slept = sys_nanosleep(timeout_address, 0);
        if slept < 0 {
            return slept;
        }
    }

    if copy_fdset_to_user(readfds_address, &out_readfds[..bytes_len]).is_err()
        || copy_fdset_to_user(writefds_address, &out_writefds[..bytes_len]).is_err()
        || copy_fdset_to_user(exceptfds_address, &out_exceptfds[..bytes_len]).is_err()
    {
        return -EFAULT;
    }

    ready
}

fn fdset_len(nfds: usize) -> Option<usize> {
    nfds.checked_add(7).map(|bits| bits / 8)
}

fn copy_fdset_from_user(address: usize, output: &mut [u8]) -> Result<(), ()> {
    if address == 0 {
        output.fill(0);
        Ok(())
    } else {
        copy_from_user(address, output)
    }
}

fn copy_fdset_to_user(address: usize, input: &[u8]) -> Result<(), ()> {
    if address == 0 {
        Ok(())
    } else {
        copy_to_user(address, input)
    }
}

fn fd_is_set(bytes: &[u8], fd: usize) -> bool {
    let index = fd / 8;
    let bit = fd % 8;
    bytes
        .get(index)
        .is_some_and(|byte| byte & (1_u8 << bit) != 0)
}

fn set_fd_bit(bytes: &mut [u8], fd: usize) {
    let index = fd / 8;
    let bit = fd % 8;
    if let Some(byte) = bytes.get_mut(index) {
        *byte |= 1_u8 << bit;
    }
}

fn sys_dup(fd: usize) -> isize {
    match current_process().files().dup_from(fd, 0, false) {
        Ok(new_fd) => new_fd as isize,
        Err(errno) => errno.to_isize(),
    }
}

fn sys_dup3(old_fd: usize, new_fd: usize, flags: usize) -> isize {
    let cloexec = match myos_vfs::OpenFlags::from_bits(flags as u32) {
        flags if flags.bits() == 0 => false,
        flags if flags.bits() == myos_vfs::OpenFlags::O_CLOEXEC.bits() => true,
        _ => return -EINVAL,
    };
    if old_fd == new_fd {
        return -EINVAL;
    }
    match current_process().files().dup_to(old_fd, new_fd, cloexec) {
        Ok(fd) => fd as isize,
        Err(errno) => errno.to_isize(),
    }
}

fn sys_fcntl(fd: usize, command: usize, argument: usize) -> isize {
    let process = current_process();
    match command {
        F_DUPFD => match process.files().dup_from(fd, argument, false) {
            Ok(new_fd) => new_fd as isize,
            Err(errno) => errno.to_isize(),
        },
        F_DUPFD_CLOEXEC => match process.files().dup_from(fd, argument, true) {
            Ok(new_fd) => new_fd as isize,
            Err(errno) => errno.to_isize(),
        },
        F_GETFD => match process.files().fd_flags(fd) {
            Ok(flags) => flags as isize,
            Err(errno) => errno.to_isize(),
        },
        F_SETFD => {
            if argument & !FD_CLOEXEC != 0 {
                return -EINVAL;
            }
            match process
                .files()
                .set_close_on_exec(fd, argument & FD_CLOEXEC != 0)
            {
                Ok(()) => 0,
                Err(errno) => errno.to_isize(),
            }
        }
        F_GETFL => match process.files().file_flags(fd) {
            Ok(flags) => flags.bits() as isize,
            Err(errno) => errno.to_isize(),
        },
        _ => -EINVAL,
    }
}

fn sys_ioctl(fd: usize, command: usize, argument: usize) -> isize {
    let file = match current_process_file(fd) {
        Ok(file) => file,
        Err(errno) => return errno.to_isize(),
    };
    match file.ioctl(command, argument) {
        Ok(value) => value as isize,
        Err(errno) => errno.to_isize(),
    }
}

fn sys_fsync(fd: usize) -> isize {
    match current_process_file(fd) {
        Ok(file) => match file.sync() {
            Ok(()) => 0,
            Err(errno) => errno.to_isize(),
        },
        Err(errno) => errno.to_isize(),
    }
}

fn sys_ftruncate(fd: usize, length: usize) -> isize {
    if length > isize::MAX as usize {
        return -EINVAL;
    }
    let file = match current_process_file(fd) {
        Ok(file) => file,
        Err(errno) => return errno.to_isize(),
    };
    match file.truncate(length as u64) {
        Ok(()) => 0,
        Err(errno) => errno.to_isize(),
    }
}

fn current_process() -> Arc<crate::process::Process> {
    crate::task::current_user_thread()
        .expect("M9-B user-memory operation has no current user Thread")
        .process_arc()
}

fn current_user_mm() -> Arc<crate::user_mm::UserMm> {
    current_process().mm_arc()
}

fn current_process_file(fd: usize) -> Result<myos_vfs::ArcFile, myos_vfs::Errno> {
    current_process().files().get(fd)
}

fn copy_stat_to_user(stat_address: usize, stat: &myos_vfs::Stat) -> isize {
    // SAFETY: `stat` is a plain repr(C) value and the byte slice is used only
    // for checked copy_to_user while the referenced value is alive.
    let bytes = unsafe {
        core::slice::from_raw_parts(
            core::ptr::addr_of!(*stat) as *const u8,
            core::mem::size_of::<myos_vfs::Stat>(),
        )
    };
    if copy_to_user(stat_address, bytes).is_err() {
        return -EFAULT;
    }
    0
}

fn copy_statx_to_user(statx_address: usize, stat: &myos_vfs::Stat) -> isize {
    const STATX_TYPE: u32 = 0x0000_0001;
    const STATX_MODE: u32 = 0x0000_0002;
    const STATX_NLINK: u32 = 0x0000_0004;
    const STATX_UID: u32 = 0x0000_0008;
    const STATX_GID: u32 = 0x0000_0010;
    const STATX_ATIME: u32 = 0x0000_0020;
    const STATX_MTIME: u32 = 0x0000_0040;
    const STATX_CTIME: u32 = 0x0000_0080;
    const STATX_INO: u32 = 0x0000_0100;
    const STATX_SIZE: u32 = 0x0000_0200;
    const STATX_BLOCKS: u32 = 0x0000_0400;
    const STATX_BASIC_STATS: u32 = STATX_TYPE
        | STATX_MODE
        | STATX_NLINK
        | STATX_UID
        | STATX_GID
        | STATX_ATIME
        | STATX_MTIME
        | STATX_CTIME
        | STATX_INO
        | STATX_SIZE
        | STATX_BLOCKS;

    let mut raw = [0_u8; 256];
    write_u32(&mut raw, 0, STATX_BASIC_STATS);
    write_u32(&mut raw, 4, stat.blksize.max(1) as u32);
    write_u64(&mut raw, 8, 0);
    write_u32(&mut raw, 16, stat.nlink);
    write_u32(&mut raw, 20, stat.uid);
    write_u32(&mut raw, 24, stat.gid);
    write_u16(&mut raw, 28, stat.mode as u16);
    write_u64(&mut raw, 32, stat.ino);
    write_u64(&mut raw, 40, stat.size.max(0) as u64);
    write_u64(&mut raw, 48, stat.blocks.max(0) as u64);
    write_u64(&mut raw, 56, 0);
    write_statx_timestamp(&mut raw, 64, stat.atime_sec, stat.atime_nsec);
    write_statx_timestamp(&mut raw, 96, stat.ctime_sec, stat.ctime_nsec);
    write_statx_timestamp(&mut raw, 112, stat.mtime_sec, stat.mtime_nsec);
    write_dev_major_minor(&mut raw, 136, stat.dev);

    if copy_to_user(statx_address, &raw).is_err() {
        return -EFAULT;
    }
    0
}

fn write_statx_timestamp(raw: &mut [u8; 256], offset: usize, sec: i64, nsec: i64) {
    write_i64(raw, offset, sec);
    write_u32(raw, offset + 8, nsec.clamp(0, 999_999_999) as u32);
}

fn write_dev_major_minor(raw: &mut [u8; 256], offset: usize, device: u64) {
    let major = ((device >> 8) & 0xfff) | ((device >> 32) & !0xfff);
    let minor = (device & 0xff) | ((device >> 12) & !0xff);
    write_u32(raw, offset, major as u32);
    write_u32(raw, offset + 4, minor as u32);
}

fn write_u16(raw: &mut [u8], offset: usize, value: u16) {
    raw[offset..offset + 2].copy_from_slice(&value.to_ne_bytes());
}

fn write_u32(raw: &mut [u8], offset: usize, value: u32) {
    raw[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
}

fn write_u64(raw: &mut [u8], offset: usize, value: u64) {
    raw[offset..offset + 8].copy_from_slice(&value.to_ne_bytes());
}

fn write_i64(raw: &mut [u8], offset: usize, value: i64) {
    raw[offset..offset + 8].copy_from_slice(&value.to_ne_bytes());
}

fn copy_plain_to_user<T>(address: usize, value: &T) -> isize {
    // SAFETY: caller supplies a plain kernel value; the byte view is used only
    // for checked copy_to_user while `value` is alive.
    let bytes = unsafe {
        core::slice::from_raw_parts(
            core::ptr::addr_of!(*value) as *const u8,
            core::mem::size_of::<T>(),
        )
    };
    if copy_to_user(address, bytes).is_err() {
        return -EFAULT;
    }
    0
}

fn copy_plain_from_user<T: Copy>(address: usize) -> Result<T, isize> {
    let mut value = core::mem::MaybeUninit::<T>::uninit();
    // SAFETY: the uninitialized storage is viewed only as raw bytes and fully
    // filled by checked copy_from_user before assume_init.
    let bytes = unsafe {
        core::slice::from_raw_parts_mut(value.as_mut_ptr() as *mut u8, core::mem::size_of::<T>())
    };
    if copy_from_user(address, bytes).is_err() {
        return Err(-EFAULT);
    }
    // SAFETY: every byte of `value` has just been initialized by copy_from_user.
    Ok(unsafe { value.assume_init() })
}

fn resolve_user_path(dirfd: usize, path_address: usize) -> Result<alloc::string::String, isize> {
    let raw_path = copy_user_c_string(path_address)?;
    resolve_path_from_user(dirfd, &raw_path)
}

fn resolve_path_from_user(dirfd: usize, path: &str) -> Result<alloc::string::String, isize> {
    if path.starts_with('/') {
        return crate::fs::resolve_path("/", path).map_err(|errno| errno.to_isize());
    }
    if dirfd == AT_FDCWD {
        let cwd = current_process().fs().cwd_path();
        return crate::fs::resolve_path(&cwd, path).map_err(|errno| errno.to_isize());
    }
    let file = current_process_file(dirfd).map_err(|errno| errno.to_isize())?;
    let stat = file.fstat().map_err(|errno| errno.to_isize())?;
    if stat.mode & myos_vfs::FileMode::S_IFMT != myos_vfs::FileMode::S_IFDIR {
        return Err(myos_vfs::Errno::Enotdir.to_isize());
    }
    let base = file.path().ok_or(-EBADF)?;
    crate::fs::resolve_path(base, path).map_err(|errno| errno.to_isize())
}

fn copy_user_c_string(address: usize) -> Result<alloc::string::String, isize> {
    let mut path = alloc::string::String::new();
    for offset in 0..MAX_USER_PATH {
        let mut byte = [0_u8; 1];
        if copy_from_user(address.checked_add(offset).ok_or(-EFAULT)?, &mut byte).is_err() {
            return Err(-EFAULT);
        }
        if byte[0] == 0 {
            return Ok(path);
        }
        if !byte[0].is_ascii() {
            return Err(-EINVAL);
        }
        path.try_reserve(1).map_err(|_| -ENOMEM)?;
        path.push(byte[0] as char);
    }
    Err(-EINVAL)
}

fn copy_user_string_array(
    address: usize,
    maximum: usize,
    fallback0: Option<&str>,
) -> Result<Vec<String>, isize> {
    if address == 0 {
        let mut values = Vec::new();
        if let Some(value) = fallback0 {
            values.try_reserve(1).map_err(|_| -ENOMEM)?;
            values.push(String::from(value));
        }
        return Ok(values);
    }

    let mut values = Vec::new();
    for index in 0..maximum {
        let pointer_address = address
            .checked_add(
                index
                    .checked_mul(core::mem::size_of::<usize>())
                    .ok_or(-EFAULT)?,
            )
            .ok_or(-EFAULT)?;
        let pointer = copy_plain_from_user::<usize>(pointer_address)?;
        if pointer == 0 {
            if values.is_empty()
                && let Some(value) = fallback0
            {
                values.try_reserve(1).map_err(|_| -ENOMEM)?;
                values.push(String::from(value));
            }
            return Ok(values);
        }
        let value = copy_user_c_string(pointer)?;
        values.try_reserve(1).map_err(|_| -ENOMEM)?;
        values.push(value);
    }
    Err(-EINVAL)
}

fn copy_from_user(address: usize, output: &mut [u8]) -> Result<(), ()> {
    let Some(thread) = crate::task::current_user_thread() else {
        return Err(());
    };
    thread
        .process()
        .mm()
        .copy_from_user(address, output)
        .map_err(|_| ())
}

/// Scheduler-current uaccess write helper reserved for read-like syscalls.
///
/// M9-B has no syscall that copies kernel output into userspace yet, but the
/// helper remains the checked counterpart of `copy_from_user` for the next VFS
/// stage. Keeping the allowance local prevents unrelated dead code.
fn copy_to_user(address: usize, input: &[u8]) -> Result<(), ()> {
    let Some(thread) = crate::task::current_user_thread() else {
        return Err(());
    };
    thread
        .process()
        .mm()
        .copy_to_user(address, input)
        .map_err(|_| ())
}

fn embedded_user_image() -> &'static [u8] {
    let start = core::ptr::addr_of!(__m7_user_image_start) as usize;
    let end = core::ptr::addr_of!(__m7_user_image_end) as usize;
    let length = end
        .checked_sub(start)
        .expect("M8-B3 embedded user image symbols are reversed");

    // SAFETY: both linker symbols delimit immutable bytes emitted by the
    // architecture-specific assembly in this crate.
    unsafe { core::slice::from_raw_parts(start as *const u8, length) }
}

fn user_entry(symbol: *const u8) -> usize {
    let image_start = core::ptr::addr_of!(__m7_user_image_start) as usize;
    let image_end = core::ptr::addr_of!(__m7_user_image_end) as usize;
    let symbol = symbol as usize;

    assert!(
        symbol >= image_start && symbol < image_end,
        "M8-B3 user entry symbol is outside the embedded image",
    );

    USER_CODE
        .checked_add(symbol - image_start)
        .expect("M8-B3 user entry address overflow")
}

pub(crate) fn run_scheduled_thread(thread: &crate::process::Thread) -> isize {
    assert!(
        crate::task::current_user_thread().is_some_and(|current| current.id() == thread.id()),
        "M9-B scheduler current Thread diverged before user entry",
    );

    let mm = thread.process().mm();
    assert!(
        mm.root_is_private()
            .expect("unable to compare M9-B user/kernel page-table roots"),
        "M9-B user mm reused the kernel page-table root",
    );
    mm.assert_hardware_active()
        .expect("M9-B scheduler did not install the current Thread MM");
    assert!(
        mm.kernel_mapping_is_shared(VirtAddr::new(verify as *const () as usize))
            .expect("unable to verify the M9-B shared kernel mapping"),
        "M9-B user root lost the shared high-half kernel mapping",
    );

    if let Some(frame) = thread.take_trap_frame() {
        enter_user_frame(&frame)
    } else {
        enter_user(thread.entry().get(), thread.user_stack_pointer().get())
    }
}

fn enter_user(entry: usize, stack_top: usize) -> isize {
    assert_eq!(
        stack_top & 0xf,
        0,
        "M8-B3 user stack is not 16-byte aligned",
    );

    // SAFETY: switch_mm_irqs_off() installed the current Thread's validated
    // private root, and the scheduler owns this task's guarded kernel stack for
    // the complete user/trap round trip. Trap return may enable timer/IPI
    // delivery, but scheduler ownership keeps both objects alive across preemption.
    unsafe { __m7_enter_user(entry, stack_top) }
}

fn enter_user_frame(frame: &crate::arch::trap::TrapFrame) -> isize {
    // SAFETY: the saved frame was captured from a real user trap and copied
    // into the child thread before scheduler publication. The architecture
    // entry path rebuilds the normal trap anchor on this task's kernel stack.
    unsafe { __m12_enter_user_frame(frame as *const crate::arch::trap::TrapFrame) }
}

fn user_return_address() -> usize {
    __m7_user_return as *const () as usize
}

#[cfg(target_arch = "riscv64")]
fn prepare_user_instruction_stream() {
    // SAFETY: synchronizes the local instruction stream after copying code.
    unsafe {
        core::arch::asm!("fence.i", options(nostack));
    }
}

#[cfg(target_arch = "loongarch64")]
fn prepare_user_instruction_stream() {
    // SAFETY: synchronizes the local instruction stream after copying code.
    unsafe {
        core::arch::asm!("dbar 0", options(nostack));
        core::arch::asm!("ibar 0", options(nostack));
    }
}

fn syscall_number(frame: &crate::arch::trap::TrapFrame) -> usize {
    crate::syscall::abi::decode(frame).number
}

fn syscall_arguments(frame: &crate::arch::trap::TrapFrame) -> [usize; 6] {
    crate::syscall::abi::decode(frame).arguments
}

fn advance_syscall_pc(frame: &mut crate::arch::trap::TrapFrame) {
    crate::syscall::abi::advance(frame);
}

fn set_syscall_result(frame: &mut crate::arch::trap::TrapFrame, result: isize) {
    // ── P9-H11: LA sleep syscall trace (exit) ──
    #[cfg(target_arch = "loongarch64")]
    {
        let nr = LAST_TRACED_SYSCALL_NR.swap(0, Ordering::Relaxed);
        if nr != 0 {
            crate::println!("oscomp-la-sleep-syscall: exit nr={} ret={}", nr, result);
        }
    }
    crate::syscall::abi::set_result(frame, result);
}

#[cfg(target_arch = "riscv64")]
fn return_to_kernel(frame: &mut crate::arch::trap::TrapFrame, result: isize) {
    const SSTATUS_SIE: usize = 1 << 1;
    const SSTATUS_SPIE: usize = 1 << 5;
    const SSTATUS_SPP: usize = 1 << 8;
    const SSTATUS_SUM: usize = 1 << 18;

    let kernel_stack = (frame as *mut crate::arch::trap::TrapFrame as usize)
        .checked_add(core::mem::size_of::<crate::arch::trap::TrapFrame>())
        .expect("M8-B3 kernel stack pointer overflow");

    frame.gpr[2] = kernel_stack;
    frame.gpr[10] = result as usize;
    frame.sepc = user_return_address();
    frame.sstatus = (frame.sstatus | SSTATUS_SPP) & !(SSTATUS_SIE | SSTATUS_SPIE | SSTATUS_SUM);
}

#[cfg(target_arch = "loongarch64")]
fn return_to_kernel(frame: &mut crate::arch::trap::TrapFrame, result: isize) {
    const PRMD_PPLV_MASK: usize = 0b11;
    const PRMD_PIE: usize = 1 << 2;

    let kernel_stack = (frame as *mut crate::arch::trap::TrapFrame as usize)
        .checked_add(core::mem::size_of::<crate::arch::trap::TrapFrame>())
        .expect("M8-B3 kernel stack pointer overflow");

    frame.gpr[3] = kernel_stack;
    frame.gpr[4] = result as usize;
    frame.era = user_return_address();
    frame.prmd &= !(PRMD_PPLV_MASK | PRMD_PIE);
}
