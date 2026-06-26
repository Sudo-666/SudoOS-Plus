# OSComp Group Matrix on fixla

## Current Stable Baseline (500+)

| Arch | Libc    | basic | busybox | lua  | libcbench |
|------|---------|-------|---------|------|-----------|
| RV   | glibc   | ✅    | ✅      | ✅   | ✅        |
| RV   | musl    | ✅    | ✅      | ✅   | ✅        |
| LA   | glibc   | ✅    | ✅      | —    | ✅        |
| LA   | musl    | ✅    | ✅      | ✅   | ✅        |

**Key**: ✅ = score-enabled, — = not yet enabled

## Red Lines

- Do **not** break RV glibc busybox direct override (`oscomp-rv-busybox-direct`)
- Do **not** break LA full basic direct runner (`oscomp_la_run_basic_direct`)
- Do **not** remove LoongArch FPD `enable_fpu` retry (`code == 15`)
- Do **not** remove `score=` / `score:` output
- Do **not** introduce global `kill_process_group`
- Do **not** introduce `run_group_with_deadline`

## Group Status Table

### basic

| Arch/Libc   | Mode            | Runner         | Shell                     | Risk  |
|-------------|-----------------|----------------|---------------------------|-------|
| RV glibc    | score-enabled   | script         | /bin/sh                   | low   |
| RV musl     | score-enabled   | script         | /bin/sh                   | low   |
| LA glibc    | score-enabled   | **direct**     | /mnt/sdcard/glibc/busybox | low   |
| LA musl     | score-enabled   | **direct**     | /mnt/sdcard/glibc/busybox | low   |

**Runner policy**: LA basic runs each binary directly via `oscomp_la_run_basic_direct`;
RV basic runs the shell script.

### busybox

| Arch/Libc   | Mode            | Shell override                                 | Risk  |
|-------------|-----------------|------------------------------------------------|-------|
| RV glibc    | score-enabled   | `/mnt/sdcard/glibc/busybox` (direct override)   | low   |
| RV musl     | score-enabled   | `/bin/sh`                                       | low   |
| LA glibc    | score-enabled   | `/mnt/sdcard/musl/busybox` (via shell probe)     | low   |
| LA musl     | score-enabled   | `/mnt/sdcard/glibc/busybox` (override: glibc shell) | low |

**Shell policy**: LA musl busybox uses glibc busybox to avoid known-bad-busybox SIGSEGV.

### lua

| Arch/Libc   | Mode            | Risk   | Notes                        |
|-------------|-----------------|--------|------------------------------|
| RV glibc    | score-enabled   | low    | whitelist present            |
| RV musl     | score-enabled   | low    | whitelist present            |
| LA glibc    | future candidate | medium | needs whitelist              |
| LA musl     | score-enabled   | medium | whitelist present            |

### libcbench

| Arch/Libc   | Mode            | Risk   | Notes                        |
|-------------|-----------------|--------|------------------------------|
| RV glibc    | score-enabled   | medium | whitelist present            |
| RV musl     | score-enabled   | medium | whitelist present            |
| LA glibc    | score-enabled   | medium | whitelist present            |
| LA musl     | score-enabled   | medium | whitelist present            |

**Required foundations**: clock_nanosleep, alarm/timer semantics

### lmbench

| Arch/Libc   | Mode            | Risk   | Notes                        |
|-------------|-----------------|--------|------------------------------|
| All         | skip (heavy)    | high   | probe-only first              |

**Future**: lightweight probe before full enable.

### cyclictest

| Arch/Libc   | Mode            | Risk   | Notes                        |
|-------------|-----------------|--------|------------------------------|
| All         | skip (heavy)    | high   | probe-only first              |

**Future**: alarm/clock_nanosleep correctness before enabling.

### iozone

| Arch/Libc   | Mode            | Risk   | Notes                        |
|-------------|-----------------|--------|------------------------------|
| All         | skip (heavy)    | high   | fs-mini probe first           |

**Future**: basic VFS write/read/lseek before filesystem benchmarks.

### iperf / netperf

| Arch/Libc   | Mode            | Risk   | Notes                        |
|-------------|-----------------|--------|------------------------------|
| All         | skip (heavy)    | extreme | net-mini probe first          |

**Future**: TCP/UDP socket baseline before network benchmarks.

### libctest

| Arch/Libc   | Mode            | Risk   | Notes                        |
|-------------|-----------------|--------|------------------------------|
| All         | disabled        | extreme | pthread_cond_smasher → scheduler recursive-lock panic |

