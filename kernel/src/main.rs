#![feature(let_chains)]
#![feature(unsigned_is_multiple_of)]
#![no_std]
#![no_main]

mod block;
mod call_function;
mod console;
mod context;
mod device;
mod devpts;
mod elf;
mod exec;
mod ext4;
mod fault;
mod fs;
mod heap;
mod initramfs;
mod ipi;
mod irq;
mod irq_lock;
mod linker;
mod lockdep;
mod memory;
mod net;
mod page_alloc;
mod panic;
mod pipe;
mod process;
mod procfs;
mod rng;
mod rtc;
mod runtime_page_table;
mod signal;
mod smp;
mod sysfs;
mod syscall;
mod task;
mod time;
mod timer;
mod tlb;
mod tracked_spin;
mod trap;
mod tty;
mod user;
mod user_mm;
mod virtio;

mod vm;
mod workqueue;
extern crate alloc;

use myos_boot::BootInfo;
use myos_fdt::{DeviceTree, FdtBlob, MemoryRegion};

use alloc::collections::BTreeSet;

#[cfg(target_arch = "riscv64")]
pub(crate) use arch_riscv64 as arch;

#[cfg(target_arch = "loongarch64")]
pub(crate) use arch_loongarch64 as arch;

#[cfg(not(any(target_arch = "riscv64", target_arch = "loongarch64")))]
compile_error!("unsupported target architecture");

/// 所有架构最终进入的公共 Rust 入口。
#[unsafe(no_mangle)]
pub extern "C" fn rust_entry(arg0: usize, arg1: usize, arg2: usize) -> ! {
    arch::smp::set_current_cpu_id(smp::CpuId::BOOT.get());
    let boot = arch::boot::from_raw(arg0, arg1, arg2).into_boot_info();

    print_boot_info(&boot);

    kernel_main(boot)
}

#[cfg(target_arch = "riscv64")]
fn boot_hardware_cpu_id(boot: &BootInfo) -> usize {
    boot.boot_cpu_id()
        .expect("RISC-V boot protocol did not provide the boot hart ID")
}

#[cfg(target_arch = "loongarch64")]
fn boot_hardware_cpu_id(_boot: &BootInfo) -> usize {
    arch::smp::hardware_cpu_id()
}

fn print_boot_info(boot: &BootInfo) {
    let raw = boot.raw_args();

    println!();
    println!("MyOS");
    println!("  architecture : {}", arch::ARCH_NAME);

    println!(
        "  firmware args: {:#018x} {:#018x} {:#018x}",
        raw[0], raw[1], raw[2],
    );

    match boot.boot_cpu_id() {
        Some(cpu_id) => {
            println!("  boot cpu      : {cpu_id}");
        }
        None => {
            println!("  boot cpu      : unavailable");
        }
    }

    match boot.device_tree() {
        Some(address) => {
            println!("  device tree   : {:#018x}", address.get());
        }
        None => {
            println!("  device tree   : unavailable");
        }
    }

    match boot.command_line() {
        Some(address) => {
            println!("  command line  : {:#018x}", address.get());
        }
        None => {
            println!("  command line  : unavailable");
        }
    }

    match boot.system_table() {
        Some(address) => {
            println!("  system table  : {:#018x}", address.get());
        }
        None => {
            println!("  system table  : unavailable");
        }
    }

    println!();
    println!("entered Rust kernel successfully");
}


