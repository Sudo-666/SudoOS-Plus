# ADR-001: 用 CherryUSB 实现 LS2K1000 USB 大容量存储

## Status
Accepted

## Date
2026-08-18

## Context

竞赛最终镜像（14 GB 级）无法装进 LS2K1000 现有的 `ram0` 竞赛盘路径
（BootRamBlockDevice 容量受 RAM 约束，fixture 固定 32 MiB）。需要在内核内
直接以块设备方式读 USB 大容量存储。

硬约束：
- 控制器是 2K1000 通用 Intel EHCI，MMIO 基址 `0x4006_0000`；U-Boot 已用它
  引导（`fatload usb 0:1 ...`），接管时 PHY/时钟/复位已就绪。
- 只需**只读** MSC Bulk-Only Transport + SCSI（Inquiry / TUR / Request
  Sense / Read Capacity / Read10）。
- 块设备缝已存在：`BlockDevice` trait（kernel/src/block.rs:44）+ 分区下降
  （K1.1）→ USB 盘注册成 `/dev/sda` 即可复用 partition / ext4 / contest
  runner。
- 无 QEMU USB host 模拟 → 枚举/BOT/SCSI 正确性只能在真机串口调。
- 许可证须宽松（内核非 GPL）；U-Boot / Linux USB 驱动只作寄存器顺序参考，
  不复制源码。
- 内核目前是**纯 Rust**：`kernel/csrc/` 不存在，构建系统零 C 编译路径。

## Decision

**vendor 固定版本的 CherryUSB，只编译 EHCI + root-hub + MSC 三件套；
LS2K1000 平台胶水用 C 写在 `kernel/csrc/usb/`；经 `kernel/build.rs` 用
`loongarch64-linux-gnu-gcc` 交叉编译为 `libsudoos_usb.a` 链进内核；Rust
侧 extern "C" 包装成 `BlockDevice` 注册 `/dev/sda`。**

关键技术决定：
1. **ABI 匹配**：C 用 `-mabi=lp64s -march=loongarch64`，与 Rust 目标
   `loongarch64-unknown-none-softfloat` 一致，全 freestanding。
2. **DMA 一致性走 uncached 窗口**：LS2K1000 直接映射提供
   `0x8000_0000_0000_0000` 强序非缓存窗口（`arch/loongarch64/
   platform/ls2k1000/memory.rs` 的 `UNCACHED_BASE`）。QH/qTD 描述符与数据
   缓冲经 `virt_to_phys | UNCACHED_BASE` 访问，CPU 写 → 物理内存、控制器
   读 → 直接可见，**无需 dcache 汇编**；CherryUSB 的 dcache 钩子做成 no-op。
3. **复用 U-Boot 初始化状态**：PHY/时钟/复位不动，先直接驱动 EHCI 寄存器；
   增量失败时再补完整复位序列。
4. **C 构建路径按平台门控**：仅 `platform-ls2k1000` 特性启用时编译，其余
   目标零 C 依赖；工具链可用 `LS2K1000_CC` / `LS2K1000_AR` 覆盖。

## Alternatives Considered

### 纯 Rust EHCI + 自写 BOT/SCSI
- Pros：零新工具链、与纯 Rust 代码库一致、无 C 构建路径。
- Cons：~1500–2500 行；枚举/BOT/SCSI 协议正确性自己扛；无 QEMU USB host
  模拟，只能真机串口调，调试周期长。
- Rejected：协议正确性风险 + 真机调试成本超过 C 工具链的一次性成本。

### TinyUSB
- Pros：MIT，MSC 支持。
- Cons：平台适配更偏特定控制器，通用 EHCI host 侧文档/社区不如 CherryUSB
  成熟。
- Rejected：CherryUSB 的 EHCI 是活跃维护主线，配置项更贴合。

### Cotton（Rust MSC/SCSI）
- Pros：Rust、CC0、可复用上层。
- Cons：没有 LS2K1000 EHCI 实现，仍要自写 EHCI + 接入层。
- Rejected：只解决了问题的一半，还要额外维护。

### CrabUSB
- Pros：Rust、MIT。
- Cons：只支持 xHCI，2K1000 是 EHCI。
- Rejected：控制器不匹配。

### U-Boot / Linux USB
- Pros：完整、真机验证过。
- Cons：GPL，不能直接复制进非 GPL 内核。
- Rejected（仅作参考）：寄存器初始化顺序作参考，不复制源码。

