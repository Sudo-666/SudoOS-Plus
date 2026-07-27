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
mod oscomp;
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
mod syscall;
mod sysfs;
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
    println!("BOOT00 entry");

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

    let (
        memory_layout,
        firmware_timer_frequency,
        virtio_regions,
        pci_hosts,
        initrd_range,
        explicit_oscomp_mode,
    ) = {
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
        let explicit_oscomp_mode = oscomp::mode_from_bootargs(tree.bootargs());
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
            explicit_oscomp_mode,
        )
    };

    println!("BOOT01 fdt-valid");
    println!("BOOT02 memory-map");
    println!("BOOT08 smp-discovery count={}", smp::discovered_cpu_count());

    memory::print_boot_memory_map(memory_layout.free());
    memory::print_virtual_layout();
    memory::validate_paging_policy();
    memory::verify_early_frame_allocator(memory_layout.free());

    let mut early_memory = memory::initialize_early_memory(memory_layout.free());
    println!("BOOT03 early-page-table");

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
        memory::install_riscv_final_page_table(&early_memory);
    }

    println!("BOOT04 final-page-table");

    /*
     * 从此处开始，不再允许使用 EarlyFrameAllocator。
     */
    let kernel_memory = memory::initialize_page_allocator(&memory_layout, early_memory);
    println!("BOOT05 buddy-ready");
    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    page_alloc::verify();

    /*
     * 必须在全局页分配器安装后启用 heap。
     */
    #[cfg(target_arch = "riscv64")]
    heap::initialize_boot();
    #[cfg(not(target_arch = "riscv64"))]
    heap::initialize();
    println!("BOOT06 heap-ready");

    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    heap::verify();

    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    irq_lock::verify();

    /*
     * 此时仍保持本地中断关闭。
     */
    trap::initialize();
    println!("BOOT07 bsp-trap-ready");
    irq::initialize();
    time::initialize(firmware_timer_frequency);
    timer::initialize();
    vm::initialize(kernel_memory);
    virtio::initialize(&virtio_regions, &pci_hosts);
    println!("BOOT12 virtio-ready");
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
    println!("BOOT13 rootfs-ready");
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
    println!("BOOT11 all-ap-online");
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
    let oscomp_mode = oscomp::select_mode(explicit_oscomp_mode);
    println!("BOOT14 user-entry");
    let contest_ran = oscomp::run(oscomp_mode);

    println!("kernel_main: initialization completed");
    println!("SMOKE_TEST: PASS");

    if contest_ran {
        // Competition: contest completed, summary/score already printed.
        // Use the same unified power-off path as the contest runner and
        // watchdog so RISC-V and LoongArch behaviour stay consistent.
        println!("contest: runner returned after summary; forcing platform shutdown");
        user::contest_platform_shutdown();
    } else {
        // No sdcard: smoke / non-contest boot — stay alive so the smoke
        // runner can capture all markers.
        println!("oscomp: no contest — idle halt");
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
            crate::println!("  files          : version cpuinfo meminfo uptime mounts self");
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
    // NOTE: /lib64 is a real directory under tmpfs, not a symlink to /lib.
    // ld-linux PT_INTERP paths like /lib64/ld-linux-... must resolve to real
    // files installed from ext4 sdcard; a symlink would break that when /lib
    // entries are materialized independently.

    // P9-G2: try vendor LoongArch busybox first (if present).
    #[cfg(all(target_arch = "loongarch64", vendor_la_busybox))]
    {
        let vendor_data: &[u8] = include_bytes!(env!("MYOS_VENDOR_LA_BUSYBOX"));
        if vendor_data.len() > 64 && &vendor_data[..4] == b"\x7fELF" {
            let entry = u64::from_le_bytes(vendor_data[24..32].try_into().unwrap());
            let phnum = u16::from_le_bytes(vendor_data[56..58].try_into().unwrap()) as usize;
            let is_bad = entry == 0x1201b640c && phnum <= 4;
            if is_bad {
                crate::println!("sdcard: vendor LA busybox rejected (known-bad static)");
            } else {
                crate::println!(
                    "sdcard: vendor LA busybox accepted size={} entry={:#x} phnum={}",
                    vendor_data.len(),
                    entry,
                    phnum,
                );
                // Install as VFS regular file using the raw byte content.
                let _ = fs::unlink("/bin/busybox", false);
                oscomp_sdcard_install_bytes("/bin/busybox", vendor_data);
                let _ = fs::symlink("/bin/busybox", "/bin/sh");
                let _ = fs::mkdir("/usr/bin", 0o755);
                let _ = fs::symlink("/bin/busybox", "/usr/bin/env");
                for applet in &[
                    "sh", "cp", "echo", "ls", "mkdir", "test", "cat", "rm", "mv", "sleep", "kill",
                    "head", "tail", "grep", "dd", "mount", "ps", "id", "uname", "df",
                ] {
                    let _ = fs::symlink("/bin/busybox", &alloc::format!("/bin/{}", applet));
                }
            }
        }
    }
    #[cfg(not(all(target_arch = "loongarch64", vendor_la_busybox)))]
    if cfg!(target_arch = "loongarch64") {
        crate::println!("sdcard: vendor LA busybox absent");
    }

    // If vendor busybox is already installed, skip sdcard sources entirely.
    let vendor_installed = fs::stat("/bin/busybox").is_ok() && fs::stat("/bin/sh").is_ok();
    if !vendor_installed {
        // Try all ext4 busybox sources.  On LoongArch some static busybox
        // binaries have unresolved linker relaxation placeholders (andi rX,r0,imm)
        // that cause 0x0 crashes.  Prefer dynamic (PT_INTERP) busybox candidates.
        let busybox_sources: &[&str] = if cfg!(target_arch = "loongarch64") {
            &[
                "/musl/busybox",
                "/glibc/busybox",
                "/busybox",
                "/busybox-static",
                "/bin/busybox",
                "/usr/bin/busybox",
                "/glibc/bin/busybox",
                "/musl/bin/busybox",
            ]
        } else {
            &[
                "/musl/busybox",
                "/busybox",
                "/busybox-static",
                "/bin/busybox",
                "/usr/bin/busybox",
            ]
        };
        for busybox_ext4 in busybox_sources {
            let _ = fs::unlink("/bin/busybox", false);
            oscomp_sdcard_install_ext4_path(busybox_ext4, "/bin/busybox");
            if fs::stat("/bin/busybox").is_err() {
                continue;
            }
            // Verify the installed binary is usable.
            // On LA: reject the known-bad static busybox (phnum=4, entry=0x1201b640c)
            // which has unresolved linker relaxation placeholders.
            let mut ok = false;
            if let Ok(file) = fs::open("/bin/busybox", myos_vfs::OpenFlags::O_RDONLY) {
                let mut hdr = [0u8; 64];
                let mut io = myos_vfs::MutableIoBuffer::new(&mut hdr);
                if file.read(&mut io).is_ok() && io.len() >= 20 && &hdr[..4] == b"\x7fELF" {
                    ok = true;
                    #[cfg(target_arch = "loongarch64")]
                    {
                        let entry = u64::from_le_bytes(hdr[24..32].try_into().unwrap());
                        let phnum = u16::from_le_bytes(hdr[56..58].try_into().unwrap()) as usize;
                        // Reject the known-bad static busybox (ext4 offset 0x8600000).
                        if entry == 0x1201b640c && phnum <= 4 {
                            crate::println!(
                                "sdcard: busybox from {} rejected (bad static, entry={:#x} phnum={})",
                                busybox_ext4,
                                entry,
                                phnum,
                            );
                            ok = false;
                        }
                    }
                }
            }
            if !ok {
                continue;
            }
            crate::println!(
                "sdcard: shell {} selected ({})",
                busybox_ext4,
                if cfg!(target_arch = "loongarch64") {
                    "LA"
                } else {
                    "RV"
                },
            );
            let _ = fs::symlink("/bin/busybox", "/bin/sh");
            let _ = fs::mkdir("/usr/bin", 0o755);
            let _ = fs::symlink("/bin/busybox", "/usr/bin/env");
            for applet in &[
                "cp", "sleep", "kill", "cat", "echo", "mv", "ln", "rm", "ls", "mkdir", "chmod",
                "grep", "dd", "mount", "ps", "head", "tail", "test", "awk", "sed", "wc", "cut",
                "tr", "which", "pidof", "printenv", "basename", "dirname", "readlink", "stat",
                "getopt", "env", "sh", "id", "uname", "df",
            ] {
                let _ = fs::symlink("/bin/busybox", &alloc::format!("/bin/{}", applet));
            }
            break;
        }
        // If no usable shell was found, install the first busybox source anyway
        // in degraded mode.  Without /bin/sh, all no-shebang scripts fail ENOENT
        // immediately.  A known-bad static busybox may still run simple commands
        // before crashing; that's better than 100% ENOENT.
        // If no good shell was found (all rejected) but a busybox VFS node
        // exists, install /bin/sh in degraded mode.  Check /bin/sh existence,
        // not /bin/busybox — the last rejected install leaves a VFS node.
        if fs::stat("/bin/sh").is_err() {
            if fs::stat("/bin/busybox").is_ok() {
                crate::println!(
                    "sdcard: WARNING shell degraded mode — using known-bad static LA busybox"
                );
                let _ = fs::symlink("/bin/busybox", "/bin/sh");
                let _ = fs::mkdir("/usr/bin", 0o755);
                let _ = fs::symlink("/bin/busybox", "/usr/bin/env");
                for applet in &[
                    "sh", "cp", "echo", "ls", "mkdir", "test", "cat", "rm", "mv", "sleep",
                ] {
                    let _ = fs::symlink("/bin/busybox", &alloc::format!("/bin/{}", applet));
                }
            } else {
                crate::println!("sdcard: WARNING no shell found — shell-script tests will fail");
            }
        }
    } // if !vendor_installed

    // P1-A: install ld-linux / ld-musl interpreters from their real ext4
    // source paths (/glibc/lib/... or /musl/lib/...) into the canonical
    // VFS paths that PT_INTERP encodes (/lib/..., /lib64/...).
    oscomp_install_runtime_loader_aliases();
    oscomp_install_runtime_lib_aliases();

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
        let Ok(entries) = crate::ext4::list_directory(alloc::sync::Arc::clone(&device), &dir)
        else {
            continue;
        };
        for entry in entries {
            let child = alloc::format!("{}/{}", dir.trim_end_matches('/'), entry.name);
            if entry.file_type == EXT4_FT_DIR {
                if dirs.len() < MAX_SCAN_DIRS {
                    dirs.push(child);
                }
            } else if oscomp_sdcard_is_test_script(&child) && test_scripts.len() < MAX_TEST_SCRIPTS
            {
                test_scripts.push(child);
            }
        }
    }

    let mut installed_scripts: alloc::vec::Vec<alloc::string::String> = alloc::vec::Vec::new();

    // Install each test script individually.
    for ext4_path in &test_scripts {
        let vfs_path = alloc::format!("/mnt/sdcard{}", ext4_path);
        oscomp_sdcard_install_ext4_path(ext4_path, &vfs_path);
        if fs::stat(&vfs_path).is_ok() {
            installed_scripts.push(vfs_path.trim_start_matches('/').into());
        }
    }

    // P1-B: materialize key ext4 directories into VFS so scripts can
    // find ./busybox, ./lua, ./lmbench_all, ./iperf3 etc.
    // Must happen *after* /mnt/sdcard skeleton exists but *before*
    // any script runs.
    oscomp_materialize_ext4_dir_flat("/glibc", "/mnt/sdcard/glibc", 512, 0);
    oscomp_materialize_ext4_dir_flat("/glibc/lib", "/mnt/sdcard/glibc/lib", 256, 0);
    oscomp_materialize_ext4_dir_flat("/glibc/basic", "/mnt/sdcard/glibc/basic", 256, 0);
    oscomp_materialize_ext4_dir_flat("/glibc/lua", "/mnt/sdcard/glibc/lua", 128, 0);
    oscomp_materialize_ext4_dir_flat("/glibc/ltp", "/mnt/sdcard/glibc/ltp", 256, 0);
    oscomp_materialize_ext4_dir_flat("/glibc/lmbench", "/mnt/sdcard/glibc/lmbench", 128, 0);
    oscomp_materialize_ext4_dir_flat("/musl", "/mnt/sdcard/musl", 512, 0);
    oscomp_materialize_ext4_dir_flat("/musl/lib", "/mnt/sdcard/musl/lib", 256, 0);
    oscomp_materialize_ext4_dir_flat("/musl/basic", "/mnt/sdcard/musl/basic", 256, 0);
    oscomp_materialize_ext4_dir_flat("/musl/lua", "/mnt/sdcard/musl/lua", 128, 0);
    oscomp_materialize_ext4_dir_flat("/musl/ltp", "/mnt/sdcard/musl/ltp", 256, 0);
    oscomp_materialize_ext4_dir_flat("/musl/lmbench", "/mnt/sdcard/musl/lmbench", 128, 0);

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