#[cfg(target_arch = "riscv64")]
#[unsafe(no_mangle)]
pub extern "C" fn __riscv_early_trap_panic() -> ! {
    let scause: usize;
    let sepc: usize;
    let stval: usize;
    let stvec: usize;
    let satp: usize;
    let sp: usize;

    unsafe {
        core::arch::asm!("csrr {out}, scause", out = out(reg) scause, options(nomem, nostack));
        core::arch::asm!("csrr {out}, sepc", out = out(reg) sepc, options(nomem, nostack));
        core::arch::asm!("csrr {out}, stval", out = out(reg) stval, options(nomem, nostack));
        core::arch::asm!("csrr {out}, stvec", out = out(reg) stvec, options(nomem, nostack));
        core::arch::asm!("csrr {out}, satp", out = out(reg) satp, options(nomem, nostack));
        core::arch::asm!("mv {out}, sp", out = out(reg) sp, options(nomem, nostack));
    }

    crate::println!();
    crate::println!("================ RISC-V EARLY TRAP ================");
    crate::println!("trap subsystem not installed yet");
    crate::println!("  scause : {:#018x}", scause);
    crate::println!("  sepc   : {:#018x}", sepc);
    crate::println!("  stval  : {:#018x}", stval);
    crate::println!("  stvec  : {:#018x}", stvec);
    crate::println!("  satp   : {:#018x}", satp);
    crate::println!("  sp     : {:#018x}", sp);
    crate::println!("===================================================");

    loop {
        arch::cpu::wait_for_interrupt();
    }
}

fn kernel_main(boot: BootInfo) -> ! {
    println!("kernel_main: initialization started");

    #[cfg(target_arch = "loongarch64")]
    memory::verify_loongarch_high_mapping();

    let fdt_address = boot
        .device_tree()
        .expect("a device tree is required at this stage")
        .get();

    /*
     * SAFETY:
     *
     * FDT 地址来自受信任的架构启动协议。
     * 当前尚未启用正式分页。
     */
    let fdt_physical = myos_mm::PhysAddr::new(fdt_address);

    let fdt_pointer =
        arch::memory::phys_access::ram_ptr::<u8>(fdt_physical).unwrap_or_else(|error| {
            panic!(
                "unable to map FDT physical address \
             {fdt_address:#x}: {error:?}",
            );
        });

    let (memory_layout, firmware_timer_frequency, virtio_regions, pci_hosts, initrd_range) = {
        // SAFETY: fdt_pointer 指向启动协议提供的只读 FDT blob。
        let blob = unsafe { FdtBlob::from_ptr(fdt_pointer) }.unwrap_or_else(|error| {
            panic!(
                "failed to validate FDT at \
             {fdt_address:#x}: {error}",
            );
        });

        let tree = DeviceTree::from_blob(&blob).unwrap_or_else(|error| {
            panic!(
                "failed to parse FDT at \
                         {fdt_address:#x}: {error}",
            );
        });

        inspect_device_tree(&boot, &blob, &tree);
        let virtio_regions = collect_virtio_mmio_regions(&tree);
        let pci_hosts = collect_pci_host_bridges(&tree);
        smp::initialize(&tree, boot_hardware_cpu_id(&boot));

        let firmware_timer_frequency = tree.timebase_frequency_hz();
        let initrd_range = tree.linux_initrd_range().unwrap_or_else(|error| {
            panic!("failed to parse /chosen initrd range: {error}");
        });
        let memory_layout = memory::build_boot_memory_layout(fdt_address, &blob, &tree)
            .unwrap_or_else(|error| {
                panic!(
                    "failed to construct physical memory layout: \
                     {error:?}",
                );
            });

        (
            memory_layout,
            firmware_timer_frequency,
            virtio_regions,
            pci_hosts,
            initrd_range,
        )
    };

    memory::print_boot_memory_map(memory_layout.free());
    memory::print_virtual_layout();
    memory::validate_paging_policy();
    memory::verify_early_frame_allocator(memory_layout.free());

    let mut early_memory = memory::initialize_early_memory(memory_layout.free());

    memory::map_boot_fdt_page(&mut early_memory, fdt_address);

    memory::prepare_kernel_image(&mut early_memory);

    #[cfg(target_arch = "riscv64")]
    {
        /*
         * Rust 此时已经通过静态临时 Sv39 在高半地址执行。
         */

        memory::prepare_riscv_direct_map(&mut early_memory, memory_layout.ram());

        memory::prepare_riscv_smp_trampoline(&mut early_memory);

        memory::prepare_riscv_early_uart_mapping(&mut early_memory);

        /*
         * FDT 低地址引用已在 memory_layout 构造作用域内结束；
         * 高半 kernel image 之前已经由
         * prepare_kernel_image() 写入正式页表。
         */
        memory::install_riscv_final_page_table(&early_memory);   }

    /*
     * 从此处开始，不再允许使用 EarlyFrameAllocator。
     */  let kernel_memory = memory::initialize_page_allocator(&memory_layout, early_memory);   #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    page_alloc::verify();

    /*
     * 必须在全局页分配器安装后启用 heap。
     */
    #[cfg(target_arch = "riscv64")] heap::initialize_boot(); #[cfg(not(target_arch = "riscv64"))] heap::initialize();

    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    heap::verify();

    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    irq_lock::verify();

    /*
     * 此时仍保持本地中断关闭。
     */
    trap::initialize();
    irq::initialize();
    time::initialize(firmware_timer_frequency);
    timer::initialize();
    vm::initialize(kernel_memory);
    virtio::initialize(&virtio_regions, &pci_hosts);
    device::initialize();
    rng::initialize();
    net::initialize();
    rtc::initialize();
    fault::initialize();
    fs::initialize();
    mount_proc();
    mount_sys();
    install_external_initramfs(initrd_range);
    mount_sdcard_if_present();
    tty::initialize();

    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    vm::verify();

    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    fault::verify();
    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    fs::verify();
    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    block::verify();
    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    virtio::verify();
    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    device::verify();
    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    rng::verify();
    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    pipe::verify();
    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    signal::verify();
    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    tty::verify();
    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    devpts::verify();
    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    rtc::verify();

    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    trap::verify_breakpoint();

    time::start_periodic();

    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    time::verify_periodic();

    task::initialize();
    smp::start_secondaries();
    task::finalize_cpu_bringup();
    workqueue::initialize();
    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    timer::verify();
    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    workqueue::verify();
    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    tracked_spin::verify();

    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    task::verify();
    user::verify();
    if initrd_range.is_some() {
        user::verify_busybox_rootfs();
    }
    user::verify_sdcard_sample();
    user::verify_sdcard_all_scripts();

    println!("kernel_main: initialization completed");
    println!("SMOKE_TEST: PASS");

    // Competition: shut down so QEMU exits and the evaluator can score.
    #[cfg(target_arch = "riscv64")]
    {
        let ret: usize;
        unsafe {
            core::arch::asm!(
                "ecall",
                inlateout("a0") 0usize => ret,
                in("a1") 0usize,
                in("a6") 0usize,
                in("a7") 0x53525354usize,
            );
        }
        println!("contest: SBI shutdown returned {}", ret);
    }
    #[cfg(target_arch = "loongarch64")]
    {
        println!("contest: loongarch shutdown — halting");
    }
    loop {
        #[cfg(target_arch = "riscv64")]
        unsafe {
            core::arch::asm!("wfi", options(nomem, nostack));
        }
        #[cfg(not(target_arch = "riscv64"))]
        {
            core::hint::spin_loop();
        }
    }
}