## Consequences

- 好处：枚举/BOT/SCSI 用成熟实现，问题域收敛到平台胶水（~500 行）；
  block 缝可复用，M4 收尾到 `/dev/sda` 很薄。
- 代价：纯 Rust 内核首次引入 C 工具链与 C 构建路径（一次性基础设施）；
  vendor 一个 Apache-2.0 栈需保留 LICENSE；CherryUSB OSAL 需映射到 SudoOS
  的 IrqSpinLock / alloc / time。
- 风险：EHCI 描述符/缓冲必须落在控制器可见物理地址（uncached 窗口）；
  2K1000 USB 是否真免 cache 维护以真机为准，必要时退回显式
  flush/invalidate。
- **M1 工具链发现（2026-08-18）**：Ubuntu 24.04 的
  `loongarch64-linux-gnu` binutils 2.42 无法汇编 LoongArch `cache` 指令
  （`as` 对 `cache op, rj, si12` 所有形式报 "no match insn"），且 `cache`
  的 5 位 op 字段是"3 位 cache 选择器 + 2 位操作码"分段编码（见 Linux
  `arch/loongarch/include/asm/cacheops.h`），非扁平值。因此显式 dcache
  指令路线在 M1 受阻。**M1 暂缓 cache 一致性**（`CONFIG_USB_DCACHE_ENABLE`
  与 `CONFIG_USB_EHCI_DESC_DCACHE_ENABLE` 均未定义，dcache 钩子为 no-op
  宏）。
- **M2 决策（2026-08-18）**：选 **uncached 窗口（方案 1）**，不修 binutils。
  具体落地：
  - `linker.ld` 新增 `.nocache_ram` NOLOAD 段，VMA = `UNCACHED_BASE | phys`，
    物理紧跟内核镜像；`arch/loongarch64/platform/ls2k1000/memory.rs` 用
    `__nocache_ram_start/__nocache_ram_end` 符号把该物理区从页分配器保留。
  - EHCI 静态描述符（QH/qTD 池、async/periodic 队列头、frame list）打
    `__attribute__((section(".nocache_ram")))`，所有 CPU 访问经 uncached
    窗口直达物理内存；`usb_ehci_physramaddr(a) = a & PHYS_MASK`（缓存直接
    映射与 uncached 窗口都是 `BASE | phys`）。
  - **动态缓冲**：`usb_malloc` 从 `.nocache_ram` 的动态池（free-list
    分配器，Rust `sudoos_usb_alloc`）切块，与控制块物理隔离。**控制块
    （信号量/互斥锁/线程表）走普通缓存堆**（`sudoos_usb_alloc_ctrl`）——
    WaitQueue/IrqSpinLock 依赖 ll/sc，绝不能落在 uncached 窗口。
  - **传输完成驱动**：2K1000 当前内核无外设中断基础设施（trap.rs 只处理
    timer/IPI 位，其余 handle_unhandled），M2 用 1 ms 轮询线程驱动
    `usb_ehci_interrupt()`（读 USBSTS→提交 hpworkq→清状态），配
    `g_usb_hc_ready` 门闩防 hc_init 前的误触发。真实 IRQ 布线留待真机确认
    后（M3+）。
  - **线程**：CherryUSB psc/hpworkq/lpworkq 线程接 SudoOS
    `spawn_kernel_thread`（`KernelThreadEntry = fn()`，槽位经 Rust trampoline
    烘焙）；`thread_suspend/resume` 保持 no-op（枚举期 lpworkq 无异步工作）。
