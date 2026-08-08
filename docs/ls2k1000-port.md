# LS2K1000 开发板移植工作总结

> 分支：`final-beta1`
> 更新时间：2026-08-08
> 目标板：龙芯 2K1000LA 星云板（LS2K1000-DP-FACTORY，CPU LA264，2GB DDR：256MB@0x0 + 1792MB@0x90000000）
> Bootloader：U-Boot 2022.04-v2.1.0 BSP（板内 Linux 系统自带的 U-Boot）
> 状态：**真机 `go` 路径已跑通到 FDT 解析**（对齐异常已修复），`bootm` 完整启动待验证

## 1. 项目背景与目标

MyOS/SudoOS 内核原本支持 riscv64 与 loongarch64（QEMU virt）两个 QEMU 目标。
本次工作目标：把 loongarch64 内核**真实移植到龙芯 2K1000 开发板**上启动运行。

当前阶段的核心是解决**从板载 U-Boot 启动内核**的问题。

## 2. 已完成的工作

### 2.1 新增 ls2k1000 平台代码

新目录 `arch/loongarch64/src/platform/ls2k1000/`：

| 文件 | 作用 |
|------|------|
| `entry.S` | 主核/副核汇编入口：保存 U-Boot 参数、关中断、CPUID 分流、建立 DMW 窗口、开启 PG、跳转 rust_entry |
| `linker.ld` | 内核链接脚本，**AT() 加载 + DDR 基址**（详见 §3.5） |
| `console.rs` | NS16550 UART0 控制台，物理 `0x1fe2_0000`，非缓存虚拟地址 `0x8000_0000_1fe2_0000` |
| `boot.rs` | `boot_context()` 解析 U-Boot 传入的参数（FDT 地址等），`rust_main_secondary()` 副核入口桩 |
| `memory.rs` | `reserve_early_memory()` 预留 U-Boot 启动数据区 |
| `secondary.S` | 副核启动入口（SMP 预留） |
| `mod.rs` | 平台模块聚合与导出 |

同时把原 `arch/loongarch64/src/asm/entry.S`、`asm/secondary.S`、根级 `linker.ld` 删除，
按平台拆分为 `qemu_virt/` 与 `ls2k1000/` 两个平台各自的启动文件。

### 2.2 内存布局平台化

`arch/loongarch64/src/memory/layout.rs` 增加了按 cargo feature 区分的常量：

- qemu_virt：`BOOT_PHYS_BASE = 0x0020_0000`，`BOOT_VIRT_BASE = 0x9000_0000_0020_0000`
  `KERNEL_PHYS_BASE = 0x0040_0000`，`KERNEL_LINK_BASE = 0x9000_0000_0040_0000`
- ls2k1000：`BOOT_PHYS_BASE = 0x9000_0000`，`BOOT_VIRT_BASE = 0x9000_0000_9000_0000`
  `KERNEL_PHYS_BASE = 0x9020_0000`，`KERNEL_LINK_BASE = 0x9000_0000_9020_0000`

### 2.3 Makefile 三平台切换

`Makefile.project` 增加 `PLATFORM ?= ls2k1000`（默认即开发板），以及：

```makefile
make kernel-ls2k1000   # 交叉编译 release 内核 ELF
make kernel.bin        # 生成 raw 二进制（供 U-Boot 'go' 命令）
make uImage            # 生成 U-Boot uImage（arch=27, LoongArch）
make all               # 三平台全部产物
```

### 2.4 uImage 转换脚本

`scripts/elf-to-uimage.py`：纯 Python 3（仅标准库），无需 mkimage 即可把 ELF 转成 U-Boot uImage。

```bash
# uImage（供 bootm）
python3 scripts/elf-to-uimage.py kernel-ls2k1000 uImage

# raw 二进制（供 go）
python3 scripts/elf-to-uimage.py --raw kernel-ls2k1000 kernel.bin
```

转换脚本已支持 AT() 链接的 ELF（段 VMA 是高 DMW 地址、e_entry 是低物理地址），
正确输出 `load=0x9000_0000`、`entry=0x9000_0000`。

### 2.5 其它

- `arch/loongarch64/Cargo.toml`：去掉默认平台 feature，避免与 kernel 的 `--no-default-features` 冲突
- `kernel/src/task/mod.rs`：WORKER 数组从 8 扩到 16（MAX_CPUS）
- `scripts/build.sh`：支持 `PLATFORM` 变量透传 cargo features

## 3. 关键技术结论（踩坑记录）

