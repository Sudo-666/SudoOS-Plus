# SudoOS M17 更新日志

## 概述

M16 完成了从"系统跑不起来 / 扫不到测试"到"能扫到 128 个测试脚本并开始执行"的跨越。M17 在此基础上修复了测试脚本运行环境不完整的核心问题：**lazy install 粒度太小，只装脚本不装同目录测试二进制和工具**。

---

## 问题诊断

M16 的 `mount_sdcard_if_present()` 有界扫描 ext4 目录，发现脚本后**只把脚本文件安装到 VFS**，没有安装脚本同目录的依赖文件（`./busybox`、`./dhry2reg`、`./lmbench_all`、`./cyclictest`、`./netperf` 等），导致脚本执行时报错：

```
./busybox: not found
./dhry2reg: not found
./lmbench_all: not found
cp: not found
sleep: not found
```

**一句话：已经从"扫不到测试"推进到"能执行测试"，卡在运行时依赖文件暴露不足。**

---

## 修复方案

### 核心思路

执行每个 `/mnt/sdcard/<dir>/<xxx_testcode.sh>` 前：

1. 找出脚本所在 ext4 目录（如 `/glibc` 或 `/musl`）
2. lazy install 该目录下所有普通文件到对应 VFS 目录
3. 设置 CWD 为脚本所在目录（M16 已实现）
4. 补全 busybox applet 符号链接
5. 动态构造 PATH / LD_LIBRARY_PATH
6. 执行脚本

### 修改文件

| 文件 | 变更 |
|------|------|
| `kernel/src/main.rs` | `mount_sdcard_if_present()` 中 busybox 安装成功后追加 28 个 applet 符号链接 |
| `kernel/src/user.rs` | `verify_sdcard_all_scripts_thread()` 重构为目录级 lazy install + 动态环境变量 |

---

## 详细变更

### `kernel/src/main.rs` — busybox symlink 补全

**位置：** `mount_sdcard_if_present()` 第 465-479 行

成功安装 `/bin/busybox` 后，除已有的 `/bin/sh`，新增 28 个常用 applet 符号链接：

```
cp, sleep, kill, cat, echo, mv, ln, rm, ls,
mkdir, chmod, grep, dd, mount, ps, head, tail, test,
awk, sed, wc, cut, tr, which, pidof, printenv,
basename, dirname, readlink, stat, getopt
```

消除 `cp: not found`、`sleep: not found`、`kill: not found` 等错误。

### `kernel/src/user.rs` — 核心重构

#### 1. Fallback busybox symlink（第 772-783 行）

如果 mount 阶段没装上 busybox symlink，在 `verify_sdcard_all_scripts_thread()` 中补装。

#### 2. 目录级 lazy install（第 828-831 行）

每个脚本执行前：
- 从 VFS 路径反推 ext4 源码目录：`/mnt/sdcard/glibc/xxx.sh` → ext4 `/glibc`
- 调用 `sdcard_install_ext4_dir_files()` 列出 ext4 目录中所有普通文件
- 安装尚未存在于 VFS 的文件（脚本本身已由 mount 阶段安装，跳过）
- 用 `expanded_dirs` 跟踪去重，每目录仅展开一次

#### 3. 动态 PATH / LD_LIBRARY_PATH（第 847-868 行）

废弃硬编码环境变量，改为运行时探测：

```
PATH=.:<cwd>:/:/bin:/sbin:/usr/bin:/usr/sbin:/usr/local/bin:<存在的ext4子目录>...
LD_LIBRARY_PATH=.:<cwd>:/:/lib:/usr/lib:/usr/local/lib:<存在的ext4 lib目录>...
```

`<cwd>` 被显式加入 PATH，确保 `./busybox` 和 `busybox` 均能解析。

#### 4. 新增辅助函数

**`sdcard_vfs_to_ext4_dir(vfs_path) -> String`**（第 892-902 行）

剥离 `/mnt/sdcard` VFS 前缀，返回 ext4 父目录路径：
- `/mnt/sdcard/glibc/unixbench_testcode.sh` → `/glibc`
- `/mnt/sdcard/musl/busybox_testcode.sh` → `/musl`
- `/mnt/sdcard/test.sh` → `/`

**`sdcard_install_ext4_dir_files(ext4_dir)`**（第 907-940 行）

1. 打开 `/dev/vda` 块设备
2. 调用 `ext4::list_directory()` 列出 ext4 目录条目
3. 过滤 `file_type == EXT4_FT_REG_FILE`（普通文件）
4. 对每项构造 ext4 路径和 VFS 路径
5. 跳过已存在的 VFS 文件（避免重复安装）
6. 调用 `fs::install_ext4_path()` 安装缺失文件
7. 打印 `sdcard: expanded /glibc -> /mnt/sdcard/glibc : N files`

#### 5. 删除死代码

移除 `sdcard_script_cwd()` 函数（逻辑已内联到主循环）。

---

## 提交历史

| 提交 | 说明 |
|------|------|
| `4b044ea` | build: bounded sdcard discovery and execution with lazy ext4 listing |
| *(本次)* | M17: directory-level lazy install + busybox symlinks + dynamic env |

---

## 测试脚本执行流程（M17 完整链路）

```
1. 双架构启动（RV / LA）
2. 内核初始化 → ext4 mount → 有界扫描目录
3. 发现 128 个测试脚本 → 存入 SCANNED_TEST_SCRIPTS
4. 安装脚本文件到 VFS（/mnt/sdcard/glibc/xxx.sh 等）
5. 安装 busybox 到 /bin/busybox + 28 个 applet symlink
6. verify_sdcard_all_scripts 遍历脚本列表：
   a. 确定 CWD = dirname(script)
   b. lazy expand ext4 父目录 → 安装所有普通文件
   c. 动态构造 PATH / LD_LIBRARY_PATH
   d. busybox sh <script> 执行
   e. 输出 PASS / FAIL / ERROR
```

