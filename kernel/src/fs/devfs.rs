use myos_vfs::{
    ArcInode, ArcSuperBlock, Dev, Errno, File, FileMode,
    FileOperations, FileSystemType, Inode, InodeId, InodeOperations, IoBuffer, MutableIoBuffer,
    PollStatus, SeekWhence, Stat, StatFs, SuperBlock, SuperBlockOperations,
    DEVFS_MAGIC, DEV_CONSOLE, DEV_NULL, DEV_ZERO,
};

// ---------------------------------------------------------------------------
// Device trait — kernel-level device abstraction
// ---------------------------------------------------------------------------

/// A device driver that can be plugged into devfs.
///
/// Each device is identified by a `Dev` (major/minor) number.
pub trait Device: Send + 'static {
    /// Read from the device at the given byte offset.
    fn read(&self, offset: u64, buf: &mut [u8]) -> Result<usize, Errno>;
    /// Write to the device at the given byte offset.
    fn write(&mut self, offset: u64, buf: &[u8]) -> Result<usize, Errno>;
    /// Device control.
    fn ioctl(&mut self, _cmd: u64, _arg: usize) -> Result<usize, Errno> {
        Err(Errno::ENOTTY)
    }
    /// Return the device type (major/minor).
    fn device_type(&self) -> Dev;
}

// ---------------------------------------------------------------------------
// Concrete devices
// ---------------------------------------------------------------------------

/// `/dev/null` — discard writes, reads return EOF.
pub struct NullDevice;

impl Device for NullDevice {
    fn read(&self, _offset: u64, _buf: &mut [u8]) -> Result<usize, Errno> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, buf: &[u8]) -> Result<usize, Errno> {
        Ok(buf.len())
    }
    fn device_type(&self) -> Dev {
        DEV_NULL
    }
}

/// `/dev/zero` — reads return infinite zeros, writes are discarded.
pub struct ZeroDevice;

impl Device for ZeroDevice {
    fn read(&self, _offset: u64, buf: &mut [u8]) -> Result<usize, Errno> {
        buf.fill(0);
        Ok(buf.len())
    }
    fn write(&mut self, _offset: u64, buf: &[u8]) -> Result<usize, Errno> {
        Ok(buf.len())
    }
    fn device_type(&self) -> Dev {
        DEV_ZERO
    }
}

/// `/dev/console` — reads from kernel console, writes to kernel console.
pub struct ConsoleDevice;

impl Device for ConsoleDevice {
    fn read(&self, _offset: u64, buf: &mut [u8]) -> Result<usize, Errno> {
        // Console reads not yet supported; return EOF.
        let _ = buf;
        Ok(0)
    }
    fn write(&mut self, _offset: u64, buf: &[u8]) -> Result<usize, Errno> {
        // Write each byte to the arch console.
        for &byte in buf {
            crate::arch::early_console::write_byte(byte);
        }
        Ok(buf.len())
    }
    fn device_type(&self) -> Dev {
        DEV_CONSOLE
    }
}

// ---------------------------------------------------------------------------
// DeviceFile — FileOperations adapter for Device trait objects
// ---------------------------------------------------------------------------

/// Bridges `Device` trait → `FileOperations` trait.
struct DeviceFile {
    device: alloc::boxed::Box<dyn Device>,
    dev: Dev,
}

impl DeviceFile {
    fn new(device: alloc::boxed::Box<dyn Device>) -> Self {
        let dev = device.device_type();
        Self { device, dev }
    }
}

impl FileOperations for DeviceFile {
    fn read(&self, file: &File, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        self.device.read(file.f_pos(), buf.as_mut_slice())
    }
    fn write(&mut self, file: &File, buf: &IoBuffer<'_>) -> Result<usize, Errno> {
        self.device.write(file.f_pos(), buf.as_bytes())
    }
    fn seek(&mut self, _file: &File, _offset: i64, _whence: SeekWhence) -> Result<u64, Errno> {
        Ok(0) // Devices are non-seekable
    }
    fn ioctl(&mut self, _file: &File, cmd: u64, arg: usize) -> Result<usize, Errno> {
        self.device.ioctl(cmd, arg)
    }
    fn fstat(&self, _file: &File) -> Result<Stat, Errno> {
        let mut stat = Stat::zeroed();
        stat.st_mode = FileMode::char_device(0o666).get();
        stat.st_rdev = self.dev.to_u64();
        stat.st_blksize = 4096;
        Ok(stat)
    }
    fn poll(&self, _file: &File) -> Result<PollStatus, Errno> {
        Ok(PollStatus {
            readable: true,
            writable: true,
            error: false,
        })
    }
}

// ---------------------------------------------------------------------------
// DevFs
// ---------------------------------------------------------------------------

/// Pre-defined devfs entry.
struct DevFsEntry {
    name: &'static str,
    mode: FileMode,
    dev: Dev,
    make_device: fn() -> alloc::boxed::Box<dyn Device>,
}

