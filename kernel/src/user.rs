use alloc::boxed::Box;
use core::sync::atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering};

use myos_mm::{
    FaultAccess, PAGE_SIZE, PhysAddr, VirtAddr, VirtRange, VmArea, VmAreaFlags, VmAreaKind,
};

use crate::user_mm::{
    UserFaultFailure, UserFaultRecovery, UserFaultResolution, UserMm, UserMmRuntimeError,
};

const USER_CODE: usize = 0x0000_0000_0040_0000;
const USER_DATA: usize = USER_CODE + PAGE_SIZE;
const USER_DEMAND: usize = 0x0000_0000_0050_0000;
const USER_HEAP_START: usize = 0x0000_0000_0060_0000;
const USER_HEAP_LIMIT: usize = 0x0000_0000_0070_0000;
const USER_STACK: usize = 0x0000_0000_0080_0000;
const USER_STACK_TOP: usize = USER_STACK + PAGE_SIZE;
const USER_MMAP_START: usize = 0x0000_0000_0100_0000;
const USER_MMAP_END: usize = 0x0000_0000_4000_0000;

const SYS_WRITE: usize = 64;
const SYS_EXIT: usize = 93;
const SYS_BRK: usize = 214;
const SYS_MUNMAP: usize = 215;
const SYS_MMAP: usize = 222;
const SYS_MPROTECT: usize = 226;

const PROT_READ: usize = 1;
const PROT_WRITE: usize = 2;
const PROT_EXEC: usize = 4;
const MAP_PRIVATE: usize = 0x02;
const MAP_ANONYMOUS: usize = 0x20;

const EBADF: isize = 9;
const ENOMEM: isize = 12;
const EFAULT: isize = 14;
const EINVAL: isize = 22;
const ENOSYS: isize = 38;

const MAX_USER_COPY: usize = 256;
const USER_MESSAGE: &[u8] = b"hello user\n";

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

#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!("user/riscv64.S"));

#[cfg(target_arch = "loongarch64")]
core::arch::global_asm!(include_str!("user/loongarch64.S"));

unsafe extern "C" {
    fn __m7_enter_user(entry: usize, stack_top: usize) -> isize;
    fn __m7_user_return();

    static __m7_user_image_start: u8;
    static __m7_user_success: u8;
    static __m7_user_unknown_syscall: u8;
    static __m7_user_bad_pointer: u8;
    static __m7_user_write_code: u8;
    static __m8_user_vm: u8;
    static __m8_user_mprotect_fault: u8;
    static __m8_user_munmap_fault: u8;
    static __m7_user_image_end: u8;
}

struct UserImage {
    mm: Box<UserMm>,
    code_physical: PhysAddr,
}

impl UserImage {
    fn create() -> Result<Self, UserMmRuntimeError> {
        let areas = [
            VmArea::new(
                VirtRange::from_bounds(USER_CODE, USER_CODE + PAGE_SIZE),
                VmAreaFlags::user_rx(),
                VmAreaKind::Anonymous,
            ),
            VmArea::new(
                VirtRange::from_bounds(USER_DATA, USER_DATA + PAGE_SIZE),
                VmAreaFlags::user_rw(),
                VmAreaKind::Anonymous,
            ),
            VmArea::new(
                VirtRange::from_bounds(USER_DEMAND, USER_DEMAND + PAGE_SIZE),
                VmAreaFlags::user_rw(),
                VmAreaKind::Anonymous,
            ),
            VmArea::new(
                VirtRange::from_bounds(USER_STACK, USER_STACK_TOP),
                VmAreaFlags::user_rw().union(VmAreaFlags::GROW_DOWN),
                VmAreaKind::Stack,
            ),
        ];
        let mut mm = Box::new(UserMm::new(&areas)?);
        if let Err(error) = mm.configure_program_break(
            VirtAddr::new(USER_HEAP_START),
            VirtAddr::new(USER_HEAP_LIMIT),
        ) {
            mm.destroy()
                .expect("unable to reclaim M8-B4 mm after brk configuration failure");
            return Err(error);
        }
        let result: Result<PhysAddr, UserMmRuntimeError> = (|| {
            let code_physical = mm.populate_page(VirtAddr::new(USER_CODE))?;
            mm.populate_page(VirtAddr::new(USER_DATA))?;
            mm.populate_page(VirtAddr::new(USER_STACK))?;
            Ok(code_physical)
        })();
        let code_physical = match result {
            Ok(physical) => physical,
            Err(error) => {
                mm.destroy()
                    .expect("unable to reclaim a partially built M8-B3 user mm");
                return Err(error);
            }
        };

        Ok(Self { mm, code_physical })
    }