### 预期日志形态

```
sdcard:
  mount         : /dev/vda (ext4)
  mounted tree  : /mnt/sdcard (lazy file install)
  root entries  : 3
  scanned dirs   : 59
  test scripts  : 128

sdcard: expanded /glibc -> /mnt/sdcard/glibc : 12 files
sdcard: expanded /musl -> /mnt/sdcard/musl : 8 files
sdcard: expanded /lmbench -> /mnt/sdcard/lmbench : 4 files

#### OS COMP TEST GROUP START /mnt/sdcard/glibc/unixbench_testcode.sh ####
/mnt/sdcard/glibc/unixbench_testcode.sh : PASS
#### OS COMP TEST GROUP END /mnt/sdcard/glibc/unixbench_testcode.sh ####
```

---

## 与 M16 的关键差异

| 维度 | M16 | M17 |
|------|-----|-----|
| 文件安装粒度 | 仅脚本文件 | 脚本所在目录全部普通文件 |
| busybox symlink | 仅 `/bin/sh` | 28 个常用 applet |
| PATH | 硬编码字符串 | 运行时动态探测拼装 |
| LD_LIBRARY_PATH | 硬编码字符串 | 运行时动态探测拼装 |
| 去重 | 无 | `expanded_dirs` 每目录仅展开一次 |
| 日志 | 无目录展开信息 | `sdcard: expanded /X -> /mnt/sdcard/X : N files` |

---

*Co-Authored-By: Claude <noreply@anthropic.com>*

---

# SudoOS 2026-08-08 更新 — LS2K1000 真机移植与堆损坏调试

## 当前状态

- **分支 `board`**,专用于 LS2K1000 真机移植;所有架构改动均以 `#[cfg(feature = "platform-ls2k1000")]` 隔离,qemu_virt / riscv64 编译不受影响(每次改动后已验证干净)。
- **真机启动链路已全部打通:** U-Boot `bootm` → uImage(厂商 mkimage 生成)→ USB 加载最小 DTB → 内核从 BOOT00 一路初始化到 BOOT13 + tty(调度器初始化之前)。
- 已修复并验证的 LA264 真机差异:
  1. **UAL 未对齐异常** — LA264 无 `target_feature=ual`,LLVM 默认 emit 非对齐 `ld.w/st.w` → `-C target-feature=-ual`;
  2. **bootm FDT 在 $a3** — 厂商 BSP 约定,非上游 $a1 → entry.S 保存 $a3,按 magic(0xd00dfeed)识别并剥离缓存窗前缀;
  3. **SPI 分区 DTB 无效** — 改用自建最小 DTB(USB 加载,含 /cpus + 1792MiB 内存 + UART0);
  4. **LA264 VALEN=40** — 页表区域从 48 位负空间迁至 40 位符号扩展空间;
  5. **STLBPS 寄存器未实现** — 写忽略读 0,跳过该寄存器校验。
- **当前阻塞点:** 调度器初始化处堆分配失败 panic。

## 主要问题(当前阻塞)

### 症状

真机 bootm 在 `task::initialize()` 处确定性 panic:

```
HEAP-STATE[pre-task-init] ... is-some=true
================ KERNEL PANIC ================
panicked at .../alloc/src/alloc.rs:438:13:
memory allocation of 176 bytes failed
```

三轮真机运行,均在调度器初始化、同一 176 字节分配、alloc.rs:438。前两轮最后可见日志行是 `kernel stack: 64 KiB plus guard pages`,第三轮是 pre-task-init 检查点。

### 已核实的关键事实

| 事实 | 依据 |
|---|---|
| 全内核唯一 `#[global_allocator]` = GLOBAL_HEAP | kernel/src/heap.rs:243,全仓库 grep 唯一 |
| `allocate()` 只有 3 条 null 路径 | ①size==0(静默)②heap-None→HEAP-NONE-ALLOC ③Err→HEAP-ALLOC-FAIL,后两条均已打点 |
| 第三轮 heap 完好 | HEAP-STATE[pre-task-init] `is-some=true try-lock=1`;words 健康(Option tag≈0x1,slab 元数据指针指向内核镜像后新分配的 buddy 帧) |
| 两条分配诊断均未触发 | 与"null 必经已打点路径"矛盾 |
| println 不分配内存 | console.rs → core::fmt::write 逐字节写 UART |
| Scheduler::new 的分配均无边界问题 | 2×180 KiB Vecs(buddy)+ 16×8 KiB run_queue + CPU1 idle 栈(vmalloc);SizeClass::for_layout / large.rs 取整与边界校验均正确 |

### 根因分析

代码上,null 只能来自已打点路径 → 两条诊断必有一者触发 → 但输出未出现。**结论:诊断输出被串口截断吞掉**(与三轮中调度器打印块都截断在 `kernel stack` 一行的现象同源;panic 由 panic handler 直接输出所以总能幸存)。

**最可能机制:** `Scheduler::new` 的分配风暴期间,slab 类 256 的空闲链表被**确定性写坏**;风暴后第一个 slab 分配(176 B → 类 256)返回 Err → HEAP-ALLOC-FAIL 打印(被吞)→ null → `handle_alloc_error` panic。确定性同点 ⇒ 确定性 bug(越界写 / 重复分配 / 邻块覆盖),非随机。

**唯一缺的关键数据:** HEAP-ALLOC-FAIL 的 `error=`(具体 HeapError 值)、`n=`(分配计数)、`caller=`(分配点地址)——都在被串口吞掉的行里。

## 下一步诊断(下板必做)