/// Install raw bytes as a VFS regular file (for vendor/userland binaries).
fn oscomp_sdcard_install_bytes(vfs_path: &str, data: &[u8]) {
    oscomp_sdcard_ensure_parent_dirs(vfs_path);
    let _ = fs::install_bytes(vfs_path, data);
}

/// P1-B: materialize an ext4 directory flat into VFS (files only; subdirs
/// created as empty directories).  Returns the number of files installed.
///
/// P1-D fix: do NOT mkdir(vfs_dir) before checking ext4 type — if ext4 path
/// is a regular file (e.g. /glibc/lua), creating a directory first pollutes
/// the VFS namespace and causes "Is a directory" errors later.
///
/// P1-E fix: also count files that already exist as "available", and log both
/// counts separately so "0 newly installed" doesn't look like a regression.
fn oscomp_materialize_ext4_dir_flat(
    ext4_dir: &str,
    vfs_dir: &str,
    max_files: usize,
    recurse_levels: usize,
) -> usize {
    let Some(device) = crate::block::open_device("vda") else {
        crate::println!("sdcard: expand {} — no device", ext4_dir);
        return 0;
    };

    const EXT4_FT_DIR: u16 = 2;
    let Ok(entries) = crate::ext4::list_directory(alloc::sync::Arc::clone(&device), ext4_dir)
    else {
        // ext4_dir may be a regular file (e.g. /glibc/lua), not a directory.
        // Install it as a regular file instead of creating a false directory.
        oscomp_sdcard_install_ext4_path(ext4_dir, vfs_dir);
        if fs::stat(vfs_dir).is_ok() {
            crate::println!(
                "sdcard: installed {} -> {} (regular file)",
                ext4_dir,
                vfs_dir
            );
            return 1;
        }
        crate::println!("sdcard: expand failed {} -> {}", ext4_dir, vfs_dir);
        return 0;
    };

    // Only create the VFS directory after confirming ext4_dir is a directory.
    oscomp_sdcard_ensure_parent_dirs(vfs_dir);
    let _ = fs::mkdir(vfs_dir, 0o755);

    let mut newly_installed: usize = 0;
    let mut already_available: usize = 0;
    for entry in entries {
        if newly_installed + already_available >= max_files {
            break;
        }
        let ext4_child = alloc::format!("{}/{}", ext4_dir.trim_end_matches('/'), entry.name);
        let vfs_child = alloc::format!("{}/{}", vfs_dir.trim_end_matches('/'), entry.name);

        if entry.file_type == EXT4_FT_DIR {
            oscomp_sdcard_ensure_parent_dirs(&vfs_child);
            let _ = fs::mkdir(&vfs_child, 0o755);
            already_available += 1;
            // Expand one more level so that rustlib/riscv64gc-.../
            // has lib/ populated with .rlib files visible to rustc.
            if recurse_levels > 0 {
    oscomp_materialize_ext4_dir_flat(&ext4_child, &vfs_child, max_files, recurse_levels - 1);
}
            continue;
        }

        if fs::stat(&vfs_child).is_ok() {
            already_available += 1;
            continue;
        }

        oscomp_sdcard_install_ext4_path(&ext4_child, &vfs_child);
        if fs::stat(&vfs_child).is_ok() {
            newly_installed += 1;
        }
    }

    crate::println!(
        "sdcard: expanded {} -> {} : {} newly installed, {} already available",
        ext4_dir,
        vfs_dir,
        newly_installed,
        already_available,
    );
    newly_installed + already_available
}