**Future**: allowlist only (identify safe tests, run one-by-one).
Do **not** bulk-enable.

### ltp

| Arch/Libc   | Mode            | Risk   | Notes                        |
|-------------|-----------------|--------|------------------------------|
| All         | skip (heavy)    | extreme | metadata scan + allowlist     |

**Future**: scan available testcases, select safe subset (filesystem/process basics), enable one at a time.

## Unlock Priority (recommended order)

1. **Stable baseline guard**: protect existing 500+ (this round)
2. **libcbench stabilization**: fix alarm/timer/signal for both archs
3. **LA lua (glibc)**: add to LA whitelist
4. **lmbench probe**: run single lightweight benchmark, check output
5. **cyclictest probe**: run with fixed iteration count
6. **iozone probe**: run with minimal file size
7. **iperf / netperf probe**: run with loopback, fixed port
8. **libctest allowlist**: identify safe individual tests
9. **ltp metadata scan**: catalog available testcases

## Notes

- The LA `oscomp_la_run_sleep_trace_probe` and `oscomp-la-basic-probe` functions
  are diagnostic-only; they do not affect scoring but may produce noise.
- `oscomp_should_skip_heavy` still excludes lmbench/netperf/iperf/iozone/
  cyclictest/ltp on RISC-V.
- On LoongArch these groups are excluded via `oscomp_la_whitelist` (only
  basic + busybox + libcbench + lua-musl are currently in the LA whitelist).

## P10-F1 Scaffold

*Added in `6.27` — read-only, does not change runner behavior.*

### New types in `kernel/src/user.rs`

| Type | Kind | Purpose |
|------|------|---------|
| `OscompLibc` | enum | Glibc / Musl / Unknown |
| `OscompGroup` | enum | Basic / Busybox / Lua / Libcbench / Lmbench / Cyclictest / Iozone / Iperf / Netperf / Libctest / Ltp / Unixbench / Unknown |
| `OscompShellPolicy` | enum | Default / RvGlibcBusyboxDirect / LaGlibcBusyboxForMusl / LaDirectBasic / ProbeOnly |
| `OscompEnvPolicy` | enum | Default / Glibc / Musl / MixedMuslWithGlibcShell / Network / FilesystemStress |
| `OscompRunPolicy` | enum | Script / DirectBasic / ProbeOnly / Skip |
| `OscompRisk` | enum | Low / Medium / High / Extreme |
| `OscompGroupSpec<'a>` | struct | Combines path + libc + group + policies + risk |

### New functions

| Function | Status | Purpose |
|----------|--------|---------|
| `oscomp_classify_script(path)` | ready | Pure path→spec classifier |
| `oscomp_log_group_spec_once(path)` | ready | Budgeted (16) debug log |
| `oscomp_group_preflight(spec)` | stub → always true | TODO P10-F2 |

### Roadmap

- **P10-F1** (this round): scaffold only, no behavior change
- **P10-F2**: `oscomp_group_preflight` — real checks (file presence, shell health, minimal probe)
- **P10-F3**: mini probe runner using `OscompRunPolicy::ProbeOnly` / `DirectBasic`
- **P10-F4+**: migrate score-enabled groups to spec-driven path

All current ScoreEnabled behavior is still controlled by the existing runner
paths (RV direct override, LA direct basic, whitelist/defer).

## P10-F2 Preflight

*Added in `6.27` — read-only, does not change runner behavior.*

### New types

| Type | Purpose |
|------|---------|
| `OscompPreflightStatus` | Ready / NotReady / Skipped |
| `OscompPreflightResult` | status + script/cwd/shell/loader/env booleans |

### New functions

| Function | Purpose |
|----------|---------|
| `oscomp_vfs_path_exists(path)` | Read-only VFS stat check |
| `oscomp_expected_cwd(spec)` | Returns expected CWD per group/libc |
| `oscomp_expected_shell(spec)` | Returns expected shell (or None for direct) |
| `oscomp_loader_ready(spec)` | Checks dynamic-linker alias existence |
| `oscomp_env_ready(spec)` | Validates env policy is recognised |
| `oscomp_group_preflight(spec)` | **Real** — combines all 5 checks → status |
| `oscomp_log_preflight_once(spec, result)` | Budgeted (24) debug log |

### What preflight checks

