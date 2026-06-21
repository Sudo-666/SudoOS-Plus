# M5 Completion and Release Gate

M5 freezes the CPU lifecycle, context/locking rules, WaitQueue/Completion
ownership, task reaping, idle wake protocol, IPI mailbox, CallFunction requests,
and explicit kernel TLB requests.

Every smoke run writes `serial.log` and `result.json`. The JSON classifies
`kernel-failure`, `missing-evidence`, `qemu-exit`, `timeout-no-output`,
`timeout-slow-progress`, `timeout-stalled`, `build-error`, and `harness-error`.
A QEMU `signal 15` line alone is not a failure; the structured result records
whether the harness intentionally terminated QEMU.

Stress runs build once per `(arch, profile)`, shuffle case order from the seed,
save immutable per-case attempts, and generate `summary.json`/`summary.md`.
A timeout retry is recorded as `flaky-timeout-recovered`; release gates reject
flaky cases.

Commands:

```bash
make harness-test
make m5-quick
make m5-full

git add -A
git commit -m "m5: close concurrency foundation"
make m5-release M5_SOAK_LOOPS=200 M5_RELEASE_SOAK_LOOPS=20
make m5-tag
git push origin main m5-complete
```

The tag helper requires a passing soak report from the current clean HEAD.

The soak release gate first runs the full debug/release matrix, then 200 debug SMP=4 loops and 20 release SMP=4 loops per architecture by default.

## Stable versus optional smoke evidence

Required milestones are stable release contracts. Human-readable detail and fallback lines are optional evidence. Missing optional evidence is recorded in `result.json` as `optional_missing_evidence` and cannot override a valid `SMOKE_TEST: PASS`.

## Phase progress semantics

`current_phase` means the latest phase for which any stable marker was
actually observed. It no longer means the earliest phase whose entire marker
set is incomplete.

`first_incomplete_phase` separately reports the earliest phase that still lacks
required evidence.

Therefore, a stale or renamed boot detail cannot make a run that reached
`SMP_TEST: PASS` appear stuck in `boot`.

## Serial evidence whitespace

Kernel status lines use padded columns, for example:

```text
online CPUs     : 4
runtime pgtbl   : active hardware root
```

The smoke harness normalizes consecutive horizontal whitespace independently
on each serial line before matching evidence. Line boundaries remain intact, so
two different lines cannot combine into a false match.

The obsolete `participating CPUs` marker is no longer required. CPU lifecycle
evidence uses the current `discovered CPUs`, `online CPUs`, and final SMP
milestones.
