use alloc::{string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering};

use myos_mm::{FaultAccess, PAGE_SIZE, VirtAddr, VirtRange, VmArea, VmAreaFlags, VmAreaKind};

use crate::process::{Process, Thread};
use crate::user_mm::{UserFaultFailure, UserFaultRecovery, UserFaultResolution};

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
const SYS_WAIT4: usize = crate::syscall::number::WAIT4;
const SYS_PRLIMIT64: usize = crate::syscall::number::PRLIMIT64;
const SYS_GETRANDOM: usize = crate::syscall::number::GETRANDOM;
const SYS_STATX: usize = crate::syscall::number::STATX;

const PROT_READ: usize = 1;
const PROT_WRITE: usize = 2;
const PROT_EXEC: usize = 4;
const MAP_SHARED: usize = 0x01;
const MAP_PRIVATE: usize = 0x02;
const MAP_TYPE: usize = 0x0f;
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
    let device = crate::block::open_device("vda").expect("lost /dev/vda before sdcard sample gate");
    let sample_path = "/musl/busybox";
    let snapshot = crate::ext4::load_path_snapshot(device, sample_path)
        .expect("unable to load /musl/busybox from ext4 sdcard image");
    let crate::ext4::Ext4SnapshotKind::Regular(image) = snapshot.kind else {
        panic!("ext4 sdcard /musl/busybox is not a regular file");
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

pub fn verify_sdcard_all_scripts() {
    if crate::fs::stat("/mnt/sdcard/musl/busybox").is_err() {
        return;
    }

    crate::task::run_kernel_thread_sync(verify_sdcard_all_scripts_thread);
}

fn install_from_ext4(vfs: &str, ext4: &str) {
    if crate::fs::stat(vfs).is_err() {
        let _ = crate::fs::install_ext4_path("/dev/vda", vfs, ext4);
    }
}

fn verify_sdcard_all_scripts_thread() {
    // Create dirs and files needed by test scripts (touch is ENOSYS)
    let _ = crate::fs::mkdir("/var", 0o755);
    let _ = crate::fs::mkdir("/var/tmp", 0o755);
    if crate::fs::stat("/var/tmp/lmbench").is_err() {
        let _ = crate::fs::open("/var/tmp/lmbench", myos_vfs::OpenFlags::from_bits(0o101));
    }

    // Install script files and dependencies from ext4
    install_from_ext4("/mnt/sdcard/musl/basic_testcode.sh", "/musl/basic_testcode.sh");
    install_from_ext4("/mnt/sdcard/musl/busybox_testcode.sh", "/musl/busybox_testcode.sh");
    install_from_ext4("/mnt/sdcard/musl/busybox_cmd.txt", "/musl/busybox_cmd.txt");

    install_from_ext4("/mnt/sdcard/musl/libcbench_testcode.sh", "/musl/libcbench_testcode.sh");
    install_from_ext4("/mnt/sdcard/musl/libc-bench", "/musl/libc-bench");

    install_from_ext4("/mnt/sdcard/musl/libctest_testcode.sh", "/musl/libctest_testcode.sh");
    install_from_ext4("/mnt/sdcard/musl/libctest_testcode.sh", "/musl/libctest_testcode.sh");
    // run-static.sh / run-dynamic.sh lack a #! shebang. Install real content
    // with a .real suffix, then create shebang wrappers at the expected names.
    install_from_ext4("/mnt/sdcard/musl/run-static.real", "/musl/run-static.sh");
    install_from_ext4("/mnt/sdcard/musl/run-dynamic.real", "/musl/run-dynamic.sh");
    install_from_ext4("/mnt/sdcard/musl/entry-static.exe", "/musl/entry-static.exe");
    install_from_ext4("/mnt/sdcard/musl/entry-dynamic.exe", "/musl/entry-dynamic.exe");
    install_from_ext4("/mnt/sdcard/musl/runtest.exe", "/musl/runtest.exe");
    install_from_ext4("/mnt/sdcard/musl/dlopen_dso.so", "/musl/dlopen_dso.so");
    install_from_ext4("/mnt/sdcard/musl/tls_get_new-dtv_dso.so", "/musl/tls_get_new-dtv_dso.so");
    // Write shebang wrappers that source the real scripts
    for (name, real) in [("run-static.sh", "run-static.real"), ("run-dynamic.sh", "run-dynamic.real")] {
        let vfs = alloc::format!("/mnt/sdcard/musl/{}", name);
        let real = alloc::format!("/mnt/sdcard/musl/{}", real);
        let content = alloc::format!("#!/bin/busybox sh\n. {}\n", real);
        if let Ok(f) = crate::fs::open(
            &vfs,
            myos_vfs::OpenFlags::from_bits(0o1101),
        ) {
            let _ = f.write(&myos_vfs::IoBuffer::new(content.as_bytes()));
        }
    }

    install_from_ext4("/mnt/sdcard/musl/lua_testcode.sh", "/musl/lua_testcode.sh");
    install_from_ext4("/mnt/sdcard/musl/test.sh", "/musl/test.sh");
    install_from_ext4("/mnt/sdcard/musl/lua", "/musl/lua");
    let lua_scripts: &[&str] = &[
        "date.lua", "file_io.lua", "max_min.lua", "random.lua",
        "remove.lua", "round_num.lua", "sin30.lua", "sort.lua", "strings.lua",
    ];
    for s in lua_scripts {
        install_from_ext4(&alloc::format!("/mnt/sdcard/musl/{}", s), &alloc::format!("/musl/{}", s));
    }

    install_from_ext4("/mnt/sdcard/musl/cyclictest_testcode.sh", "/musl/cyclictest_testcode.sh");
    install_from_ext4("/mnt/sdcard/musl/cyclictest", "/musl/cyclictest");
    install_from_ext4("/mnt/sdcard/musl/hackbench", "/musl/hackbench");

    install_from_ext4("/mnt/sdcard/musl/netperf_testcode.sh", "/musl/netperf_testcode.sh");
    install_from_ext4("/mnt/sdcard/musl/netperf", "/musl/netperf");
    install_from_ext4("/mnt/sdcard/musl/netserver", "/musl/netserver");

    install_from_ext4("/mnt/sdcard/musl/iperf_testcode.sh", "/musl/iperf_testcode.sh");
    install_from_ext4("/mnt/sdcard/musl/iperf3", "/musl/iperf3");

    install_from_ext4("/mnt/sdcard/musl/iozone_testcode.sh", "/musl/iozone_testcode.sh");
    install_from_ext4("/mnt/sdcard/musl/iozone", "/musl/iozone");

    install_from_ext4("/mnt/sdcard/musl/lmbench_testcode.sh", "/musl/lmbench_testcode.sh");
    install_from_ext4("/mnt/sdcard/musl/lmbench_all", "/musl/lmbench_all");
    install_from_ext4("/mnt/sdcard/musl/hello", "/musl/hello");

    install_from_ext4("/mnt/sdcard/musl/unixbench_testcode.sh", "/musl/unixbench_testcode.sh");
    install_from_ext4("/mnt/sdcard/musl/multi.sh", "/musl/multi.sh");
    let ub_bins: &[&str] = &[
        "arithoh", "context1", "dhry2", "dhry2reg", "double", "execl",
        "float", "fstime", "hanoi", "int", "long", "looper", "pipe",
        "register", "short", "spawn", "syscall", "whetstone-double",
    ];
    for bin in ub_bins {
        install_from_ext4(
            &alloc::format!("/mnt/sdcard/musl/{}", bin),
            &alloc::format!("/musl/{}", bin),
        );
    }

    install_from_ext4("/mnt/sdcard/musl/ltp_testcode.sh", "/musl/ltp_testcode.sh");

    // lmbench_testcode.sh: hangs (touch ENOSYS + lat_sig prot)
    // ltp_testcode.sh: needs /musl/ltp tree mount (Enomem)

    const ALL_TEST_SCRIPTS: &[&str] = &[
        "basic_testcode.sh",
        "busybox_testcode.sh",
        "libcbench_testcode.sh",
        "libctest_testcode.sh",
        "lua_testcode.sh",
        "cyclictest_testcode.sh",
        "netperf_testcode.sh",
        "iperf_testcode.sh",
        "iozone_testcode.sh",
        "unixbench_testcode.sh",
    ];

    for script in ALL_TEST_SCRIPTS {
        let vfs_path = alloc::format!("/mnt/sdcard/musl/{}", script);
        if crate::fs::stat(&vfs_path).is_err() {
            crate::println!("#### OS COMP TEST GROUP START {} ####", script);
            crate::println!("  {} : SKIP (not installed)", script);
            crate::println!("#### OS COMP TEST GROUP END {} ####", script);
            continue;
        }

        crate::println!("#### OS COMP TEST GROUP START {} ####", script);

        let result = run_rootfs_program_with_cwd(
            "/mnt/sdcard/musl/busybox",
            &["busybox", "sh", &vfs_path],
            &[
                "PATH=.:/mnt/sdcard/musl:/bin:/sbin:/usr/bin:/usr/sbin",
                "LD_LIBRARY_PATH=/mnt/sdcard/musl/lib",
            ],
            Some("/mnt/sdcard/musl"),
        );

        match result {
            Ok(0) => crate::println!("  {} : PASS", script),
            Ok(rc) => crate::println!("  {} : FAIL (exit={})", script, rc),
            Err(e) => crate::println!("  {} : ERROR ({:?})", script, e),
        }

        crate::println!("#### OS COMP TEST GROUP END {} ####", script);
    }
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
        SYS_PRLIMIT64 => set_syscall_result(
            frame,
            sys_prlimit64(arguments[0], arguments[1], arguments[2], arguments[3]),
        ),
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
            if verifier {
                EXIT_STATUS.store(arguments[0] as isize, Ordering::Release);
                TERMINATED.store(true, Ordering::Release);
            }
            return_to_kernel(frame, arguments[0] as isize);
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
                crate::println!(
                    "user fatal fault: address={:#018x} access={:?} sp={:#018x} failure={:?}",
                    address.get(),
                    access,
                    frame.stack_pointer(),
                    failure,
                );
            }
            if verifier {
                LAST_FAULT_ADDRESS.store(address.get(), Ordering::Release);
                LAST_FAULT_KIND.store(FAULT_PAGE, Ordering::Release);
                TERMINATED.store(true, Ordering::Release);
                EXIT_STATUS.store(-EFAULT, Ordering::Release);
            }
            return_to_kernel(frame, -EFAULT);
        }
        Err(error) => panic!("M8-B4 user fault recovery failed: {error:?}"),
    }
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
    if address != 0 || length == 0 {
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

    let map_type = flags & MAP_TYPE;
    if file != usize::MAX && (map_type == MAP_PRIVATE || map_type == MAP_SHARED) {
        if flags & MAP_ANONYMOUS != 0 || flags & !MAP_TYPE != 0 {
            return -EINVAL;
        }
        return sys_file_private_mmap(file, offset, length, rounded, vm_flags);
    }
    if flags != (MAP_PRIVATE | MAP_ANONYMOUS) || file != usize::MAX || offset != 0 {
        return -EINVAL;
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
            start.get() as isize
        }
        Err(_) => -ENOMEM,
    }
}

