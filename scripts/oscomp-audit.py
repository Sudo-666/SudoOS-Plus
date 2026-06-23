#!/usr/bin/env python3
from __future__ import annotations
import re, subprocess, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PASS=[]; WARN=[]; FAIL=[]

def read(rel):
    try:
        return (ROOT / rel).read_text(encoding='utf-8', errors='ignore')
    except Exception:
        return ''

def exists(rel):
    return (ROOT / rel).exists()

def add(kind, msg):
    {'PASS': PASS, 'WARN': WARN, 'FAIL': FAIL}[kind].append(msg)

def grep(pattern, paths=None, flags=re.I):
    rx = re.compile(pattern, flags)
    if paths is None:
        paths = [p for p in ROOT.rglob('*') if p.is_file() and '.git' not in p.parts and 'target' not in p.parts and '.oscomp_patch_backup' not in p.parts]
    for p in paths:
        try:
            if rx.search(p.read_text(encoding='utf-8', errors='ignore')):
                return True
        except Exception:
            pass
    return False

def source_files():
    roots = [ROOT / x for x in ['kernel', 'arch', 'src', 'user', 'userspace', 'crates']]
    out = []
    for r in roots:
        if r.exists():
            out.extend([p for p in r.rglob('*') if p.is_file() and p.suffix in {'.rs', '.c', '.h', '.S', '.s', '.asm'}])
    return out

def grep_source(pattern, flags=re.I):
    return grep(pattern, source_files(), flags)

def run(cmd):
    try:
        return subprocess.check_output(cmd, cwd=ROOT, text=True, stderr=subprocess.STDOUT)
    except Exception as e:
        return str(e)

tc = read('rust-toolchain.toml') + read('rust-toolchain')
if 'nightly-2025-01-18' in tc:
    add('PASS', 'Rust toolchain pinned to contest-friendly nightly-2025-01-18')
elif re.search(r'channel\s*=\s*"nightly"|^nightly\s*$', tc, re.M):
    add('FAIL', 'floating nightly detected; judge may download latest nightly and timeout')
else:
    add('WARN', 'no exact contest nightly pin found')

if exists('cargo-dot/config.toml'):
    add('PASS', 'non-hidden cargo-dot/config.toml present')
else:
    add('FAIL', 'cargo-dot/config.toml missing; hidden .cargo is filtered by judge')
if exists('.cargo'):
    add('WARN', '.cargo exists locally; do not rely on it being present after judge clone')
if exists('vendor/cargo'):
    add('PASS', 'vendor/cargo present for offline Cargo dependencies')
elif exists('Cargo.lock') and grep(r'crates\.io|source = "registry|source = "git\+', [ROOT / 'Cargo.lock']):
    add('FAIL', 'Cargo.lock references external registry/git dependencies but vendor/cargo is missing')
else:
    add('WARN', 'vendor/cargo missing; okay only if the project has no external Cargo deps')

mk = read('Makefile')
mkp = read('Makefile.project')
if 'scripts/oscomp-build.sh' in mk and exists('Makefile.project'):
    add('PASS', 'root Makefile all is competition wrapper and original Makefile is preserved')
else:
    add('FAIL', 'Makefile wrapper not installed or Makefile.project missing')
if re.search(r'(^|\n)all\s*:', mkp) and re.search(r'smoke|qemu|stress|soak', mkp, re.I):
    add('WARN', 'original Makefile mentions smoke/qemu/stress; wrapper should prevent all from running them')

for name in ['kernel-rv', 'kernel-la']:
    p = ROOT / name
    if p.exists() and p.stat().st_size > 0:
        info = run(['file', name]).strip()
        add('PASS', f'{name} exists: {info}')
    else:
        add('WARN', f'{name} not built yet; run make all')

if grep_source(r'OS COMP TEST GROUP START|COMP TEST GROUP'):
    add('PASS', 'runtime output marker text appears in source')
else:
    add('FAIL', 'runtime output lacks OS COMP group markers')
if grep_source(r'_testcode\.sh|testcode|root directory|readdir|scan.*dir'):
    add('PASS', 'source appears to scan or reference test scripts')
else:
    add('FAIL', 'no obvious *_testcode.sh/root directory scanner found')
if grep_source(r'ext4|Ext4|EXT4'):
    add('PASS', 'EXT4 support appears in source')
else:
    add('FAIL', 'no EXT4 support string found')
if grep_source(r'virtio.*blk|blk.*virtio|VirtioBlk|virtio-blk|virtio_mmio|virtio-pci'):
    add('PASS', 'virtio block support appears in source')
else:
    add('FAIL', 'no virtio block support string found')
if grep_source(r'shutdown|poweroff|sbi_shutdown|system_reset|QEMU_EXIT|pm_poweroff'):
    add('PASS', 'shutdown/poweroff path appears in source')
else:
    add('FAIL', 'no shutdown path found; judge may wait until timeout')

print('OSKernel2026 submission audit')
for label, items in [('PASS', PASS), ('WARN', WARN), ('FAIL', FAIL)]:
    print(f'{label}={len(items)}')
    for x in items:
        print(f'  {label}: {x}')
if FAIL:
    sys.exit(1)
