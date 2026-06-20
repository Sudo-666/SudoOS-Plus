use alloc::{sync::Arc, vec::Vec};
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
const SYS_RT_SIGRETURN: usize = crate::syscall::number::RT_SIGRETURN;
const SYS_UNAME: usize = crate::syscall::number::UNAME;
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

const PROT_READ: usize = 1;
const PROT_WRITE: usize = 2;
const PROT_EXEC: usize = 4;
const MAP_PRIVATE: usize = 0x02;
const MAP_ANONYMOUS: usize = 0x20;
const VFS_PROBE_DATA: &[u8] = b"/m11-user\0......................uvfs";
const M12_PROBE_DATA: &[u8] = b"pipe";

const EBADF: isize = crate::syscall::errno::EBADF;
const ECHILD: isize = crate::syscall::errno::ECHILD;
const ENOMEM: isize = crate::syscall::errno::ENOMEM;
const EFAULT: isize = crate::syscall::errno::EFAULT;
const EINVAL: isize = crate::syscall::errno::EINVAL;
const ENOSYS: isize = crate::syscall::errno::ENOSYS;
const ERANGE: isize = 34;

const MAX_USER_COPY: usize = 256;
const MAX_USER_PATH: usize = 256;
const USER_MESSAGE: &[u8] = b"hello user\n";
const AT_FDCWD: usize = usize::MAX - 99;
const AT_REMOVEDIR: usize = 0x200;
const AT_SYMLINK_NOFOLLOW: usize = 0x100;
const AT_SYMLINK_FOLLOW: usize = 0x400;
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
                argv0: "/init",
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
        ACTIVE.load(Ordering::Acquire),
        "user syscall arrived without an active M8-B3 session",
    );
    assert!(
        frame.previous_mode_was_user(),
        "syscall trap did not originate in user mode",
    );

    SYSCALL_COUNT.fetch_add(1, Ordering::AcqRel);
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
        SYS_READLINKAT => set_syscall_result(
            frame,
            sys_readlinkat(arguments[0], arguments[1], arguments[2], arguments[3]),
        ),
        SYS_PPOLL => set_syscall_result(frame, sys_ppoll(arguments[0], arguments[1], arguments[2])),
        SYS_PSELECT6 => set_syscall_result(frame, sys_pselect6(arguments[0], arguments[4])),
        SYS_NEWFSTATAT => set_syscall_result(
            frame,
            sys_newfstatat(arguments[0], arguments[1], arguments[2], arguments[3]),
        ),
        SYS_FSTAT => set_syscall_result(frame, sys_fstat(arguments[0], arguments[1])),
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
            assert!(
                thread.schedule_count() > schedules_before,
                "M9-B sched_yield returned without switching to a runnable peer",
            );
            SCHED_YIELD_SWITCH_COUNT.fetch_add(1, Ordering::AcqRel);
        }
        SYS_EXIT | SYS_EXIT_GROUP => {
            EXIT_STATUS.store(arguments[0] as isize, Ordering::Release);
            TERMINATED.store(true, Ordering::Release);
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
        ACTIVE.load(Ordering::Acquire),
        "user fault arrived without an active M8-B4 session",
    );
    assert!(
        frame.previous_mode_was_user(),
        "M8-B4 user fault handler received a kernel fault",
    );

    FAULT_COUNT.fetch_add(1, Ordering::AcqRel);
    let user_sp = VirtAddr::new(frame.stack_pointer());
    match current_user_mm().resolve_user_fault(address, access, user_sp) {
        Ok(UserFaultResolution::Recovered(recovery)) => {
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
        Ok(UserFaultResolution::Fatal(failure)) => {
            assert!(
                !matches!(failure, UserFaultFailure::KernelBug),
                "M8-B4 fault planner classified a user trap as a kernel bug",
            );
            LAST_FAULT_ADDRESS.store(address.get(), Ordering::Release);
            LAST_FAULT_KIND.store(FAULT_PAGE, Ordering::Release);
            TERMINATED.store(true, Ordering::Release);
            EXIT_STATUS.store(-EFAULT, Ordering::Release);
            return_to_kernel(frame, -EFAULT);
        }
        Err(error) => panic!("M8-B4 user fault recovery failed: {error:?}"),
    }
}

pub fn handle_exception(frame: &mut crate::arch::trap::TrapFrame, _code: usize) {
    assert!(
        ACTIVE.load(Ordering::Acquire),
        "user exception arrived without an active M8-B3 session",
    );
    assert!(
        frame.previous_mode_was_user(),
        "M8-B3 user exception handler received a kernel exception",
    );

    LAST_FAULT_ADDRESS.store(0, Ordering::Release);
    LAST_FAULT_KIND.store(FAULT_EXCEPTION, Ordering::Release);
    FAULT_COUNT.fetch_add(1, Ordering::AcqRel);
    TERMINATED.store(true, Ordering::Release);
    EXIT_STATUS.store(-EFAULT, Ordering::Release);
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
            BRK_COUNT.fetch_add(1, Ordering::AcqRel);
            new_break.get() as isize
        }
        Err(_) => current.get() as isize,
    }
}