### 3.1 U-Boot 2022.04 的 arch 枚举是"位置枚举"，与新版 mkimage 不同

| 架构 | U-Boot 2022.04 BSP | mkimage 2025.10 |
|------|--------------------|-----------------|
| MIPS | 5 | 10 |
| MIPS64 | 6 | 35 |
| LoongArch | **27**（紧跟 RISCV=26） | 43 |

uImage 头中 `ih_arch = 27` 才能被板载 U-Boot 识别为 "LoongArch U-Boot Kernel Image"。

### 3.2 UART 地址（决定性发现）

板级 FDT 显示串口为 `serial@1fe20000`，**不是** QEMU 风格的 `0x1fe001e0`。
- 物理地址：`0x1fe2_0000`
- 非缓存虚拟地址（PG=1 下访问）：`0x8000_0000_1fe2_0000`
- 寄存器：`+0` 数据、`+5` LSR，NS16550 轮询写。

### 3.3 DMW 窗口与 PG

- `DMW0 = 0x8000000000000001`（VSEG=0x8，uncached）、`DMW1 = 0x9000000000000011`（VSEG=0x9，cached）
- 开启 PG 前需要临时 `DMW0 = 0x11`（VSEG=0）让当前低地址 PC 继续取指，跳高地址别名后再重配
- PG=1 后**物理地址访问会触发 TLB 异常**，所有外设访问必须走 DMW 虚拟地址
- **2K1000 的 1GB DDR 物理基址是 `0x9000_0000`**（非 0x0），内核必须加载到 DDR 内

### 3.4 go vs bootm vs bootelf

- `go <addr>`：裸跳转，不解析镜像头，DMW 保持 PG=1 → **当前唯一能跑通的路**
- `bootm <addr>`：解析 uImage 头 → memmove 到 load 地址 → 跳转
- `bootelf <addr>`：解析 ELF 段 → 加载到物理地址 → 跳转

### 3.5 链接/加载地址的根因修正（本次改进核心）

对照 RocketOS（哈工大，决赛一阶段第 2 名，2K1000 工作版）的分析，发现
**旧版 ls2k1000 链接/加载地址方案存在四个根因问题**：

| 问题 | 旧方案 | 修正 |
|------|--------|------|
| linker.ld 无 AT()，paddr=vaddr=高 VMA `0x9000_0000_0020_0000` | uImage 的 32 位 load/entry 字段溢出；bootelf 加载到 36TB 物理地址；raw 提取产生超大二进制 → 三种引导全部损坏 | 改为 **AT() 策略**（与 qemu_virt 已验证结构一致）：段按高 VMA 链接、AT() 加载到低物理地址，`ENTRY(__kernel_entry_phys)` |
| 镜像放在 `0x0020_0000`，不在 DDR | 2K1000 DDR 物理基址是 `0x9000_0000`，镜像落在 SoC 低地址区而非真实内存 | 加载/链接搬到 **DDR 基址**：`.boot`@`0x9000_0000`，正式内核@`0x9020_0000`，uImage load=entry=`0x9000_0000`（32 位可容纳） |
| `boot.rs` 把 FDT 地址 `\| CACHED_BASE` | `kernel_main` 把 `device_tree()` 当**物理地址**传给 `ram_ptr` → `phys_to_cached` 的 48 位掩码检查直接拒绝 → 启动早期 FDT panic | FDT / command line **原样透传物理地址**（去掉 OR） |
| entry.S 开头物理 UART 写 'B' | `go` 路径进入时 U-Boot 保持 PG=1，物理写触发 TLB 异常 | **CRMD.PG==0 才写**物理串口；PG=1 走 DMW 虚拟 UART |

**为什么这样可行（对照 RocketOS）：**
- RocketOS 的 2K1000 内核直接链接在物理 `0x90000000`（`KERNEL_BASE=0`，VMA=PADDR），
  DMW 窗口按当前 PC 的段号动态配置，内核全程跑在低物理地址上——从根上绕开了
  "uImage 32 位地址装不下高 VMA" 的问题。
- 我们的内核保持"高半 VMA + DMW"模型不变（`KERNEL_LINK_BASE` 仍在 cached DMW 内、
  `indices(KERNEL_LINK_BASE).is_none()` 仍成立），只把物理加载地址搬到 DDR，
  **mm / task / trap 全部无需改动**，风险远低于整套改成 KERNEL_BASE=0。
