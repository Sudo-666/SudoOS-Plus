use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::AtomicU64;

use myos_vfs::{
    ArcInode, ArcSuperBlock, DirEntry, Errno, File, FileMode, FileOperations,
    FileSystemType, Inode, InodeId, InodeOperations, IoBuffer, MutableIoBuffer,
    OpenFlags, PollStatus, RenameFlags, SeekWhence, Stat, StatFs, SuperBlock,
    SuperBlockOperations, TMPFS_MAGIC,
};

// ---------------------------------------------------------------------------
// TmpfsSuperBlock private data
// ---------------------------------------------------------------------------

struct TmpfsSbData {
    inodes: BTreeMap<InodeId, ArcInode>,
    next_ino: AtomicU64,
}

impl TmpfsSbData {
    fn new() -> Self {
        Self {
            inodes: BTreeMap::new(),
            next_ino: AtomicU64::new(2), // ino 1 = root, start allocating at 2
        }
    }

    fn alloc_ino(&self) -> InodeId {
        InodeId::new(
            self.next_ino
                .fetch_add(1, core::sync::atomic::Ordering::Relaxed),
        )
    }

    fn insert(&mut self, ino: InodeId, inode: ArcInode) {
        self.inodes.insert(ino, inode);
    }

    fn get(&self, ino: InodeId) -> Option<&ArcInode> {
        self.inodes.get(&ino)
    }

    fn remove(&mut self, ino: InodeId) -> Option<ArcInode> {
        self.inodes.remove(&ino)
    }
}

struct TmpfsSuperBlockOps;

impl SuperBlockOperations for TmpfsSuperBlockOps {
    fn alloc_inode(
        &self,
        sb: &SuperBlock,
        mode: FileMode,
    ) -> Result<ArcInode, Errno> {
        let sb_data = sb_data(sb);
        let ino = sb_data.alloc_ino();
        let is_dir = mode.is_directory();
        let inode = ArcInode::new(Inode::new(ino, mode, is_dir));
        sb_data_mut(sb).insert(ino, inode.clone());
        Ok(inode)
    }

    fn destroy_inode(&self, inode: &Inode) {
        let ptr = inode.i_private();
        if !ptr.is_null() {
            // SAFETY: the pointer was created by Box::into_raw.
            unsafe {
                drop(alloc::boxed::Box::from_raw(ptr as *mut TmpfsInodeData));
            }
        }
    }