1. **分配失败路径改为致命且自包含:** 裸写 UART(绕过 println / 控制台锁)输出 `error=`/`caller=`/`n=` + slab 空闲链表 dump + halt,让信息直接进入 panic 流、物理上不可被串口吞掉;
2. **修正 `dump_heap_state` 的 words 起点:** 当前从 `&GLOBAL_HEAP.heap`(IrqSpinLock 起点)读,读到的是锁内部状态(如 w0=0x3c),应跨过锁结构后读 HeapAllocator 本体;
3. 拿到真实 `error=` 后按值定位 slab 损坏源(越界写 vs 重复分配)。

## 提交历史

| 提交 | 说明 |
|------|------|
| `39e49668` | scripts: add /cpus node to minimal LS2K1000 DTB |
| `0efc2417` | feat: adapt ls2k1000 to LA264 40-bit VA (cfg-isolated) |
| `19db7e32` | feat: tolerate unimplemented STLBPS on LA264 (cfg-isolated) |
| `5ef00a89` | feat: ls2k1000 heap-corruption diagnostics (HEAP-NONE/STATE/INSTALLED) [cfg-isolated] |

## 待办(后续)

- 用户态 ALE(Ecode 0x09)处理 — OSCOMP LoongArch 用户程序(GCC/musl,默认 ual)可能未对齐访问出错;
- 次核 SMP — 当前 `rust_main_secondary` 为存根(仅驻留),`start_secondaries` 会超时;
- 外设驱动轮询适配。

---

# SudoOS 2026-08-09 更新 — LS2K1000 176B OOM 第 10 轮取证:Round-2 补丁无效,根因收敛到 codegen

## 当前状态

- **Round-2 补丁**(移除 alloc crate 全部 `assert_unchecked` + 2048B 栈 dump + `OOM code-words` 扫描)已上板(18:45 uImage),第 10 轮真机结果:**OOM 签名与第 8/9 轮逐字节一致,补丁无效**。
- 新增 `OOM code-words` 诊断 + addr2line 全量解析,首次拿到失败布局与调用链指纹。
- 新增 `OOM_STACK_SNAP` 入口快照(run 11 准备),解决 handler 自身 fmt 帧污染栈的问题。

## 第 10 轮真机证据

### OOM 签名(三轮不变)

```
OOM-HANDLER size=176 align=10376293543883483600(=0x90000000905aa9d0)
ra=0x90000000902e8514 raw_a0=0x90000000905aa9d0 raw_a1=0xb0 count=89
```

- `align=0x90000000905aa9d0` = `lockdep::MAX_IRQ_OFF_CYCLES` 的 BSS cached-VA(nm 验证)。跨第 8/9/10 三轮完全一致 → 确定性寄存器残留值。
- Round-2 后 `__rg_oom` 从 0x902e8360 移到 0x902e8500(证明新二进制在跑),但失败行为逐字节相同。

### code-words 解析(新证据)

- `KernelGlobalAllocator::allocate`(sp+0x4d8)、`finish_grow`(sp+0x5a8):**最后成功分配 R[088] 的陈旧帧**,非失败路径。
- `reprogram_local`(sp+0x688)、`kernel_main`(sp+0x778):更早的陈旧帧。
- 结论:2048B 实时栈 dump 的 ~70% 是 handler 打印 89 行 ring 时的 fmt 帧(`bool`/`usize`/`LowerHex` fmt),真正触发 OOM 的调用者帧被埋在 sp+0x400 以下未暴露。

### 反汇编确认 swap 链

`handle_alloc_error`(0x902014e4)swap a0/a1 → `__rust_alloc_error_handler`(0x90201000)**不**swap → `__rg_oom`(0x902e8500)swap a0/a1。双 swap 净零 → handler 的 raw_a0/a1 = **失败点原始 layout** = (align=0x90000000905aa9d0, size=176)。

### 关键事实:失败分配从未进入我们的分配器

- ring 记录在 `allocate()` 最顶(volatile 写),count=89 停在 R[088],**无 R[089]**。第 90 次分配(176B)未进 `allocate()`。
- 我们的 `allocate()` 对 size>0 永不返回 null(Err→HEAP_FATAL 停机)。OOM 走了 `handle_alloc_error`,说明 **null 在 alloc crate 层(我们的分配器之上)被制造**。

## 根因假设(已排除两轮,收敛到 codegen)

| 轮次 | 假设 | 结果 |
|---|---|---|
| Round-1 | finish_grow `assert_unchecked` UB 消除分配器调用 | 无效(第 9 轮证明) |
| Round-2 | 移除全部 5 处剩余 `assert_unchecked` | 无效(第 10 轮证明) |
| 现行 | **LoongArch codegen 把 lockdep static 地址当 align 传** | 待验证 |

现行机制:176B 请求的 align 参数是 `MAX_IRQ_OFF_CYCLES` 的地址(`heap.lock` → lockdep 跟踪时加载进寄存器),形成非法 Layout(align 非 2 的幂)。非法 Layout 的 UB 让优化器在调用点之上消除分配器调用并假定失败 → `handle_alloc_error`。这发生在任何我们可补丁的代码之上,所以 alloc crate 和 heap.rs 的补丁都触达不到。

## Run 11 诊断改进(`OOM_STACK_SNAP`)

- 新增 `OOM_STACK_SNAP[256]`(2048B)+ `OOM_STACK_SNAP_SP` 静态,handler 入口(任何 puts/fmt 之前)用 volatile 逐字拷贝栈顶 2048 字节,打印阶段读快照。
- 反汇编验证:ldx.d/stx.d 逐元素 volatile 拷贝,在 puts 之前执行。
- 预期:run 11 的 code-words 将露出 176B 分配点的真实 `$ra`(分配点 → handle_alloc_error 内联 → __rg_oom 帧),addr2line 直接定位。

## 产物与隔离

- kernel-ls2k1000: 7228488 B(uImage 6537280 B,IMAGE CHECK PASS)
- kernel-la(qemu_virt): 8822064 B,**ZERO** ls2k 标记(隔离保持)

## 上板指令(同前)

