# VisionFive 2 (JH7110) 开发板移植总结

> 分支：`final-beta1`
> 更新时间：2026-08-06
> 目标板：昉·星光 2 (VisionFive 2)，StarFive JH7110 SoC，4× SiFive U74 RV64GC
> Bootloader：U-Boot (S-mode) + OpenSBI (M-mode)
> 对照参考：哈工大 RocketOS 对 VisionFive 2 的适配（本仓库 `D:\T202510213995926-2475-main`）

## 1. 项目背景与目标

内核原已支持 riscv64（QEMU virt，OpenSBI 引导）与 loongarch64（QEMU virt + 2K1000 真机）。
本次目标：把 riscv64 内核**真实移植到 VisionFive 2 开发板**上启动运行。

本阶段完成 riscv64 平台化拆分与 VisionFive 2 平台代码，产物已通过编译验证；
真机串口验证（U-Boot `bootm` 加载 uImage）待上板。

## 2. VisionFive 2 启动环境

### 2.1 固件链与特权级

```
ZSBL(mask ROM) → U-Boot SPL(M-mode) → OpenSBI(M-mode, FW_TEXT_START=0x4000_0000)
                                     → U-Boot proper(S-mode, 由 OpenSBI 在 0x4020_0000 启动)
                                     → 内核(S-mode, SBI ecall 由 OpenSBI 服务)
```

U-Boot proper 运行在 **S-mode**，跳转内核时按 RISC-V Linux 启动协议设置寄存器
（`arch/riscv/lib/bootm.c` 的 `boot_jump_linux`）：

```c
kernel(gd->arch.boot_hart, images->ft_addr);
// a0 = boot hart ID
// a1 = FDT 物理地址
```

这与 QEMU virt 的 OpenSBI 约定一致：**a0=hart id、a1=FDT、a2 未定义**，
因此 `arch/riscv64/src/boot.rs` 的启动约定在真机上直接成立。

### 2.2 关键地址

| 项目 | 值 | 说明 |
|------|-----|------|
| DDR 物理基址 | `0x4000_0000` | 板载 2/4/8 GB LPDDR4，**不是** QEMU virt 的 `0x8000_0000` |
| 内核加载地址 | `0x4020_0000` | U-Boot `kernel_addr_r`，与 Linux Image 一致 |
| OpenSBI 固件区 | `[0x4000_0000, 0x4020_0000)` | 需保留给 SBI 运行时，不可释放 |
| UART0 | `0x1000_0000` | 板级控制台 `serial0`（`stdout-path="serial0:115200n8"`） |
| U-Boot FDT | `0x4600_0000` | `fdt_addr_r` |

### 2.3 UART0 寄存器布局（与 QEMU 不同，关键坑）

JH7110 UART0 是 Synopsys **DW_apb_uart**（8250 兼容），设备树属性：

```
reg-io-width = <4>;   // 32 位寄存器访问
reg-shift    = <2>;   // 寄存器步长 4 字节
```

| 寄存器 | 字节步长 16550 (QEMU) | DW_apb_uart (JH7110) |
|--------|----------------------|----------------------|
| THR | `+0x00` | `+0x00`（32 位写，低 8 位有效） |
| LSR | `+0x05` | `+0x14`（bit5 = THRE） |

QEMU virt 的 ns16550a 是字节步长（LSR 在 +5），直接套用到真机读到的会是错误寄存器。
因此控制台必须按平台拆分实现（见 §4）。

## 3. RocketOS 对照

RocketOS（哈工大，2K1000 + VisionFive 2 双板）对 VisionFive 2 的适配要点
（`os/src/arch/riscv64/boards/qemu.rs`、`entry.S`、`docs/content/board.typ`）：

- **物理基址 `0x4000_0000`**，内核运行 VMA `0xffffffc040200000`
  （`KERNEL_BASE=0xffffffc000000000` + `0x4020_0000`）。
- 启动页表同时建立低地址恒等映射与高地址映射，保证 Sv39 开启瞬间取指不断。
- 板卡 MMIO 寄存器（UART、网卡、SDIO）全部走物理地址/恒等映射访问。
- 板卡 `MEMORY_SIZE=0x2000_0000`（512 MB）只是常量，实际以 DTB memory 节点为准。
- RocketOS 的 VisionFive 2 支持依赖 **SBI 调用**（timer/IPI/HSM），与 QEMU 环境同构，
  内核无需为真机引入 M-mode 裸机代码。

**本内核的移植策略**：保持既有"高半 VMA + Sv39 + SBI"模型不变，仅把
物理基址/加载地址/控制台按平台参数化。`mm/task/trap/smp` 全部无需改动：
- 运行时 direct map 由 FDT RAM 范围驱动（`prepare_riscv_direct_map`，基址无关）；
- SMP 副核经 SBI HSM + 链接符号 trampoline（`secondary.S`，平台无关）；
- 定时器走 SBI TIME 扩展，频率取自 FDT `timebase-frequency`。

## 4. 已完成的工作

### 4.1 riscv64 平台化拆分（仿 loongarch64 模式）

