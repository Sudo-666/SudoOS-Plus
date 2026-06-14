# CI and the M6 local release gate

## What CI is

Continuous Integration (CI) means a clean machine automatically checks a
commit whenever code is pushed or a pull request is opened. Typical checks are
formatting, compilation, unit tests, static audits and selected QEMU smoke
runs.

CI does not make kernel code correct by itself. Its value is preventing a
known check from being forgotten and proving that the project does not only
work because of untracked files or state left on one developer's computer.

## Why CI previously failed

OS CI often fails for environmental reasons rather than kernel regressions:
missing cross targets, unavailable QEMU variants, firmware differences,
runner time limits, no cache, or a workflow trying to run a long SMP matrix on
every push.

The correct response is not to let a permanently red workflow become noise.
Use a small reliable workflow first, and keep expensive QEMU soak testing in a
manual local release gate.

## Current M6 policy

GitHub Actions is optional. M6-C installs no active workflow by default.

For a single developer, these local commands provide the important protection:

```bash
make m6-quick
make m6-full
make m6-release
```

Use CI later when:

- more than one person pushes to `main`;
- development happens on multiple machines;
- pull requests need automatic review evidence;
- a clean Linux runner is useful to catch hidden local dependencies.

## Conservative optional workflow

The M6-C package contains `optional/github-actions/m6.yml`. It only runs source
hygiene, formatting, host tests and the M6 static audit. It intentionally does
not run the full dual-architecture QEMU matrix.

To enable it later:

```bash
mkdir -p .github/workflows
cp /path/to/package/optional/github-actions/m6.yml .github/workflows/m6.yml
git add .github/workflows/m6.yml
```

After the workflow is stable, add one small QEMU smoke job. Do not begin by
running the 48-case full matrix on every push.