| Check | How |
|-------|-----|
| script_exists | `crate::fs::stat(path).is_ok()` |
| cwd_exists | `oscomp_expected_cwd` → `stat` |
| shell_exists | `oscomp_expected_shell` → `stat` (or true if None) |
| loader_ready | `stat` on arch-specific loader alias |
| env_ready | env policy enum matched (no network/fs probe) |

### Status

| Condition | Status |
|-----------|--------|
| `run_policy == Skip` | Skipped |
| `group == Unknown \|\| libc == Unknown` | NotReady |
| script + cwd + shell + env are all ok | Ready |
| Any missing | NotReady |

### Important

- Preflight is **read-only**: no VFS writes, no sdcard expansion, no process creation.
- Preflight does **not** unskip any group. All heavy groups remain skipped.
- `Ready` ≠ test will pass — it only means the entry conditions (file/cwd/shell) exist.
- P10-F3 will connect preflight to mini-probe execution.

## P10-F3 Mini Probe Catalog

*Added in `6.27` — scaffold only, no probes executed.*

### New types

| Type | Purpose |
|------|---------|
| `OscompProbeKind` | ShellTrue / ShellEcho / DirectBinary / ScriptSmoke / FsMini / NetTcpMini / NetUdpMini / LtpScan |
| `OscompMiniProbe` | name + kind + path + argv0 + cwd + risk |
| `OscompProbeRunStatus` | NotRun / Pass / Fail / Missing / Timeout |

### New functions

| Function | Status | Purpose |
|----------|--------|---------|
| `oscomp_mini_probes_for(spec)` | ready | Returns probe list per group/libc |
| `oscomp_log_probe_catalog_once(spec)` | ready | Budgeted (16) summary log |
| `oscomp_run_mini_probe(probe)` | stub → NotRun | TODO P10-F4 |
| `oscomp_env_for_policy(policy)` | ready | Env strings per policy |

### Probe catalog by group

| Group | Probes | Risk |
|-------|--------|------|
| Lua | shell-true, shell-echo, lua-smoke | Medium |
| Libcbench | shell-true, shell-echo, libcbench-smoke | Medium |
| Lmbench | lat_syscall_null, lat_syscall_read, lat_pipe, lat_proc_fork | High |
| Cyclictest | clock_gettime, nanosleep, clock_nanosleep, sched_yield | High |
| Iozone | fs_create_4k, fs_write_4k, fs_readback_4k, fs_ftruncate, fs_fsync, fs_statfs, fs_unlink | High |
| Iperf/Netperf | tcp_socket, tcp_bind_listen, tcp_connect_accept, tcp_send_recv, udp_sendto_recvfrom, poll_select | Extreme |
| Libctest | nonpthread_smoke, malloc_stdio_smoke, signal_basic_smoke, futex_basic (**NO** pthread_cond_smasher) | Extreme |
| LTP | metadata_scan, syscall_basic_allowlist, fs_small_allowlist, time_small_allowlist | Extreme |

### Important

- **No probes are executed** in P10-F3. `oscomp_run_mini_probe` always returns `NotRun`.
- **No skip is removed**. All heavy groups remain skipped.
- **pthread_cond_smasher is banned** from the catalog.
- P10-F4 will implement real mini-probe execution.

## P10-F4 Mini Probe Runner

*Real execution for low-risk probe types. Not called from contest runner.*

### Implementation status

| ProbeKind | Status | Method |
|-----------|--------|--------|
| ShellTrue | **executed** | `run_rootfs_program_with_cwd(shell, ["busybox","true"], ...)` |
| ShellEcho | **executed** | `run_rootfs_program_with_cwd(shell, ["busybox","sh","-c","echo probe_ok"], ...)` |
| ScriptSmoke | **executed** | `run_rootfs_program_with_cwd(shell, ["busybox","sh",script], ...)` |
| DirectBinary | **executed** | `run_rootfs_program_with_cwd(path, [argv0], ...)` |
| FsMini | NotRun | P10-F5+ |
| NetTcpMini | NotRun | P10-F5+ |
| NetUdpMini | NotRun | P10-F5+ |
| LtpScan | NotRun | P10-F5+ |

### New helpers

| Function | Purpose |
|----------|---------|
| `oscomp_probe_path_exists(path)` | Stat check before executing probe |
| `oscomp_probe_shell_for(probe)` | Choose shell per cwd/arch |
| `oscomp_run_probe_catalog_for_spec(spec)` | Run all probes for a group → pass count |

### Safety

- **Not called from contest runner**
- Budget limits log output to 64 lines
- Missing paths return `Missing`, not Fail
- No fake `testcase success` output
