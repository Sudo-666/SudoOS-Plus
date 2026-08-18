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
- 退路：若 CherryUSB OSAL 适配成本过高，退回"自写 Rust EHCI + Cotton
  MSC/SCSI"（保留 C 构建路径作为通用设施）。