    fn publish(&self) {
        assert!(
            !ACTIVE.load(Ordering::Acquire),
            "M8-B3 attempted to publish two user sessions",
        );
        self.mm.bind().expect("unable to bind the M8-B3 user mm");
        ACTIVE.store(true, Ordering::Release);
    }

    fn unpublish(&self) {
        let was_active = ACTIVE.swap(false, Ordering::AcqRel);
        assert!(
            was_active,
            "M8-B3 attempted to unpublish an inactive session"
        );
        self.mm.unbind();
    }

    fn load_code(&self) {
        let image = embedded_user_image();
        assert!(
            !image.is_empty() && image.len() <= PAGE_SIZE,
            "M8-B3 embedded user image does not fit in one page",
        );
        copy_to_physical(self.code_physical, image);
        prepare_user_instruction_stream();
    }

    fn activate_current_cpu(&self) {
        self.mm
            .activate_current_cpu()
            .expect("unable to activate the M8-B3 user page-table root");
    }

    fn deactivate_current_cpu(&self) {
        self.mm
            .deactivate_current_cpu()
            .expect("unable to restore the kernel page-table root");
    }

    fn assert_private_hardware_state(&self) {
        assert!(
            self.mm
                .root_is_private()
                .expect("unable to compare M8-B3 page-table roots"),
            "M8-B3 user mm reused the kernel page-table root",
        );
        self.mm
            .assert_hardware_active()
            .expect("M8-B3 hardware root/ASID verification failed");
        assert!(
            self.mm
                .kernel_mapping_is_shared(VirtAddr::new(verify as usize))
                .expect("unable to verify the shared kernel mapping"),
            "M8-B3 user root lost the shared high-half kernel mapping",
        );
    }

