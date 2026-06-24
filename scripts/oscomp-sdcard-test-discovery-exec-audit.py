#!/usr/bin/env python3
from pathlib import Path
root = Path(__file__).resolve().parents[1]
main = (root / 'kernel/src/main.rs').read_text(encoding='utf-8')
user = (root / 'kernel/src/user.rs').read_text(encoding='utf-8')
checks = []

def check(name, ok):
    checks.append((name, bool(ok)))

check('sdcard mounts full ext4 subtree at /mnt/sdcard', 'mount_ext4_subtree("/dev/vda", "/mnt/sdcard", "/", 0)' in main)
check('sdcard recursively loads root snapshot', 'load_path_snapshot' in main and 'collect_sdcard_test_scripts' in main)
check('sdcard discovers nested testcode scripts', '_testcode.sh' in main and 'testcode.sh' in main)
check('sdcard accepts test shell names', 'run_test.sh' in main and 'runtest.sh' in main and 'test.sh' in main)
check('sdcard stores absolute /mnt/sdcard paths', 'String::from("/mnt/sdcard")' in main and 'SCANNED_TEST_SCRIPTS' in main)
check('sdcard prints discovered script paths', 'test script' in main and 'test scripts  :' in main)
check('sdcard installs busybox shell for execution', '/bin/busybox' in main and '/bin/sh' in main and 'install_ext4_path' in main)
check('sdcard script runner uses scanned paths directly', 'let vfs_path = if script.starts_with' in user)
check('sdcard script runner prints OS COMP group markers', 'OS COMP TEST GROUP START' in user and 'OS COMP TEST GROUP END' in user)
check('sdcard script runner executes via shell', '"busybox", "sh"' in user or '"sh", &vfs_path' in user)
check('sdcard script runner reports PASS FAIL ERROR', ': PASS' in user and ': FAIL' in user and ': ERROR' in user)
check('sdcard script runner sets cwd to script directory', 'sdcard_script_cwd' in user and 'Some(cwd.as_str())' in user)

passed = sum(ok for _, ok in checks)
failed = len(checks) - passed
for name, ok in checks:
    print(('PASS' if ok else 'FAIL') + ': ' + name)
print(f'oscomp-sdcard-test-discovery-exec-audit: PASS={passed}, FAIL={failed}')
raise SystemExit(0 if failed == 0 else 1)