/// Lazy on-demand materialize: when execve/open/stat encounters ENOENT
/// on a path under /mnt/sdcard, try installing the parent directory's
/// children from ext4 before giving up.
pub fn ensure_sdcard_dir_materialized(vfs_path: &str) -> bool {
    let rel = match vfs_path.strip_prefix("/mnt/sdcard/") {
        Some(r) if !r.is_empty() => r,
        _ => return false,
    };

    let parent = rel.rsplit_once('/').map_or("", |(parent, _)| parent);
    let mut ext4_dir = alloc::string::String::from("/");
    let mut vfs_dir = alloc::string::String::from("/mnt/sdcard");

    // Directory snapshots initially contain only empty placeholders for
    // children. Walk from the mounted root so deeply nested toolchain paths
    // such as /usr/libexec/gcc/... can be populated on first access.
    for component in parent.split('/').filter(|component| !component.is_empty()) {
        let next_vfs = alloc::format!("{}/{}", vfs_dir, component);
        if crate::fs::stat(&next_vfs).is_err() {
            oscomp_materialize_ext4_dir_flat(&ext4_dir, &vfs_dir, 4096, 2);
        }
        if crate::fs::stat(&next_vfs).is_err() {
            return false;
        }

        if ext4_dir == "/" {
            ext4_dir.push_str(component);
        } else {
            ext4_dir.push('/');
            ext4_dir.push_str(component);
        }
        vfs_dir = next_vfs;
    }

    let count = oscomp_materialize_ext4_dir_flat(&ext4_dir, &vfs_dir, 4096, 2);
    count > 0
}