fn mount_proc() {
    match fs::mkdir("/proc", 0o755) {
        Ok(()) | Err(myos_vfs::Errno::Eexist) => {}
        Err(error) => panic!("failed to create /proc: {error:?}"),
    }
    match fs::mount(Some("proc"), "/proc", "proc", 0) {
        Ok(()) => {
            crate::println!("procfs:");
            crate::println!("  mount          : /proc (proc)");
            crate::println!(
                "  files          : version cpuinfo meminfo uptime mounts self"
            );
        }
        Err(error) => panic!("failed to mount /proc: {error:?}"),
    }
}

fn mount_sys() {
    match fs::mkdir("/sys", 0o755) {
        Ok(()) | Err(myos_vfs::Errno::Eexist) => {}
        Err(error) => panic!("failed to create /sys: {error:?}"),
    }
    match fs::mount(Some("sysfs"), "/sys", "sysfs", 0) {
        Ok(()) => {
            crate::println!("sysfs:");
            crate::println!("  mount          : /sys (sysfs)");
            crate::println!("  dirs           : kernel devices class");
        }
        Err(error) => panic!("failed to mount /sys: {error:?}"),
    }
}

fn mount_sdcard_if_present() {
    if crate::block::open_device("vda").is_none() {
        return;
    }
    let device = match crate::block::open_device("vda") {
        Some(device) => device,
        None => {
            crate::println!("sdcard: /dev/vda open failed — skipping mount");
            return;
        }
    };
    let mut magic = [0_u8; 2];
    if crate::block::read_at(&device, 1024 + 56, &mut magic).is_err()
        || u16::from_le_bytes(magic) != 0xef53
    {
        crate::println!("sdcard: not an ext4 filesystem — skipping mount");
        return;
    }

    let root_entries = match crate::ext4::list_directory(alloc::sync::Arc::clone(&device), "/") {
        Ok(entries) => entries,
        Err(error) => {
            crate::println!("sdcard: failed to list ext4 root directory: {error:?}");
            return;
        }
    };

    let _ = fs::mkdir("/mnt", 0o755);
    let _ = fs::mkdir("/mnt/sdcard", 0o755);
    let _ = fs::mkdir("/bin", 0o755);
    let _ = fs::mkdir("/lib", 0o755);
    let _ = fs::mkdir("/lib64", 0o755);
    let _ = fs::mkdir("/usr", 0o755);
    let _ = fs::mkdir("/usr/lib", 0o755);
    let _ = fs::mkdir("/usr/lib64", 0o755);
    // /lib64 is commonly a symlink to /lib on many Linux distributions.
    let _ = fs::symlink("/lib", "/lib64");

    for busybox_ext4 in &["/musl/busybox", "/busybox", "/busybox-static", "/bin/busybox", "/usr/bin/busybox"] {
        oscomp_sdcard_install_ext4_path(busybox_ext4, "/bin/busybox");
        if fs::stat("/bin/busybox").is_ok() {
            let _ = fs::symlink("/bin/busybox", "/bin/sh");
            // Create /usr/bin/env for scripts that use #!/usr/bin/env.
            let _ = fs::mkdir("/usr/bin", 0o755);
            let _ = fs::symlink("/bin/busybox", "/usr/bin/env");
            for applet in &[
                "cp", "sleep", "kill", "cat", "echo", "mv", "ln", "rm", "ls",
                "mkdir", "chmod", "grep", "dd", "mount", "ps", "head", "tail", "test",
                "awk", "sed", "wc", "cut", "tr", "which", "pidof", "printenv",
                "basename", "dirname", "readlink", "stat", "getopt", "env",
                "sh", "id", "uname", "df",
            ] {
                let _ = fs::symlink("/bin/busybox", &alloc::format!("/bin/{}", applet));
            }
            break;
        }
    }

    // Install musl/glibc dynamic linker and libc from ext4 sdcard.
    // The dynamic linker path must match what PT_INTERP encodes in the ELF.
    for lib_path in &[
        // musl
        "/lib/libc.so", "/usr/lib/libc.so", "/musl/lib/libc.so",
        "/lib/ld-musl-riscv64-sf.so", "/lib/ld-musl-loongarch64-sf.so",
        "/lib/ld-musl-riscv64.so.1", "/lib/ld-musl-loongarch64.so.1",
        // glibc
        "/lib/ld-linux-riscv64-lp64d.so.1", "/lib/ld-linux-riscv64-lp64.so.1",
        "/lib/ld-linux-loongarch64-lp64d.so.1", "/lib/ld-linux-loongarch64-lp64.so.1",
        "/lib/libc.so.6", "/lib/libpthread.so.0", "/lib/libdl.so.2",
        "/lib/librt.so.1", "/lib/libm.so.6", "/lib/libresolv.so.2",
        "/lib/libutil.so.1", "/lib/libnsl.so.1", "/lib/libcrypt.so.1",
        // Extra glibc paths
        "/usr/lib/libc.so.6", "/usr/lib/libpthread.so.0",
        "/lib64/ld-linux-riscv64-lp64d.so.1",
        "/lib64/ld-linux-loongarch64-lp64d.so.1",
    ] {
        let vfs_path = if lib_path.starts_with("/musl/") {
            alloc::format!("/mnt/sdcard{}", lib_path)
        } else {
            alloc::string::String::from(*lib_path)
        };
        oscomp_sdcard_install_ext4_path(lib_path, &vfs_path);
    }

    // Create /lib → /mnt/sdcard/glibc/lib fallback symlinks for
    // dynamically-linked glibc programs whose PT_INTERP points to
    // paths under /lib or /lib64.
    let glibc_lib_paths = ["/glibc/lib", "/musl/lib"];
    for glibc_dir in &glibc_lib_paths {
        let src = alloc::format!("/mnt/sdcard{}", glibc_dir);
        if fs::stat(&src).is_ok() {
            // For each .so file in the glibc/musl lib dir, create a symlink
            // under /lib if it doesn't already exist.
            if let Ok(file) = fs::open(&src, myos_vfs::OpenFlags::O_RDONLY | myos_vfs::OpenFlags::O_DIRECTORY) {
                let mut buf = [0_u8; 4096];
                let mut output = myos_vfs::MutableIoBuffer::new(&mut buf);
                if let Ok(_) = file.readdir(&mut output) {
                    // readdir fills the buffer; we could parse and create symlinks.
                    // For now, rely on the explicit path list above.
                }
            }
            break;
        }
    }

    const EXT4_FT_DIR: u16 = 2;
    const MAX_SCAN_DIRS: usize = 96;
    const MAX_TEST_SCRIPTS: usize = 128;
    let mut dirs: alloc::vec::Vec<alloc::string::String> = alloc::vec::Vec::new();
    let mut test_scripts: alloc::vec::Vec<alloc::string::String> = alloc::vec::Vec::new();

    for entry in &root_entries {
        let path = alloc::format!("/{}", entry.name);
        if entry.file_type == EXT4_FT_DIR {
            if dirs.len() < MAX_SCAN_DIRS {
                dirs.push(path.clone());
            }
        } else if oscomp_sdcard_is_test_script(&path) && test_scripts.len() < MAX_TEST_SCRIPTS {
            test_scripts.push(path);
        }
    }

    let mut index = 0_usize;
    while index < dirs.len() && index < MAX_SCAN_DIRS {
        let dir = dirs[index].clone();
        index += 1;
        let Ok(entries) = crate::ext4::list_directory(alloc::sync::Arc::clone(&device), &dir) else {
            continue;
        };
        for entry in entries {
            let child = alloc::format!("{}/{}", dir.trim_end_matches('/'), entry.name);
            if entry.file_type == EXT4_FT_DIR {
                if dirs.len() < MAX_SCAN_DIRS {
                    dirs.push(child);
                }
            } else if oscomp_sdcard_is_test_script(&child) && test_scripts.len() < MAX_TEST_SCRIPTS {
                test_scripts.push(child);
            }
        }
    }

    let mut installed_scripts: alloc::vec::Vec<alloc::string::String> = alloc::vec::Vec::new();
    // Track which ext4 directories we have already expanded to avoid
    // redundant materialization.
    let mut expanded_dirs: alloc::collections::BTreeSet<alloc::string::String> =
        alloc::collections::BTreeSet::new();
    let mut total_expanded: usize = 0;
    const MAX_TOTAL_EXPAND: usize = 2048;

    for ext4_path in &test_scripts {
        let vfs_path = alloc::format!("/mnt/sdcard{}", ext4_path);
        oscomp_sdcard_install_ext4_path(ext4_path, &vfs_path);
        if fs::stat(&vfs_path).is_ok() {
            installed_scripts.push(vfs_path.trim_start_matches('/').into());
        }
        // Expand the script's parent directory so ./busybox etc. resolve.
        if let Some(slash) = ext4_path.rfind('/') {
            let parent = &ext4_path[..slash];
            if parent.len() > 1 && !expanded_dirs.contains(parent) {
                expanded_dirs.insert(alloc::string::String::from(parent));
                if total_expanded < MAX_TOTAL_EXPAND {
                    let count = expand_ext4_directory(parent, MAX_TOTAL_EXPAND - total_expanded);
                    total_expanded += count;
                }
            }
        }
    }

    // Expand additional commonly-needed directories under /glibc and /musl.
    for common_dir in &[
        "/glibc", "/glibc/basic", "/glibc/lib", "/glibc/lua",
        "/glibc/ltp", "/glibc/lmbench", "/musl", "/musl/basic",
        "/musl/lib", "/musl/lua", "/musl/ltp", "/musl/lmbench",
    ] {
        if total_expanded >= MAX_TOTAL_EXPAND {
            break;
        }
        if expanded_dirs.contains(*common_dir) {
            continue;
        }
        // Try expanding; skip silently if the dir doesn't exist on ext4.
        expanded_dirs.insert(alloc::string::String::from(*common_dir));
        let count = expand_ext4_directory(common_dir, MAX_TOTAL_EXPAND - total_expanded);
        total_expanded += count;
    }

    println!("sdcard:");
    println!("  mount         : /dev/vda (ext4)");
    println!("  mounted tree  : /mnt/sdcard (lazy file install)");
    println!("  root entries  : {}", root_entries.len());
    println!("  scanned dirs   : {}", dirs.len().min(MAX_SCAN_DIRS));
    println!("  test scripts  : {}", installed_scripts.len());
    for script in installed_scripts.iter().take(8) {
        println!("  test script   : /{}", script);
    }
    if installed_scripts.len() > 8 {
        println!("  test script   : ... {} more", installed_scripts.len() - 8);
    }
    SCANNED_TEST_SCRIPTS.lock().clone_from(&installed_scripts);
}