| 文件 | 改动 |
|------|------|
| `arch/riscv64/Cargo.toml` | 新增 feature `platform-qemu-virt`（默认）/ `platform-visionfive2` |
| `arch/riscv64/src/platform/mod.rs` | cfg 门控平台选择；`visionfive2` 显式启用时优先；三项目契约 `boot_context` / `write_console_byte` / `reserve_early_memory` |
| `arch/riscv64/src/lib.rs` | 平台入口 asm 移出，仅保留共享 `secondary.S` / `trap/entry.S` / `task/switch.S` |
| `arch/riscv64/src/boot.rs` | `from_raw` 委托 `crate::platform::boot_context`；`BootContext::new()` / `with_device_tree()` 构建器 |
| `arch/riscv64/src/early_console.rs` | `write_byte` 委托平台；保留共享 `MMIO_BASE`/`virtual_base()` |
| `arch/riscv64/src/memory/mod.rs` | `reserve_early_platform_memory` 委托平台 |
| `arch/riscv64/src/memory/layout.rs` | `BOOT_PHYS_BASE`/`KERNEL_PHYS_BASE` 按平台 cfg |
| `arch/riscv64/src/platform/qemu_virt/` | 原 `entry.S`/`linker.ld` 移入，行为不变 |
| `kernel/Cargo.toml` | 新增 `platform-visionfive2` 透传 |
| `kernel/build.rs` | riscv64 按 feature 选择链接脚本 |

### 4.2 新增 VisionFive 2 平台

`arch/riscv64/src/platform/visionfive2/`：

| 文件 | 内容 |
|------|------|
| `entry.S` | 基于 qemu_virt 入口，改地址常量；新增物理 UART 诊断 'B' |
| `linker.ld` | `BOOT_PHYS_BASE=0x4020_0000`、`KERNEL_PHYS_BASE=0x4040_0000`、SMP trampoline `0x4030_0000` |
| `boot.rs` | a0/a1/a2 约定 + **FDT magic 校验**（`0xd00dfeed`，防 `go` 垃圾 a1） |
| `console.rs` | DW_apb_uart 32 位寄存器（THR+0x00、LSR+0x14） |
| `memory.rs` | 显式保留 OpenSBI `[0x4000_0000, 0x4020_0000)` |

入口页表关键改动（相对 qemu_virt）：

| 映射 | qemu_virt | visionfive2 |
|------|-----------|-------------|
| boot 恒等映射 | root[2]，PA `0x8020_0000` | root[1]，PA `0x4020_0000` |
| 临时 direct map | root[346]，PA `0x8000_0000`，24 GiB 页 | root[345]，PA `0x4000_0000`，8 GiB 页（覆盖 8 GB 变体） |
| 高半内核 | root[510] → PA `0x8040_0000` | root[510] → PA `0x4040_0000` |
| UART 恒等映射 | root[0] @ `0x1000_0000` | root[0] @ `0x1000_0000`（相同） |

### 4.3 构建与产物

```bash
# 构建 visionfive2 内核 ELF（release）
ARCH=riscv64 PLATFORM=visionfive2 PROFILE=release ./scripts/build.sh
# 或
make kernel-visionfive2

# 生成 uImage（arch=26, load=entry=0x40200000）
make uImage-vf2

# 生成 raw 二进制（仅 go 快速诊断用，不传 FDT）
make kernel-vf2.bin
```

验证产物：

```bash
readelf -h kernel-visionfive2   # Entry point = 0x40200000
riscv64-linux-gnu-objdump -d --section=.boot.text kernel-visionfive2
```

## 5. 板级启动流程（U-Boot 控制台）

uImage 需要配合板级 DTB 供 `bootm` 传 a0/a1（U-Boot `boot_prep_linux` 强制要求 FDT）：

```sh
# 把 uImage-vf2 和板级 dtb 放到 SD 卡 FAT 分区
StarFive# fatload mmc 0:1 0x40200000 uImage-vf2
StarFive# fatload mmc 0:1 0x46000000 jh7110-visionfive-v2.dtb
StarFive# bootm 0x40200000 - 0x46000000
```

- 控制台：UART0 @ `0x10000000`（40 针排针 GPIO8/10，115200 8N1），与 U-Boot 串口相同。
- 板级 DTB 来源：SD 卡 Linux 分区 `/boot/`、`u-boot.itb` 内嵌，或内核
  `arch/riscv/boot/dts/starfive/jh7110-visionfive-v2.dtb`。
- 上电打印顺序：`B`（入口诊断）→ 内核 Rust 侧 `BOOT00 entry` → ...
- 若 `bootm` 报 "Device tree not found"，补显式 FDT 参数（如上第 2 条）。

## 5.1 FIT / TFTP 启动（VF2.6）

U-Boot 通过以太网从 TFTP 服务器下载单个 FIT，再 `bootm` 分发内核 + DTB + initramfs。
产物与脚本（见 `scripts/`）：