fn sys_mmap(arguments: [usize; 6]) -> isize {
    let [address, length, protection, flags, file, offset] = arguments;
    if address != 0
        || length == 0
        || flags != (MAP_PRIVATE | MAP_ANONYMOUS)
        || file != usize::MAX
        || offset != 0
    {
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
    match current_user_mm().map_anonymous(
        VirtRange::from_bounds(USER_MMAP_START, USER_MMAP_END),
        rounded,
        vm_flags,
    ) {
        Ok(start) => {
            MMAP_COUNT.fetch_add(1, Ordering::AcqRel);
            start.get() as isize
        }
        Err(_) => -ENOMEM,
    }
}

fn sys_munmap(address: usize, length: usize) -> isize {
    let range = match syscall_range(address, length) {
        Some(range) => range,
        None => return -EINVAL,
    };
    match current_user_mm().unmap_range(range) {
        Ok(()) => {
            MUNMAP_COUNT.fetch_add(1, Ordering::AcqRel);
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
            MPROTECT_COUNT.fetch_add(1, Ordering::AcqRel);
            0
        }
        Err(_) => -EINVAL,
    }
}

fn syscall_range(address: usize, length: usize) -> Option<VirtRange> {
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
    if length > MAX_USER_COPY {
        return -EINVAL;
    }

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
            WRITE_COUNT.fetch_add(1, Ordering::AcqRel);
            written as isize
        }
        Err(errno) => errno.to_isize(),
    }
}

fn sys_read(fd: usize, address: usize, length: usize) -> isize {
    if length > MAX_USER_COPY {
        return -EINVAL;
    }
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
    let path = match copy_user_c_string(arguments[0]) {
        Ok(path) => path,
        Err(errno) => return errno,
    };
    let image = match load_exec_image(&path) {
        Ok(image) => image,
        Err(errno) => return errno,
    };
    let extra_areas = [VmArea::new(
        VirtRange::from_bounds(USER_DEMAND, USER_DEMAND + PAGE_SIZE),
        VmAreaFlags::user_rw(),
        VmAreaKind::Anonymous,
    )];
    let prepared = match crate::exec::prepare_elf(
        &image,
        crate::exec::ExecConfig {
            argv0: &path,
            stack: VirtRange::from_bounds(USER_STACK, USER_STACK_TOP),
            heap_start: VirtAddr::new(USER_HEAP_START),
            heap_limit: VirtAddr::new(USER_HEAP_LIMIT),
            extra_areas: &extra_areas,
        },
    ) {
        Ok(prepared) => prepared,
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

fn load_exec_image(path: &str) -> Result<Vec<u8>, isize> {
    const MAX_EXEC_IMAGE: usize = 64 * 1024;
    if path == "/init" {
        let entry = VirtAddr::new(user_entry(core::ptr::addr_of!(__m12_exec_success)));
        return crate::elf::build_static_exec(
            entry,
            embedded_user_image(),
            VirtAddr::new(USER_DATA),
        )
        .map_err(|_| -EINVAL);
    }

    let file = crate::fs::open(path, myos_vfs::OpenFlags::O_RDONLY).map_err(|e| e.to_isize())?;
    let mut image = Vec::new();
    image.try_reserve(MAX_EXEC_IMAGE).map_err(|_| -ENOMEM)?;
    image.resize(MAX_EXEC_IMAGE, 0);
    let mut output = myos_vfs::MutableIoBuffer::new(&mut image);
    let read = file.read(&mut output).map_err(|e| e.to_isize())?;
    image.truncate(read);
    Ok(image)
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
    let cycles = crate::time::now().cycles();
    let ns =
        (u128::from(cycles) * 1_000_000_000_u128) / u128::from(crate::time::clock_frequency_hz());
    let ts = KernelTimespec {
        sec: (ns / 1_000_000_000) as isize,
        nsec: (ns % 1_000_000_000) as isize,
    };
    copy_plain_to_user(timespec_address, &ts)
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

fn sys_pselect6(nfds: usize, timeout_address: usize) -> isize {
    if nfds != 0 {
        return -ENOSYS;
    }
    if timeout_address != 0 {
        sys_nanosleep(timeout_address, 0)
    } else {
        0
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
    if dirfd != AT_FDCWD {
        return Err(-EBADF);
    }
    let cwd = current_process().fs().cwd_path();
    crate::fs::resolve_path(&cwd, path).map_err(|errno| errno.to_isize())
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
