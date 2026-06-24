#!/usr/bin/env python3
from pathlib import Path
root = Path(__file__).resolve().parents[1]
main = (root / 'kernel/src/main.rs').read_text()
ext4 = (root / 'kernel/src/ext4.rs').read_text()
user = (root / 'kernel/src/user.rs').read_text()
checks = [
    ('ext4 bounded list_directory API present', 'pub fn list_directory(' in ext4),
    ('directory listing does not require root snapshot', 'load_root_snapshot' not in main),
    ('lazy file install mount label', 'lazy file install' in main),
    ('bounded scan dirs limit', 'MAX_SCAN_DIRS' in main),
    ('bounded test script limit', 'MAX_TEST_SCRIPTS' in main),
    ('test scripts installed under /mnt/sdcard', '/mnt/sdcard{}' in main),
    ('SCANNED_TEST_SCRIPTS stores installed paths', 'installed_scripts' in main and 'clone_from(&installed_scripts)' in main),
    ('OS COMP START marker present', '#### OS COMP TEST GROUP START' in user),
    ('OS COMP END marker present', '#### OS COMP TEST GROUP END' in user),
    ('script cwd uses parent directory', 'vfs_path.rfind' in user),
]
failed = 0
for name, ok in checks:
    print(('PASS' if ok else 'FAIL') + ': ' + name)
    failed += 0 if ok else 1
print(f'oscomp-sdcard-bounded-discovery-exec-audit: PASS={len(checks)-failed} FAIL={failed}')
raise SystemExit(1 if failed else 0)