fn oscomp_sdcard_is_test_script(path: &str) -> bool {
    let lower = path;
    lower.ends_with("_testcode.sh")
        || lower.ends_with("testcode.sh")
        || lower.ends_with("/run_test.sh")
        || lower.ends_with("/runtest.sh")
        || lower.ends_with("/test.sh")
        || (lower.ends_with(".sh") && lower.contains("test"))
}

fn oscomp_sdcard_install_ext4_path(ext4_path: &str, vfs_path: &str) {
    oscomp_sdcard_ensure_parent_dirs(vfs_path);
    let _ = fs::install_ext4_path("/dev/vda", vfs_path, ext4_path);
}

/// Expand an ext4 directory and its first-level children into VFS.
/// This ensures scripts can find `./busybox`, `./lua`, `./lmbench_all` etc.
/// Limits prevent OOM on large sdcard images.
fn expand_ext4_directory(ext4_dir: &str, max_files: usize) -> usize {
    const MAX_EXPAND: usize = 512;
    const EXT4_FT_DIR: u16 = 2;
    let limit = core::cmp::min(max_files, MAX_EXPAND);

    let device = match crate::block::open_device("vda") {
        Some(d) => d,
        None => return 0,
    };
    let entries = match crate::ext4::list_directory(alloc::sync::Arc::clone(&device), ext4_dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };

    let vfs_dir = alloc::format!("/mnt/sdcard{}", ext4_dir);
    let _ = fs::mkdir(&vfs_dir, 0o755);
    let mut installed: usize = 0;

    for entry in &entries {
        if installed >= limit {
            break;
        }
        let ext4_path = alloc::format!("{}/{}", ext4_dir.trim_end_matches('/'), entry.name);
        let vfs_path = alloc::format!("/mnt/sdcard{}", ext4_path);

        if entry.file_type == EXT4_FT_DIR {
            // Create directory in VFS and expand one level deeper.
            let _ = fs::mkdir(&vfs_path, 0o755);
            // Expand subdir entries (files only, one level).
            if let Ok(sub_entries) =
                crate::ext4::list_directory(alloc::sync::Arc::clone(&device), &ext4_path)
            {
                for sub in &sub_entries {
                    if installed >= limit {
                        break;
                    }
                    let sub_ext4 = alloc::format!("{}/{}", ext4_path.trim_end_matches('/'), sub.name);
                    let sub_vfs = alloc::format!("{}/{}", vfs_path.trim_end_matches('/'), sub.name);
                    if sub.file_type != EXT4_FT_DIR {
                        oscomp_sdcard_ensure_parent_dirs(&sub_vfs);
                        if fs::install_ext4_path("/dev/vda", &sub_vfs, &sub_ext4).is_ok() {
                            installed += 1;
                        }
                    } else {
                        let _ = fs::mkdir(&sub_vfs, 0o755);
                    }
                }
            }
        } else {
            // Regular file or symlink: install into VFS.
            oscomp_sdcard_ensure_parent_dirs(&vfs_path);
            if fs::install_ext4_path("/dev/vda", &vfs_path, &ext4_path).is_ok() {
                installed += 1;
            }
        }
    }
    installed
}