static DEVFS_ENTRIES: &[DevFsEntry] = &[
    DevFsEntry {
        name: "null",
        mode: FileMode::char_device(0o666),
        dev: DEV_NULL,
        make_device: || alloc::boxed::Box::new(NullDevice),
    },
    DevFsEntry {
        name: "zero",
        mode: FileMode::char_device(0o666),
        dev: DEV_ZERO,
        make_device: || alloc::boxed::Box::new(ZeroDevice),
    },
    DevFsEntry {
        name: "console",
        mode: FileMode::char_device(0o620),
        dev: DEV_CONSOLE,
        make_device: || alloc::boxed::Box::new(ConsoleDevice),
    },
];

struct DevFsInodeOps;

impl InodeOperations for DevFsInodeOps {
    fn create(&self, _dir: &Inode, _name: &str, _mode: FileMode) -> Result<InodeId, Errno> {
        Err(Errno::EROFS)
    }

    fn lookup(&self, _dir: &Inode, name: &str) -> Result<InodeId, Errno> {
        for (idx, entry) in DEVFS_ENTRIES.iter().enumerate() {
            if entry.name == name {
                // Use inode number = idx + 2 (1 is root)
                return Ok(InodeId::new(idx as u64 + 2));
            }
        }
        Err(Errno::ENOENT)
    }

    fn link(&self, _old: &Inode, _dir: &Inode, _name: &str) -> Result<(), Errno> {
        Err(Errno::EROFS)
    }

    fn unlink(&self, _dir: &Inode, _name: &str) -> Result<(), Errno> {
        Err(Errno::EROFS)
    }

    fn mkdir(&self, _dir: &Inode, _name: &str, _mode: FileMode) -> Result<InodeId, Errno> {
        Err(Errno::EROFS)
    }

    fn rmdir(&self, _dir: &Inode, _name: &str) -> Result<(), Errno> {
        Err(Errno::EROFS)
    }

    fn rename(
        &self,
        _old_dir: &Inode,
        _old_name: &str,
        _new_dir: &Inode,
        _new_name: &str,
        _flags: myos_vfs::RenameFlags,
    ) -> Result<(), Errno> {
        Err(Errno::EROFS)
    }

    fn symlink(&self, _dir: &Inode, _name: &str, _target: &str) -> Result<InodeId, Errno> {
        Err(Errno::EROFS)
    }

    fn readlink(&self, _inode: &Inode, _buffer: &mut [u8]) -> Result<usize, Errno> {
        Err(Errno::EINVAL)
    }

    fn mknod(
        &self,
        _dir: &Inode,
        _name: &str,
        _mode: FileMode,
        _dev: Dev,
    ) -> Result<InodeId, Errno> {
        Err(Errno::EROFS)
    }

    fn getattr(&self, inode: &Inode) -> Result<Stat, Errno> {
        let idx = inode.i_ino().get() as usize;
        if idx >= 2 && idx < DEVFS_ENTRIES.len() + 2 {
            let entry = &DEVFS_ENTRIES[idx - 2];
            let mut stat = Stat::zeroed();
            stat.st_ino = inode.i_ino().get();
            stat.st_mode = entry.mode.get();
            stat.st_rdev = entry.dev.to_u64();
            stat.st_blksize = 4096;
            stat.st_nlink = 1;
            Ok(stat)
        } else {
            Err(Errno::ENOENT)
        }
    }

    fn open(
        &self,
        inode: &Inode,
    ) -> Result<alloc::boxed::Box<dyn FileOperations>, Errno> {
        let idx = inode.i_ino().get() as usize;
        if idx == 1 {
            // Root directory — return a simple directory file ops that
            // can return static entries via readdir.
            Ok(alloc::boxed::Box::new(DevFsDirFile))
        } else if idx >= 2 && idx < DEVFS_ENTRIES.len() + 2 {
            let device = (DEVFS_ENTRIES[idx - 2].make_device)();
            Ok(alloc::boxed::Box::new(DeviceFile::new(device)))
        } else {
            Err(Errno::ENODEV)
        }
    }
}

struct DevFsSuperBlockOps;

impl SuperBlockOperations for DevFsSuperBlockOps {
    fn alloc_inode(
        &self,
        _sb: &SuperBlock,
        _mode: FileMode,
    ) -> Result<ArcInode, Errno> {
        Err(Errno::EROFS)
    }

    fn destroy_inode(&self, _inode: &Inode) {}

    fn statfs(&self, _sb: &SuperBlock) -> Result<StatFs, Errno> {
        Ok(StatFs {
            f_type: DEVFS_MAGIC,
            f_bsize: 4096,
            f_blocks: 0,
            f_bfree: 0,
            f_bavail: 0,
            f_files: 0,
            f_ffree: 0,
            f_fsid: [0; 2],
            f_namelen: 255,
            f_frsize: 4096,
            f_flags: 0,
            __spare: [0; 4],
        })
    }
}

struct DevFsFsType;