- 注意：RocketOS 的 `docs/content/board.typ` 第 8 行称 2K1000 是"MIPS64 指令集"，
  这是笔误（实际是 LoongArch），不影响其适配结论。

### 3.6 真机发现：U-Boot 的 LMB 用"缓存窗口地址"（决定性）

板载 U-Boot 的 `fatload`/`ext4load` 会做 **LMB（Logical Memory Block）保留区检查**，
`lmb_dump_all` 显示内存区全部是 **64 位缓存窗口虚拟地址**：

```
memory[0]  [0x9000000000000000-0x900000000fffffff]   256MB  @ 物理 0x0
memory[1]  [0x9000000090000000-0x90000000ffffffff]  1792MB  @ 物理 0x90000000
reserved[0] [0x900000000cbf4c90-0x900000000ebfffff]         ← U-Boot 自身
reserved[1] [0x900000000f000000-0x900000000fffffff]
```

**给裸物理地址 `0x90000000` 会直接报 `Reading file would overwrite reserved memory`**
（裸值不在任何 memory 区）。必须用对应的缓存窗口地址 **`0x9000000090000000`**。
厂商环境变量印证：`loadaddr=0x9000000098000000`、`fdt_addr=0x900000000a000000`、
`boot_params=0x900000000cc17740`，全部是缓存窗口形式。

- 加载命令：`fatload usb 0:1 0x9000000090000000 kernel.bin`
- `go` 也跳缓存窗口地址：`go 0x9000000090000000`
- 本板 **2GB DDR 分两个 bank**：物理 `[0x0, 0x10000000)` 256MB + `[0x90000000, 0x100000000)` 1792MB。
  内核的 `PHYS_MEMORY_SIZE=2GB` 兜底假设 [0x90000000, +2GB) 并不精确（该 bank 只有 1792MB），
  但真实启动以 FDT memory 节点为准，此兜底仅在 FDT 缺失时使用。

### 3.7 真机发现：rustc 默认启用 `ual` → 未对齐异常（关键坑）

`rustc --print cfg --target loongarch64-unknown-none-softfloat` 显示 **`target_feature="ual"` 默认启用**。
LLVM 因此自由生成未对齐的 `ld.w`/`st.w`（如 `core::fmt` 构建 `Arguments` 时 `ld.w $sp, 225`）。
而 **2K1000 的 LA264 硬件不支持 UAL**，未对齐访问触发 AddressNotAligned 异常。

此时内核还没装自己的异常向量（EENTRY 仍指向 U-Boot），异常被 **U-Boot 的
`exception_vector`**（`vendor/u-boot.../arch/loongarch/cpu/start.S`）捕获，
打印 `CPU0 exception!` + CSR 转储后死循环：

```
CPU0 exception!
csr 0x04 (ECFG)  -> 0x0
csr 0x05 (ESTAT) -> 0x90000
csr 0x06 (ERA)   -> 0x90000000902bda1c   ← ld.w $a1, $sp, 225
csr 0x07 (BADV)  -> 0x90000000905b3eb1   ← SP+225（未对齐）
csr 0x08 (BADI)  -> 0x0
```

**修复**：在 cargo 配置中给 loongarch64 target 禁用 `ual`，强制 LLVM 对齐全部访存：

```toml
# .cargo/config.toml 与 cargo-dot/config.toml（构建时恢复 .cargo 的源）
[target.loongarch64-unknown-none-softfloat]
rustflags = ["-C", "target-feature=-ual"]
```

反汇编验证：print_boot_info 中旧版的 `ld.w $sp,225 / st.w $sp,105` 等未对齐访问
全部改为对齐的 `st.d` + `st.b`，崩溃消除。

## 4. 当前调试状态（2026-08-08 真机）

| 路径 | 结果 |
|------|------|
| `go` + 最小诊断代码（虚拟 UART） | ✅ 打印 `MYOSX` |
| `go` + 完整内核（旧，无 -ual） | ❌ U-Boot `CPU0 exception!`（未对齐 `ld.w` → AddressNotAligned，见 §8）|
| **`go` + 完整内核（-ual 修复后）** | ✅ **跑通到 FDT 解析**：print_boot_info 全输出 → `kernel_main` → `BOOT00` → `verify_loongarch_high_mapping()` 通过（DMW0/DMW1 正确、high execution verified）→ 在 `main.rs:226` 因 `go` 传入的 argv 指针非 FDT 而 panic（**预期**）|
| `bootm` + uImage(arch=27) | ⏳ 待真机验证（U-Boot 从 SPI Flash 读 DTB 传给内核） |