fn oscomp_sdcard_ensure_parent_dirs(path: &str) {
    let mut components: alloc::vec::Vec<&str> = alloc::vec::Vec::new();
    for component in path.split('/') {
        if !component.is_empty() {
            components.push(component);
        }
    }
    if components.len() <= 1 {
        return;
    }
    let mut current = alloc::string::String::new();
    for component in components.iter().take(components.len() - 1) {
        current.push('/');
        current.push_str(component);
        let _ = fs::mkdir(&current, 0o755);
    }
}

fn collect_sdcard_test_scripts(
    path: &str,
    node: &crate::ext4::Ext4SnapshotNode,
    scripts: &mut alloc::vec::Vec<alloc::string::String>,
    visited: &mut usize,
) {
    if *visited >= 8192 || scripts.len() >= 256 {
        return;
    }
    *visited += 1;
    match &node.kind {
        crate::ext4::Ext4SnapshotKind::Directory(children) => {
            for entry in children {
                let mut child_path = alloc::string::String::new();
                if path.is_empty() {
                    child_path.push('/');
                    child_path.push_str(&entry.name);
                } else {
                    child_path.push_str(path);
                    if !path.ends_with('/') {
                        child_path.push('/');
                    }
                    child_path.push_str(&entry.name);
                }
                collect_sdcard_test_scripts(&child_path, &entry.node, scripts, visited);
            }
        }
        crate::ext4::Ext4SnapshotKind::Regular(_) => {
            if sdcard_is_test_script(path) {
                let mut vfs_path = alloc::string::String::from("/mnt/sdcard");
                if !path.starts_with('/') {
                    vfs_path.push('/');
                }
                vfs_path.push_str(path);
                scripts.push(vfs_path);
            }
        }
        crate::ext4::Ext4SnapshotKind::Symlink(_) => {}
    }
}