- **M2.7 决策（2026-08-18，真机首发后）**：**早期 USB 探测与线程化初始化
  分离**。真机日志显示 `USB-glue M0 probe=0x2a4a0001` 后立即 panic
  `kernel scheduler is not initialized`（task/mod.rs:2645）——根因是
  `cusb::init()` 在 main.rs:480（scheduler @ task::initialize 之前）调用
  `sudoos_usb_init()`→`usbh_initialize()`，后者立刻 `usb_osal_thread_create`
  spawn psc/hpworkq/lpworkq 三个内核线程，撞未初始化的 `SCHEDULER`。
  落地：
  - **早期 `cusb::early_probe()`**（boot 路径、scheduler 前）：只做
    `sudoos_usb_early_probe()`——纯 MMIO 有界轮询探针（M0–M9：基址/能力/
    控制器复位/主机运行/端口检测），所有等待有 deadline，失败只打日志返回
    负值，**绝不 panic**；`dma_pool_init()` 仍在早期（纯内存，无 task 依赖）。
  - **晚期 `cusb::late_start()`**（main.rs，`task::finalize_cpu_bringup()`
    之后）：spawn 专用 `usb_init_thread` → `sudoos_usb_init()`（此刻 scheduler
    active、中断使能），成功才 spawn poller/monitor；失败打日志继续启动——
    **USB 探测失败可接受，但不能挡在 /init 之前**。
  - 附带修正：`usb_hc_low_level_init` 的 USBSTS 打印索引错误（`hcor[2]` 实为
    USBINTR@0x08）；`usbh_get_port_speed` 双 bug——用 1-based
    `EHCI_PORTSC_OFFSET(n)` 解 0-based `port`（port=0 读到 CONFIGFLAG@0x40）
    且读 bits[11:10]（LSTATUS 线状态）而非 EHCI 2.0 的 PORTSPD@[15:13]。
- **M2.8 决策（2026-08-18，U 盘整盘烧录镜像上板闭环）**：真机寄存器证据
  定位根因——U-Boot 阶段 `PORTSC0=0x1005`（PP+PE+CCS 全在），SudoOS 早期
  探针执行 HCRESET 后 `PORTSC0` 被清成 0，后续没恢复 PP，U 盘上不了电。
  落地（CodePlan §2–§8 一步到位）：
  - **早期探测改只读**：`sudoos_usb_early_probe` 只打印 CAPLENGTH/HCSPARAMS/
    USBCMD/USBSTS/CONFIGFLAG/PORTSC0-2，**绝不写控制寄存器**。CherryUSB
    的 `usb_hc_init` 成为 EHCI 唯一初始化者。
  - **复位后恢复端口供电**：定义 `CONFIG_USB_EHCI_CONFIGFLAG`；`usb_hc_init`
    的 CONFIGFLAG 块后补 LS2K1000 专用块——遍历 root 端口，屏蔽 W1C change
    位（CSC/PEC/OCC）后写 PP，打印 `USB-EHCI: PORTSCn=...`。psc 线程临界区
    内不能 sleep，端口稳定由 psc 的 `usb_reset_port + msleep(200)` 承担。
    `CONFIG_USBHOST_RHPORTS 1→3`（本版本无 `MAX_RHPORTS` 宏，按实际宏名）。
  - **C façade**：`sudoos_usb_host_start/host_poll/msc_is_ready/capacity/
    read_blocks`；vendored `usbh_msc.c` 的 connect/disconnect 回调
    `sudoos_usb_msc_connected/disconnected` 原子发布 MSC 指针+容量（取代原
    `msc_test()` 调试残留）。read10 校验 block_size≠0、LBA/count/buffer 长度
    越界、单请求（g_msc_busy 防重入），阻塞传输由 1ms poll 线程驱动。
  - **Rust `UsbMscBlockDevice`（只读）**：VFS → read_block → `.nocache_ram`
    uncached DMA32 bounce → `sudoos_usb_msc_read_blocks` → 拷回调用方。
    EHCI 32 位 DMA，缓冲必须物理连续落低 4GB（池满足）。
  - **延迟注册 + 启动顺序**：`fs::install_block_device_node`（fs::initialize
    后补 /dev 节点，幂等 Eexist）；`partition::register_partitions("sda")` 单盘
    扫描；main.rs 早期 `mount_sdcard_if_present` 对 LS2K1000 gate 掉，改在
    scheduler 后 `initialize_ls2k_contest_usb` 等 USB 存储就绪
    （`USB_STORAGE_READY` Completion，12s 上限）再走统一选择/挂载。VF2/QEMU
    保持早期挂载路径不变。
  - **`sudoos.contest.required=1`**：找不到竞赛存储打印 `CONTEST_ERROR` +
    `CONTEST_RESULT ... fail` 并显式 halt，不静默跑 preliminary。竞赛 DTB
    `sudoos.contest.dev=sda required=1 oscomp=final-all`（不带 rdinit=/init），
    保留 debug shell DTB。
- 退路：若 CherryUSB OSAL 适配成本过高，退回"自写 Rust EHCI + Cotton
  MSC/SCSI"（保留 C 构建路径作为通用设施）。