```text
build/visionfive2/tftp/sudoos/vf2/
├── sudoos-visionfive2.itb     # 单个 FIT: raw kernel + 3 个派生 DTB + cpio
├── sudoos-vf2-tftp.cmd        # 网络地址无关的 U-Boot 脚本源码
├── sudoos-vf2-tftp.scr        # mkimage -T script 编译产物
└── visionfive2-manifest.txt   # commit/toolchain/全部 sha256
```

构建（必须提供与 PCB 匹配的外部完整板级 DTB，禁止手写 minimal DTB）：

```sh
make visionfive2-tftp-bundle \
  VISIONFIVE2_DTB=/absolute/path/to/jh7110-starfive-visionfive-2-v1.3b.dtb
```

脚本链：

- `build-visionfive2-dtb.sh`：验证 `model`/`compatible`/CPU/memory/UART0/`stdout-path`，
  再按 bootargs 派生三个 DTB 变体（`conf-selftest` / `conf-single` / `conf-smp`），
  **不**写入 `linux,initrd-*`（由 U-Boot `bootm` 启动时填 `/chosen`）。
- `visionfive2-fit.its.in` + `build-visionfive2-fit.sh`：`mkimage -f` 生成 FIT，
  kernel 节点 `os="linux"` 只为选 RISC-V handoff，`load=entry=0x4020_0000`；
  三个 FDT 节点固定 `load=0x4600_0000`、ramdisk 固定 `load=0x4610_0000`——
  bootm 按 FIT `load` 属性把组件搬到 8 字节对齐地址再交内核（内核
  `valid_fdt_address()` 要求 8 字节对齐），1 MiB 间距保证 bootm 原地扩展 DTB
  不会长进 initramfs（U-Boot 自动分配的 FDT/ramdisk 曾实测重叠 ~10.6 KiB）。
- `check-visionfive2-fit.py`：`mkimage -l`/`dumpimage` 验证 default=conf-smp、
  三个 config、kernel load/entry、SHA-256、以及逐字节一致性与 staging 不重叠。
- `visionfive2-tftp.cmd` / `build-visionfive2-uboot-script.sh`：网络地址无关的
  启动脚本（`tftpboot 0x60000000 ... && iminfo && bootm 0x60000000#${sudoos_conf}`）。

FIT 配置与 bootargs：

| 配置 | bootargs | 用途 |
|---|---|---|
| `conf-selftest` | `console=ttyS0,115200n8 sudoos.maxcpus=1` | Gate A |
| `conf-single` | `console=ttyS0,115200n8 rdinit=/init init.debug=1 sudoos.maxcpus=1` | Gate B |
| `conf-smp`（default） | `console=ttyS0,115200n8 rdinit=/init init.debug=1 sudoos.maxcpus=4` | Gate C/D |

## 6. 已修复的既有问题

- `kernel/src/task/mod.rs`：ls2k1000 移植把 worker 验证数组从 8 扩到 16 个显式元素，
  导致 **riscv64（MAX_CPUS=8）debug 构建失败**。已改为按 `MAX_CPUS` 的 const-repeat 数组 +
  固定 16 项 `WORKER_ENTRIES` 函数表（实际下标受 `topology_worker_count ≤ MAX_CPUS` 约束）。

## 7. 回归验证（WSL Ubuntu 交叉编译）

| 目标 | 结果 |
|------|------|
| riscv64 qemu-virt release/debug | ✅ entry=`0x8020_0000` 不变 |
| riscv64 visionfive2 release/debug | ✅ entry=`0x4020_0000` |
| loongarch64 qemu-virt release | ✅ |
| loongarch64 ls2k1000 release | ✅ |
| uImage-vf2 | ✅ arch=26, load=entry=`0x4020_0000`, 3.5 MiB |

## 8. 待办事项（TODO）

- [ ] 真机串口验证 `B` 诊断与内核 `BOOT00` 启动（需拿到板级 DTB）
- [ ] 验证 FDT 传入（`a1`）与内存布局（`memory_regions()` 驱动 RAM）
- [ ] 验证 SBI timer / IPI / HSM SMP（4× U74）与 U-Boot 传参的 a2 无关性
- [ ] UART 波特率/时钟确认（U-Boot 已初始化 UART0，若改波特率需在 dts 校准）
- [ ] 网卡（StarFive GMAC）与 SDIO 驱动轮询适配（参考 RocketOS `os/src/drivers/net/starfive/`）
- [ ] 板级 DTB 缺失时提供 minimal FDT 兜底（对应 2K1000 移植 §6 的同类 TODO）

## 9. 相关文件清单

```
arch/riscv64/src/platform/qemu_virt/     # qemu 平台（拆分自原扁平结构）
arch/riscv64/src/platform/visionfive2/   # VisionFive 2 平台（新增）
arch/riscv64/src/platform/mod.rs         # 平台选择
arch/riscv64/src/memory/layout.rs        # 平台化加载地址
kernel/Cargo.toml / build.rs             # feature 透传 + 链接脚本选择
scripts/build.sh                         # PLATFORM=visionfive2
scripts/elf-to-uimage.py                 # --arch riscv (26)
Makefile.project                         # kernel-visionfive2 / uImage-vf2
```