fn sdcard_is_test_script(path: &str) -> bool {
    path.ends_with("_testcode.sh")
        || path.ends_with("testcode.sh")
        || path.ends_with("/run_test.sh")
        || path.ends_with("/runtest.sh")
        || path.ends_with("/test.sh")
        || (path.ends_with(".sh") && path.contains("test"))
}

pub(crate) static SCANNED_TEST_SCRIPTS: crate::irq_lock::IrqSpinLock<alloc::vec::Vec<alloc::string::String>> =
    crate::irq_lock::IrqSpinLock::new_with_class(
        alloc::vec::Vec::new(),
        crate::lockdep::LockClass::new("sdcard.scripts", crate::lockdep::LockRank::Vfs, 4),
    );

fn install_external_initramfs(range: Option<MemoryRegion>) {
    let Some(range) = range else {
        println!("initramfs:");
        println!("  external      : unavailable");
        return;
    };

    let pointer =
        crate::arch::memory::phys_access::ram_ptr::<u8>(myos_mm::PhysAddr::new(range.start()))
            .unwrap_or_else(|error| {
                panic!(
                    "unable to map external initramfs at {:#018x}: {error:?}",
                    range.start(),
                );
            });

    // SAFETY: the initrd range comes from validated FDT `/chosen` properties
    // and has been reserved from the page allocator before heap/page use.
    let archive_bytes = unsafe { core::slice::from_raw_parts(pointer, range.size()) };
    let archive = crate::initramfs::Initramfs::parse(archive_bytes).unwrap_or_else(|error| {
        panic!("external initramfs is not a valid newc archive: {error:?}");
    });
    let installed = crate::fs::unpack_initramfs(&archive).unwrap_or_else(|error| {
        panic!("failed to unpack external initramfs into rootfs: {error:?}");
    });

    println!("initramfs:");
    println!(
        "  external      : [{:#018x}, {:#018x})",
        range.start(),
        range.end().unwrap_or(usize::MAX),
    );
    println!("  rootfs entries: {installed}");
}

