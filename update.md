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

*Co-Authored-By: Claude <noreply@anthropic.com>*
