# LS2K1000 开发板移植工作总结

> 分支：`final-beta1`
> 更新时间：2026-08-08
> 目标板：龙芯 2K1000LA 星云板（LS2K1000-DP-FACTORY，CPU LA264，2GB DDR：256MB@0x0 + 1792MB@0x90000000）
> Bootloader：U-Boot 2022.04-v2.1.0 BSP（板内 Linux 系统自带的 U-Boot）
> 状态：**真机 `go` 路径已跑通到 FDT 解析**（对齐异常已修复）；厂商 mkimage 已接入（uImage 生成 + 镜像检查），`bootm` 完整启动待验证

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

### 3.8 厂商 mkimage 与 legacy uImage 协议核对（2026-08-08 源码确认）

引入 `vendor/u-boot-2022.04-2k1000-dp-src` 后，从厂商源码核对镜像协议，结论：

| 项 | 结论 | 厂商源码依据 |
|----|------|--------------|
| 架构名 | `-A loongarch`（显示名 `LoongArch`） | `boot/image.c` uimage_arch 表 |
| 编号 | `IH_ARCH_LA` = **27**（紧跟 RISCV=26，位置枚举） | `include/image.h` |
| 32 位 load/entry | legacy 头 `ih_load`/`ih_ep` 是 `uint32_t`；mkimage 用 `strtoull` 读入但 `image_set_hdr_l` 写入时**截断** → 必须传 `0x90000000` 这类 32 位物理值，不能把 `0x9000000090000000` 塞进头 | `include/image.h`、`tools/default_image.c` |
| 0x90000000 → cached | `boot_jump_linux` 用 `map_to_sysmem(ep)`；`0x90000000` ∈ HIGH_MEM_WIN `[0x80000000, 0x100000000)` → `PHYS_TO_CACHED` = `0x9000000090000000` | `arch/loongarch/lib/bootm.c`、`mach-loongson/mapmem.h` |
| legacy 额外校验 | magic(`0x27051956`) → hcrc → dcrc(verify) → `image_check_target_arch`（`IH_ARCH_DEFAULT=IH_ARCH_LA`，非 LoongArch 镜像拒绝启动） | `boot/bootm.c`、`arch/loongarch/include/asm/u-boot.h` |

**⚠️ 关键发现（已修复 2026-08-08）**：
`CONFIG_LOONGSON_BOOT_FIXUP` 对 MACH_LOONGSON `default y`，厂商 bootm 走
`kernel(linux_argc, linux_argv, bootparam, fdt)`，**FDT 在 $a3**（源：env `fdt_addr`，
是 cached 窗口 VA）。而我们原本把 $a1 当 FDT（主线上游 bootm 的约定是
`kernel(-2, ft_addr, 0, 0)`，FDT 在 $a1）。
**修复**：entry.S 保存 $a3 到 BOOT_ARGS[3]；`rust_entry`/`from_raw` 增第 4 参
（riscv64 忽略）；ls2k1000 `boot_context` 按 FDT magic（0xd00dfeed）识别 $a1/$a3，
并把 cached 窗口前缀剥成物理地址再传给 `with_device_tree`。

**板上 SPI dtb 分区内容不是有效 DTB**（真机 2026-08-08 实测）：
`bootm ... - ${fdt_addr}` 报 `Could not find a valid device tree`，`genimg_get_format`
两个 magic 都不匹配。需要用 `scripts/build-ls2k1000-dtb.sh` 生成 minimal DTB
（物理地址形式，U-Boot 运行时 DTS 的内存节点是 cached 窗口地址、会被内核
48 位掩码拒绝）从 USB 加载，见 §4 命令。

**bootm 搬运行为**（`bootm_load_os`）：`IH_COMP_NONE` + `IH_TYPE_KERNEL` 会
`memmove_wd` 把 payload 从暂存处搬到 `load` 地址。若把 uImage 直接 fatload 到
load 地址本身（`0x9000000090000000`），源==目标做自搬移，虽能工作但依赖 memmove 语义；
更稳妥是**暂存到低内存**（与目标不重叠，见 §4 命令）。

### 3.9 真机发现：LA264 只有 40 位虚拟地址 → 内核页表区必须用 bit39 符号扩展地址（2026-08-08）