/// P1-A: install ELF dynamic linker (ld-linux / ld-musl) from their real
/// ext4 source paths into the canonical /lib and /lib64 VFS paths that
/// PT_INTERP encodes.
fn oscomp_install_runtime_loader_aliases() {
    const ALIASES: &[(&str, &[&str])] = &[
        ("/glibc/lib/ld-linux-riscv64-lp64d.so.1", &[
            "/lib/ld-linux-riscv64-lp64d.so.1",
            "/lib64/ld-linux-riscv64-lp64d.so.1",
        ]),
        ("/glibc/lib/ld-linux-riscv64-lp64.so.1", &[
            "/lib/ld-linux-riscv64-lp64.so.1",
            "/lib64/ld-linux-riscv64-lp64.so.1",
        ]),
        ("/glibc/lib/ld-linux-loongarch-lp64d.so.1", &[
            "/lib/ld-linux-loongarch-lp64d.so.1",
            "/lib/ld-linux-loongarch64-lp64d.so.1",
            "/lib64/ld-linux-loongarch-lp64d.so.1",
            "/lib64/ld-linux-loongarch64-lp64d.so.1",
        ]),
        ("/glibc/lib/ld-linux-loongarch-lp64.so.1", &[
            "/lib/ld-linux-loongarch-lp64.so.1",
            "/lib/ld-linux-loongarch64-lp64.so.1",
            "/lib64/ld-linux-loongarch-lp64.so.1",
            "/lib64/ld-linux-loongarch64-lp64.so.1",
        ]),
        ("/musl/lib/ld-musl-riscv64.so.1", &[
            "/lib/ld-musl-riscv64.so.1",
            "/lib64/ld-musl-riscv64.so.1",
        ]),
        ("/musl/lib/ld-musl-riscv64-sf.so.1", &[
            "/lib/ld-musl-riscv64-sf.so.1",
            "/lib64/ld-musl-riscv64-sf.so.1",
        ]),
        ("/musl/lib/ld-musl-loongarch64.so.1", &[
            "/lib/ld-musl-loongarch64.so.1",
            "/lib64/ld-musl-loongarch64.so.1",
        ]),
        ("/musl/lib/ld-musl-loongarch64-sf.so.1", &[
            "/lib/ld-musl-loongarch64-sf.so.1",
            "/lib64/ld-musl-loongarch64-sf.so.1",
        ]),
        // B1-B2: musl LoongArch LP64D variants — the exact name that
        // PT_INTERP encodes on many LA musl binaries.
        ("/musl/lib/ld-musl-loongarch-lp64d.so.1", &[
            "/lib/ld-musl-loongarch-lp64d.so.1",
            "/lib64/ld-musl-loongarch-lp64d.so.1",
        ]),
        // musl LA: ld-musl-loongarch64-lp64d name variant.
        ("/musl/lib/ld-musl-loongarch64-lp64d.so.1", &[
            "/lib/ld-musl-loongarch64-lp64d.so.1",
            "/lib64/ld-musl-loongarch64-lp64d.so.1",
        ]),
        // Fallback: if there's no separate ld-musl, the musl libc.so
        // itself can act as the dynamic linker.
        ("/musl/lib/libc.so", &[
            "/lib/ld-musl-loongarch-lp64d.so.1",
            "/lib64/ld-musl-loongarch-lp64d.so.1",
            "/lib/ld-musl-loongarch64-lp64d.so.1",
        ]),
    ];

    for (src, dsts) in ALIASES {
        for dst in *dsts {
            oscomp_sdcard_install_ext4_path(src, dst);
        }
    }
    crate::println!("sdcard: installed runtime loader aliases");
}

