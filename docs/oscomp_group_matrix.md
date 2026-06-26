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