```
sf probe
fatload usb 0:1 0x9000000002000000 kernel-ls2k1000.uImage
fatload usb 0:1 0x900000000a000000 ls2k1000-minimal.dtb
iminfo  0x9000000002000000
bootm   0x9000000002000000 - 0x900000000a000000
```

看 `stack-snapshot` 段的 `OOM code-words`(应为非 fmt 的真实调用链)。

## 提交历史

| 提交 | 说明 |
|------|------|
| *(本次)* | heap: add OOM_STACK_SNAP entry snapshot + run-10 code-words analysis |

---

# SudoOS 2026-08-09 更新(二)— PR-0 可复现构建 + PR-1 检查点 + PR-3 探针

## 核心结论(评审确认 + 新证据)

评审与取证达成一致:**这次 OOM 排除了"物理内存不足"和"slab 直接返回错误"**。

- 启动链全部通过(BOOT05 buddy / BOOT06 heap / BOOT12 virtio / BOOT13 rootfs),`DMA32 free: 454461 pages`(~1.73 GiB 空闲)。
- 176B 请求未进入 `KernelGlobalAllocator::allocate()`(ring 停在 count=89,无 R[089])→ null 在内核分配器之上被制造。
- 未进入 `Scheduler::new()`(无 `TASK00 enter discovered=2`)→ 停止范围精确到 `dump_heap_state("pre-task-init")` 收尾 → `task::initialize()` 入口。

### 新证据:非法 align 位移 0x808 = 新增 BSS 变量

| run | align | 地址 |
|---|---|---|
| 8/9/10 | `0x90000000905aa9d0` | `lockdep::MAX_IRQ_OFF_CYCLES`(nm 验证) |
| 10(新快照版) | `0x90000000905ab1d8` | `MAX_IRQ_OFF_CYCLES` + **`0x808`** |

`0x905ab1d8 - 0x905aa9d0 = 0x808` = `OOM_STACK_SNAP[256]`(256×8 = 0x800)+ `OOM_STACK_SNAP_SP`(0x8)。**新增 BSS 变量把 `MAX_IRQ_OFF_CYCLES` 整体后移,非法 align 跟着精确移动** —— 这不是随机损坏,而是某条错误路径把此前用过的静态地址当成 `Layout.align`。当前最强调用链:

```
dump_heap_state() → IrqSpinLockGuard::drop() → IrqSaveGuard::drop()
    → record_irq_off() → update_max(&MAX_IRQ_OFF_CYCLES, ...)
         ↑ 该静态地址成为活跃寄存器值
随后某条 cold/error 路径 → 构造/传递 Layout 时 align 字段取到该寄存器
    → handle_alloc_error(Layout { size:176, align:静态地址 })
```

### 评审纠正的三点(已接受)

1. **raw_a0/raw_a1 不代表函数入口寄存器**:inline asm 在普通 Rust 函数内、函数序言之后执行,编译器可能已移动参数/复用 a0/a1。故"双 swap 后 raw_a0/a1 = 原始 Layout"不能作为独立证据——但 **0x808 位移证据独立成立**,不依赖寄存器捕获时序。
2. **2048B 快照不是纯调用栈**:handler 序言已分配局部帧,快照里大量 0x902072e0 是 fmt 指针/局部变量/陈旧栈,不是可靠返回链。
3. **镜像不可由当前分支复现(最严重)**:见 PR-0。

## PR-0:可复现构建(最高优先级,已完成)

### 事实纠正:Round-2 补丁从未进入 repo

`diff` 证实:
- **WSL sysroot 的 alloc.rs 有 `sanitize_layout` + assert_unchecked 移除(手工补丁,未入库)**;raw_vec.rs 移除 4 处 assert_unchecked。
- **repo `vendor/rust-src` 是 pristine nightly-2025-01-18**(alloc.rs 2 处 / raw_vec.rs 4 处 assert_unchecked 与原始源码一致)。
- 旧 `oscomp-prepare-rust-src.sh` 第 42-44 行"目录完整就 exit 0"→ **sysroot 的补丁从未被回退,每次上板镜像都构建自未入库的手改 sysroot**。
- **第 8/9/10 轮是带 sanitize+assert-移除的 sysroot 构建的,仍逐字节同失败** → 失败发生在 `__rust_alloc` 的 sanitize 之上的路径(RawVec 的 `Allocator::allocate` 走 trait 方法,绕过 global shim 的 sanitize),与 ring"未进内核 allocate()"证据自洽。

### 修复

1. **`oscomp-prepare-rust-src.sh` 强制按差异同步**:`diff -rq` 比较 vendor/ 与 sysroot 全树,任一文件内容/存在性不同 → `rm -rf` + `cp -a` 全量重装;并输出 vendored alloc.rs / raw_vec.rs 的 SHA256。
2. **`scripts/build.sh` 写 `<elf>.buildinfo`**:记录 git commit / branch / dirty 文件数、rustc / cargo 版本、release profile(opt-level=3 lto=thin codegen-units=1 panic=abort overflow-checks)、vendored alloc.rs/raw_vec.rs SHA256、ELF SHA256/大小。
3. **`Makefile.project` 新增 `kernel-ls2k1000.buildinfo`**:复制 buildinfo 并追加 uImage SHA256,产物四件套 ELF / uImage / buildinfo(/.map 可选)。
4. **`scripts/ls2k_*.sh` 去硬编码**:`ls2k_addr2line.sh` / `ls2k_verify_build.sh` / `ls2k_verify_la.sh` / `ls2k_handler_strings.py` 改为从 argv 接收 ELF 路径(默认 `./kernel-ls2k1000`),不再写死 `/mnt/d/oskernel...`。
5. **sysroot 已回退 pristine**:本轮构建经 force-sync 把 sysroot 恢复为与 vendored 一致的原始 nightly 源码,上板镜像 = repo + 文档化 toolchain。