impl FileSystemType for DevFsFsType {
    fn name(&self) -> &'static str {
        "devfs"
    }

    fn mount(
        &self,
        _source: Option<&str>,
        _flags: u64,
        _data: Option<&str>,
    ) -> Result<ArcSuperBlock, Errno> {
        // Root directory inode
        let root_ino = InodeId::new(1);
        let mut root_inode = Inode::new(root_ino, FileMode::DIR_DEFAULT, true);
        root_inode.set_i_op(alloc::boxed::Box::new(DevFsInodeOps));

        let ops: alloc::boxed::Box<dyn SuperBlockOperations> =
            alloc::boxed::Box::new(DevFsSuperBlockOps);
        let sb = SuperBlock::new(ArcInode::new(root_inode), ops);
        Ok(ArcSuperBlock::new(sb))
    }
}

// ---------------------------------------------------------------------------
// DevFsDirFile — FileOperations for the root directory
// ---------------------------------------------------------------------------

struct DevFsDirFile;

impl FileOperations for DevFsDirFile {
    fn read(&self, _file: &File, _buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        Err(Errno::EISDIR)
    }
    fn write(&mut self, _file: &File, _buf: &IoBuffer<'_>) -> Result<usize, Errno> {
        Err(Errno::EISDIR)
    }
    fn seek(&mut self, _file: &File, _offset: i64, _whence: SeekWhence) -> Result<u64, Errno> {
        Ok(0)
    }
    fn readdir(
        &self,
        _file: &File,
        entries: &mut myos_vfs::ReadDirEntries<'_>,
    ) -> Result<usize, Errno> {
        for (idx, entry) in DEVFS_ENTRIES.iter().enumerate() {
            let d_type = myos_vfs::DirEntry::DT_CHR;
            let Some(de) = myos_vfs::DirEntry::new(idx as u64 + 2, 0, d_type, entry.name) else {
                continue;
            };
            if entries.push(&de).is_none() {
                break;
            }
        }
        Ok(entries.written())
    }
    fn fstat(&self, _file: &File) -> Result<Stat, Errno> {
        let mut stat = Stat::zeroed();
        stat.st_ino = 1;
        stat.st_mode = FileMode::DIR_DEFAULT.get();
        stat.st_nlink = 1;
        stat.st_blksize = 4096;
        Ok(stat)
    }
    fn poll(&self, _file: &File) -> Result<PollStatus, Errno> {
        Ok(PollStatus {
            readable: true,
            writable: false,
            error: false,
        })
    }
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

#[cfg(debug_assertions)]
pub fn verify() {
    crate::context::assert_task_context();
    crate::println!("devfs test:");

    let sb = mount_devfs();

    test_null_device(&sb);
    test_zero_device(&sb);
    test_console_device(&sb);

    crate::println!("  /dev/null       : verified");
    crate::println!("  /dev/zero       : verified");
    crate::println!("  /dev/console    : verified");
}

#[cfg(debug_assertions)]
fn mount_devfs() -> ArcSuperBlock {
    let fs_type: &dyn FileSystemType = &DevFsFsType;
    fs_type.mount(None, 0, None).expect("devfs mount failed")
}

#[cfg(debug_assertions)]
fn test_null_device(sb: &ArcSuperBlock) {
    let root = sb.as_ref().root_inode().as_ref();
    let ops = root.i_op().expect("root has no i_op");

    // Lookup /dev/null
    let null_ino = ops.lookup(root, "null").expect("lookup /dev/null failed");
    assert_eq!(null_ino.get(), 2);

    // Open and test read (should return 0 bytes)
    let _file_ops = ops.open(root).unwrap();
    let _root_file = crate::fs::make_test_dentry("/", sb.as_ref().root_inode().clone(), sb.clone());
    let _open_root_ops = ops.open(root).unwrap();
    // For a quick test, directly use the device
    let mut null_dev = NullDevice;
    let mut buf = [1u8; 16];
    assert_eq!(null_dev.read(0, &mut buf).unwrap(), 0);
    assert_eq!(null_dev.write(0, b"hello").unwrap(), 5);
}

#[cfg(debug_assertions)]
fn test_zero_device(sb: &ArcSuperBlock) {
    let root = sb.as_ref().root_inode().as_ref();
    let ops = root.i_op().expect("root has no i_op");

    let zero_ino = ops.lookup(root, "zero").expect("lookup /dev/zero failed");
    assert_eq!(zero_ino.get(), 3);

    let zero_dev = ZeroDevice;
    let mut buf = [1u8; 16];
    assert_eq!(zero_dev.read(0, &mut buf).unwrap(), 16);
    assert!(buf.iter().all(|&b| b == 0), "/dev/zero did not fill with zeros");
}

#[cfg(debug_assertions)]
fn test_console_device(sb: &ArcSuperBlock) {
    let root = sb.as_ref().root_inode().as_ref();
    let ops = root.i_op().expect("root has no i_op");

    let con_ino = ops.lookup(root, "console").expect("lookup /dev/console failed");
    assert_eq!(con_ino.get(), 4);

    let mut con_dev = ConsoleDevice;
    // Write to console (should appear in QEMU serial output)
    assert_eq!(con_dev.write(0, b"\n[devfs verify] console output\n").unwrap(), 31);
}
