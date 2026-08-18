#![feature(let_chains)]
#![feature(unsigned_is_multiple_of)]
#![feature(alloc_error_handler)]
#![no_std]
#![no_main]

mod block;
mod boot_ramdisk;
mod bootargs;
mod call_function;
mod console;
mod context;
#[cfg(feature = "platform-ls2k1000")]
mod cusb;
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
mod mmc;
mod net;
mod oscomp;
mod page_alloc;
mod panic;
mod partition;
mod pipe;
mod process;
mod procfs;
mod rng;
mod rtc;
mod runtime_page_table;
mod signal;
mod smp;
mod storage;
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
///
/// 四个参数按各架构启动约定传递（riscv64/loongarch64 qemu 只用前三个；
/// LS2K1000 厂商 bootm 的 `CONFIG_LOONGSON_BOOT_FIXUP` 把 FDT 放 $a3）。
#[unsafe(no_mangle)]
pub extern "C" fn rust_entry(arg0: usize, arg1: usize, arg2: usize, arg3: usize) -> ! {
    arch::smp::set_current_cpu_id(smp::CpuId::BOOT.get());
    let boot = arch::boot::from_raw(arg0, arg1, arg2, arg3).into_boot_info();

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

#[cfg(target_arch = "loongarch64")]
fn direct_boot_oscomp_mode(boot: &BootInfo) -> Option<oscomp::RunMode> {
    oscomp::mode_from_bootargs(direct_boot_command_line(boot).as_deref())
}

/// Read the QEMU direct-boot command line from the boot protocol pointer.
#[cfg(target_arch = "loongarch64")]
fn direct_boot_command_line(boot: &BootInfo) -> Option<alloc::string::String> {
    const MAX_COMMAND_LINE_BYTES: usize = 1024;

    let base = boot.command_line()?.get();
    let mut bytes = [0_u8; MAX_COMMAND_LINE_BYTES];
    let mut length = 0_usize;
    while length < bytes.len() {
        let address = base.checked_add(length)?;
        let pointer =
            arch::memory::phys_access::ram_ptr::<u8>(myos_mm::PhysAddr::new(address)).ok()?;
        // SAFETY: QEMU's direct-boot protocol owns this bounded, NUL-terminated
        // command-line buffer and keeps it alive for the duration of boot.
        let byte = unsafe { core::ptr::read_volatile(pointer) };
        if byte == 0 {
            return core::str::from_utf8(&bytes[..length]).ok().map(Into::into);
        }
        bytes[length] = byte;
        length += 1;
    }
    None
}

#[cfg(target_arch = "riscv64")]
fn direct_boot_oscomp_mode(_boot: &BootInfo) -> Option<oscomp::RunMode> {
    None
}

#[cfg(target_arch = "riscv64")]
fn direct_boot_command_line(_boot: &BootInfo) -> Option<alloc::string::String> {
    None
}

/// How the kernel should hand control to userland once the VFS is ready.
///
/// `SelfTest` keeps the existing M8/M9/M10 + BusyBox + oscomp self-test
/// sequence (the historical behaviour on qemu and the stage-4 DTB). When the
/// bootargs contain `rdinit=/init`, the kernel instead skips every test that
/// would allocate a throwaway Process and boots the real `/init` as PID 1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UserlandBootMode {
    SelfTest,
    InitramfsInit,
}