fn inspect_device_tree(boot: &BootInfo, blob: &FdtBlob<'_>, tree: &DeviceTree<'_>) {
    let address = boot.device_tree().expect("device tree address disappeared");

    println!("fdt:");
    println!("  address       : {:#018x}", address.get(),);
    println!("  total size    : {} bytes", blob.total_size(),);

    match tree.model() {
        Some(model) => {
            println!("  model         : {model}");
        }

        None => {
            println!("  model         : unavailable");
        }
    }
    match tree.first_compatible() {
        Some(compatible) => {
            println!("  compatible    : {compatible}");
        }

        None => {
            println!("  compatible    : unavailable");
        }
    }
    println!("  cpu count     : {}", tree.cpu_count(),);

    match tree.timebase_frequency_hz() {
        Some(frequency) => {
            println!("  timer frequency: {} Hz", frequency);
        }
        None => {
            println!("  timer frequency: architecture-defined");
        }
    }

    match tree.bootargs() {
        Some(arguments) => {
            println!("  bootargs      : {arguments}");
        }

        None => {
            println!("  bootargs      : unavailable");
        }
    }

    match tree.linux_initrd_range() {
        Ok(Some(region)) => {
            println!(
                "  initrd        : [{:#018x}, {:#018x}) {} KiB",
                region.start(),
                region.end().unwrap_or(usize::MAX),
                region.size() / 1024,
            );
        }
        Ok(None) => {
            println!("  initrd        : unavailable");
        }
        Err(error) => {
            println!("  initrd        : malformed ({error})");
        }
    }

    println!("  memory:");

    let mut memory_count = 0;

    for region in tree.memory_regions() {
        memory_count += 1;

        println!(
            "    [{:#018x}, {:#018x})  {} MiB",
            region.start(),
            region.end().unwrap_or(usize::MAX),
            region.size() / 1024 / 1024,
        );
    }

    if memory_count == 0 {
        println!("    unavailable");
    }

    println!("  virtio-mmio:");

    let mut virtio_count = 0;

    for region in tree.virtio_mmio_regions() {
        virtio_count += 1;

        println!(
            "    {}: base={:#018x}, size={:#x}",
            region.name(),
            region.base(),
            region.size(),
        );
    }

    if virtio_count == 0 {
        println!("    unavailable");
    }

    println!("  pci-host:");

    let mut pci_count = 0;
    for host in tree.pci_host_bridges() {
        pci_count += 1;
        let ecam = host.ecam();
        let mem32 = host.mem32();
        println!(
            "    {}: ecam=[{:#018x}, {:#018x}) mem32=[{:#018x}, {:#018x}) bus={}..{}",
            host.name(),
            ecam.start(),
            ecam.end().unwrap_or(usize::MAX),
            mem32.start(),
            mem32.end().unwrap_or(usize::MAX),
            host.first_bus(),
            host.last_bus(),
        );
    }

    if pci_count == 0 {
        println!("    unavailable");
    }
}

fn collect_virtio_mmio_regions(tree: &DeviceTree<'_>) -> virtio::MmioRegions {
    let mut regions = virtio::MmioRegions::new();

    for region in tree.virtio_mmio_regions() {
        regions.push(virtio::MmioRegion::new(region.base(), region.size()));
    }

    regions
}

fn collect_pci_host_bridges(tree: &DeviceTree<'_>) -> virtio::PciHostBridges {
    let mut hosts = virtio::PciHostBridges::new();

    for host in tree.pci_host_bridges() {
        hosts.push(virtio::PciHostBridge::new(
            host.name(),
            host.ecam(),
            host.mem32(),
            host.first_bus(),
            host.last_bus(),
        ));
    }

    hosts
}
