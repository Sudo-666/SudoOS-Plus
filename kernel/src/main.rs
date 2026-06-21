#![no_std]
#![no_main]

mod block;
mod call_function;
mod console;
mod context;
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
mod page_alloc;
mod panic;
mod pipe;
mod process;
mod runtime_page_table;
mod signal;
mod smp;
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
        memory::install_riscv_final_page_table(&early_memory);
    }

    /*
     * 从此处开始，不再允许使用 EarlyFrameAllocator。
     */
    let kernel_memory = memory::initialize_page_allocator(&memory_layout, early_memory);

    #[cfg(debug_assertions)]
    page_alloc::verify();

    /*
     * 必须在全局页分配器安装后启用 heap。
     */
    heap::initialize();

    #[cfg(debug_assertions)]
    heap::verify();

    #[cfg(debug_assertions)]
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
    fault::initialize();
    fs::initialize();
    install_external_initramfs(initrd_range);
    mount_sdcard_if_present();
    tty::initialize();

    #[cfg(debug_assertions)]
    vm::verify();

    #[cfg(debug_assertions)]
    fault::verify();
    #[cfg(debug_assertions)]
    fs::verify();
    #[cfg(debug_assertions)]
    block::verify();
    #[cfg(debug_assertions)]
    virtio::verify();
    #[cfg(debug_assertions)]
    pipe::verify();
    #[cfg(debug_assertions)]
    signal::verify();
    #[cfg(debug_assertions)]
    tty::verify();

    #[cfg(debug_assertions)]
    trap::verify_breakpoint();

    time::start_periodic();

    #[cfg(debug_assertions)]
    time::verify_periodic();

    task::initialize();
    smp::start_secondaries();
    task::finalize_cpu_bringup();
    workqueue::initialize();
    #[cfg(debug_assertions)]
    timer::verify();
    #[cfg(debug_assertions)]
    workqueue::verify();
    #[cfg(debug_assertions)]
    tracked_spin::verify();

    #[cfg(debug_assertions)]
    task::verify();
    user::verify();
    if initrd_range.is_some() {
        user::verify_busybox_rootfs();
    }
    user::verify_sdcard_sample();
    user::verify_sdcard_basic_script();

    println!("kernel_main: initialization completed");
    println!("SMOKE_TEST: PASS");

    task::boot_idle_loop()
}

fn mount_sdcard_if_present() {
    if crate::block::open_device("vda").is_none() {
        return;
    }

    match fs::mkdir("/mnt", 0o755) {
        Ok(()) | Err(myos_vfs::Errno::Eexist) => {}
        Err(error) => panic!("failed to create /mnt before sdcard mount: {error:?}"),
    }
    match fs::mkdir("/mnt/sdcard", 0o755) {
        Ok(()) | Err(myos_vfs::Errno::Eexist) => {}
        Err(error) => panic!("failed to create /mnt/sdcard before sdcard mount: {error:?}"),
    }
    match fs::mkdir("/mnt/sdcard/musl", 0o755) {
        Ok(()) | Err(myos_vfs::Errno::Eexist) => {}
        Err(error) => panic!("failed to create /mnt/sdcard/musl before sdcard mount: {error:?}"),
    }
    match fs::mkdir("/mnt/sdcard/musl/lib", 0o755) {
        Ok(()) | Err(myos_vfs::Errno::Eexist) => {}
        Err(error) => {
            panic!("failed to create /mnt/sdcard/musl/lib before sdcard mount: {error:?}")
        }
    }
    match fs::mkdir("/bin", 0o755) {
        Ok(()) | Err(myos_vfs::Errno::Eexist) => {}
        Err(error) => panic!("failed to create /bin before sdcard userland setup: {error:?}"),
    }
    match fs::mkdir("/code", 0o755) {
        Ok(()) | Err(myos_vfs::Errno::Eexist) => {}
        Err(error) => panic!("failed to create /code before sdcard userland setup: {error:?}"),
    }
    match fs::mkdir("/code/mnt", 0o755) {
        Ok(()) | Err(myos_vfs::Errno::Eexist) => {}
        Err(error) => panic!("failed to create /code/mnt for sdcard tests: {error:?}"),
    }
    match fs::install_ext4_path("/dev/vda", "/bin/busybox", "/musl/busybox") {
        Ok(()) | Err(myos_vfs::Errno::Eexist) => {}
        Err(error) => panic!("failed to install /bin/busybox from sdcard: {error:?}"),
    }
    match fs::symlink("/bin/busybox", "/bin/sh") {
        Ok(()) | Err(myos_vfs::Errno::Eexist) => {}
        Err(error) => panic!("failed to install /bin/sh symlink for sdcard scripts: {error:?}"),
    }
    match fs::install_ext4_path("/dev/vda", "/mnt/sdcard/musl/busybox", "/musl/busybox") {
        Ok(()) | Err(myos_vfs::Errno::Eexist) => {}
        Err(error) => panic!("failed to install /musl/busybox from sdcard: {error:?}"),
    }
    match fs::install_ext4_path(
        "/dev/vda",
        "/mnt/sdcard/musl/basic_testcode.sh",
        "/musl/basic_testcode.sh",
    ) {
        Ok(()) | Err(myos_vfs::Errno::Eexist) => {}
        Err(error) => panic!("failed to install /musl/basic_testcode.sh from sdcard: {error:?}"),
    }
    match fs::install_ext4_path(
        "/dev/vda",
        "/mnt/sdcard/musl/lib/libc.so",
        "/musl/lib/libc.so",
    ) {
        Ok(()) | Err(myos_vfs::Errno::Eexist) => {}
        Err(error) => panic!("failed to install /musl/lib/libc.so from sdcard: {error:?}"),
    }
    match fs::install_ext4_path("/dev/vda", "/text.txt", "/musl/basic/text.txt") {
        Ok(()) | Err(myos_vfs::Errno::Eexist) => {}
        Err(error) => panic!("failed to install /musl/basic/text.txt from sdcard: {error:?}"),
    }
    match fs::install_ext4_path("/dev/vda", "/code/text.txt", "/musl/basic/text.txt") {
        Ok(()) | Err(myos_vfs::Errno::Eexist) => {}
        Err(error) => panic!("failed to install /code/text.txt from sdcard: {error:?}"),
    }
    match fs::install_ext4_path("/dev/vda", "/code/test_echo", "/musl/basic/test_echo") {
        Ok(()) | Err(myos_vfs::Errno::Eexist) => {}
        Err(error) => panic!("failed to install /code/test_echo from sdcard: {error:?}"),
    }
    match fs::mkdir("/mnt/sdcard/musl/basic", 0o755) {
        Ok(()) | Err(myos_vfs::Errno::Eexist) => {}
        Err(error) => {
            panic!("failed to create /mnt/sdcard/musl/basic before sdcard mount: {error:?}")
        }
    }
    match fs::mount_ext4_subtree("/dev/vda", "/mnt/sdcard/musl/basic", "/musl/basic", 1) {
        Ok(()) | Err(myos_vfs::Errno::Ebusy) => {}
        Err(error) => {
            panic!("failed to mount /dev/vda:/musl/basic on /mnt/sdcard/musl/basic: {error:?}")
        }
    }
    println!("sdcard:");
    println!("  mount         : /dev/vda:/musl/basic -> /mnt/sdcard/musl/basic (ext4 ro)");
    println!(
        "  files         : /bin/sh /musl/busybox /musl/basic_testcode.sh /musl/lib/libc.so /text.txt /code/text.txt /code/test_echo"
    );
}

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