**真机串口输出（-ual 修复后，`go` 路径）关键片段：**

```
My / MyOS / firmware args: 0x1 0x900000000cbf5f10 0x900000000cbf5f10
  device tree : 0x900000000cbf5f10   ← go 把 argv 指针当 FDT（非有效 FDT）
entered Rust kernel successfully
kernel_main: initialization started
BOOT00 entry
LoongArch DMW:
  current PC : 0x90000000902c3e48   physical alias : 0x00000000902c3e48
  uncached alias : 0x80000000902c3e48
  DMW0 : 0x8000000000000001   DMW1 : 0x9000000000000011
  high execution : verified
KERNEL PANIC at kernel/src/main.rs:226: unable to map FDT physical address
  0x900000000cbf5f10: AddressOutOfRange
```

**真机 U-Boot 引导命令**（真机实测 2026-08-08）：

```sh
# ── 首选：USB 存储 + go（raw 二进制；注意用缓存窗口地址，见 §3.6）──
fatload usb 0:1 0x9000000090000000 kernel.bin
go 0x9000000090000000

# ── 完整启动：bootm（厂商从 SPI Flash 读 DTB 传 $a1，需 uImage 在存储上）──
sf probe
sf read ${fdt_addr} dtb
fatload usb 0:1 ${loadaddr} uImage     # loadaddr=0x9000000098000000
bootm

# ── 或 TFTP（tftp 不做 LMB 检查，裸地址即可）──
tftp 0x90000000 kernel.bin
go 0x90000000
```

## 5. 构建方法（WSL Ubuntu）

前置：rustup nightly-2025-01-18 + `loongarch64-unknown-none-softfloat` target，Python3。
**必选：`-C target-feature=-ual` 已写入 cargo 配置（见 §3.7），构建命令无需额外参数。**

```bash
# 1. 编译 ls2k1000 内核 ELF
make kernel-ls2k1000

# 2. 生成 uImage（board 更新菜单认这个名字）
make uImage

# 3. 生成 raw bin（go 命令用）
make kernel.bin

# 产物
kernel-ls2k1000        # 内核 ELF
uImage                 # U-Boot uImage (arch=27, load=0x90000000)
kernel.bin             # raw 二进制
```

验证产物地址：

```bash
python3 scripts/elf-to-uimage.py kernel-ls2k1000 uImage   # 应打印 load=0x0000000090000000 entry=0x0000000090000000
python3 scripts/elf-to-uimage.py --raw kernel-ls2k1000 kernel.bin
```

## 6. 待办事项（TODO）

- [x] 真机验证 DDR 基址引导（`go` 路径已通；`bootm` 待验证）
- [x] 编译器 `-ual` 问题：**已在构建侧禁用 `ual`**（§3.7），内核不再触发未对齐异常
- [ ] **`bootm` 完整启动**：`sf probe; sf read ${fdt_addr} dtb; fatload usb 0:1 ${loadaddr} uImage; bootm`
      —— 验证板载 U-Boot 把 SPI Flash DTB 通过 `$a1` 传给内核，走通完整内核初始化
- [ ] 验证 `$a1` 传 FDT；若 bootm 也不传有效 FDT，需在 ls2k1000 平台提供内存布局兜底
      （用 `PHYS_MEMORY_BASE`/`PHYS_MEMORY_SIZE` 合成 minimal FDT 或直接填 BootInfo）
- [ ] **用户态 ALE 异常处理**：OSCOMP 用户程序用 GCC/musl 编译（其 LoongArch 默认开 ual），
      可能产生未对齐访问触发 ALE；内核 `trap.rs` 需为 user mode 加 Ecode 0x09 处理
- [ ] 副核 SMP 启动（`secondary.S`、`rust_main_secondary` 桩）
- [ ] 串口/网卡/存储驱动在真机上的轮询适配（RocketOS 采用纯轮询驱动）

## 7. 相关文件清单

```
arch/loongarch64/src/platform/ls2k1000/    # 平台代码（新增）
arch/loongarch64/src/platform/qemu_virt/   # qemu 平台启动文件（拆分）
arch/loongarch64/src/memory/layout.rs      # 平台化内存布局
Makefile.project                           # 三平台切换
scripts/build.sh                           # PLATFORM 透传
scripts/elf-to-uimage.py                   # uImage 转换脚本（新增，支持 AT() ELF）
ls2k1000_manual.txt                        # 开发板手册文字提取（参考）
```