fn userland_boot_mode(bootargs: Option<&str>) -> UserlandBootMode {
    let mut mode = UserlandBootMode::SelfTest;
    if let Some(arguments) = bootargs {
        for word in arguments.split_whitespace() {
            if let Some(init) = word.strip_prefix("rdinit=") {
                // Accept both the canonical "/init" and a bare "init" spelling.
                if init == "/init" || init == "init" {
                    mode = UserlandBootMode::InitramfsInit;
                }
            }
        }
    }
    mode
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
        userland_mode,
        contest_config,
        boot_ramdisks,
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
        mmc::discover_hosts(&tree);
        let max_cpus = match bootargs::max_cpus(tree.bootargs()) {
            Ok(Some(requested)) => {
                println!("SMP: maxcpus requested={requested}");
                requested
            }
            Ok(None) => smp::MAX_CPUS,
            Err(error) => panic!("invalid boot argument: {error}"),
        };
        smp::initialize(&tree, boot_hardware_cpu_id(&boot), max_cpus);

        let firmware_timer_frequency = tree.timebase_frequency_hz();
        let explicit_oscomp_mode =
            oscomp::mode_from_bootargs(tree.bootargs()).or_else(|| direct_boot_oscomp_mode(&boot));
        let direct_command_line = direct_boot_command_line(&boot);
        let userland_mode = userland_boot_mode(tree.bootargs().or(direct_command_line.as_deref()));
        let contest_config =
            storage::config_from_bootargs(tree.bootargs().or(direct_command_line.as_deref()));
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

        // 固件加载的竞赛镜像区域（LS2K1000 U-Boot → ram0）。这些区域由
        // build_boot_memory_layout 通过 /reserved-memory 从 free memory
        // 中排除；这里收集其细节供注册只读块设备。
        let mut boot_ramdisks: alloc::vec::Vec<myos_fdt::BootRamdiskRegion> =
            alloc::vec::Vec::new();
        tree.for_each_boot_ramdisk(|region| {
            boot_ramdisks.push(region);
        })
        .unwrap_or_else(|error| {
            panic!("failed to parse boot ramdisk regions: {error}");
        });

        (
            memory_layout,
            firmware_timer_frequency,
            virtio_regions,
            pci_hosts,
            initrd_range,
            explicit_oscomp_mode,
            userland_mode,
            contest_config,
            boot_ramdisks,
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
    #[cfg(feature = "platform-ls2k1000")]
    {
        // 真机确认异常向量已页对齐：expected 必须 4 KiB 对齐、installed 回读
        // 必须等于 expected（硬件把 EENTRY[11:0] 清零）、ECFG.VS 必须为 0。
        // 修复前这里会打印 installed 与 expected 不同的错位地址。
        let expected = crate::arch::trap::ls2k_eentry_expected();
        let installed = crate::arch::trap::ls2k_eentry_installed();
        let vs = (crate::arch::trap::ls2k_ecfg() >> 16) & 0x7;
        crate::console::raw::puts("TRAP-VECTOR expected=");
        crate::console::raw::puthex(expected);
        crate::console::raw::puts(" installed=");
        crate::console::raw::puthex(installed);
        crate::console::raw::puts(" vs=");
        crate::console::raw::putdec(vs);
        if expected & 0xfff == 0 && installed == expected && vs == 0 {
            crate::console::raw::puts(" PASS\n");
        } else {
            /*
             * 异常向量仍错位：这是板上不可恢复的致命状态。继续启动会像伪 OOM
             * 一样产生误导性故障，因此关中断后直接停机。
             */
            crate::console::raw::puts(" FAIL\n");
            crate::arch::interrupt::disable();
            loop {
                crate::arch::cpu::wait_for_interrupt();
            }
        }

        // 共享 trap body 的 breakpoint 自测：只在诊断构建（boot-selftest 特性）
        // 启用，稳定基线不执行。
        #[cfg(feature = "boot-selftest")]
        {
            crate::arch::trap::trigger_breakpoint();
            crate::console::raw::puts("BREAKPOINT-TRAP PASS\n");
        }
    }
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
    register_boot_ramdisks(&boot_ramdisks);
    mmc::initialize_storage();
    // LS2K1000 USB 早期轮询探针（M0–M9）。scheduler 尚未初始化，只做有界
    // 轮询 MMIO 探测，绝不 spawn 线程（CherryUSB 的 psc/hpworkq/lpworkq
    // 线程化初始化见 `late_start()`，在 scheduler 就绪后触发——否则
    // usbh_initialize 内部 spawn 会撞 "kernel scheduler is not initialized"）。
    #[cfg(feature = "platform-ls2k1000")]
    cusb::early_probe();
    // 在所有基础块设备注册完成后、devfs 建立前，统一扫描并注册 GPT/MBR
    // 分区（vdaN/ram0pN/mmcblk1pN），使分区设备出现在 /dev 树且存储选择
    // 能自动降级到 ext4 分区（K1.1）。
    partition::register_all_partitions();
    fs::initialize();
    mount_proc();
    mount_sys();
    install_external_initramfs(initrd_range);
    mount_sdcard_if_present(&contest_config);
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
    boot_ramdisk::verify();
    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    storage::verify();
    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    partition::verify();
    // mmc::verify 只跑 mock 逻辑（无硬件/架构依赖），riscv64 也执行，
    // 便于在 QEMU 上验证 DW-MMC 状态机。
    #[cfg(debug_assertions)]
    mmc::verify();
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

    /*
     * 调度器生命周期顺序：先构造/发布/注册 Scheduler，再启动周期定时器并
     * 开启中断，最后标记 BSP scheduler active 并孵化 reaper。调度定时器
     * 绝不在 Scheduler 发布之前启动。
     */
    task::initialize();
    time::start_periodic();
    task::start_boot_scheduler();

    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    time::verify_periodic();

    smp::start_secondaries();
    task::finalize_cpu_bringup();
    // LS2K1000 USB 线程化初始化：scheduler 已就绪、副核在线，此时才能
    // spawn CherryUSB 的 psc/hpworkq/lpworkq 线程。专用线程异步执行，失败
    // 只打日志继续启动（USB 探测失败可接受，不能挡在 /init 之前）。
    #[cfg(feature = "platform-ls2k1000")]
    cusb::late_start();
    println!("BOOT11 all-ap-online");
    #[cfg(feature = "platform-ls2k1000")]
    {
        // 板级 per-CPU 中断路径检查（真实硬件）。
        //
        // 注意：启动检查窗口内只有 boot CPU 保持调度 tick。副核进入
        // tickless NO_HZ idle 后，time::enter_idle 会在本地无软件定时器时
        // shutdown 掉硬件定时器（TCFG=0），因此副核在 idle 期间收不到
        // timer IRQ——这是有意设计，不是故障。所以：
        //   * boot CPU：用 timer IRQ 验证（tick 保持 armed）；
        //   * 副核：用真实 reschedule IPI 往返验证——唤醒 idle → trap
        //     entry → ECODE_INTERRUPT → IPI 分发 → acknowledge，覆盖与
        //     timer IRQ 完全相同的异常入口与中断分发代码。
        let discovered = crate::smp::discovered_cpu_count();
        let boot = crate::smp::CpuId::BOOT;

        // 1) boot CPU 定时器路径：等待首个 timer IRQ（启动检查期间
        //    boot CPU 的 tick 不会进入 NO_HZ，计数必然增长）。
        let mut waits = 0u32;
        while crate::trap::ls2k_timer_irq_count(0) == 0 && waits < 50 {
            crate::arch::cpu::wait_for_interrupt();
            waits += 1;
        }

        // 2) 副核中断路径：向每个副核发送真实 IPI 并等待接收确认。
        for logical in 1..discovered {
            let cpu = crate::smp::CpuId::new(logical).expect("discovered CPU exceeds MAX_CPUS");
            let mut round = 0u32;
            while crate::ipi::interrupt_count(cpu) == 0 && round < 50 {
                crate::smp::send_ipi(cpu);
                crate::arch::cpu::wait_for_interrupt();
                round += 1;
            }
        }

        for logical in 0..discovered {
            let cpu = crate::smp::CpuId::new(logical).expect("discovered CPU exceeds MAX_CPUS");
            crate::console::raw::puts("CPU-CNTR cpu=");
            crate::console::raw::putdec(logical);
            crate::console::raw::puts(" timer=");
            crate::console::raw::putdec(crate::trap::ls2k_timer_irq_count(logical) as usize);
            crate::console::raw::puts(" ipi_recv=");
            crate::console::raw::putdec(crate::ipi::interrupt_count(cpu) as usize);
            crate::console::raw::puts(" ipi_send=");
            crate::console::raw::putdec(crate::ipi::ls2k_ipi_send_count(cpu) as usize);
            crate::console::raw::puts("\n");
        }
        assert!(
            crate::trap::ls2k_timer_irq_count(0) > 0,
            "boot CPU timer IRQ never arrived",
        );
        assert!(
            (1..discovered).all(|logical| {
                let cpu = crate::smp::CpuId::new(logical).expect("discovered CPU exceeds MAX_CPUS");
                crate::ipi::interrupt_count(cpu) > 0
            }),
            "secondary CPU IPI round-trip did not reach every online CPU",
        );
        assert!(
            crate::ipi::ls2k_ipi_send_count(boot) >= discovered as u64 - 1,
            "boot CPU did not issue the secondary verification IPIs",
        );
        crate::console::raw::puts("CPU-COUNTERS PASS\n");
    }
    #[cfg(feature = "platform-visionfive2")]
    {
        // 板级 per-CPU timer/IPI 中断路径检查（真实硬件）。
        //
        // 与 ls2k1000 相同的模型：启动检查窗口内只有 boot CPU 保持调度
        // tick。副核进入 tickless NO_HZ idle 后，time::enter_idle 会在本地
        // 无软件定时器时 shutdown 硬件定时器，因此副核收不到 timer IRQ——
        // 这是有意设计，不是故障。所以：
        //   * boot CPU：用 timer IRQ 验证（tick 保持 armed）；
        //   * 副核：用真实 reschedule IPI 往返验证——唤醒 idle → trap
        //     入口 → SUPERVISOR_SOFTWARE → IPI 分发 → acknowledge，覆盖与
        //     timer IRQ 完全相同的异常入口与中断分发代码。
        let discovered = crate::smp::discovered_cpu_count();

        // 1) boot CPU 定时器路径：等待首个 timer IRQ（启动检查期间
        //    boot CPU 的 tick 不会进入 NO_HZ，计数必然增长）。
        let mut waits = 0u32;
        while crate::irq::timer_irq_count(0) == 0 && waits < 50 {
            crate::arch::cpu::wait_for_interrupt();
            waits += 1;
        }

        // 2) 副核中断路径：向逻辑 CPU 1..N 各发送真实 IPI 并等待接收确认。
        for logical in 1..discovered {
            let cpu = crate::smp::CpuId::new(logical).expect("discovered CPU exceeds MAX_CPUS");
            let mut round = 0u32;
            while crate::ipi::interrupt_count(cpu) == 0 && round < 50 {
                crate::smp::send_ipi(cpu);
                crate::arch::cpu::wait_for_interrupt();
                round += 1;
            }
        }

        for logical in 0..discovered {
            let cpu = crate::smp::CpuId::new(logical).expect("discovered CPU exceeds MAX_CPUS");
            crate::println!(
                "CPU-CNTR cpu={} timer={} ipi_recv={}",
                logical,
                crate::irq::timer_irq_count(logical),
                crate::ipi::interrupt_count(cpu),
            );
        }
        assert!(
            crate::irq::timer_irq_count(0) > 0,
            "boot CPU timer IRQ never arrived",
        );
        assert!(
            (1..discovered).all(|logical| {
                let cpu = crate::smp::CpuId::new(logical).expect("discovered CPU exceeds MAX_CPUS");
                crate::ipi::interrupt_count(cpu) > 0
            }),
            "secondary CPU IPI round-trip did not reach every online CPU",
        );
        crate::println!("CPU-COUNTERS PASS");
    }
    workqueue::initialize();
    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    timer::verify();
    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    workqueue::verify();
    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    tracked_spin::verify();

    #[cfg(all(debug_assertions, not(target_arch = "riscv64")))]
    task::verify();

    // Gate B: an explicit rdinit= bootarg routes the kernel to the real
    // /init instead of the self-test sequence. The split must happen before
    // user::verify() so no throwaway test Process can consume PID 1.
    match userland_mode {
        UserlandBootMode::InitramfsInit => {
            println!("userland boot: rdinit=/init");
            // Feed UART RX bytes into the console TTY before init starts;
            // BusyBox askfirst blocks on /dev/console reads immediately.
            crate::console::start_uart_input_poller();
            // init_supervisor never returns: it publishes /init as PID 1,
            // arms the PID 1 exit monitor, and enters the boot idle loop.
            user::init_supervisor("/init");
        }
        UserlandBootMode::SelfTest => {
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

fn mount_sdcard_if_present(config: &storage::ContestStorageConfig) {
    let selected = match storage::select_device(config) {
        Ok(Some(selected)) => selected,
        Ok(None) => {
            crate::println!("sdcard: no contest storage found — skipping mount");
            return;
        }
        Err(error) => {
            crate::println!("sdcard: contest storage selection failed: {error:?}");
            return;
        }
    };
    let device = selected.device();
    let device_name = alloc::string::String::from(selected.name());
    if let Err(error) = storage::mount_selected(&selected) {
        crate::println!("sdcard: contest storage mount failed: {error:?}");
        return;
    }
    contest_fixture_probe(&device);
    install_sdcard_contents(&device, &device_name);
}

/// C5: 把 FDT 声明的固件加载竞赛镜像注册为只读块设备（`ram0`）。
///
/// 必须在 `fs::initialize()` 之前调用，使 `/dev/ram0` 在 devfs 建立时
/// 可见。区域已由 `memory::build_boot_memory_layout` 从 free memory 排除。
fn register_boot_ramdisks(regions: &[myos_fdt::BootRamdiskRegion]) {
    for region in regions {
        crate::println!(
            "LS2K-RAMDISK00 region=[{:#018x}, {:#018x}) size={} block-size={} read-only={}",
            region.base(),
            region.end().unwrap_or(usize::MAX),
            region.size(),
            region.block_size(),
            region.read_only(),
        );
        match crate::boot_ramdisk::register_boot_ramdisk(
            myos_mm::PhysAddr::new(region.base()),
            region.size(),
            region.block_size(),
        ) {
            Ok(()) => {
                crate::println!("LS2K-RAMDISK01 registered=/dev/ram0");
            }
            Err(error) => {
                crate::println!("ramdisk: register ram0 failed: {error:?}");
            }
        }
    }
}

/// C3: 识别自动生成的竞赛 fixture（root 含 `/SUDOOS_CONTEST_FIXTURE`）并打印
/// 验收标记。这把存储链（BlockDevice → ext4 → VFS 挂载/读取）在 QEMU 上
/// 验证出来，不需要正式评测镜像。
fn contest_fixture_probe(device: &alloc::sync::Arc<dyn crate::block::BlockDevice>) {
    let Ok(entries) = crate::ext4::list_directory(alloc::sync::Arc::clone(device), "/") else {
        return;
    };
    if !entries
        .iter()
        .any(|entry| entry.name == "SUDOOS_CONTEST_FIXTURE")
    {
        return;
    }
    crate::println!("CONTEST_FIXTURE: arch={}", arch::ARCH_NAME);
    let required = [
        "/SUDOOS_CONTEST_FIXTURE",
        "/arch",
        "/glibc/cagent_testcode.sh",
        "/musl/cagent_testcode.sh",
        "/work/tgoskits/Cargo.toml",
    ];
    let mut missing = 0_usize;
    for path in required {
        let ok = crate::ext4::load_path_snapshot(alloc::sync::Arc::clone(device), path).is_ok();
        crate::println!(
            "CONTEST_FIXTURE: path={} {}",
            path,
            if ok { "present" } else { "missing" },
        );
        if !ok {
            missing += 1;
        }
    }
    crate::println!("CONTEST_FIXTURE: paths-missing={}", missing);
    if missing == 0 {
        crate::println!("FIXTURE_OSCOMP_PASS");
    } else {
        crate::println!("FIXTURE_OSCOMP_FAIL missing={}", missing);
    }
}

/// Install the ext4 contest image contents (busybox, libs, scripts, dirs)
/// into the VFS /mnt/sdcard tree. Runs only after a device was selected and
/// mounted by `storage`; the whole body is device-agnostic.
fn install_sdcard_contents(
    device: &alloc::sync::Arc<dyn crate::block::BlockDevice>,
    device_name: &str,
) {
    let root_entries = match crate::ext4::list_directory(alloc::sync::Arc::clone(device), "/") {
        Ok(entries) => entries,
        Err(error) => {
            // CLOUD_EXT4_MOUNT_FAILURE_V1
            crate::println!("sdcard: failed to list ext4 root directory: {error:?}");
            crate::println!(
                "sdcard: final-all fallback remains enabled; inspect ext4-super diagnostics above"
            );
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
            oscomp_sdcard_install_ext4_path(device_name, busybox_ext4, "/bin/busybox");
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
    oscomp_install_runtime_loader_aliases(device_name);
    oscomp_install_runtime_lib_aliases(device_name);

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
        let Ok(entries) = crate::ext4::list_directory(alloc::sync::Arc::clone(device), &dir) else {
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
        oscomp_sdcard_install_ext4_path(device_name, ext4_path, &vfs_path);
        if fs::stat(&vfs_path).is_ok() {
            installed_scripts.push(vfs_path.trim_start_matches('/').into());
        }
    }

    // P1-B: materialize key ext4 directories into VFS so scripts can
    // find ./busybox, ./lua, ./lmbench_all, ./iperf3 etc.
    // Must happen *after* /mnt/sdcard skeleton exists but *before*
    // any script runs.
    oscomp_materialize_ext4_dir_flat(device, device_name, "/glibc", "/mnt/sdcard/glibc", 512, 0);
    oscomp_materialize_ext4_dir_flat(
        device,
        device_name,
        "/glibc/lib",
        "/mnt/sdcard/glibc/lib",
        256,
        0,
    );
    oscomp_materialize_ext4_dir_flat(
        device,
        device_name,
        "/glibc/basic",
        "/mnt/sdcard/glibc/basic",
        256,
        0,
    );
    oscomp_materialize_ext4_dir_flat(
        device,
        device_name,
        "/glibc/lua",
        "/mnt/sdcard/glibc/lua",
        128,
        0,
    );
    oscomp_materialize_ext4_dir_flat(
        device,
        device_name,
        "/glibc/ltp",
        "/mnt/sdcard/glibc/ltp",
        256,
        0,
    );
    oscomp_materialize_ext4_dir_flat(
        device,
        device_name,
        "/glibc/lmbench",
        "/mnt/sdcard/glibc/lmbench",
        128,
        0,
    );
    oscomp_materialize_ext4_dir_flat(device, device_name, "/musl", "/mnt/sdcard/musl", 512, 0);
    oscomp_materialize_ext4_dir_flat(
        device,
        device_name,
        "/musl/lib",
        "/mnt/sdcard/musl/lib",
        256,
        0,
    );
    oscomp_materialize_ext4_dir_flat(
        device,
        device_name,
        "/musl/basic",
        "/mnt/sdcard/musl/basic",
        256,
        0,
    );
    oscomp_materialize_ext4_dir_flat(
        device,
        device_name,
        "/musl/lua",
        "/mnt/sdcard/musl/lua",
        128,
        0,
    );
    oscomp_materialize_ext4_dir_flat(
        device,
        device_name,
        "/musl/ltp",
        "/mnt/sdcard/musl/ltp",
        256,
        0,
    );
    oscomp_materialize_ext4_dir_flat(
        device,
        device_name,
        "/musl/lmbench",
        "/mnt/sdcard/musl/lmbench",
        128,
        0,
    );

    println!("sdcard:");
    println!("  mount         : /dev/{} (ext4)", device_name);
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

fn oscomp_sdcard_install_ext4_path(device_name: &str, ext4_path: &str, vfs_path: &str) {
    oscomp_sdcard_ensure_parent_dirs(vfs_path);
    let source = alloc::format!("/dev/{}", device_name);
    let _ = fs::install_ext4_path(&source, vfs_path, ext4_path);
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
    device: &alloc::sync::Arc<dyn crate::block::BlockDevice>,
    device_name: &str,
    ext4_dir: &str,
    vfs_dir: &str,
    max_files: usize,
    recurse_levels: usize,
) -> usize {
    const EXT4_FT_DIR: u16 = 2;
    let Ok(entries) = crate::ext4::list_directory(alloc::sync::Arc::clone(device), ext4_dir) else {
        // ext4_dir may be a regular file (e.g. /glibc/lua), not a directory.
        // Install it as a regular file instead of creating a false directory.
        oscomp_sdcard_install_ext4_path(device_name, ext4_dir, vfs_dir);
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
                oscomp_materialize_ext4_dir_flat(
                    device,
                    device_name,
                    &ext4_child,
                    &vfs_child,
                    max_files,
                    recurse_levels - 1,
                );
            }
            continue;
        }

        if fs::stat(&vfs_child).is_ok() {
            already_available += 1;
            continue;
        }

        oscomp_sdcard_install_ext4_path(device_name, &ext4_child, &vfs_child);
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
    // BuildStorm remounts /mnt/sdcard as a native lazy ext4 overlay. Its VFS
    // lookup already resolves children and symlinks directly from ext4, so
    // the legacy tmpfs materializer is both redundant and harmful there.
    if fs::is_ext4_overlay_directory("/mnt/sdcard") {
        return false;
    }

    // No local contest storage: skip ext4 materialisation so exec/open can
    // safely report ENOENT without touching a block device.
    let Some(device) = crate::storage::contest_storage_device() else {
        return false;
    };
    let Some(device_name) = crate::storage::contest_storage_name() else {
        return false;
    };

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
            oscomp_materialize_ext4_dir_flat(&device, &device_name, &ext4_dir, &vfs_dir, 4096, 2);
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

    let count =
        oscomp_materialize_ext4_dir_flat(&device, &device_name, &ext4_dir, &vfs_dir, 4096, 2);
    count > 0
}

/// P1-A: install ELF dynamic linker (ld-linux / ld-musl) from their real
/// ext4 source paths into the canonical /lib and /lib64 VFS paths that
/// PT_INTERP encodes.
fn oscomp_install_runtime_loader_aliases(device_name: &str) {
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
            oscomp_sdcard_install_ext4_path(device_name, src, dst);
        }
    }
    crate::println!("sdcard: installed runtime loader aliases");
}

/// P1-A: install common glibc/musl shared libraries from ext4 /glibc/lib
/// and /musl/lib into /lib, /lib64, and /usr/lib so ld-linux can find
/// them via openat.
fn oscomp_install_runtime_lib_aliases(device_name: &str) {
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
        oscomp_sdcard_install_ext4_path(device_name, &glibc_src, &lib_dst);
        oscomp_sdcard_install_ext4_path(device_name, &glibc_src, &lib64_dst);
        oscomp_sdcard_install_ext4_path(device_name, &glibc_src, &usr_dst);

        if fs::stat(&lib_dst).is_err() {
            oscomp_sdcard_install_ext4_path(device_name, &musl_src, &lib_dst);
        }
        if fs::stat(&lib64_dst).is_err() {
            oscomp_sdcard_install_ext4_path(device_name, &musl_src, &lib64_dst);
        }
        if fs::stat(&usr_dst).is_err() {
            oscomp_sdcard_install_ext4_path(device_name, &musl_src, &usr_dst);
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