## PR-1:定位 dump_heap_state guard 析构 vs Scheduler::new(已完成)

裸串口检查点(全 raw UART,绕过 println/控制台锁):

```
MAIN40 before-preheap        PROBE176-A
  HEAPD00 enter
  HEAPD01 locked             ← 获取 heap 锁后
    → 持锁拷贝 12 words + is_some(volatile 读)
    → drop(lock_state) 显式析构(此处 IrqSaveGuard::drop → MAX_IRQ_OFF_CYCLES)
  HEAPD02 guard-dropped      ← 显式析构完成,未触发故障
    → 打印 line 1(已释放 heap 锁,打印不持锁)
  HEAPD03 line1-done
    → 打印 line 2
  HEAPD04 done
MAIN41 after-preheap         PROBE176-B
  TINIT00 entry
  TINIT01 discovered=2
  TASK00 enter discovered=2  (已有,Scheduler::new 内)
  TASK01..TASK20             (已有,分阶段)
```

判读表:

| 最后输出 | 故障位置 |
|---|---|
| `HEAPD01` | heap guard / lockdep / IrqSaveGuard 析构 |
| `HEAPD02/03` | line1/line2 打印(控制台锁死锁) |
| `HEAPD04` | `dump_heap_state()` 返回过程 |
| `MAIN41` | `task::initialize()` 入口 |
| `TINIT01` | `Scheduler::new()` 调用或函数序言 |
| `TASK01/03/19` | 对应 Vec / run_queue / idle 栈分配 |

关键改进:**显式 `drop(lock_state)`** + **拷贝后释放锁再打印**(不持 heap 锁获取 console 锁、不做长格式化)。

## PR-3:PROBE176-A/B 隔离探针(已完成)

`dump_heap_state` 前后各一次,直接调 `GLOBAL_HEAP.alloc(Layout(176,8))` → 填满 176 字节 → 逐字节校验 → 释放,raw UART 输出:

```
PROBE176-A PASS          ← 分配器本身健康
PROBE176-B PASS          ← guard 析构后分配器仍健康
```

- 探针绕过 RawVec/`__rust_alloc` shim(不构造可疑的 2 字 Layout 跨函数传参),只验证**分配器能否满足 176B**。
- 若 PASS 而真实 176B 仍 OOM → 故障在 Layout/RawVec 构造层(align 损坏),不在 slab/buddy。

## UB 修复:ALLOC_RING 静态写(已完成)

`ALLOC_RING_RA/SIZE` 原是 `static [usize; 128]` 经 `as_ptr() as *mut` + `write_volatile` 改写 → **修改非 UnsafeCell 的 immutable static 是 UB**,编译器可假定数组永不变化。改为:

```rust
static ALLOC_RING_RA: [AtomicUsize; 128] =
    [const { AtomicUsize::new(0) }; 128];
// 写: ALLOC_RING_RA[slot].store(caller, Relaxed)
// 读: ALLOC_RING_RA[slot].load(Relaxed)
```

Relaxed 原子保持 hot path 零开销、对 OOM handler 可见,消除 UB。`OOM_STACK_SNAP` 已是 `static mut`(每 boot 单次进入,无并发),保留。

## 本轮上板产物

- kernel-ls2k1000(ELF)、kernel-ls2k1000.uImage、kernel-ls2k1000.buildinfo(git/toolchain/alloc 哈希/ELF/uImage 哈希)。
- sysroot = pristine nightly-2025-01-18 == vendored == repo(可复现)。

## 上板指令(同前)

```
sf probe
fatload usb 0:1 0x9000000002000000 kernel-ls2k1000.uImage
fatload usb 0:1 0x900000000a000000 ls2k1000-minimal.dtb
iminfo  0x9000000002000000
bootm   0x9000000002000000 - 0x900000000a000000
```

**本次重点**:最后 ~40 行,看 `PROBE176-A/B PASS`、`MAIN40/41`、`HEAPD00-04`、`TINIT00/01`、`TASK00/01/02` 走到哪一行停 —— 直接判定故障在 guard 析构还是 `Scheduler::new` 边界。若仍 OOM,`stack-snapshot` 段的 `OOM code-words` 应为非 fmt 的真实调用链。

# SudoOS 2026-08-09 更新(三)— PR-2:alloc crate 层标量追踪环

## 动机

PR-0 已证实失败分配从未进入内核 `allocate()`(ring 停在 count=89 无 R[089]),结论是"null 在 alloc crate 层被制造"。但那是推论——没有直接证据指出具体是哪一层、shim 到底返回了什么。PR-2 在 vendored alloc 内部插入标量追踪 hook,下一趟上板直接回答:

1. 失败分配(176B)是否到达 alloc crate 的 `alloc()` / `alloc_impl` / shim?
2. shim 返回了什么——`0`(制造 null)还是**垃圾非零**(制造坏指针,与 0x808 位移同源)?

## 实现

`vendor/rust-src/library/alloc/src/trace.rs`(已提交,可复现)定义 16 项标量追踪环 + last-trace,全部 relaxed atomic——无分配、无格式化、无 UART、无固定地址。hook 点(与内核 `ls2k_trace_tag_name` 一一对应):

| tag | 名称 | 位置 | value 含义 |
|-----|------|------|-----------|
| 1 | ALLOC_FN | `alloc()` 入口 | size |
| 2 | ALLOC_IMPL_ENTER | `Global::alloc_impl` 入口 | size |
| 3 | ALLOC_IMPL_AFTER | `alloc_impl` 拿到 shim 返回值后 | **raw_ptr(0=null!)** |
| 4 | EXCHANGE_MALLOC | Box `exchange_malloc` | size |
| 5 | HANDLE_OOM | `handle_alloc_error` rt_error | size |
| 6 | CAP_OVERFLOW | raw_vec `capacity_overflow` | 0 |
| 7 | RAW_ALLOC_OK | raw_vec `try_allocate_in` Ok | size |
| 8 | RAW_ALLOC_ERR | raw_vec `try_allocate_in` Err | size |
| 9 | REALLOC_NULL | `Global::grow_impl` realloc 返回 null | new_size |