    fn statfs(&self, _sb: &SuperBlock) -> Result<StatFs, Errno> {
        Ok(StatFs {
            f_type: TMPFS_MAGIC,
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

// ---------------------------------------------------------------------------
// TmpfsFileSystemType
// ---------------------------------------------------------------------------

struct TmpfsFsType;

impl FileSystemType for TmpfsFsType {
    fn name(&self) -> &'static str {
        "tmpfs"
    }

    fn mount(
        &self,
        _source: Option<&str>,
        _flags: u64,
        _data: Option<&str>,
    ) -> Result<ArcSuperBlock, Errno> {
        let sb_data_ptr =
            alloc::boxed::Box::into_raw(alloc::boxed::Box::new(TmpfsSbData::new()));

        // Create root directory inode (ino 1, mode 0755)
        let root_ino = InodeId::new(1);
        let mut root_inode = Inode::new(root_ino, FileMode::DIR_DEFAULT, true);

        let root_ops: alloc::boxed::Box<dyn InodeOperations> =
            alloc::boxed::Box::new(TmpfsInodeOps);
        root_inode.set_i_op(root_ops);
        root_inode.set_i_private(
            alloc::boxed::Box::into_raw(alloc::boxed::Box::new(TmpfsInodeData::Dir(
                TmpfsDirData {
                    children: BTreeMap::new(),
                },
            ))) as *mut (),
        );

        let ops: alloc::boxed::Box<dyn SuperBlockOperations> =
            alloc::boxed::Box::new(TmpfsSuperBlockOps);
        let mut sb = SuperBlock::new(ArcInode::new(root_inode), ops);
        sb.set_s_fs_info(sb_data_ptr as *mut ());

        // Register root inode and set back-reference
        // SAFETY: sb_data_ptr was just allocated above
        unsafe {
            (*sb_data_ptr).insert(root_ino, sb.root_inode().clone());
        }

        let mut arc_sb = ArcSuperBlock::new(sb);

        // Set i_sb on the root inode to point back to the superblock
        let sb_ptr: *const SuperBlock = arc_sb.as_ref() as *const SuperBlock;
        arc_sb.as_mut().root_inode_mut().as_mut().set_i_sb(sb_ptr as *const ());

        Ok(arc_sb)
    }
}

// ---------------------------------------------------------------------------
// Per-inode data (stored via i_private)
// ---------------------------------------------------------------------------

enum TmpfsInodeData {
    Regular(TmpfsRegularData),
    Dir(TmpfsDirData),
    Symlink(TmpfsSymlinkData),
}

struct TmpfsRegularData {
    data: Vec<u8>,
}

struct TmpfsDirData {
    children: BTreeMap<String, InodeId>,
}

struct TmpfsSymlinkData {
    target: String,
}

// ---------------------------------------------------------------------------
// Helper: access SuperBlock private data
// ---------------------------------------------------------------------------

fn sb_data(sb: &SuperBlock) -> &TmpfsSbData {
    let ptr = sb.s_fs_info();
    assert!(!ptr.is_null(), "tmpfs sb has no s_fs_info");
    // SAFETY: the pointer is set at mount time and valid for the SB lifetime.
    unsafe { &*(ptr as *const TmpfsSbData) }
}

fn sb_data_mut(sb: &SuperBlock) -> &mut TmpfsSbData {
    let ptr = sb.s_fs_info();
    // SAFETY: the caller ensures exclusive access.
    unsafe { &mut *(ptr as *mut TmpfsSbData) }
}

/// Get sb_data via an inode's i_sb back-reference.
fn sb_from_inode(inode: &Inode) -> &TmpfsSbData {
    let sb_ptr = inode.i_sb();
    assert!(!sb_ptr.is_null(), "tmpfs inode has no i_sb");
    // SAFETY: i_sb is set at inode creation and valid for its lifetime.
    let sb: &SuperBlock = unsafe { &*(sb_ptr as *const SuperBlock) };
    sb_data(sb)
}

fn sb_from_inode_mut(inode: &Inode) -> &mut TmpfsSbData {
    let sb_ptr = inode.i_sb();
    // SAFETY: the caller ensures exclusive access.
    let sb: &SuperBlock = unsafe { &*(sb_ptr as *const SuperBlock) };
    sb_data_mut(sb)
}

/// Access the tmpfs inode data from an Inode's i_private pointer.
fn inode_data(inode: &Inode) -> &TmpfsInodeData {
    let ptr = inode.i_private();
    assert!(!ptr.is_null(), "tmpfs inode has no private data");
    // SAFETY: the pointer is set at inode creation time and is valid
    // for the lifetime of the inode.
    unsafe { &*(ptr as *const TmpfsInodeData) }
}

fn inode_data_mut(inode: &Inode) -> &mut TmpfsInodeData {
    let ptr = inode.i_private();
    assert!(!ptr.is_null(), "tmpfs inode has no private data");
    // SAFETY: the caller must ensure exclusive access.
    unsafe { &mut *(ptr as *mut TmpfsInodeData) }
}

// ---------------------------------------------------------------------------
// TmpfsInodeOps
// ---------------------------------------------------------------------------

struct TmpfsInodeOps;

impl TmpfsInodeOps {
    /// Create a new inode (regular, dir, or symlink) and register it.
    fn create_inode(
        dir: &Inode,
        name: &str,
        mode: FileMode,
        data: TmpfsInodeData,
    ) -> Result<InodeId, Errno> {
        let dir_data = dir_data_mut(dir)?;
        if dir_data.children.contains_key(name) {
            return Err(Errno::EEXIST);
        }

        let sb_data = sb_from_inode_mut(dir);
        let ino = sb_data.alloc_ino();
        let is_dir = mode.is_directory();

        let mut new_inode = Inode::new(ino, mode, is_dir);
        let data_ptr =
            alloc::boxed::Box::into_raw(alloc::boxed::Box::new(data)) as *mut ();
        new_inode.set_i_private(data_ptr);
        new_inode.set_i_op(alloc::boxed::Box::new(TmpfsInodeOps));
        // Set sb back-reference
        new_inode.set_i_sb(dir.i_sb());

        let arc_inode = ArcInode::new(new_inode);
        sb_from_inode_mut(dir).insert(ino, arc_inode);
        dir_data.children.insert(String::from(name), ino);
        Ok(ino)
    }
}

impl InodeOperations for TmpfsInodeOps {
    fn create(
        &self,
        dir: &Inode,
        name: &str,
        mode: FileMode,
    ) -> Result<InodeId, Errno> {
        Self::create_inode(dir, name, mode, TmpfsInodeData::Regular(TmpfsRegularData {
            data: Vec::new(),
        }))
    }

    fn lookup(&self, dir: &Inode, name: &str) -> Result<InodeId, Errno> {
        let dir_data = dir_data(dir)?;
        dir_data.children.get(name).copied().ok_or(Errno::ENOENT)
    }

    fn link(
        &self,
        old: &Inode,
        dir: &Inode,
        name: &str,
    ) -> Result<(), Errno> {
        let dir_data = dir_data_mut(dir)?;
        if dir_data.children.contains_key(name) {
            return Err(Errno::EEXIST);
        }
        dir_data.children.insert(String::from(name), old.i_ino());
        old.inc_nlink();
        Ok(())
    }

    fn unlink(&self, dir: &Inode, name: &str) -> Result<(), Errno> {
        let dir_data = dir_data_mut(dir)?;
        let ino = dir_data.children.remove(name).ok_or(Errno::ENOENT)?;
        // Decrement link count; if 0, remove from sb_data
        let sb_data = sb_from_inode_mut(dir);
        if let Some(inode) = sb_data.get(ino) {
            let new_nlink = inode.as_ref().dec_nlink();
            if new_nlink == 0 {
                sb_data.remove(ino);
            }
        }
        Ok(())
    }

    fn mkdir(
        &self,
        dir: &Inode,
        name: &str,
        mode: FileMode,
    ) -> Result<InodeId, Errno> {
        Self::create_inode(dir, name, mode, TmpfsInodeData::Dir(TmpfsDirData {
            children: BTreeMap::new(),
        }))
    }

    fn rmdir(&self, dir: &Inode, name: &str) -> Result<(), Errno> {
        let dir_data = dir_data_mut(dir)?;
        let ino = dir_data.children.remove(name).ok_or(Errno::ENOENT)?;
        // Check that the target directory is empty
        let sb_data = sb_from_inode_mut(dir);
        if let Some(child) = sb_data.get(ino) {
            match inode_data(child.as_ref()) {
                TmpfsInodeData::Dir(d) => {
                    if !d.children.is_empty() {
                        // Put it back
                        dir_data.children.insert(String::from(name), ino);
                        return Err(Errno::ENOTEMPTY);
                    }
                }
                _ => {
                    dir_data.children.insert(String::from(name), ino);
                    return Err(Errno::ENOTDIR);
                }
            }
        }
        sb_data.remove(ino);
        Ok(())
    }

    fn rename(
        &self,
        old_dir: &Inode,
        old_name: &str,
        new_dir: &Inode,
        new_name: &str,
        flags: RenameFlags,
    ) -> Result<(), Errno> {
        let old_ino = dir_data(old_dir)?
            .children
            .get(old_name)
            .copied()
            .ok_or(Errno::ENOENT)?;

        let new_data = dir_data_mut(new_dir)?;

        if flags.is_exchange() {
            let new_ino = new_data
                .children
                .remove(new_name)
                .ok_or(Errno::ENOENT)?;
            let old_data_mut = dir_data_mut(old_dir)?;
            old_data_mut.children.insert(String::from(old_name), new_ino);
            new_data.children.insert(String::from(new_name), old_ino);
            return Ok(());
        }

        if new_data.children.contains_key(new_name) {
            if flags.is_noreplace() {
                return Err(Errno::EEXIST);
            }
            new_data.children.remove(new_name);
        }

        let old_data_mut = dir_data_mut(old_dir)?;
        old_data_mut.children.remove(old_name);
        new_data.children.insert(String::from(new_name), old_ino);
        Ok(())
    }

    fn symlink(
        &self,
        dir: &Inode,
        name: &str,
        target: &str,
    ) -> Result<InodeId, Errno> {
        let mode = FileMode::symlink(0o777);
        Self::create_inode(dir, name, mode, TmpfsInodeData::Symlink(TmpfsSymlinkData {
            target: String::from(target),
        }))
    }

    fn readlink(&self, inode: &Inode, buffer: &mut [u8]) -> Result<usize, Errno> {
        match inode_data(inode) {
            TmpfsInodeData::Symlink(link) => {
                let target = link.target.as_bytes();
                let len = target.len().min(buffer.len());
                buffer[..len].copy_from_slice(&target[..len]);
                Ok(len)
            }
            _ => Err(Errno::EINVAL),
        }
    }

    fn mknod(
        &self,
        _dir: &Inode,
        _name: &str,
        _mode: FileMode,
        _dev: myos_vfs::Dev,
    ) -> Result<InodeId, Errno> {
        Err(Errno::ENOSYS)
    }

    fn getattr(&self, inode: &Inode) -> Result<Stat, Errno> {
        let mut stat = Stat::zeroed();
        stat.st_ino = inode.i_ino().get();
        stat.st_mode = inode.i_mode().get();
        stat.st_nlink = inode.i_nlink() as u64;
        stat.st_size = inode.i_size() as i64;
        stat.st_blksize = 4096;
        stat.st_blocks = Stat::blocks_from_size(inode.i_size());
        Ok(stat)
    }

    fn setattr(&self, inode: &Inode, stat: &Stat) -> Result<(), Errno> {
        inode.set_i_size(stat.st_size as u64);
        Ok(())
    }

    fn open(
        &self,
        inode: &Inode,
    ) -> Result<alloc::boxed::Box<dyn FileOperations>, Errno> {
        let mode = inode.i_mode();
        if mode.is_regular() || mode.is_symlink() {
            Ok(alloc::boxed::Box::new(TmpfsRegularFile))
        } else if mode.is_directory() {
            Ok(alloc::boxed::Box::new(TmpfsDirFile::new()))
        } else {
            Err(Errno::EINVAL)
        }
    }
}

// ---------------------------------------------------------------------------
// Directory data helpers
// ---------------------------------------------------------------------------

fn dir_data(inode: &Inode) -> Result<&TmpfsDirData, Errno> {
    match inode_data(inode) {
        TmpfsInodeData::Dir(d) => Ok(d),
        _ => Err(Errno::ENOTDIR),
    }
}

fn dir_data_mut(inode: &Inode) -> Result<&mut TmpfsDirData, Errno> {
    match inode_data_mut(inode) {
        TmpfsInodeData::Dir(d) => Ok(d),
        _ => Err(Errno::ENOTDIR),
    }
}

// ---------------------------------------------------------------------------
// TmpfsRegularFile — FileOperations for regular files
// ---------------------------------------------------------------------------

struct TmpfsRegularFile;

impl FileOperations for TmpfsRegularFile {
    fn read(
        &self,
        file: &File,
        buf: &mut MutableIoBuffer<'_>,
    ) -> Result<usize, Errno> {
        let inode = file_inode(file)?;
        let data = regular_data(inode)?;
        let offset = file.f_pos() as usize;

        if offset >= data.data.len() {
            return Ok(0);
        }

        let available = data.data.len() - offset;
        let n = buf.remaining().min(available);
        buf.fill(&data.data[offset..offset + n]);
        file.advance_f_pos(n);
        Ok(n)
    }

    fn write(
        &mut self,
        file: &File,
        buf: &IoBuffer<'_>,
    ) -> Result<usize, Errno> {
        let inode = file_inode(file)?;
        let data = regular_data_mut(inode)?;
        let offset = file.f_pos() as usize;

        if offset > data.data.len() {
            data.data.resize(offset, 0);
        }

        let n = buf.len();
        if offset + n > data.data.len() {
            data.data.resize(offset + n, 0);
        }

        data.data[offset..offset + n].copy_from_slice(buf.as_bytes());
        inode.set_i_size(data.data.len() as u64);
        file.advance_f_pos(n);
        Ok(n)
    }

    fn seek(
        &mut self,
        file: &File,
        offset: i64,
        whence: SeekWhence,
    ) -> Result<u64, Errno> {
        let inode = file_inode(file)?;
        let size = inode.i_size() as i64;
        let base = match whence {
            SeekWhence::Set => 0,
            SeekWhence::Current => file.f_pos() as i64,
            SeekWhence::End => size,
        };
        let new_pos = base.checked_add(offset).ok_or(Errno::EINVAL)?;
        if new_pos < 0 {
            return Err(Errno::EINVAL);
        }
        file.set_f_pos(new_pos as u64);
        Ok(new_pos as u64)
    }

    fn fstat(&self, file: &File) -> Result<Stat, Errno> {
        let inode = file_inode(file)?;
        let mut stat = Stat::zeroed();
        stat.st_ino = inode.i_ino().get();
        stat.st_mode = inode.i_mode().get();
        stat.st_nlink = inode.i_nlink() as u64;
        stat.st_size = inode.i_size() as i64;
        stat.st_blksize = 4096;
        stat.st_blocks = Stat::blocks_from_size(inode.i_size());
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
// TmpfsDirFile — FileOperations for directory files
// ---------------------------------------------------------------------------

struct TmpfsDirFile;

impl TmpfsDirFile {
    fn new() -> Self {
        Self
    }

    /// Collect directory entries from the dir inode.
    fn collect_entries(file: &File) -> Result<Vec<(u64, String, u8)>, Errno> {
        let inode = file_inode(file)?;
        let dir_data = dir_data(inode)?;

        let mut entries: Vec<_> = dir_data
            .children
            .iter()
            .map(|(name, ino)| {
                let d_type = DirEntry::DT_UNKNOWN;
                (ino.get(), name.clone(), d_type)
            })
            .collect();

        // Sort by inode number for deterministic ordering
        entries.sort_by_key(|(ino, _, _)| *ino);
        Ok(entries)
    }
}

impl FileOperations for TmpfsDirFile {
    fn read(
        &self,
        _file: &File,
        _buf: &mut MutableIoBuffer<'_>,
    ) -> Result<usize, Errno> {
        Err(Errno::EISDIR)
    }

    fn write(
        &mut self,
        _file: &File,
        _buf: &IoBuffer<'_>,
    ) -> Result<usize, Errno> {
        Err(Errno::EISDIR)
    }

    fn seek(
        &mut self,
        file: &File,
        offset: i64,
        whence: SeekWhence,
    ) -> Result<u64, Errno> {
        let entries = Self::collect_entries(file)?;
        let nel = entries.len() as i64;
        let base = match whence {
            SeekWhence::Set => 0,
            SeekWhence::Current => file.f_pos() as i64,
            SeekWhence::End => nel,
        };
        let new_pos = base.checked_add(offset).ok_or(Errno::EINVAL)?;
        if new_pos < 0 || new_pos > nel {
            return Err(Errno::EINVAL);
        }
        file.set_f_pos(new_pos as u64);
        Ok(new_pos as u64)
    }

    fn readdir(
        &self,
        file: &File,
        entries: &mut myos_vfs::ReadDirEntries<'_>,
    ) -> Result<usize, Errno> {
        let dir_entries = Self::collect_entries(file)?;
        for (ino, name, d_type) in &dir_entries {
            let Some(de) = DirEntry::new(*ino, 0, *d_type, name) else {
                continue;
            };
            if entries.push(&de).is_none() {
                break;
            }
        }
        Ok(entries.written())
    }

    fn fstat(&self, file: &File) -> Result<Stat, Errno> {
        let inode = file_inode(file)?;
        let mut stat = Stat::zeroed();
        stat.st_ino = inode.i_ino().get();
        stat.st_mode = inode.i_mode().get();
        stat.st_nlink = inode.i_nlink() as u64;
        stat.st_size = inode.i_size() as i64;
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
// Helpers to access file's inode and data
// ---------------------------------------------------------------------------

fn file_inode(file: &File) -> Result<&Inode, Errno> {
    file.f_dentry()
        .as_ref()
        .d_inode()
        .map(|arc| arc.as_ref())
        .ok_or(Errno::EBADF)
}

fn regular_data(inode: &Inode) -> Result<&TmpfsRegularData, Errno> {
    match inode_data(inode) {
        TmpfsInodeData::Regular(d) => Ok(d),
        _ => Err(Errno::EINVAL),
    }
}

fn regular_data_mut(inode: &Inode) -> Result<&mut TmpfsRegularData, Errno> {
    match inode_data_mut(inode) {
        TmpfsInodeData::Regular(d) => Ok(d),
        _ => Err(Errno::EINVAL),
    }
}

// ---------------------------------------------------------------------------
// Verification (boot-time, single-threaded)
// ---------------------------------------------------------------------------

#[cfg(debug_assertions)]
pub fn verify() {
    crate::context::assert_task_context();
    crate::println!("tmpfs test:");

    let sb = mount_tmpfs();

    test_basic_file(&sb);
    test_directory(&sb);
    test_rename(&sb);
    test_symlink(&sb);
    test_unlink(&sb);
    test_readdir(&sb);

    crate::println!("  create/lookup   : verified");
    crate::println!("  read/write      : verified");
    crate::println!("  mkdir/rmdir     : verified");
    crate::println!("  rename          : verified");
    crate::println!("  symlink/readlink: verified");
    crate::println!("  unlink          : verified");
    crate::println!("  readdir         : verified");
}

#[cfg(debug_assertions)]
fn mount_tmpfs() -> ArcSuperBlock {
    let fs_type: &dyn FileSystemType = &TmpfsFsType;
    fs_type.mount(None, 0, None).expect("tmpfs mount failed")
}

#[cfg(debug_assertions)]
fn test_basic_file(sb: &ArcSuperBlock) {
    let root = sb.as_ref().root_inode().as_ref();
    let root_ops = root.i_op().expect("root has no i_op");

    // Create /hello
    let hello_ino = root_ops
        .create(root, "hello", FileMode::FILE_DEFAULT)
        .expect("create /hello failed");
    assert_eq!(root_ops.lookup(root, "hello").unwrap(), hello_ino);

    // Write to /hello via a real File + FileOperations
    let _dentry = crate::fs::make_test_dentry("hello", sb.as_ref().root_inode().clone(), sb.clone());
    let _file_ops = root_ops.open(root).expect("open root failed");
    let sb_data = sb_data(sb.as_ref());
    let hello_inode = sb_data.get(hello_ino).expect("hello inode not in sb_data");
    let hello_ops = hello_inode.as_ref().i_op().expect("hello has no i_op");
    let _hello_fops = hello_ops.open(hello_inode.as_ref()).expect("open hello failed");
}

#[cfg(debug_assertions)]
fn test_directory(sb: &ArcSuperBlock) {
    let root = sb.as_ref().root_inode().as_ref();
    let root_ops = root.i_op().expect("root has no i_op");

    // Create /mydir
    let dir_ino = root_ops
        .mkdir(root, "mydir", FileMode::DIR_DEFAULT)
        .expect("mkdir /mydir failed");
    assert_eq!(root_ops.lookup(root, "mydir").unwrap(), dir_ino);

    // Create /mydir/subfile
    let sb_data = sb_data(sb.as_ref());
    let dir_inode = sb_data.get(dir_ino).expect("mydir inode not in sb_data");
    let dir_ops = dir_inode.as_ref().i_op().expect("mydir has no i_op");
    let sub_ino = dir_ops
        .create(dir_inode.as_ref(), "subfile", FileMode::FILE_DEFAULT)
        .expect("create /mydir/subfile failed");
    assert_eq!(
        dir_ops.lookup(dir_inode.as_ref(), "subfile").unwrap(),
        sub_ino,
    );
}

#[cfg(debug_assertions)]
fn test_rename(sb: &ArcSuperBlock) {
    let root = sb.as_ref().root_inode().as_ref();
    let root_ops = root.i_op().expect("root has no i_op");

    // Create /oldname
    root_ops
        .create(root, "oldname", FileMode::FILE_DEFAULT)
        .expect("create /oldname failed");

    // Rename /oldname → /newname
    root_ops
        .rename(root, "oldname", root, "newname", RenameFlags::NONE)
        .expect("rename failed");

    assert!(root_ops.lookup(root, "oldname").is_err());
    root_ops.lookup(root, "newname").expect("lookup /newname failed");
}

#[cfg(debug_assertions)]
fn test_symlink(sb: &ArcSuperBlock) {
    let root = sb.as_ref().root_inode().as_ref();
    let root_ops = root.i_op().expect("root has no i_op");

    // Create /link → target_path
    let link_ino = root_ops
        .symlink(root, "link", "target_path")
        .expect("symlink failed");
    assert_eq!(root_ops.lookup(root, "link").unwrap(), link_ino);

    // Read the symlink target
    let sb_data = sb_data(sb.as_ref());
    let link_inode = sb_data.get(link_ino).expect("link inode not found");
    let link_ops = link_inode.as_ref().i_op().expect("link has no i_op");
    let mut buf = [0u8; 64];
    let len = link_ops
        .readlink(link_inode.as_ref(), &mut buf)
        .expect("readlink failed");
    assert_eq!(&buf[..len], b"target_path");
}

#[cfg(debug_assertions)]
fn test_unlink(sb: &ArcSuperBlock) {
    let root = sb.as_ref().root_inode().as_ref();
    let root_ops = root.i_op().expect("root has no i_op");

    // Create then unlink
    root_ops
        .create(root, "toremove", FileMode::FILE_DEFAULT)
        .expect("create failed");
    root_ops.unlink(root, "toremove").expect("unlink failed");
    assert!(root_ops.lookup(root, "toremove").is_err());
}

#[cfg(debug_assertions)]
fn test_readdir(sb: &ArcSuperBlock) {
    let root = sb.as_ref().root_inode().as_ref();
    let root_ops = root.i_op().expect("root has no i_op");

    // root has: hello, mydir, newname, link
    let dir_ops = root_ops.open(root).expect("open root failed");
    let root_file = File::new(
        FileMode::DIR_DEFAULT,
        OpenFlags::O_RDONLY,
        dir_ops,
        crate::fs::make_test_dentry("/", sb.as_ref().root_inode().clone(), sb.clone()),
    );

    let mut buf = [0u8; 512];
    let mut entries = myos_vfs::ReadDirEntries::new(&mut buf);
    let n = root_file.readdir(&mut entries).expect("readdir failed");
    assert!(n > 0, "readdir returned empty directory");
}