    fn destroy(mut self) {
        self.mm
            .destroy()
            .expect("unable to destroy the M8-B3 user address space");
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

    assert!(
        !ACTIVE.load(Ordering::Acquire),
        "M8-B3 verifier leaked an active user session",
    );
    crate::user_mm::assert_no_leaks();

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
}

fn run_session(
    entry_symbol: *const u8,
    initial_data: Option<&[u8]>,
    exercise_copy_guards: bool,
) -> SessionObserved {
    reset_session_state();

    let image = UserImage::create().expect("unable to create M8-B3 user image");
    image.load_code();
    image.publish();

    if let Some(data) = initial_data {
        copy_to_user(USER_DATA, data).expect("checked copy_to_user rejected M8-B3 user data");
        let mut round_trip = [0_u8; USER_MESSAGE.len()];
        copy_from_user(USER_DATA, &mut round_trip)
            .expect("checked copy_from_user rejected M8-B3 user data");
        assert_eq!(&round_trip, data, "M8-B3 user copy changed data");
    }

    if exercise_copy_guards {
        verify_copy_guards();
    }

    let entry = user_entry(entry_symbol);
    let result = {
        /*
         * The M8 verifier retains M7's synchronous, non-preemptible user
         * round trip. M9 will attach an mm to schedulable Process/Thread
         * objects and move root switching into the context-switch path.
         */
        let _interrupt_guard = crate::context::IrqSaveGuard::new();
        image.activate_current_cpu();
        image.assert_private_hardware_state();
        let result = enter_user(entry, USER_STACK_TOP);
        image.deactivate_current_cpu();
        result
    };

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

fn verify_copy_guards() {
    assert!(
        copy_to_user(USER_CODE, &[0]).is_err(),
        "copy_to_user wrote through an RX user mapping",
    );

    let mut crossing = [0_u8; 2];
    assert!(
        copy_from_user(USER_DATA + PAGE_SIZE - 1, &mut crossing).is_err(),
        "copy_from_user accepted a cross-VMA range",
    );
    assert!(
        copy_from_user(usize::MAX - 1, &mut crossing).is_err(),
        "copy_from_user accepted an overflowing range",
    );

    let mut empty = [];
    assert!(
        copy_from_user(usize::MAX, &mut empty).is_ok(),
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

    match number {
        SYS_WRITE => {
            let result = sys_write(arguments[0], arguments[1], arguments[2]);
            set_syscall_result(frame, result);
        }
        SYS_BRK => set_syscall_result(frame, sys_brk(arguments[0])),
        SYS_MUNMAP => set_syscall_result(frame, sys_munmap(arguments[0], arguments[1])),
        SYS_MMAP => set_syscall_result(frame, sys_mmap(arguments)),
        SYS_MPROTECT => set_syscall_result(
            frame,
            sys_mprotect(arguments[0], arguments[1], arguments[2]),
        ),
        SYS_EXIT => {
            EXIT_STATUS.store(arguments[0] as isize, Ordering::Release);
            TERMINATED.store(true, Ordering::Release);
            return_to_kernel(frame, arguments[0] as isize);
        }
        _ => set_syscall_result(frame, -ENOSYS),
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
    match crate::user_mm::resolve_active_fault(address, access, user_sp) {
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
    let current = match crate::user_mm::active_program_break() {
        Ok(current) => current,
        Err(_) => return -ENOMEM,
    };
    if address == 0 {
        return current.get() as isize;
    }

    match crate::user_mm::set_active_program_break(VirtAddr::new(address)) {
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
    match crate::user_mm::map_active_anonymous(
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
    match crate::user_mm::unmap_active_range(range) {
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
    match crate::user_mm::protect_active_range(range, flags) {
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
    if fd != 1 {
        return -EBADF;
    }
    if length > MAX_USER_COPY {
        return -EINVAL;
    }

    let mut buffer = [0_u8; MAX_USER_COPY];
    if copy_from_user(address, &mut buffer[..length]).is_err() {
        return -EFAULT;
    }

    let text = match core::str::from_utf8(&buffer[..length]) {
        Ok(text) => text,
        Err(_) => return -EINVAL,
    };

    crate::print!("{text}");
    WRITE_COUNT.fetch_add(1, Ordering::AcqRel);
    length as isize
}

fn copy_from_user(address: usize, output: &mut [u8]) -> Result<(), ()> {
    crate::user_mm::copy_from_active(address, output).map_err(|_| ())
}

fn copy_to_user(address: usize, input: &[u8]) -> Result<(), ()> {
    crate::user_mm::copy_to_active(address, input).map_err(|_| ())
}

fn copy_to_physical(physical: PhysAddr, bytes: &[u8]) {
    let destination = crate::arch::memory::phys_access::ram_mut_ptr::<u8>(physical)
        .expect("M8-B3 backing page is outside RAM");

    // SAFETY: the caller owns a zeroed full page and has checked bytes.len().
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len());
    }
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

fn enter_user(entry: usize, stack_top: usize) -> isize {
    assert_eq!(
        stack_top & 0xf,
        0,
        "M8-B3 user stack is not 16-byte aligned",
    );

    // SAFETY: the verifier installed a validated private user root, keeps the
    // current kernel stack alive, and disables local interrupts until return.
    unsafe { __m7_enter_user(entry, stack_top) }
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
        core::arch::asm!("ibar 0", options(nostack));
    }
}

#[cfg(target_arch = "riscv64")]
fn syscall_number(frame: &crate::arch::trap::TrapFrame) -> usize {
    frame.gpr[17]
}

#[cfg(target_arch = "loongarch64")]
fn syscall_number(frame: &crate::arch::trap::TrapFrame) -> usize {
    frame.gpr[11]
}

#[cfg(target_arch = "riscv64")]
fn syscall_arguments(frame: &crate::arch::trap::TrapFrame) -> [usize; 6] {
    [
        frame.gpr[10],
        frame.gpr[11],
        frame.gpr[12],
        frame.gpr[13],
        frame.gpr[14],
        frame.gpr[15],
    ]
}

#[cfg(target_arch = "loongarch64")]
fn syscall_arguments(frame: &crate::arch::trap::TrapFrame) -> [usize; 6] {
    [
        frame.gpr[4],
        frame.gpr[5],
        frame.gpr[6],
        frame.gpr[7],
        frame.gpr[8],
        frame.gpr[9],
    ]
}

fn advance_syscall_pc(frame: &mut crate::arch::trap::TrapFrame) {
    frame.advance_pc(4);
}

#[cfg(target_arch = "riscv64")]
fn set_syscall_result(frame: &mut crate::arch::trap::TrapFrame, result: isize) {
    frame.gpr[10] = result as usize;
}

#[cfg(target_arch = "loongarch64")]
fn set_syscall_result(frame: &mut crate::arch::trap::TrapFrame, result: isize) {
    frame.gpr[4] = result as usize;
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