`#[no_mangle]` 导出 6 个 getter(`sudoos_alloc_trace_count/last_tag/last_val/ring_len/ring_tag/ring_val`)。内核 `ls2k_alloc_error_handler`(cfg-gated)在 OOM 时新增两处输出:

- **早期哨兵 `TRALL0 tag=N val=X`**:裸 `putdec`,在 disable/mask 之前落地 last-trace,即使后续格式化崩溃也先抓到。
- **`TRALL count=N len=16` + 尾部 16 条 `TR[i] tag=NAME val=...`**:RING dump 之后打印,与内核 ALLOC_RING 对照。

## 隔离

trace.rs 只写 in-crate atomics,注释已中性化(不含 ls2k 字面量,避免 debuginfo 污染 ELF)。qemu_virt kernel-la 共享同一 alloc crate,但无人安装 reader → 每分配多几次 relaxed atomic store,零行为变化。Kernel 侧 extern/getter/打印全部 `#[cfg(feature = "platform-ls2k1000")]`。

## 判读表(下板看 TRALL 末条 + 176B 的 val)

| 末条 tag | 结论 | 下一步 |
|----------|------|--------|
| ALLOC_IMPL_AFTER val=0 | **shim 制造 null**:内核 `allocate()` 对合法 layout 返回 0 | 查内核堆 Option/锁/buddy/slab 为何 null |
| ALLOC_IMPL_AFTER val=垃圾非零 | **shim 返回坏指针** | 与 0x808 BSS 位移一致,查返回路径地址损坏 |
| RAW_ALLOC_ERR | RawVec 层拿到 AllocError 未走 shim | 查 `alloc_guard` / layout 计算 |
| CAP_OVERFLOW | 容量算术溢出 | 与 align 损坏同源?查 LoongArch 溢出 |
| REALLOC_NULL | realloc 返回 null(ring 不记录 realloc) | 查内核 realloc 覆写路径 |
| HANDLE_OOM | 失败分配确认到达 abort 入口 | 链完整,聚焦 shim 之上 |

## 上板指令(同前)

```
sf probe
fatload usb 0:1 0x9000000002000000 kernel-ls2k1000.uImage
fatload usb 0:1 0x900000000a000000 ls2k1000-minimal.dtb
iminfo  0x9000000002000000
bootm   0x9000000002000000 - 0x900000000a000000
```

**判读顺序**:`PROBE176-A/B` → `HEAPD00-04` → `TINIT00/01` → `TASK00..`;若 OOM,`TRALL0` 一行给出 last-trace,`TRALL` 段尾部 16 条给出失败瞬间的 crate 层事件流,与 `RING total` 对照即得完整结论。

## 提交历史

| 提交 | 说明 |
|------|------|
| *(本次)* | PR-0 reproducible sysroot/buildinfo + PR-1 checkpoints + PR-3 probes + PR-2 alloc trace ring + ring UB fix |

*Co-Authored-By: Claude <noreply@anthropic.com>*

---

# SudoOS 2026-08-09 更新(四)— 根因定性:LoongArch 异常向量页对齐错误(伪 OOM)

## 定性:176B "OOM" 是伪 OOM

不是 heap / slab / 真实 176B 分配失败。根因是 **LoongArch EENTRY 安装地址低 12 位被硬件清零**,而共享 `__loongarch_trap_entry` 未页对齐——第一个 timer IRQ 直接跳进它所在页的**页首**,而页首恰好是 `__rust_alloc_error_handler`(allocator shim),于是把现场寄存器误当成 OOM 参数打印:

```text
size = 176 = 0xb0   ← 正是 CRMD(CSR 0x0)当前映射模式值
align = 0x9000...5b3ad8  ← lockdep 静态地址(MAX_IRQ_OFF_CYCLES 一族)
```

## ELF 实证(run-12 上板产物)

```
9000000090200000 T __text_start
9000000090201000 T __rust_alloc_error_handler    ← 页首 = shim
9000000090201ba0 T __loongarch_trap_entry        ← & 0xfff = 0xba0,未页对齐!
```

`EENTRY[11:0]` 硬件恒为 0,所以 EENTRY=0x...1000(页首)= `__rust_alloc_error_handler` 首指令。这完整解释:为什么没有任何 alloc trace、为什么直接进 OOM handler、为什么参数是 CRMD/lockdep、为什么总在 `time::start_periodic()` 后 ~10ms、为什么之前改 slab/RawVec/Layout 都无效。

## 修复(PR-4)

`platform/ls2k1000` 独立页对齐 trampoline(不改共享 `.text` 布局,保持 kernel-la 隔离):

| 文件 | 改动 |
|------|------|
| `platform/ls2k1000/entry.S` | `.text.trap_entry` 段 + `.balign 4096` + `__loongarch_trap_entry_ls2k: b __loongarch_trap_entry`(裸分支零 GPR 扰动,与直接进入共享 trap body 等价) |
| `platform/ls2k1000/linker.ld` | `.text` 内为 `.text.trap_entry` 独占一个 4 KiB 页;`ASSERT(__loongarch_trap_entry_ls2k == __trap_vector_start)` + `ASSERT(段 ≤ 4K)` 构建期保护 |
| `trap/mod.rs` | EENTRY 源按 `#[cfg(feature = "platform-ls2k1000")]` 选择 trampoline,其余平台编译产物逐字节不变;新增 cfg-gated `ls2k_eentry_expected/installed/ecfg` 读取 |
| `kernel/main.rs` | 修复后 `trap::initialize()` 后打印 `TRAP-VECTOR expected=... installed=... vs=... PASS/FAIL` + `BREAKPOINT-TRAP PASS`(错位立即崩溃,早于第一个 IRQ 暴露) |
| `kernel/trap.rs` | 第一个 `ECODE_INTERRUPT` 打印 `TIMER-IRQ-FIRST pending=0x... era=0x...` + `TIMER-IRQ-FIRST DONE`(证明 timer IRQ 真正进入处理器) |