/// P1-A: install common glibc/musl shared libraries from ext4 /glibc/lib
/// and /musl/lib into /lib, /lib64, and /usr/lib so ld-linux can find
/// them via openat.
fn oscomp_install_runtime_lib_aliases() {
    const LIBS: &[&str] = &[
        "libc.so.6",
        "libpthread.so.0",
        "libdl.so.2",
        "librt.so.1",
        "libm.so.6",
        "libresolv.so.2",
        "libutil.so.1",
        "libnsl.so.1",
        "libcrypt.so.1",
        "libgcc_s.so.1",
        "libstdc++.so.6",
        "libc.so",
        "libm.so",
        "libpthread.so",
    ];

    let mut installed: usize = 0;
    for name in LIBS {
        let glibc_src = alloc::format!("/glibc/lib/{}", name);
        let musl_src = alloc::format!("/musl/lib/{}", name);

        let lib_dst = alloc::format!("/lib/{}", name);
        let lib64_dst = alloc::format!("/lib64/{}", name);
        let usr_dst = alloc::format!("/usr/lib/{}", name);

        // Prefer glibc; fall back to musl only if glibc source is absent.
        oscomp_sdcard_install_ext4_path(&glibc_src, &lib_dst);
        oscomp_sdcard_install_ext4_path(&glibc_src, &lib64_dst);
        oscomp_sdcard_install_ext4_path(&glibc_src, &usr_dst);

        if fs::stat(&lib_dst).is_err() {
            oscomp_sdcard_install_ext4_path(&musl_src, &lib_dst);
        }
        if fs::stat(&lib64_dst).is_err() {
            oscomp_sdcard_install_ext4_path(&musl_src, &lib64_dst);
        }
        if fs::stat(&usr_dst).is_err() {
            oscomp_sdcard_install_ext4_path(&musl_src, &usr_dst);
        }

        if fs::stat(&lib_dst).is_ok() || fs::stat(&lib64_dst).is_ok() {
            installed += 1;
        }
    }
    crate::println!("sdcard: installed {} runtime library aliases", installed);

    // Self-check: verify a few critical aliases are real readable files
    // AND that the VFS read path actually returns the correct bytes.
    for check in &[
        "/lib/ld-linux-riscv64-lp64d.so.1",
        "/lib64/ld-linux-loongarch-lp64d.so.1",
        "/lib/libc.so.6",
        "/lib/libm.so.6",
    ] {
        match fs::open(check, myos_vfs::OpenFlags::O_RDONLY) {
            Ok(file) => {
                let stat = match file.fstat() {
                    Ok(s) => s,
                    Err(_) => {
                        crate::println!("sdcard: alias {} exists but fstat failed", check);
                        continue;
                    }
                };
                // Read first 64 bytes to verify the data path works.
                let mut buf = [0u8; 64];
                let mut io = myos_vfs::MutableIoBuffer::new(&mut buf);
                let read_ret = file.read(&mut io);
                let magic = if io.len() >= 4 {
                    alloc::format!(
                        "{:02x}{:02x}{:02x}{:02x}",
                        io.filled_bytes()[0],
                        io.filled_bytes()[1],
                        io.filled_bytes()[2],
                        io.filled_bytes()[3]
                    )
                } else {
                    alloc::string::String::from("????")
                };
                crate::println!(
                    "sdcard: alias {} size={} mode={:#o} read={} magic={}",
                    check,
                    stat.size,
                    stat.mode,
                    read_ret.as_ref().map_or_else(
                        |e| alloc::format!("err({})", e.to_isize()),
                        |n| alloc::format!("{}", n)
                    ),
                    magic,
                );
            }
            Err(_) => {
                // Not fatal — the file may not be on this arch's sdcard.
            }
        }
    }
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

pub(crate) static SCANNED_TEST_SCRIPTS: crate::irq_lock::IrqSpinLock<
    alloc::vec::Vec<alloc::string::String>,
> = crate::irq_lock::IrqSpinLock::new_with_class(
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