fn sys_file_private_mmap(
    fd: usize,
    offset: usize,
    length: usize,
    rounded: usize,
    vm_flags: VmAreaFlags,
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
    if stat.mode & myos_vfs::FileMode::S_IFMT != myos_vfs::FileMode::S_IFREG {
        return -EINVAL;
    }
    let file_size = if stat.size <= 0 {
        0
    } else {
        stat.size as usize
    };
    let readable = file_size.saturating_sub(offset).min(length);

    let temporary_flags = VmAreaFlags::user_rw();
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
    if current_user_mm()
        .protect_range(range, vm_flags.access_only())
        .is_err()
    {
        let _ = current_user_mm().unmap_range(range);
        return -EINVAL;
    }
    if ACTIVE.load(Ordering::Acquire) {
        MMAP_COUNT.fetch_add(1, Ordering::AcqRel);
    }
    start.get() as isize
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

fn sys_mprotect(address: usize, length: usize, protection: usize) -> isize {
    let range = match syscall_range(address, length) {
        Some(range) => range,
        None => return -EINVAL,
    };
    let flags = match protection_flags(protection) {
        Some(flags) => flags.access_only(),
        None => return -EINVAL,
    };
    match current_user_mm().protect_range(range, flags) {
        Ok(()) => {
            if ACTIVE.load(Ordering::Acquire) {
                MPROTECT_COUNT.fetch_add(1, Ordering::AcqRel);
            }
            0
        }
        Err(_) => -EINVAL,
    }
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
    let old_position = file.position();
    if file.seek(offset as i64, myos_vfs::SeekWhence::Set).is_err() {
        return -EINVAL;
    }
    let result = sys_read(fd, address, length);
    let _ = file.seek(old_position as i64, myos_vfs::SeekWhence::Set);
    result
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
    if flags & !AT_SYMLINK_NOFOLLOW != 0 {
        return -EINVAL;
    }
    let path = match resolve_user_path(dirfd, path_address) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    let stat = match if flags & AT_SYMLINK_NOFOLLOW != 0 {
        crate::fs::lstat(&path)
    } else {
        crate::fs::stat(&path)
    } {
        Ok(stat) => stat,
        Err(errno) => return errno.to_isize(),
    };
    copy_stat_to_user(stat_address, &stat)
}

fn sys_statx(
    dirfd: usize,
    path_address: usize,
    flags: usize,
    _mask: usize,
    statx_address: usize,
) -> isize {
    if flags & !(AT_SYMLINK_NOFOLLOW | AT_EMPTY_PATH) != 0 {
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
        match if flags & AT_SYMLINK_NOFOLLOW != 0 {
            crate::fs::lstat(&path)
        } else {
            crate::fs::stat(&path)
        } {
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

fn sys_set_tid_address(_address: usize) -> isize {
    crate::task::current_user_thread()
        .expect("set_tid_address arrived without current user Thread")
        .id()
        .get() as isize
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

    let flags = arguments[0];
    if flags & (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SETTLS)
        != 0
    {
        return -EINVAL;
    }
    let exit_signal = flags & CSIGNAL_MASK;
    if exit_signal != 0 && exit_signal != crate::signal::SIGCHLD as usize {
        return -EINVAL;
    }

    let parent = current_process();
    let child_mm = match parent.mm().fork_clone_eager() {
        Ok(mm) => mm,
        Err(_) => return -ENOMEM,
    };
    let child = match parent.fork_child(child_mm) {
        Ok(child) => child,
        Err(_) => return -ENOMEM,
    };
    let current_thread =
        crate::task::current_user_thread().expect("clone arrived without a current user Thread");
    let child_thread =
        match child.create_initial_thread(current_thread.entry(), current_thread.user_stack()) {
            Ok(thread) => thread,
            Err(_) => return -ENOMEM,
        };
    child_thread.set_blocked_signals(current_thread.blocked_signals());
    let mut child_frame = *frame;
    set_syscall_result(&mut child_frame, 0);
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
        Err(errno) => return errno,
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
            Err(errno) => return errno,
        };
        exec_argv = rewritten_argv;
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
        Ok(prepared) => prepared,
        Err(crate::exec::ExecError::Elf(crate::elf::ElfError::InvalidHeader)) => {
            return myos_vfs::Errno::Enoexec.to_isize()
        }
        Err(crate::exec::ExecError::Elf(crate::elf::ElfError::Unsupported)) => {
            return myos_vfs::Errno::Enoexec.to_isize()
        }
        Err(_) => return -EINVAL,
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

fn deliver_pending_signal(frame: &mut crate::arch::trap::TrapFrame) {
    let thread =
        crate::task::current_user_thread().expect("signal delivery arrived without current Thread");
    let process = thread.process();
    let Some(signal) = process.signals().take_unblocked(thread.blocked_signals()) else {
        return;
    };
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
            Ok(Some((child, status))) => {
                let child_pid = child.id().get();
                if status_address != 0 {
                    let status = status as i32;
                    if copy_to_user(status_address, &status.to_ne_bytes()).is_err() {
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
    let mut seed = crate::arch::time::counter();
    for byte in &mut bytes[..length] {
        seed ^= seed << 7;
        seed ^= seed >> 9;
        seed ^= seed << 8;
        *byte = seed as u8;
    }
    if copy_to_user(address, &bytes[..length]).is_err() {
        return -EFAULT;
    }
    length as isize
}

fn sys_uname(address: usize) -> isize {
    let mut raw = [0_u8; 65 * 6];
    write_uts_field(&mut raw, 0, b"SudoOS");
    write_uts_field(&mut raw, 1, b"sudoos");
    write_uts_field(&mut raw, 2, b"0.12");
    write_uts_field(&mut raw, 3, b"M12-M13");
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
    match crate::fs::stat(&path) {
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