## 修复后布局验证(本次构建)

```
9000000090200000 T __text_start
9000000090201000 T __loongarch_trap_entry_ls2k   ← 页对齐(低 12 位=0)
9000000090201004 T __trap_vector_end             ← 4 字节分支
9000000090202000 T __rust_alloc_error_handler    ← shim 移到第 2 页
9000000090202e40 T __loongarch_trap_entry        ← 共享 trap body
EENTRY install(两处): pcalau12i -116/-218 → 0x...201000 → csrwr $r12,0xc  ✓
```

## 隔离(强制约束)

- qemu_virt kernel-la:`.text/.rodata/.data/.bss` **逐字节不变**(与 stash 掉 trap/mod.rs 的对照构建逐段 diff 为空;整体 ELF hash 仅 `.debug_*` 行号表不同)。EENTRY install 仍是原 `la.pcrel __loongarch_trap_entry` 三指令。
- `ls2k_verify_la.sh` ZERO 标记复核:OOM-HANDLER/HEAP-FATAL/ls2k/LS2K/TASK00/... 全 0(`memory allocation of`=1 为共享 vendored alloc 既有运行时字符串,两镜像一致,非泄漏)。
- 确定性:同源重建两次 hash 一致(150433f3)。

## 上板指令(run-14,同前)

```
sf probe
fatload usb 0:1 0x9000000002000000 kernel-ls2k1000.uImage
fatload usb 0:1 0x900000000a000000 ls2k1000-minimal.dtb
iminfo  0x9000000002000000
bootm   0x9000000002000000 - 0x900000000a000000
```

**期望输出(判读顺序)**:

```text
TRAP-VECTOR expected=0x...1000 installed=0x...1000 vs=0 PASS   ← 修复确认
BREAKPOINT-TRAP PASS
TIMER-IRQ-FIRST pending=0x800 era=...
TIMER-IRQ-FIRST DONE                                           ← 第一个 timer IRQ 进入真处理器
PROBE176-A PASS
HEAPD00..04 / MAIN40/41
PROBE176-B PASS
TINIT00/01
TASK00 enter ...
```

若 `TRAP-VECTOR ... FAIL` 或 `BREAKPOINT-TRAP` 后崩溃 → 向量仍未对齐(回查链接 ASSERT 与安装目标)。若正常链一直走到 `TASK00` 之后再次中断 → 那才是真正的分配问题,再回到 PR-2 TRALL 判读表。

## 待办(修复确认后)

- 删除 OOM 的非法 align 钳制、栈快照、`IrqSpinLock` 原始 words 读取等误导性诊断;
- heap 状态检查改成类型安全统计接口;
- 保留 buildinfo、ELF/uImage hash 与 EENTRY 链接检查。

## 提交历史

| 提交 | 说明 |
|------|------|
| *(本次)* | PR-4:trap vector 页对齐(伪 OOM 根因)+ TRAP-VECTOR/BREAKPOINT-TRAP/TIMER-IRQ-FIRST 标记 |

---

# SudoOS 2026-08-09 更新(五)— 固化 LS2K1000 稳定基线(阶段 1/2/3)

分支 `ls2k-stabilize`,基于 PR-4(异常向量页对齐)根因修复后的收尾固化。

## 阶段一:清理诊断代码,建立稳定基线

| 提交 | 内容 |
|------|------|
| `c714ef59` | **修复 raw::puthex 移位错误**:`(0..64).rev().step_by(4)` 生成 63,59,...,3,step_by 取索引 0,4,... 恒跳过 shift 0 → 最低 4 bit 丢失(`0x1234` 打成 `0x123`)。改为 nibble*4 移位。ls2k 专属代码 |
| `09659ad7` | **恢复标准分配器**:删除非法 align 钳制、ls2k realloc 覆写、OOM 栈快照/代码字扫描、PR-2 vendored alloc 追踪环、heap 原始 words 读取、PROBE176/HEAPDxx/TASKxx/TINITxx/VMxx 检查点。保留最小化 raw UART OOM(size/align+停机)、HEAP-STATE 类型安全统计接口、HEAP_FATAL 哨兵。vendored alloc hook 无条件,故 kernel-la 同样缩小 |
| `438e478e` | **固化异常向量修复**:TRAP-VECTOR 检查失败 → 关中断停机(不再继续误导启动);breakpoint 自测移入 opt-in `boot-selftest` 特性(build.sh 新增 `EXTRA_FEATURES` hook,默认空)。.text.trap_entry 独立段/4KiB 对齐/链接 ASSERT/EENTRY 回读/ECFG.VS==0 永久保留 |

## 阶段二:定时器与调度器生命周期

| 提交 | 内容 |
|------|------|
| `95adcc16` | **重排启动顺序**:BSP trap → 构造/发布/注册 Scheduler → `time::start_periodic`(启动定时器+开中断)→ 标记 BSP active → 孵化 reaper → 启动 CPU1 → 等 online → 用户态。`task::initialize()` 只做构造/发布/注册,新增 `task::start_boot_scheduler()` 在 start_periodic 之后标记 active 并 spawn reaper(因 Scheduler::spawn 断言目标 CPU 已 active)。**调度定时器绝不在 Scheduler 发布前启动**。同时把一次性全局 AtomicBool 标记换成 per-CPU 计数器:`TIMER_IRQ_COUNT`(trap.rs)、`IPI_SEND_COUNT`(ipi.rs)、IPI 接收复用 ipi.rs 的 per-CPU `interrupt_count`,并新增启动期 `CPU-COUNTERS` 检查(等每个在线 CPU 收到 timer IRQ,打印 timer/ipi 计数并断言) |

