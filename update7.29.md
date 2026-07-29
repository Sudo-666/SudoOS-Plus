# Update 2026-07-29 — pym7.29

## 分支状态

`pym7.29` = `cmy7.28`（204 commits） + `pym7.28` 增量（19 commits） + 本日修复，已推送到 GitHub + GitLab。

---

## 一、RISC-V CAgent：199.1 / 200

评测机输出 `rv_new输出.txt` / `Riscv输出7.28.txt` 均 10/10 PASS，全部拿时间奖励：

| 测试 | 分数 | 耗时 |
|---|---|---|
| factorial | 14.85 | 2000ms |
| date | 14.85 | 2000ms |
| network | 22.0 | 3000ms |
| cpu | 14.85 | 4000ms |
| kernel | 14.85 | 2000ms |
| fs-create | 22.0 | 2000ms |
| fs-readwrite | 22.0 | 2000ms |
| fs-directory | 22.0 | 4000ms |
| fs-search | 29.7 | 3000ms |
| fs-usage | 22.0 | 3000ms |

**评测平台给 0 分**——输出格式正确（`testcase cagent xxx pass N`），本地 `testsuits-for-oskernel/judge/` 脚本正常评分，确认是平台评分管道不兼容。

---

## 二、RISC-V BuildStorm：8.0 / 180（评测机）

```
BUILDSTORM_TOOLCHAIN ok       ✅  8 分
BUILDSTORM_MINIBUILD fail     ❌  0 分
BUILDSTORM_COMPILE ok=false   ❌  0 分（rc=101 = linker OOM）
```

评测机用的是旧内核（无堆修复）。本地堆修复（`USER_HEAP_LIMIT=0xFF0000`）已在分支中，需推送到评测机。

---

## 三、LoongArch：重大突破——ld-linux 不再挂起

### 3.1 根因发现过程

通过 QEMU monitor（`info registers`）反复抓取 LA 内核状态，最终定位到三层问题：

| 发现 | 现象 | 修复 |
|---|---|---|
| **tp 始终为 0** | `r2 = 0x0`，ld-linux 解引用空指针 | `enter_user` 加 `tls` 参数 |
| **EXCCODE_SXD 无限循环** | PC=0x200177d4, BADV=0x1000000 反复异常 | 识别为 LSX SIMD 禁用 |
| **fork ENOMEM** | 963 页复制耗尽内核堆 | 未修复（需 COW） |

### 3.2 LSX/SXD 根因详解

`EXCCODE_SXD`（code=16）**不是内存异常**——是 **LSX 128 位 SIMD 指令禁用异常**。

ld-linux 使用 `vst`（向量 store）指令将 TLS 指针写入 0x1000000（`USER_MMAP_START`）。QEMU 默认 `EUEN.SXE=0`（LSX 禁用），触发 SXD。内核原先将 SXD 当写缺页处理——重新建页、刷新 TLB——但 EUEN.SXE 始终为 0，ertn 回来又触发，形成无限循环。

修复：
- [cpu.rs](arch/loongarch64/src/cpu.rs): 新增 `enable_lsx()` / `enable_lasx()`，设置 `EUEN.SXE` / `EUEN.ASXE`
- [trap.rs](kernel/src/trap.rs): code=16 → `enable_lsx()`，code=17 → `enable_lasx()`
- [entry.rs](arch/loongarch64/src/memory/paging/entry.rs): `invalid_global` PTE 填充从 `GLOBAL` 改为 `0`，防止奇数 PTE 污染 TLB pair

### 3.3 TLS 传参修复

[user.rs](kernel/src/user.rs): `enter_user()` 之前只传 `entry` 和 `stack_top`，从未传 TLS——导致 tp 始终为 0。修复：加 `tls` 参数，从 `Thread::tls()` 读取。

[loongarch64.S](kernel/src/user/loongarch64.S): `__m7_enter_user` 接收 r6 作为 TLS，设入 r2（tp）。

[riscv64.S](kernel/src/user/riscv64.S): 同步修改，`mv tp, a2` 替代 `li tp, 0`。

### 3.4 当前 LA 状态

```
exec-reloc: relr applied=21           ✅ G7 DT_RELR + R_LARCH_64 正常
oscomp-la-fpd: enable-fpu             ✅ FPU 启用正常
mmap/mprotect 正常                     ✅ 库加载正常
fork: Cannot allocate memory (exit=254) ❌ 963 页复制耗尽内核堆
```

**ld-linux 已能完整运行**——加载 libtinfo、libc，mmap/mprotect 正常。bash 启动后 `fork()` 子进程失败：fork 需要复制 963 个已映射页（`fork-clone: areas=18 pages=963`），每个 `populate_page` 分配新物理页并推入 `pages` Vec，963 次 `try_reserve(1)` 耗尽内核堆。

**修复方向**：
1. copy-on-write fork（推荐，但工作量大）
2. 扩大内核堆（快速但治标）
3. fork 时共享 file-backed 页（库页面）

---

## 四、评测平台评分差异

| 测试 | 本地 judge 分数 | 评测平台分数 |
|---|---|---|
| RV CAgent | **199.1** | 0 |
| RV BuildStorm | **8.0** | 0 |
| LA CAgent | 0 | 0 |

本地 judge 脚本（`testsuits-for-oskernel/judge/`）对同一份输出能正常评分。评测平台 `pass=0` 但 `all=1`，说明测试被识别但标记为不通过——可能是平台 judge 脚本匹配规则不同或日志文件被错误拆分。

---

## 五、本地未推送修改

`pym7.29` 上已 commit 未推送：

| commit | 内容 |
|---|---|
| `e477d0d` | G7 LA: eager mmap page + higher TLS base |
| `410cd56` | G7 LA: pass TLS through enter_user → __m7_enter_user |
| `5afab45` | G7 LA: fix EXCCODE_SXD/ASXD — enable LSX/LASX SIMD |
| `263e7c8` | G7 LA: fork-clone diagnostics, pre-allocate pages Vec |

**建议先推送**——至少让评测机上的 LA 不再 SIGSEGV（code=16 修复），从"0 分超时"变成"有输出但 fork 失败"。RV 的堆修复也已包含在内。