**现象**：bootm 跑通 FDT 解析后，内核在激活运行时页表时 panic：

```
panicked at kernel/src/vm.rs:128:13:
unable to activate runtime page table: HardwarePaging(
    VirtualAddressBitsTooSmall { available: 40, required: 48 })
```

**根因**：QEMU 的 LA464 核 VALEN=48，而板载 **LA264 核 VALEN=40**（CPUCFG0
bits[15:12]+1 = 40）。`read_capabilities()` 硬编码要求 48 位虚拟地址。

**LA264 的 40 位地址空间语义**（依据 Loongson 开发者在 GCC 补丁中的说明 +
LoongArch 规范 "Virtual Address Reduction Mode" + Linux `setup_ptwalker`）：

- 虚拟地址缩减：合法地址的 bits[63:40] 必须是 **bit39 的符号扩展**。
  - 用户空间：`[0x0, 0x7f_ffff_ffff]`（bit39=0，即 [0, 2^39)）
  - 内核空间：`[0xffff_ff80_0000_0000, 0xffff_ffff_ffff_ffff]`（bit39=1，符号扩展到 64 位）
- **DMW 窗口（0x8000…/0x9000…）不依赖 VALEN**，内核镜像跑在 DMW1
  `0x9000_0000_9020_0000` 因此不受影响——这也是内核能跑到这步的原因。
- PGD 选择：用户（bit39=0）→ PGDL，内核（bit39=1）→ PGDH，与现设计一致。
- 页表遍历：Linux 的 `setup_ptwalker()` 用 `pgd_i = PGDIR_SHIFT(=39)`、
  `pgd_w = PAGE_SHIFT-3(=9)`，顶层 PGD 取 **bits[47:39]**。40 位内核地址
  符号扩展后 bits[47:39] = 0x1FF = 511，落在 PGD[511]——与 QEMU 平台
  48 位高半区的 PGD 索引**完全一致**，页表几何无需改动。

**修复（cfg 隔离，qemu_virt 不动）**：改动只在 `arch/loongarch64` 内，
全部 `#[cfg(feature = "platform-ls2k1000")]` 门控：

| 文件 | 改动 |
|------|------|
| `memory/layout.rs` | `USER_RANGE` → `[0, 0x80_0000_0000)`；`VMALLOC` → `[0xffff_ff80_0000_0000, 0xffff_ffc0_0000_0000)`；`MODULES` → `[0xffff_ffc0_0000_0000, 0xffff_ffc1_0000_0000)`；`FIXMAP` 不变（0xffff_fffe… 在 40 位范围内同样合法） |
| `memory/paging/geometry.rs` | `VIRTUAL_ADDRESS_BITS` 48→40（仅打印元数据；GEOMETRY/PWCL/PWCH/refill.S 全部不变） |
| `memory/paging/hardware.rs` | `REQUIRED_VIRTUAL_ADDRESS_BITS`、`REQUIRED_PHYSICAL_ADDRESS_BITS` 48→40 |
| `memory/paging/entry.rs` | PTE 物理地址掩码 `PHYSICAL_ADDRESS_BITS` 48→40（匹配 PALEN） |

**为什么页表几何不用变**：内核页表区从 48 位负地址（0xffff8000…，在 LA264 上
bits[63:40]=0xFFFF80 不是合法符号扩展）搬到 bit39 符号扩展地址（0xffff_ff80…，
bits[63:40]=0xFF 合法），其顶层 PGD 索引（bits[47:39]）仍是 511，与硬件遍历
行为一致。用户区缩到 [0, 2^39)，bit39=0 → PGD[0]。

**注意**：上述结论建立在"LA264 页表遍历使用完整 64 位符号扩展地址"（Linux
`setup_ptwalker` 固定 9 位顶层即证明）之上。若真机上出现 TLB refill 类故障，
备选方案是把顶层改为 bit39 宽 1（`PWCH.Dir3_width` 9→1 + `indices()` 顶层掩 1 位）。

### 3.10 真机发现：LA264 的 STLBPS 未实现（写被忽略、读回 0）（2026-08-08）

**现象**：VALEN=40 修复后再次上板，bootm 跑完整个初始化链（memory→buddy→
heap→trap→irq→time）后在 `vm.rs:128` 换一个 panic：