## 阶段三:稳定性测试协议

| 提交 | 内容 |
|------|------|
| `1e7f57c8` | `docs/ls2k-stability-test.md`:9 项测试矩阵(冷启动×20/60min 双核/SMP/IPI/进程/VM/IPC 压力/内存回收/错误检查)+ run-15 bootm 序列 + 启动判读表 + `ls2k-core-v0.1` 打标签命令。`scripts/ls2k_package.sh` 生成 elf/bin/uImage/buildinfo |

## 隔离与产物验证

- kernel-la(qemu_virt)二进制**零 ls2k 标记**(`scripts/ls2k_la_check.py` 全 0);`sudoos_alloc_trace` 已随 vendored hook 删除。
- `boot-selftest` 门控验证:开启版含 `BREAKPOINT-TRAP`,默认版不含;两版均含永久 `TRAP-VECTOR`。
- 确定性:同源重建 hash 一致;buildinfo `git_dirty_files=0`。
- QEMU 冒烟说明:本环境无 loongarch UEFI 固件,kernel-la 在 QEMU `-kernel` 直启下收不到 EFI system table/FDT(panic 于 main.rs:217 未改代码处)——预存在环境限制,非本轮改动回归。启动顺序正确性以断言链 + 真机验证为准。

## run-15 上板(产物 hash 见 `kernel-ls2k1000.buildinfo`)

```text
sf probe
fatload usb 0:1 0x9000000002000000 kernel-ls2k1000.uImage
fatload usb 0:1 0x900000000a000000 ls2k1000-minimal.dtb
iminfo  0x9000000002000000
bootm   0x9000000002000000 - 0x900000000a000000
```

**期望输出**(阶段 1/2 后的新判读表):

```text
TRAP-VECTOR expected=0x...1000 installed=0x...1000 vs=0 PASS   ← 永久检查(失败即停机)
HEAP-STATE[pre-task-init] initialized=true stats=...
CPU-CNTR cpu=0 timer=N ipi_recv=M ipi_send=K                    ← per-CPU 计数
CPU-CNTR cpu=1 timer=N ipi_recv=M ipi_send=K
CPU-COUNTERS PASS
BOOT11 all-ap-online
BOOT14 user-entry
SMOKE_TEST: PASS
```

## run-15 真机结果(2026-08-09):tickless idle 与副核 timer 的矛盾

bootm 首次完整跑通到 CPU-COUNTERS 检查(TRAP-VECTOR PASS、HEAP-STATE 干净、
CPU0 timer=62),但 **CPU1 timer=0** → `kernel/src/main.rs:487` panic
`timer IRQ did not reach every online CPU`。

**根因**(非隐藏硬件 bug,是检查与 NO_HZ 设计矛盾):

1. 副核 `kernel_secondary_entry` 确实调用 `arm_periodic_secondary()` 装好定时器;
2. 但紧接着 `idle_thread_bootstrap` → `idle_until_interrupt`,**副核是非 BOOT CPU,
   走 `time::enter_idle()`**:清 `SCHEDULER_TICK_ACTIVE`,本地无软件定时器时
   `reprogram_local(None)` 走 `shutdown()` → **TCFG=0 停掉硬件定时器**;
3. CPU0 因 `cpu != BOOT` 跳过(共享 idle 路径注释明确:secondary 保留完整 NO_HZ),
   保持 tick → 62 次;CPU1 从此只被 IPI 唤醒,`ls2k_timer_irq_count(1)` 恒 0。

这是共享 idle 路径的**有意设计**,阶段 2 的检查假定 idle 副核也收 timer IRQ,
与真实模型矛盾,首次真机跑通才暴露。

**修复**(`main.rs` CPU-COUNTERS 块,仍全在 `#[cfg(feature="platform-ls2k1000")]` 内,
qemu_virt/riscv64 零改动):boot 核保留 timer IRQ 验证;副核改用**真实
reschedule IPI 往返**验证(`interrupt_count(cpu) > 0`)——唤醒 idle → trap entry
→ ECODE_INTERRUPT → IPI 分发 → acknowledge,与 timer IRQ 走完全同一条 trap 路径。

**run-16 新判读表**(`cpu=1 timer=0` 是预期,`ipi_recv>0` 证明副核中断路径):

```text
TRAP-VECTOR ... PASS
CPU-CNTR cpu=0 timer=N ipi_recv=M ipi_send>=1
CPU-CNTR cpu=1 timer=0 ipi_recv>=1 ipi_send=0     ← idle 副核:无 timer=预期,有 IPI 接收
CPU-COUNTERS PASS
BOOT11 all-ap-online
BOOT14 user-entry
SMOKE_TEST: PASS
```

通过 20 次冷启动 + 60 分钟双核运行后打标签:

```bash
git tag -a ls2k-core-v0.1 -m "LS2K1000 core platform stable"
```

## 待办(后续阶段)

- 阶段四:initramfs/BusyBox 启动真实用户环境(验证真实 ELF/libc/系统调用/用户态启动)。
- 阶段五:完整 DTB 解析、PCI host、真实块设备、ext4、磁盘 `/sbin/init`、oscomp runner。
- 外设顺序:RTC → reboot/shutdown → GMAC 网络 → USB → 其他板级设备。

## 提交历史(本轮)

| 提交 | 说明 |
|------|------|
| `c714ef59` | fix: ls2k raw puthex dropped the low nibble |
| `09659ad7` | refactor: strip ls2k fake-OOM diagnostics; restore stock allocator |
| `438e478e` | harden: fail-stop on trap-vector check; gate breakpoint self-test |
| `95adcc16` | sched: start periodic timer only after Scheduler is published |
| `1e7f57c8` | docs: LS2K1000 stability test protocol for ls2k-core-v0.1 |

*Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>*