```
panicked at kernel/src/vm.rs:128:13:
unable to activate runtime page table: HardwarePaging(
    RegisterMismatch { register: "STLBPS", expected: 12, actual: 0 })
```

**根因**：`activate()` 写 `STLBPS = PAGE_SHIFT (12)` 后读回校验，但 **LA264
不实现 STLBPS**（CSR 0x1E，手册标注 STLB/Static TLB 域）：写入被忽略、读回
恒为 0。关键旁证：**同一次运行里 PGDL/PGDH/PWCL/PWCH 的回读校验全部通过**，
说明 LA264 的 CSR 表与 QEMU 完全一致，唯独 STLBPS 是"手册有、硅片无"。

**为什么不影响正确性**（依据 QEMU [CSR_STLBPS 补丁系列]
(https://patchew.org/QEMU/20250903084827.3085911-1-maobibo@loongson.cn/20250903084827.3085911-6-maobibo@loongson.cn/)）：

- STLBPS 只是**硬件 STLB（单页大小 TLB）缓存**的页大小配置。refill 时页大小
  等于 STLBPS 的条目进 STLB，否则进 MTLB。STLBPS 无效时条目照常进 MTLB，
  无正确性问题，仅失去一点缓存收益。
- 本内核的 TLB 项**全部**来自 refill 路径（`refill.S` 的 `ldpte`/`tlbfill`，
  页大小取自 refill 期间硬件维护的 `TLBREHI.PS`），软件从不直接 `TLBWR`，
  因此 `TLBREHI.PS` 软件写值是否回读同样无关紧要。
- Linux 也写 STLBPS 但从不在写后回读校验，所以真机上该差异对 Linux 无感；
  本内核的 `verify_register` 自我校验是唯一暴露点。

**修复（cfg 隔离，qemu_virt 不动）**：`memory/paging/hardware.rs` 把 STLBPS
与 TLBREHI.PS 的回读校验抽成 `verify_page_size_registers()`：

- 非 ls2k1000（QEMU LA464）：保持原样校验（LA464 这两项是普通 R/W，回读一致）。
- ls2k1000：跳过两项回读校验（STLBPS 写保持——对 LA264 是无害空操作，对 QEMU
  仍启用 STLB）。

**注意**：LA264 的 `TLBREHI.PS` 是硬件在 refill 异常时维护的字段，即便本次
没在校验点暴露，也不代表软件写值会被硬件采用——页大小以 refill 时硬件写入
的为准（本内核恒为 4K，PS=12），无需软件干预。

## 4. 当前调试状态（2026-08-08 真机）

| 路径 | 结果 |
|------|------|
| `go` + 最小诊断代码（虚拟 UART） | ✅ 打印 `MYOSX` |
| `go` + 完整内核（旧，无 -ual） | ❌ U-Boot `CPU0 exception!`（未对齐 `ld.w` → AddressNotAligned，见 §8）|
| **`go` + 完整内核（-ual 修复后）** | ✅ **跑通到 FDT 解析**：print_boot_info 全输出 → `kernel_main` → `BOOT00` → `verify_loongarch_high_mapping()` 通过（DMW0/DMW1 正确、high execution verified）→ 在 `main.rs:226` 因 `go` 传入的 argv 指针非 FDT 而 panic（**预期**）|
| `bootm` + 厂商 mkimage uImage + minimal DTB | ✅ **跑通到激活运行时页表**：FDT 识别 → $a3 传参 → BOOT00 → DMW 校验 → FDT 解析（/cpus 修复后 cpu count=2）→ 内存初始化 → buddy → heap → trap/irq/time 全部完成 → 卡在 `vm.rs:128` **VALEN=40**（已修复 §3.9）→ 再卡 **STLBPS 未实现**（已修复 §3.10，重新上板验证，预期通过寄存器校验并继续 virtio/device/fs） |

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

# ── 完整启动：bootm（厂商 mkimage uImage + minimal DTB；都暂存低内存）──
#    uImage 暂存物理 0x02000000（cached 0x9000000002000000）
#    kernel 目标物理 0x90000000（cached 0x9000000090000000），两者不重叠
#    minimal DTB 加载到物理 0x0a000000（cached 0x900000000a000000），与暂存区分开
#    注意：板上 SPI dtb 分区不是有效 DTB，别用 `sf read ${fdt_addr} dtb`；
#    用 build-ls2k1000-dtb.sh 生成 ls2k1000-minimal.dtb 从 USB 加载
sf probe
fatload usb 0:1 0x9000000002000000 kernel-ls2k1000.uImage
fatload usb 0:1 0x900000000a000000 ls2k1000-minimal.dtb
iminfo  0x9000000002000000             # 应显示 LoongArch + 两个 CRC OK
bootm   0x9000000002000000 - 0x900000000a000000
# 或省略第三参数，靠 BOOT_FIXUP 读 env fdt_addr（此时已指向 valid minimal DTB）：
# bootm   0x9000000002000000

# ── 或 TFTP（tftp 不做 LMB 检查，裸地址即可）──
tftp 0x90000000 kernel.bin
go 0x90000000
```

## 5. 构建方法（WSL Ubuntu）

前置：rustup nightly-2025-01-18 + `loongarch64-unknown-none-softfloat` target，Python3，
gcc/make（WSL 内，厂商 mkimage 需要）。
**必选：`-C target-feature=-ual` 已写入 cargo 配置（见 §3.7）；uImage 必须由厂商
mkimage 生成（见 §3.8），不能手工拼头或使用系统 mkimage。**

```bash
# 1. 构建/复用厂商 mkimage（首次约 1-2 分钟，之后秒回）
make ls2k1000-mkimage

# 2. 编译 ls2k1000 内核 ELF（自动依赖厂商 mkimage）
make kernel-ls2k1000

# 3. 产物链：kernel ELF -> kernel.bin -> 厂商 mkimage -> uImage
make kernel-ls2k1000.bin
make kernel-ls2k1000.uImage

# 4. 镜像检查（ELF/uImage/CRC/arch/payload/暂存不重叠）
make check-ls2k1000-image

# 产物（build/ 目录已被 .gitignore 忽略）
kernel-ls2k1000.elf       # 内核 ELF
kernel-ls2k1000.bin       # raw 二进制（go 命令用）
kernel-ls2k1000.uImage    # 厂商 mkimage uImage（bootm 用, arch=27, load=entry=0x90000000）
uImage / kernel.bin       # 旧名别名（板卡更新菜单 / go）
```

- 手动覆盖 mkimage：`LS2K1000_MKIMAGE=/path/to/mkimage make kernel-ls2k1000`
- 厂商 mkimage 构建产物：`build/host-tools/ls2k1000/mkimage`（不安装到 /usr/bin）
- 非 ls2k1000 平台（riscv/qemu）仍走纯 Python 头路径（不带 `--platform ls2k1000`）

## 6. 待办事项（TODO）

- [x] 真机验证 DDR 基址引导（`go` 路径已通；`bootm` 待验证）
- [x] 编译器 `-ual` 问题：**已在构建侧禁用 `ual`**（§3.7），内核不再触发未对齐异常
- [x] 厂商 mkimage 接入：`build-ls2k1000-mkimage.sh` + `elf-to-uimage.py --platform ls2k1000`
      + `check-ls2k1000-image`（§3.8/§5）
- [x] **kernel 侧 FDT 传参适配（§3.8 发现）**：entry.S 保存 $a3、`rust_entry`/`from_raw` 增第 4 参、
      `boot_context` 按 FDT magic 识别 $a1/$a3 并剥 cached 前缀（2026-08-08 已实现并编译通过）
- [x] **minimal DTB 补 /cpus 节点**：内核 SMP 发现要求读 `/cpus/cpu@*/reg`，否则
      `MissingRequiredNode("/cpus")` panic；已加 cpu@0/1（commit 39e49668，离线 myos-fdt 验证通过）
- [x] **LA264 VALEN=40 页表适配（§3.9，2026-08-08 已实现并两平台编译通过）**：
      内核页表区搬到 bit39 符号扩展地址、用户区缩到 [0, 2^39)、能力检查 48→40，
      全部 cfg 隔离，qemu_virt 不变
- [ ] **`bootm` 完整启动（真机，重新上板验证 VALEN=40 + STLBPS 两处修复）**：
      `sf probe; fatload usb 0:1 0x9000000002000000 kernel-ls2k1000.uImage;
      fatload usb 0:1 0x900000000a000000 ls2k1000-minimal.dtb;
      iminfo 0x9000000002000000; bootm 0x9000000002000000 - 0x900000000a000000`
      —— 预期过页表激活（`kernel vm:` / `runtime pgtbl` / `address bits VA=40 PA=40`）继续初始化
- [ ] 验证 FDT 内存解析：minimal DTB 声明 [0x90000000, 0x100000000) 1792MiB，
      内核应打印该 RAM 区域并完成内存初始化（如需要再调整 DTB 内存节点）
- [ ] **用户态 ALE 异常处理**：OSCOMP 用户程序用 GCC/musl 编译（其 LoongArch 默认开 ual），
      可能产生未对齐访问触发 ALE；内核 `trap.rs` 需为 user mode 加 Ecode 0x09 处理
- [ ] 副核 SMP 启动（`secondary.S`、`rust_main_secondary` 桩）
- [ ] 串口/网卡/存储驱动在真机上的轮询适配（RocketOS 采用纯轮询驱动）

## 6.5 Stage-4：外部 initramfs 上板（Gate A）

厂商 U-Boot 没有 raw initrd 支持（LoongArch 未定义 `CONFIG_SYS_BOOT_RAMDISK_HIGH`，
`images->initrd_start/end` 保持 0），内核只从 FDT `/chosen` 的
`linux,initrd-start/end` 获取 initrd。因此用 U-Boot 把 newc cpio 直接加载到
固定地址，由 DTB 构建脚本把范围写进 `/chosen`，仍用 `bootm kernel - dtb`。

构建：

```bash
make -f Makefile.project m14-vendor-userland-audit-strict
make -f Makefile.project busybox-initramfs-loongarch64   # build/initramfs/busybox-loongarch64.cpio（72 entries）
make -f Makefile.project ls2k1000-stage4-bundle          # uImage + cpio + stage4 DTB + 审计
```

地址布局（U-Boot cached VA = 物理地址 + 0x9000000000000000）：

| 产物             | 物理地址      | U-Boot cached 地址         |
| ---------------- | ----------: | -----------------------: |
| kernel uImage    | `0x02000000` | `0x9000000002000000` |
| DTB（stage4）     | `0x0a000000` | `0x900000000a000000` |
| raw initramfs    | `0x0b000000` | `0x900000000b000000` |

上板：

```text
usb reset
fatload usb 0:1 0x9000000002000000 kernel-ls2k1000.uImage
fatload usb 0:1 0x900000000a000000 ls2k1000-stage4.dtb
fatload usb 0:1 0x900000000b000000 busybox-loongarch64.cpio
bootm 0x9000000002000000 - 0x900000000a000000
```

Gate A 判读（必须看到）：

```text
  initrd        : [0x000000000b000000, 0x000000000b2446b4) 2321 KiB
initramfs:
  external      : [0x000000000b000000, 0x000000000b2446b4)
  rootfs entries: 72
oscomp-la-ale: fixup count=1 era=0x... badv=0x...   ← 预期，见下
M14 BusyBox rootfs gate:
  /bin/busybox true : verified
SMOKE_TEST: PASS
```

`rootfs entries: 72` + `/bin/busybox true : verified` 即传递门通过；未通过则
不进入 PID 1 改造（Commit 4.4+）。

**ALE fixup 说明**：BusyBox 是 GCC/musl 编译的 LoongArch 静态 ELF，默认开 ual，
会执行未对齐访存并依赖 OS 修复（LA264 抛 ALE，Ecode 0x09）。内核 `user.rs`
对 ls2k1000 提供未对齐访存模拟（decode ld/st → 字节读写 → era+=4），
`oscomp-la-ale: fixup count=N`（前 16 次）证明模拟器生效。没有该修复时，
`/bin/busybox true` 会以 `-EFAULT(-14)` 退出并触发断言 panic
（`user.rs:999 assertion left == right failed: left: -14`）。qemu_virt 不启用
该模拟器，保持原有 fail-fast 行为。

**ALE 解码表**（`kernel/src/user/ale_decode.rs`，与独立自检
`scripts/ale_decode_check.rs` 同一文件）分两组：
- `ldptr/stptr`（si14）：8 位操作码在 bits[31:24]，mask `0xff00_0000`，
  `ldptr.w=0x2400_0000`、`stptr.w=0x2500_0000`、`ldptr.d=0x2600_0000`、
  `stptr.d=0x2700_0000`；
- `ld/st`（si12）：10 位操作码在 bits[31:22]，mask `0xffc0_0000`，
  `ld.b=0x2800_0000` … `ld.wu=0x2a80_0000`。

访存地址直接取 `badv`（ALE 时硬件记录的实际故障地址），不重新推导
rj+si12/si14（避免 si14<<2 规则）。指令从 `badi`（非零时）或 `era` 取回。
模拟失败时打印 `oscomp-la-ale-fail: era=… badv=… badi=… reason=…`（前 8 次），
`reason=UnsupportedOpcode(0x…)` 说明缺哪类编码（如 FP 未对齐 ld/st），
然后进程按 -EFAULT 终止。

## 6.6 Stage-4：rdinit 启动 + PID 1（Gate B/C，Commit 4.4~4.7）

Gate A 通过后，内核支持两种 userland 启动模式，由 `/chosen/bootargs` 分流
（`kernel/src/main.rs` 的 `UserlandBootMode`，在 `user::verify()` 之前分流，
避免测试进程抢先消耗 PID 1）：

- **SelfTest**（无 `rdinit=`）：保持原有 M8/M9/M10 + BusyBox true + oscomp
  自检序列 —— qemu_virt（kernel-la）与 stage-4 DTB 回归基线不受影响；
- **InitramfsInit**（`rdinit=/init`）：跳过全部会创建测试进程的自检，
  直接启动真正的 `/init` 作为 PID 1。

四个提交的职责：

| Commit | 内容 | 文件 |
| --- | --- | --- |
| 4.4 | initramfs 增 `/etc/inittab`、`/etc/profile`、`/sbin/{init,reboot,poweroff,halt}` | `scripts/build-static-busybox-initramfs.py`、`scripts/m14-busybox-artifact-audit.py` |
| 4.5 | rdinit 启动模式 + `ls2k1000-stage4-init.dtb` | `kernel/src/main.rs`、`scripts/build-ls2k1000-dtb.sh`、`Makefile.project` |
| 4.6 | 真 PID 1：VFS exec + `init_supervisor` | `kernel/src/exec.rs`、`kernel/src/user.rs` |
| 4.7 | UART RX 轮询（10ms delayed work） | `arch/.../ls2k1000/console.rs`、`arch/.../qemu_virt/console.rs`、`kernel/src/console.rs` |

inittab：

```text
::sysinit:/bin/echo SUDOOS_INIT_READY
::askfirst:-/bin/sh
::restart:/sbin/init
```

`profile` 导出 `PATH/HOME/TERM/PS1='sudoos:${PWD}# '`（动态目录提示符，
`cd` 后提示符跟随当前目录）。`/init` 是 busybox 的
symlink，argv[0]=`/init` 触发 busybox init applet。

PID 1 路径（`user::init_supervisor`）：`exec::kernel_execve_from_vfs("/init")`
→ 断言 PID==1（`NEXT_PROCESS_ID` 从 1 开始，InitramfsInit 下首个进程）→
`exec_elf` 已把 `/dev/console` 装到 fd 0/1/2 → spawn 用户线程 →
另起 kernel thread 监控 PID 1 退出（`INIT-EXIT` 后 panic）→ 内核进
`boot_idle_loop`。

UART RX：ls2k1000 console 读 NS16550 LSR bit 0（DR）→ `try_read_console_byte()`；
qemu_virt 恒返回 None。kernel 的 `console::start_uart_input_poller()` 用
workqueue `queue_delayed(10ms)` 自续轮询，每 tick 最多 64 字节喂给
`tty::input_byte`（回显、唤醒 /dev/console 读、Ctrl-C 投 SIGINT）。仅在
rdinit 模式启用；qemu_virt 编译为空操作。

新 DTB（保留 stage-4 作 Gate A 回归基线）：

```bash
make -f Makefile.project ls2k1000-stage4-init.dtb
#   bootargs = "console=ttyS0,115200n8 rdinit=/init init.debug=1"
```

上板序列与 Gate A 相同，只换 DTB：

```text
usb reset
fatload usb 0:1 0x9000000002000000 kernel-ls2k1000.uImage
fatload usb 0:1 0x900000000a000000 ls2k1000-stage4-init.dtb
fatload usb 0:1 0x900000000b000000 busybox-loongarch64.cpio
bootm 0x9000000002000000 - 0x900000000a000000
```

Gate B 判读（UART RX 未接通前按键无响应属预期）：

```text
userland boot: rdinit=/init
tty: uart rx poller active (10 ms delayed work)
INIT: exec pid=1 path=/init
SUDOOS_INIT_READY
Please press Enter to activate this console.
```

Gate C（RX 轮询生效后）：按 Enter 出现 `sudoos:/#` 可交互 shell（PS1 内嵌
`${PWD}`，`cd /bin` 后提示符变为 `sudoos:/bin#`）。若 shell 后
缺系统调用/ioctl，按 `init.debug=1` 观察 `unknown-syscall`、`ioctl-fail`、
`INIT-EXIT`、`oscomp-la-ale-fail`、`user-exception` 逐项补齐（Commit 4.8），
不预先实现未观察到的调用。

### Gate C 最终回归（Commit 4.8.12–4.8.16）

一次短按 Ctrl-C 只产生一个 VINTR 字节，串口日志应出现一次
`TTY-SIGINT: seq=N sid=… pgrp=… delivered=…`（预算 64 条后停止打印）。
热路径诊断（UART-RX/TTY-READ/TTY-IOCTL/clone-frame/mprotect: ok/
pipe-create/HEAP-STATE）必须为零。

**A. 动态提示符**

```sh
cd /bin; pwd      # sudoos:/bin#
cd /tmp; pwd      # sudoos:/tmp#
cd /; pwd         # sudoos:/#
```

**B. 可中断 sleep（nanosleep EINTR + 相对睡眠剩余时间）**

```sh
start=$(date +%s); sleep 30
# 一次短按 Ctrl-C
rc=$?; end=$(date +%s); echo rc=$rc elapsed=$((end-start))
```
判读：`rc=130`（128+SIGINT），`elapsed < 3`。

**C. 管道进程组信号（pipe read EINTR）**

```sh
sh -c 'sleep 30 | cat'
# 一次短按 Ctrl-C
ps; echo survived
```
判读：`sleep`/`cat` 均退出（无残留），shell 存活并回提示符。

**D. VEOF 边界（partial line 不附加 `\n`）**

```sh
cat; abc
# Ctrl-D：输出 abc 且不带换行（read 返回 3 字节 "abc"，不补 '\n'）
cat
# 空行 Ctrl-D：read 返回 0，cat 退出
```

**E. 压力项**

- 长字符串（`printf '0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ\n'`）连续粘贴 ≥20 次，每次完整无丢字；
- `exit` → Enter → 重新出现 `Please press Enter to activate this console.` + `sudoos:/#`，循环 20 次；
- 全程无 `unknown-syscall`、panic、OOM、`oscomp-la-ale-fail`。

## 7. 相关文件清单

```
arch/loongarch64/src/platform/ls2k1000/    # 平台代码（新增）
arch/loongarch64/src/platform/qemu_virt/   # qemu 平台启动文件（拆分）
arch/loongarch64/src/memory/layout.rs      # 平台化内存布局
Makefile.project                           # 三平台切换 + 厂商 mkimage 链路
scripts/build.sh                           # PLATFORM 透传
scripts/build-ls2k1000-mkimage.sh          # 厂商 mkimage 构建（新增）
scripts/build-ls2k1000-dtb.sh              # minimal 内核 DTB 构建（新增）
scripts/elf-to-uimage.py                   # ELF→uImage 转换（--platform ls2k1000 走厂商 mkimage）
scripts/check-ls2k1000-image.py            # uImage 镜像检查（新增）
vendor/u-boot-2022.04-2k1000-dp-src/       # 厂商 U-Boot BSP 源码（新增，参考）
ls2k1000_manual.txt                        # 开发板手册文字提取（参考）
```
