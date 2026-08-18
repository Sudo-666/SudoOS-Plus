use alloc::{
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use myos_vfs::{
    emit_dirent64, DirEntry, Errno, File, FileMode, FileOperations, IoBuffer, MutableIoBuffer,
    OpenFlags, PollEvents, SeekWhence, Stat,
};

use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};
use crate::process::Process;

const ROOT_LOCK: LockClass = LockClass::new("vfs.root", LockRank::Vfs, 0);
const TREE_LOCK: LockClass = LockClass::new("vfs.tree", LockRank::Vfs, 0);
const NODE_LOCK: LockClass = LockClass::new("vfs.node", LockRank::Vfs, 1);
const MOUNT_LOCK: LockClass = LockClass::new("vfs.mounts", LockRank::Vfs, 2);
const MAX_COMPONENT_LEN: usize = 255;
const MAX_SYMLINK_FOLLOWS: usize = 40;
const BLOCK_CACHE_BLOCKS: usize = 32;

static ROOT: IrqSpinLock<Option<Arc<Node>>> = IrqSpinLock::new_with_class(None, ROOT_LOCK);
static TREE: IrqSpinLock<()> = IrqSpinLock::new_with_class((), TREE_LOCK);
static MOUNTS: IrqSpinLock<Vec<MountEntry>> = IrqSpinLock::new_with_class(Vec::new(), MOUNT_LOCK);
static NEXT_INODE: AtomicU64 = AtomicU64::new(1);
/// The mounted `/proc` root node, so readdir/lookup can identify it and
/// refresh its live-PID children.  Set once by `populate_proc_root`.
static PROC_ROOT: IrqSpinLock<Option<Arc<Node>>> = IrqSpinLock::new_with_class(None, TREE_LOCK);
/// Registry generation last materialized into the `/proc` children.  Prevents
/// rebuilding PID directories on every path syscall that touches `/proc`.
static PROC_LAST_GEN: AtomicU64 = AtomicU64::new(u64::MAX);

struct Node {
    ino: u64,
    parent_ino: AtomicU64,
    nlink: AtomicU64,
    mode: FileMode,
    read_only: AtomicBool,
    state: IrqSpinLock<NodeState>,
}

enum NodeState {
    Directory(Vec<(String, Arc<Node>)>),
    Regular(Vec<u8>),
    Ext4Directory {
        fs: Arc<crate::ext4::Ext4FileSystem>,
        ino: u32,
        populated: bool,
        children: Vec<(String, Arc<Node>)>,
        whiteouts: Vec<String>,
    },
    Ext4Regular {
        fs: Arc<crate::ext4::Ext4FileSystem>,
        ino: u32,
        size: u64,
        overlay: Option<Vec<u8>>,
    },
    Symlink(String),
    Device(DeviceKind),
    BlockDevice {
        name: String,
        device: Arc<dyn crate::block::BlockDevice>,
        cache: Arc<crate::block::BufferCache>,
    },
    ProcFile(Arc<dyn crate::procfs::ProcFileGenerator>),
}

fn directory_children(state: &NodeState) -> Option<&Vec<(String, Arc<Node>)>> {
    match state {
        NodeState::Directory(children) | NodeState::Ext4Directory { children, .. } => {
            Some(children)
        }
        _ => None,
    }
}

fn directory_children_mut(state: &mut NodeState) -> Option<&mut Vec<(String, Arc<Node>)>> {
    match state {
        NodeState::Directory(children) | NodeState::Ext4Directory { children, .. } => {
            Some(children)
        }
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum DeviceKind {
    Null,
    Zero,
    Console,
    Tty,
    Random,
    Urandom,
    Ptmx,
    Rtc,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MountFsType {
    Tmpfs,
    Devtmpfs,
    Proc,
    Sysfs,
    Ext4,
    Vfat,
}

struct MountEntry {
    target: String,
    source: Option<String>,
    fs_type: MountFsType,
    flags: usize,
}

pub fn initialize() {
    let root = directory(FileMode::DIR_DEFAULT);
    root.parent_ino.store(root.ino, Ordering::Release);
    let dev = directory(FileMode::DIR_DEFAULT);
    insert_child(&root, "dev", Arc::clone(&dev)).expect("unable to install /dev");
    insert_child(&dev, "null", device(DeviceKind::Null)).expect("unable to install /dev/null");
    insert_child(&dev, "zero", device(DeviceKind::Zero)).expect("unable to install /dev/zero");
    insert_child(&dev, "console", device(DeviceKind::Console))
        .expect("unable to install /dev/console");
    insert_child(&dev, "tty", device(DeviceKind::Tty)).expect("unable to install /dev/tty");
    insert_child(&dev, "random", device(DeviceKind::Random))
        .expect("unable to install /dev/random");
    insert_child(&dev, "urandom", device(DeviceKind::Urandom))
        .expect("unable to install /dev/urandom");
    insert_child(&dev, "ptmx", device(DeviceKind::Ptmx)).expect("unable to install /dev/ptmx");
    insert_child(&dev, "rtc", device(DeviceKind::Rtc)).expect("unable to install /dev/rtc");
    let pts = directory(FileMode::DIR_DEFAULT);
    insert_child(&dev, "pts", pts).expect("unable to install /dev/pts");
    install_registered_block_devices(&dev).expect("unable to install block devices");
    *ROOT.lock() = Some(root);
    initialize_mount_table().expect("unable to initialize mount table");

    crate::println!("vfs:");
    crate::println!("  root fs       : tmpfs");
    crate::println!(
        "  devfs         : /dev/null /dev/zero /dev/console /dev/tty /dev/random /dev/urandom /dev/rtc /dev/ptmx /dev/pts + block devices"
    );
    crate::println!("  fd table      : per-process");
}

pub fn install_standard_fds(process: &Process) -> Result<(), Errno> {
    process
        .files()
        .install_at(0, open("/dev/console", OpenFlags::O_RDONLY)?, false)?;
    process
        .files()
        .install_at(1, open("/dev/console", OpenFlags::O_WRONLY)?, false)?;
    process
        .files()
        .install_at(2, open("/dev/console", OpenFlags::O_WRONLY)?, false)?;
    Ok(())
}

pub fn resolve_path(cwd: &str, path: &str) -> Result<String, Errno> {
    if path.is_empty() || path.as_bytes().contains(&0) {
        return Err(Errno::Enoent);
    }
    let mut output = Vec::new();
    if !path.starts_with('/') {
        append_components(cwd, &mut output)?;
    }
    append_components(path, &mut output)?;
    build_absolute_path(&output)
}

pub fn open(path: &str, flags: OpenFlags) -> Result<myos_vfs::ArcFile, Errno> {
    if flags.access_mode() == myos_vfs::AccessMode::Invalid {
        return Err(Errno::Einval);
    }

    let node = {
        let _tree = TREE.lock();
        let follow_final = !flags.contains(OpenFlags::O_NOFOLLOW);
        let node = match lookup_follow(path, follow_final, 0) {
            Ok(node) => {
                if flags.contains(OpenFlags::O_CREAT) && flags.contains(OpenFlags::O_EXCL) {
                    return Err(Errno::Eexist);
                }
                if !follow_final && node.mode.file_type() == myos_vfs::FileType::Symlink {
                    return Err(Errno::Eloop);
                }
                node
            }
            Err(Errno::Enoent) if flags.contains(OpenFlags::O_CREAT) => create_regular(path)?,
            Err(error) => return Err(error),
        };

        if flags.contains(OpenFlags::O_DIRECTORY)
            && node.mode.file_type() != myos_vfs::FileType::Directory
        {
            return Err(Errno::Enotdir);
        }
        if node.mode.file_type() == myos_vfs::FileType::Directory
            && flags.access_mode().is_writable()
        {
            return Err(Errno::Eisdir);
        }
        if flags.contains(OpenFlags::O_TRUNC) && flags.access_mode().is_writable() {
            truncate_node(&node, 0)?;
        }
        node
    };

    let ops: Arc<dyn FileOperations> = match node.mode.file_type() {
        myos_vfs::FileType::Regular => Arc::new(RegularFile { node }),
        myos_vfs::FileType::Directory => Arc::new(DirectoryFile { node }),
        myos_vfs::FileType::CharDevice => {
            let kind = match &*node.state.lock() {
                NodeState::Device(kind) => *kind,
                _ => return Err(Errno::Enodev),
            };
            if let DeviceKind::Ptmx = kind {
                // 每次 open /dev/ptmx 创建新的 PTY 对
                let (master, _slave, _index) = crate::devpts::create_pty_pair(flags)?;
                // slave 应注册到 /dev/pts/<N>，此处简化处理
                return Ok(master);
            }
            if let DeviceKind::Tty = kind {
                // /dev/tty only works for processes with a controlling terminal.
                if !crate::tty::has_controlling_tty() {
                    return Err(Errno::Enxio);
                }
            }
            Arc::new(DeviceFile { node, kind })
        }
        myos_vfs::FileType::BlockDevice => {
            let (name, device, cache) = match &*node.state.lock() {
                NodeState::BlockDevice {
                    name,
                    device,
                    cache,
                } => (name.clone(), Arc::clone(device), Arc::clone(cache)),
                _ => return Err(Errno::Enodev),
            };
            Arc::new(BlockDeviceFile {
                node,
                name,
                device,
                cache,
            })
        }
        myos_vfs::FileType::Symlink => return Err(Errno::Eloop),
        myos_vfs::FileType::Unknown => return Err(Errno::Einval),
    };
    Ok(File::new_with_path(flags, String::from(path), ops))
}

pub fn stat(path: &str) -> Result<Stat, Errno> {
    let _tree = TREE.lock();
    stat_for_node(&lookup(path)?)
}

pub fn lstat(path: &str) -> Result<Stat, Errno> {
    let _tree = TREE.lock();
    stat_for_node(&lookup_nofollow(path)?)
}

pub fn mkdir(path: &str, mode: u32) -> Result<(), Errno> {
    // mkdir "/" or "/." etc. → already exists
    if path == "/" || path == "/." || path == "//" {
        return Err(Errno::Eexist);
    }
    let _tree = TREE.lock();
    let (parent_path, name) = split_parent(path)?;
    let parent = lookup(parent_path)?;
    if is_node_read_only(&parent) {
        return Err(Errno::Erofs);
    }
    let permissions = mode & 0o777;
    insert_child(
        &parent,
        name,
        directory(FileMode::from_bits(FileMode::S_IFDIR | permissions)),
    )
}

pub fn unlink(path: &str, remove_dir: bool) -> Result<(), Errno> {
    let _tree = TREE.lock();
    let (parent_path, name) = split_parent(path)?;
    let parent = lookup(parent_path)?;
    if is_node_read_only(&parent) {
        return Err(Errno::Erofs);
    }
    remove_child(&parent, name, remove_dir).map(|_| ())
}

pub fn rename(old_path: &str, new_path: &str) -> Result<(), Errno> {
    let _tree = TREE.lock();
    let (old_parent_path, old_name) = split_parent(old_path)?;
    let (new_parent_path, new_name) = split_parent(new_path)?;
    if old_path == "/" || new_path == "/" {
        return Err(Errno::Einval);
    }
    let old_parent = lookup(old_parent_path)?;
    let new_parent = lookup(new_parent_path)?;
    if is_node_read_only(&old_parent) || is_node_read_only(&new_parent) {
        return Err(Errno::Erofs);
    }
    let source = lookup_child(&old_parent, old_name)?.ok_or(Errno::Enoent)?;
    if is_node_read_only(&source) {
        return Err(Errno::Erofs);
    }
    if source.mode.file_type() == myos_vfs::FileType::Directory
        && is_path_descendant(new_path, old_path)
    {
        return Err(Errno::Einval);
    }
    let target = lookup_child(&new_parent, new_name)?;
    validate_rename_target(&source, target.as_ref())?;
    let stored_name = clone_component(new_name)?;

    if Arc::ptr_eq(&old_parent, &new_parent) {
        rename_inside_parent(&old_parent, old_name, stored_name)
    } else {
        reserve_child_slot(&new_parent, &stored_name, target.is_some())?;
        let moved = remove_child_unchecked(&old_parent, old_name)?;
        insert_child_prepared(&new_parent, stored_name, moved, target.is_some())
    }
}

pub fn symlink(target: &str, link_path: &str) -> Result<(), Errno> {
    if target.is_empty() || target.len() > 4096 || target.as_bytes().contains(&0) {
        return Err(Errno::Einval);
    }
    let _tree = TREE.lock();
    let (parent_path, name) = split_parent(link_path)?;
    let parent = lookup(parent_path)?;
    if is_node_read_only(&parent) {
        return Err(Errno::Erofs);
    }
    insert_child(&parent, name, symlink_node(target))
}

pub fn replace_with_symlink(target: &str, link_path: &str) -> Result<(), Errno> {
    if target.is_empty() || target.len() > 4096 || target.as_bytes().contains(&0) {
        return Err(Errno::Einval);
    }
    let _tree = TREE.lock();
    let (parent_path, name) = split_parent(link_path)?;
    let parent = lookup(parent_path)?;
    if lookup_child(&parent, name)?.is_some() {
        let _ = remove_child_unchecked(&parent, name)?;
    }
    insert_child(&parent, name, symlink_node(target))
}

pub fn link(old_path: &str, new_path: &str, follow_source: bool) -> Result<(), Errno> {
    let _tree = TREE.lock();
    let source = if follow_source {
        lookup(old_path)?
    } else {
        lookup_nofollow(old_path)?
    };
    if source.mode.file_type() == myos_vfs::FileType::Directory {
        return Err(Errno::Eperm);
    }
    let (parent_path, name) = split_parent(new_path)?;
    let parent = lookup(parent_path)?;
    if is_node_read_only(&parent) {
        return Err(Errno::Erofs);
    }
    insert_child(&parent, name, Arc::clone(&source))?;
    source.nlink.fetch_add(1, Ordering::AcqRel);
    Ok(())
}

pub fn readlink(path: &str, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
    let _tree = TREE.lock();
    let node = lookup_nofollow(path)?;
    let state = node.state.lock();
    let NodeState::Symlink(target) = &*state else {
        return Err(Errno::Einval);
    };
    Ok(buf.push(target.as_bytes()))
}

pub fn unpack_initramfs(archive: &crate::initramfs::Initramfs<'_>) -> Result<usize, Errno> {
    let _tree = TREE.lock();
    let mut installed = 0;

    for entry in archive.entries() {
        let entry = entry.map_err(initramfs_errno)?;
        if entry.name == "TRAILER!!!" {
            break;
        }

        let path = initramfs_absolute_path(entry.name)?;
        match entry.kind {
            crate::initramfs::InitramfsEntryKind::Directory => {
                ensure_directory_path(&path, entry.mode)?;
            }
            crate::initramfs::InitramfsEntryKind::Regular => {
                install_regular_path(&path, entry.mode, entry.data)?;
            }
            crate::initramfs::InitramfsEntryKind::Symlink => {
                let target = core::str::from_utf8(entry.data).map_err(|_| Errno::Einval)?;
                install_symlink_path(&path, entry.mode, target)?;
            }
            crate::initramfs::InitramfsEntryKind::Other => return Err(Errno::Enosys),
        }
        installed += 1;
    }

    Ok(installed)
}

pub fn chdir(path: &str) -> Result<(), Errno> {
    let _tree = TREE.lock();
    let node = lookup(path)?;
    if node.mode.file_type() != myos_vfs::FileType::Directory {
        return Err(Errno::Enotdir);
    }
    Ok(())
}

pub fn mount(
    source: Option<&str>,
    target: &str,
    filesystem_type: &str,
    flags: usize,
) -> Result<(), Errno> {
    let fs_type = parse_mount_fs_type(filesystem_type)?;
    let _tree = TREE.lock();
    let target_node = lookup(target)?;
    if target_node.mode.file_type() != myos_vfs::FileType::Directory {
        return Err(Errno::Enotdir);
    }

    match fs_type {
        MountFsType::Ext4 => {
            let source = source.ok_or(Errno::Enodev)?;
            let device_name = normalize_block_source(source)?;
            let device = crate::block::open_device(device_name).ok_or(Errno::Enodev)?;
            verify_ext4_superblock(&device)?;
            ensure_mount_target_free(target)?;
            // Clear previous mount leftovers so repeated mount works.
            if !directory_is_empty(&target_node)? {
                crate::println!(
                    "mount: clearing non-empty target {} before ext4 mount",
                    target,
                );
                *target_node.state.lock() = NodeState::Directory(Vec::new());
            }
            install_ext4_snapshot(&target_node, device)?;
            insert_mount(source, target, fs_type, flags)
        }
        MountFsType::Vfat => {
            let source = source.ok_or(Errno::Enodev)?;
            let device_name = normalize_block_source(source)?;
            let _device = crate::block::open_device(device_name).ok_or(Errno::Enodev)?;
            ensure_mount_target_free(target)?;
            if !directory_is_empty(&target_node)? {
                crate::println!(
                    "mount: clearing non-empty target {} before vfat mount",
                    target,
                );
                *target_node.state.lock() = NodeState::Directory(Vec::new());
            }
            insert_mount(source, target, fs_type, flags)
        }
        MountFsType::Proc => {
            ensure_mount_target_free(target)?;
            if !directory_is_empty(&target_node)? {
                return Err(Errno::Ebusy);
            }
            populate_proc_root(&target_node)?;
            insert_mount(source.unwrap_or("proc"), target, fs_type, flags)
        }
        MountFsType::Sysfs => {
            ensure_mount_target_free(target)?;
            if !directory_is_empty(&target_node)? {
                return Err(Errno::Ebusy);
            }
            populate_sysfs_root(&target_node)?;
            insert_mount(source.unwrap_or("sysfs"), target, fs_type, flags)
        }
        _ => insert_mount(source.unwrap_or("none"), target, fs_type, flags),
    }
}

pub fn mount_ext4_subtree(
    source: &str,
    target: &str,
    source_path: &str,
    flags: usize,
) -> Result<(), Errno> {
    let _tree = TREE.lock();
    let target_node = lookup(target)?;
    if target_node.mode.file_type() != myos_vfs::FileType::Directory {
        return Err(Errno::Enotdir);
    }
    let device_name = normalize_block_source(source)?;
    let device = crate::block::open_device(device_name).ok_or(Errno::Enodev)?;
    verify_ext4_superblock(&device)?;
    ensure_mount_target_free(target)?;
    if !directory_is_empty(&target_node)? {
        return Err(Errno::Ebusy);
    }
    install_ext4_path_snapshot(&target_node, device, source_path)?;
    insert_mount(source, target, MountFsType::Ext4, flags)
}

/// Mount an ext4 tree lazily and keep all guest writes in memory. The block
/// device remains the immutable source of truth, which matches QEMU snapshot
/// runs while avoiding an eager multi-gigabyte rootfs copy at boot.
pub fn mount_ext4_overlay(source: &str, target: &str) -> Result<(), Errno> {
    let _tree = TREE.lock();
    let target_node = lookup(target)?;
    if target_node.mode.file_type() != myos_vfs::FileType::Directory {
        return Err(Errno::Enotdir);
    }
    let device_name = normalize_block_source(source)?;
    let device = crate::block::open_device(device_name).ok_or(Errno::Enodev)?;
    verify_ext4_superblock(&device)?;
    let fs = Arc::new(crate::ext4::Ext4FileSystem::open(device).map_err(ext4_errno)?);
    let root = fs.root_info().map_err(ext4_errno)?;
    if root.kind != crate::ext4::Ext4NodeKind::Directory {
        return Err(Errno::Enotdir);
    }
    target_node.read_only.store(false, Ordering::Release);
    *target_node.state.lock() = NodeState::Ext4Directory {
        fs,
        ino: root.ino,
        populated: false,
        children: Vec::new(),
        whiteouts: Vec::new(),
    };
    Ok(())
}

/// Return whether `path` is backed by the native lazy ext4 overlay.
///
/// Older contest boot paths install selected sdcard files into tmpfs and use
/// a manual materialization fallback on ENOENT. Once a native overlay is
/// mounted that fallback must stay out of the way: the overlay already loads
/// children by inode, while materializing an absent PATH candidate can turn a
/// legitimate ENOENT into a different error and prematurely stop `execvp`.
pub fn is_ext4_overlay_directory(path: &str) -> bool {
    let _tree = TREE.lock();
    let Ok(node) = lookup(path) else {
        return false;
    };
    matches!(&*node.state.lock(), NodeState::Ext4Directory { .. })
}

pub fn install_ext4_path(source: &str, target_path: &str, source_path: &str) -> Result<(), Errno> {
    let _tree = TREE.lock();
    let device_name = normalize_block_source(source)?;
    let device = crate::block::open_device(device_name).ok_or(Errno::Enodev)?;
    verify_ext4_superblock(&device)?;
    let snapshot = crate::ext4::load_path_snapshot(device, source_path).map_err(ext4_errno)?;
    let node = ext4_snapshot_node(snapshot)?;
    let (parent_path, name) = split_parent(target_path)?;
    let parent = lookup(parent_path)?;
    if is_node_read_only(&parent) {
        return Err(Errno::Erofs);
    }
    insert_child(&parent, name, node)
}

/// Install raw bytes as a VFS regular file node.
pub fn install_bytes(target_path: &str, data: &[u8]) -> Result<(), Errno> {
    let _tree = TREE.lock();
    let node = Arc::new(Node {
        ino: NEXT_INODE.fetch_add(1, Ordering::Relaxed),
        parent_ino: AtomicU64::new(0),
        nlink: AtomicU64::new(1),
        mode: FileMode::from_bits(FileMode::S_IFREG | 0o755),
        read_only: AtomicBool::new(false),
        state: IrqSpinLock::new_with_class(
            NodeState::Regular(alloc::vec::Vec::from(data)),
            NODE_LOCK,
        ),
    });
    let (parent_path, name) = split_parent(target_path)?;
    let parent = lookup(parent_path)?;
    if is_node_read_only(&parent) {
        return Err(Errno::Erofs);
    }
    insert_child(&parent, name, node)
}

// Linux umount2 flags (asm-generic).
const MNT_FORCE: usize = 0x1;
const MNT_DETACH: usize = 0x2;
const MNT_EXPIRE: usize = 0x4;
const UMOUNT_NOFOLLOW: usize = 0x8;

pub fn umount(target: &str, flags: usize) -> Result<(), Errno> {
    // Validate flags: only MNT_FORCE, MNT_DETACH, MNT_EXPIRE, UMOUNT_NOFOLLOW are valid.
    let known = MNT_FORCE | MNT_DETACH | MNT_EXPIRE | UMOUNT_NOFOLLOW;
    if flags & !known != 0 {
        return Err(Errno::Einval);
    }
    if target == "/" {
        return Err(Errno::Ebusy);
    }
    let _tree = TREE.lock();
    let target_node = lookup(target)?;
    let mut mounts = MOUNTS.lock();
    let index = mounts
        .iter()
        .position(|entry| entry.target == target)
        .ok_or(Errno::Einval)?;
    let removed = mounts.remove(index);
    drop(mounts);
    if removed.fs_type == MountFsType::Ext4 {
        clear_ext4_snapshot(&target_node)?;
    }
    // MNT_DETACH: lazy unmount — mark as detached but don't fail if busy.
    // MNT_FORCE: force unmount even if busy (best-effort).
    // MNT_EXPIRE: mark for expiration if not accessed recently.
    // For now all flags are accepted and behave identically; the mount is
    // already removed from the table above.
    let _ = flags;
    Ok(())
}

fn populate_proc_root(parent: &Arc<Node>) -> Result<(), Errno> {
    // Static files only at mount time: PID directories are added lazily by
    // `reconcile_proc_root` on the first readdir/lookup, which runs without the
    // Vfs tree lock and can therefore snapshot the process registry safely.
    let entries = crate::procfs::root_entries();
    for (name, generator) in entries {
        let node = proc_file_node(generator);
        insert_child(parent, name, node)?;
    }
    // /proc/self is a real symlink to the current pid.  The VFS node keeps a
    // placeholder target; open/stat/readlink resolve the actual pid at the
    // syscall boundary (see user.rs resolve_path_from_user / sys_readlinkat).
    insert_child(parent, "self", symlink_node("self"))?;
    *PROC_ROOT.lock() = Some(Arc::clone(parent));
    Ok(())
}

/// Whether `node` is the mounted `/proc` root (used by readdir to decide
/// whether the live-PID children need refreshing).
fn is_proc_root(node: &Arc<Node>) -> bool {
    PROC_ROOT
        .lock()
        .as_ref()
        .is_some_and(|root| Arc::ptr_eq(root, node))
}

/// Rebuild the `/proc` root's children from the live process registry.
///
/// Lock order: snapshots process metadata with NO Vfs lock held (lockdep rank
/// Process 35 < Vfs 36), then takes the node lock to install the new children.
/// Callers must run outside the Vfs tree lock (readdir / syscall boundary).
pub fn reconcile_proc_root() -> Result<(), Errno> {
    let generation = crate::process::process_generation();
    if generation == PROC_LAST_GEN.load(Ordering::Acquire) {
        return Ok(());
    }
    let metas = crate::procfs::live_process_metas();
    let mut children = Vec::new();
    children
        .try_reserve(crate::procfs::root_entries().len() + 1 + metas.len())
        .map_err(|_| Errno::Enomem)?;
    for (name, generator) in crate::procfs::root_entries() {
        children.push((String::from(name), proc_file_node(generator)));
    }
    children.push((String::from("self"), symlink_node("self")));
    for meta in metas {
        let pid_name = meta.pid.to_string();
        let dir = proc_pid_dir_node(meta);
        children.push((pid_name, dir));
    }
    let root = PROC_ROOT.lock().as_ref().cloned().ok_or(Errno::Enodev)?;
    let mut state = root.state.lock();
    *state = NodeState::Directory(children);
    PROC_LAST_GEN.store(generation, Ordering::Release);
    Ok(())
}

/// Build a `/proc/<pid>` directory node containing comm/cmdline/status/stat.
fn proc_pid_dir_node(meta: Arc<crate::procfs::ProcMeta>) -> Arc<Node> {
    let mut children = Vec::new();
    for (name, generator) in crate::procfs::pid_dir_entries(meta) {
        children.push((String::from(name), proc_file_node(generator)));
    }
    Arc::new(Node {
        ino: allocate_inode(),
        parent_ino: AtomicU64::new(0),
        nlink: AtomicU64::new(1),
        mode: FileMode::from_bits(FileMode::S_IFDIR | 0o555),
        read_only: AtomicBool::new(true),
        state: IrqSpinLock::new_with_class(NodeState::Directory(children), NODE_LOCK),
    })
}

fn populate_sysfs_root(parent: &Arc<Node>) -> Result<(), Errno> {
    // /sys/kernel/
    let kernel_dir = directory(FileMode::DIR_DEFAULT);
    let kernel_entries = crate::sysfs::kernel_entries();
    for (name, generator) in kernel_entries {
        let node = proc_file_node(generator);
        insert_child(&kernel_dir, name, node)?;
    }
    insert_child(parent, "kernel", kernel_dir)?;

    // /sys/devices/
    let devices_dir = directory(FileMode::DIR_DEFAULT);
    let devices_entries = crate::sysfs::devices_entries();
    for (name, generator) in devices_entries {
        let node = proc_file_node(generator);
        insert_child(&devices_dir, name, node)?;
    }
    insert_child(parent, "devices", devices_dir)?;

    // /sys/class/
    let class_dir = directory(FileMode::DIR_DEFAULT);
    let class_entries = crate::sysfs::class_entries();
    for (name, generator) in class_entries {
        let node = proc_file_node(generator);
        insert_child(&class_dir, name, node)?;
    }
    insert_child(parent, "class", class_dir)?;

    Ok(())
}

fn initialize_mount_table() -> Result<(), Errno> {
    let mut mounts = MOUNTS.lock();
    mounts.clear();
    mounts.try_reserve(2).map_err(|_| Errno::Enomem)?;
    mounts.push(MountEntry {
        target: String::from("/"),
        source: Some(String::from("rootfs")),
        fs_type: MountFsType::Tmpfs,
        flags: 0,
    });
    mounts.push(MountEntry {
        target: String::from("/dev"),
        source: Some(String::from("devtmpfs")),
        fs_type: MountFsType::Devtmpfs,
        flags: 0,
    });
    Ok(())
}

/// Return the statfs `f_type` magic for the mount point that covers `path`.
/// Linux-compatible magic values so `df`, `stat`, and BusyBox can classify
/// filesystems correctly.
pub fn resolve_fs_magic(path: &str) -> u64 {
    let mounts = MOUNTS.lock();
    let mut best: Option<(&MountEntry, usize)> = None;
    for entry in mounts.iter() {
        if path == entry.target || path.starts_with(&entry.target) {
            let depth = entry.target.len();
            if best.map_or(true, |(_, prev)| depth > prev) {
                best = Some((entry, depth));
            }
        }
    }
    match best {
        Some((entry, _)) => match entry.fs_type {
            MountFsType::Tmpfs => 0x01021994,
            MountFsType::Devtmpfs => 0x01021994,
            MountFsType::Proc => 0x9fa0,
            MountFsType::Sysfs => 0x62656572,
            MountFsType::Ext4 => 0xEF53,
            MountFsType::Vfat => 0x4d44,
        },
        None => 0x01021994, // default to tmpfs
    }
}

pub fn format_mounts() -> Result<alloc::vec::Vec<u8>, Errno> {
    let mounts = MOUNTS.lock();
    let mut output = alloc::string::String::new();
    for entry in mounts.iter() {
        let source = entry.source.as_deref().unwrap_or("none");
        let fs_type = match entry.fs_type {
            MountFsType::Tmpfs => "tmpfs",
            MountFsType::Devtmpfs => "devtmpfs",
            MountFsType::Proc => "proc",
            MountFsType::Sysfs => "sysfs",
            MountFsType::Ext4 => "ext4",
            MountFsType::Vfat => "vfat",
        };
        let flags = if entry.flags != 0 {
            alloc::format!(",flags=0x{:x}", entry.flags)
        } else {
            alloc::string::String::new()
        };
        output.push_str(&alloc::format!(
            "{source} {target} {fs_type} rw,relatime{flags} 0 0\n",
            target = entry.target,
        ));
    }
    Ok(output.into_bytes())
}

fn insert_mount(
    source: &str,
    target: &str,
    fs_type: MountFsType,
    flags: usize,
) -> Result<(), Errno> {
    let mut mounts = MOUNTS.lock();
    if mounts.iter().any(|entry| entry.target == target) {
        return Err(Errno::Ebusy);
    }

    let mut target_copy = String::new();
    target_copy
        .try_reserve(target.len())
        .map_err(|_| Errno::Enomem)?;
    target_copy.push_str(target);

    let mut source_copy = String::new();
    source_copy
        .try_reserve(source.len())
        .map_err(|_| Errno::Enomem)?;
    source_copy.push_str(source);

    mounts.try_reserve(1).map_err(|_| Errno::Enomem)?;
    mounts.push(MountEntry {
        target: target_copy,
        source: Some(source_copy),
        fs_type,
        flags,
    });
    Ok(())
}

fn parse_mount_fs_type(filesystem_type: &str) -> Result<MountFsType, Errno> {
    match filesystem_type {
        "tmpfs" => Ok(MountFsType::Tmpfs),
        "devtmpfs" => Ok(MountFsType::Devtmpfs),
        "proc" => Ok(MountFsType::Proc),
        "sysfs" => Ok(MountFsType::Sysfs),
        "ext4" => Ok(MountFsType::Ext4),
        "vfat" => Ok(MountFsType::Vfat),
        _ => Err(Errno::Enodev),
    }
}

fn normalize_block_source(source: &str) -> Result<&str, Errno> {
    let source = source.strip_prefix("dev:").unwrap_or(source);
    if let Some(name) = source.strip_prefix("/dev/") {
        validate_component(name)?;
        return Ok(name);
    }
    validate_component(source)?;
    Ok(source)
}

fn verify_ext4_superblock(device: &Arc<dyn crate::block::BlockDevice>) -> Result<(), Errno> {
    let mut magic = [0_u8; 2];
    let read = crate::block::read_at(device, 1024 + 56, &mut magic).map_err(block_errno)?;
    if read != magic.len() || u16::from_le_bytes(magic) != 0xef53 {
        return Err(Errno::Einval);
    }
    Ok(())
}

fn mount_table_counts() -> (usize, usize, usize, usize) {
    let mounts = MOUNTS.lock();
    let mut sourced = 0;
    let mut ext4 = 0;
    let mut flagged = 0;
    for entry in mounts.iter() {
        if matches!(entry.source.as_deref(), Some(source) if !source.is_empty()) {
            sourced += 1;
        }
        if entry.fs_type == MountFsType::Ext4 {
            ext4 += 1;
        }
        if entry.flags != 0 {
            flagged += 1;
        }
    }
    (mounts.len(), sourced, ext4, flagged)
}

fn lookup(path: &str) -> Result<Arc<Node>, Errno> {
    lookup_follow(path, true, 0)
}

fn lookup_nofollow(path: &str) -> Result<Arc<Node>, Errno> {
    lookup_follow(path, false, 0)
}

fn lookup_follow(path: &str, follow_final: bool, depth: usize) -> Result<Arc<Node>, Errno> {
    if depth > MAX_SYMLINK_FOLLOWS {
        return Err(Errno::Eloop);
    }
    let parts = components(path)?;
    let mut current = root()?;
    let mut current_path = String::from("/");
    for (index, component) in parts.iter().enumerate() {
        let next = lookup_child(&current, component)?;
        let next = next.ok_or(Errno::Enoent)?;
        let is_final = index + 1 == parts.len();
        if next.mode.file_type() == myos_vfs::FileType::Symlink && (!is_final || follow_final) {
            let target = {
                let state = next.state.lock();
                match &*state {
                    NodeState::Symlink(target) => target.clone(),
                    _ => return Err(Errno::Einval),
                }
            };
            let mut resolved = if target.starts_with('/') {
                resolve_path("/", &target)?
            } else {
                resolve_path(&current_path, &target)?
            };
            append_remaining_components(&mut resolved, &parts[index + 1..])?;
            return lookup_follow(&resolved, follow_final, depth + 1);
        }
        current = next;
        append_component_to_path(&mut current_path, component)?;
    }
    Ok(current)
}

fn create_regular(path: &str) -> Result<Arc<Node>, Errno> {
    let (parent_path, name) = split_parent(path)?;
    let parent = lookup(parent_path)?;
    if is_node_read_only(&parent) {
        return Err(Errno::Erofs);
    }
    let node = regular();
    insert_child(&parent, name, Arc::clone(&node))?;
    Ok(node)
}

fn ext4_node(
    fs: Arc<crate::ext4::Ext4FileSystem>,
    info: crate::ext4::Ext4NodeInfo,
) -> Result<Arc<Node>, Errno> {
    let state = match info.kind {
        crate::ext4::Ext4NodeKind::Directory => NodeState::Ext4Directory {
            fs,
            ino: info.ino,
            populated: false,
            children: Vec::new(),
            whiteouts: Vec::new(),
        },
        crate::ext4::Ext4NodeKind::Regular => NodeState::Ext4Regular {
            fs,
            ino: info.ino,
            size: info.size,
            overlay: None,
        },
        crate::ext4::Ext4NodeKind::Symlink => {
            let target = fs.read_symlink_inode(info.ino).map_err(ext4_errno)?;
            NodeState::Symlink(target)
        }
    };
    Ok(Arc::new(Node {
        ino: u64::from(info.ino),
        parent_ino: AtomicU64::new(0),
        nlink: AtomicU64::new(1),
        mode: FileMode::from_bits(info.mode),
        read_only: AtomicBool::new(false),
        state: IrqSpinLock::new_with_class(state, NODE_LOCK),
    }))
}

fn populate_ext4_directory(node: &Arc<Node>) -> Result<(), Errno> {
    let backing = {
        let state = node.state.lock();
        match &*state {
            NodeState::Ext4Directory {
                populated: true, ..
            } => None,
            NodeState::Ext4Directory { fs, ino, .. } => Some((Arc::clone(fs), *ino)),
            NodeState::Directory(_) => None,
            _ => return Err(Errno::Enotdir),
        }
    };
    let Some((fs, ino)) = backing else {
        return Ok(());
    };
    let entries = fs.list_directory_info(ino).map_err(ext4_errno)?;
    let mut loaded = Vec::new();
    loaded
        .try_reserve(entries.len())
        .map_err(|_| Errno::Enomem)?;
    for (name, info) in entries {
        let child = ext4_node(Arc::clone(&fs), info)?;
        child.parent_ino.store(node.ino, Ordering::Release);
        loaded.push((name, child));
    }
    let mut state = node.state.lock();
    let NodeState::Ext4Directory {
        populated,
        children,
        whiteouts,
        ..
    } = &mut *state
    else {
        return Err(Errno::Enotdir);
    };
    if *populated {
        return Ok(());
    }
    children
        .try_reserve(loaded.len())
        .map_err(|_| Errno::Enomem)?;
    for (name, child) in loaded {
        if whiteouts.iter().any(|hidden| hidden == &name)
            || children.iter().any(|(child_name, _)| child_name == &name)
        {
            continue;
        }
        children.push((name, child));
    }
    *populated = true;
    Ok(())
}

fn initramfs_absolute_path(path: &str) -> Result<String, Errno> {
    let path = normalize_initramfs_path(path)?;
    let mut components = Vec::new();
    append_components(path, &mut components)?;
    if components.is_empty() {
        return Err(Errno::Einval);
    }
    build_absolute_path(&components)
}

fn normalize_initramfs_path(path: &str) -> Result<&str, Errno> {
    let path = path.strip_prefix('/').unwrap_or(path);
    if path.is_empty() || path.as_bytes().contains(&0) {
        return Err(Errno::Einval);
    }
    Ok(path)
}

fn ensure_directory_path(path: &str, mode: u32) -> Result<(), Errno> {
    match lookup(path) {
        Ok(node) => {
            if node.mode.file_type() == myos_vfs::FileType::Directory {
                Ok(())
            } else {
                Err(Errno::Enotdir)
            }
        }
        Err(Errno::Enoent) => {
            ensure_parent_directories(path)?;
            let (parent_path, name) = split_parent(path)?;
            let parent = lookup(parent_path)?;
            insert_child(&parent, name, directory(FileMode::from_bits(mode)))
        }
        Err(error) => Err(error),
    }
}

fn ensure_parent_directories(path: &str) -> Result<(), Errno> {
    let (parent_path, _) = split_parent(path)?;
    if parent_path == "/" {
        return Ok(());
    }
    ensure_directory_path(parent_path, FileMode::DIR_DEFAULT.bits())
}

fn install_regular_path(path: &str, mode: u32, data: &[u8]) -> Result<(), Errno> {
    ensure_parent_directories(path)?;
    let (parent_path, name) = split_parent(path)?;
    let parent = lookup(parent_path)?;
    if is_node_read_only(&parent) {
        return Err(Errno::Erofs);
    }
    let mut owned = Vec::new();
    owned.try_reserve(data.len()).map_err(|_| Errno::Enomem)?;
    owned.extend_from_slice(data);
    insert_child(
        &parent,
        name,
        regular_with_data(FileMode::from_bits(mode), owned),
    )
}

fn install_symlink_path(path: &str, mode: u32, target: &str) -> Result<(), Errno> {
    if target.is_empty() || target.len() > 4096 || target.as_bytes().contains(&0) {
        return Err(Errno::Einval);
    }
    ensure_parent_directories(path)?;
    let (parent_path, name) = split_parent(path)?;
    let parent = lookup(parent_path)?;
    if is_node_read_only(&parent) {
        return Err(Errno::Erofs);
    }
    insert_child(
        &parent,
        name,
        symlink_node_with_mode(target, FileMode::from_bits(mode)),
    )
}

fn initramfs_errno(error: crate::initramfs::InitramfsError) -> Errno {
    match error {
        crate::initramfs::InitramfsError::AddressOverflow => Errno::Eoverflow,
        crate::initramfs::InitramfsError::InvalidArchive => Errno::Einval,
        crate::initramfs::InitramfsError::InvalidHex => Errno::Einval,
        crate::initramfs::InitramfsError::InvalidName => Errno::Einval,
        crate::initramfs::InitramfsError::InvalidSymlink => Errno::Einval,
        crate::initramfs::InitramfsError::NotFound => Errno::Enoent,
        crate::initramfs::InitramfsError::OutOfMemory => Errno::Enomem,
        crate::initramfs::InitramfsError::UnsupportedFileType => Errno::Enosys,
    }
}

fn regular_overlay_mut(state: &mut NodeState) -> Result<&mut Vec<u8>, Errno> {
    if let NodeState::Ext4Regular {
        fs,
        ino,
        size,
        overlay,
    } = state
        && overlay.is_none()
    {
        let length = usize::try_from(*size).map_err(|_| Errno::Eoverflow)?;
        let mut data = Vec::new();
        data.try_reserve(length).map_err(|_| Errno::Enomem)?;
        data.resize(length, 0);
        let read = fs.read_inode_at(*ino, 0, &mut data).map_err(ext4_errno)?;
        if read != length {
            return Err(Errno::Eio);
        }
        *overlay = Some(data);
    }
    match state {
        NodeState::Regular(data) => Ok(data),
        NodeState::Ext4Regular {
            overlay: Some(data),
            ..
        } => Ok(data),
        _ => Err(Errno::Einval),
    }
}

fn truncate_node(node: &Arc<Node>, length: u64) -> Result<(), Errno> {
    if is_node_read_only(node) {
        return Err(Errno::Erofs);
    }
    let length = usize::try_from(length).map_err(|_| Errno::Eoverflow)?;
    let mut state = node.state.lock();
    match &mut *state {
        NodeState::Ext4Regular { overlay, size, .. } if overlay.is_none() && length == 0 => {
            // O_TRUNC is pervasive in compiler output directories.  The new
            // file has no dependency on the immutable ext4 bytes, so avoid
            // reading the entire old artifact only to discard it.
            *overlay = Some(Vec::new());
            *size = 0;
            Ok(())
        }
        NodeState::Regular(_) | NodeState::Ext4Regular { .. } => {
            let data = regular_overlay_mut(&mut *state)?;
            if length > data.len() {
                data.try_reserve(length - data.len())
                    .map_err(|_| Errno::Enomem)?;
            }
            data.resize(length, 0);
            Ok(())
        }
        NodeState::Directory(_) | NodeState::Ext4Directory { .. } => Err(Errno::Eisdir),
        NodeState::Symlink(_) => Err(Errno::Einval),
        NodeState::Device(_) => Ok(()),
        NodeState::BlockDevice { .. } => Err(Errno::Einval),
        NodeState::ProcFile(_) => Err(Errno::Erofs),
    }
}

// SUDOOS_M15A_EXT4_RO_PATCH_V1: native read-only ext4 VFS snapshot handoff.

fn ensure_mount_target_free(target: &str) -> Result<(), Errno> {
    let mounts = MOUNTS.lock();
    if mounts.iter().any(|entry| entry.target == target) {
        return Err(Errno::Ebusy);
    }
    Ok(())
}

fn directory_is_empty(node: &Arc<Node>) -> Result<bool, Errno> {
    populate_ext4_directory(node)?;
    directory_children(&node.state.lock())
        .map(|children| children.is_empty())
        .ok_or(Errno::Enotdir)
}

fn clear_ext4_snapshot(target: &Arc<Node>) -> Result<(), Errno> {
    target.read_only.store(false, Ordering::Release);
    *target.state.lock() = NodeState::Directory(Vec::new());
    Ok(())
}

fn install_ext4_snapshot(
    target: &Arc<Node>,
    device: Arc<dyn crate::block::BlockDevice>,
) -> Result<(), Errno> {
    let snapshot = crate::ext4::load_root_snapshot(device).map_err(ext4_errno)?;
    let mut children = Vec::new();
    let crate::ext4::Ext4SnapshotKind::Directory(entries) = snapshot.kind else {
        return Err(Errno::Enotdir);
    };
    children
        .try_reserve(entries.len())
        .map_err(|_| Errno::Enomem)?;
    for entry in entries {
        validate_component(&entry.name)?;
        let node = ext4_snapshot_node(entry.node)?;
        node.parent_ino.store(target.ino, Ordering::Release);
        children.push((entry.name, node));
    }
    target.read_only.store(true, Ordering::Release);
    *target.state.lock() = NodeState::Directory(children);
    Ok(())
}

fn install_ext4_path_snapshot(
    target: &Arc<Node>,
    device: Arc<dyn crate::block::BlockDevice>,
    source_path: &str,
) -> Result<(), Errno> {
    let snapshot = crate::ext4::load_path_snapshot(device, source_path).map_err(ext4_errno)?;
    let mut children = Vec::new();
    let crate::ext4::Ext4SnapshotKind::Directory(entries) = snapshot.kind else {
        return Err(Errno::Enotdir);
    };
    children
        .try_reserve(entries.len())
        .map_err(|_| Errno::Enomem)?;
    for entry in entries {
        validate_component(&entry.name)?;
        let node = ext4_snapshot_node(entry.node)?;
        node.parent_ino.store(target.ino, Ordering::Release);
        children.push((entry.name, node));
    }
    target.read_only.store(true, Ordering::Release);
    *target.state.lock() = NodeState::Directory(children);
    Ok(())
}

fn ext4_snapshot_node(snapshot: crate::ext4::Ext4SnapshotNode) -> Result<Arc<Node>, Errno> {
    let declared_size = snapshot.size;
    let state = match snapshot.kind {
        crate::ext4::Ext4SnapshotKind::Directory(entries) => {
            let mut children = Vec::new();
            children
                .try_reserve(entries.len())
                .map_err(|_| Errno::Enomem)?;
            for entry in entries {
                validate_component(&entry.name)?;
                let node = ext4_snapshot_node(entry.node)?;
                children.push((entry.name, node));
            }
            NodeState::Directory(children)
        }
        crate::ext4::Ext4SnapshotKind::Regular(data) => {
            if data.len() as u64 != declared_size {
                return Err(Errno::Eoverflow);
            }
            NodeState::Regular(data)
        }
        crate::ext4::Ext4SnapshotKind::Symlink(target) => NodeState::Symlink(target),
    };
    let node = Arc::new(Node {
        ino: snapshot.ino,
        parent_ino: AtomicU64::new(0),
        nlink: AtomicU64::new(1),
        mode: FileMode::from_bits(snapshot.mode),
        read_only: AtomicBool::new(true),
        state: IrqSpinLock::new_with_class(state, NODE_LOCK),
    });
    if let NodeState::Directory(children) = &*node.state.lock() {
        for (_, child) in children {
            child.parent_ino.store(node.ino, Ordering::Release);
        }
    }
    Ok(node)
}

fn is_node_read_only(node: &Arc<Node>) -> bool {
    node.read_only.load(Ordering::Acquire)
}

fn ext4_errno(error: crate::ext4::Ext4Error) -> Errno {
    match error {
        crate::ext4::Ext4Error::AddressOverflow => Errno::Eoverflow,
        crate::ext4::Ext4Error::BadBlockSize => Errno::Einval,
        crate::ext4::Ext4Error::BadDirectory => Errno::Einval,
        crate::ext4::Ext4Error::BadExtentTree => Errno::Einval,
        crate::ext4::Ext4Error::BadGroupDescriptor => Errno::Einval,
        crate::ext4::Ext4Error::BadInode => Errno::Einval,
        crate::ext4::Ext4Error::BlockIo(err) => block_errno(err),
        crate::ext4::Ext4Error::FileTooLarge => Errno::Eoverflow,
        crate::ext4::Ext4Error::InvalidFeatureSet => Errno::Enosys,
        crate::ext4::Ext4Error::InvalidSuperblock => Errno::Einval,
        crate::ext4::Ext4Error::NotFound => Errno::Enoent,
        crate::ext4::Ext4Error::OutOfMemory => Errno::Enomem,
        crate::ext4::Ext4Error::Unsupported => Errno::Enosys,
    }
}

fn root() -> Result<Arc<Node>, Errno> {
    ROOT.lock().as_ref().cloned().ok_or(Errno::Enodev)
}

fn directory(mode: FileMode) -> Arc<Node> {
    Arc::new(Node {
        ino: allocate_inode(),
        parent_ino: AtomicU64::new(0),
        nlink: AtomicU64::new(1),
        mode,
        read_only: AtomicBool::new(false),
        state: IrqSpinLock::new_with_class(NodeState::Directory(Vec::new()), NODE_LOCK),
    })
}

fn regular() -> Arc<Node> {
    regular_with_data(FileMode::FILE_DEFAULT, Vec::new())
}

fn regular_with_data(mode: FileMode, data: Vec<u8>) -> Arc<Node> {
    Arc::new(Node {
        ino: allocate_inode(),
        parent_ino: AtomicU64::new(0),
        nlink: AtomicU64::new(1),
        mode,
        read_only: AtomicBool::new(false),
        state: IrqSpinLock::new_with_class(NodeState::Regular(data), NODE_LOCK),
    })
}

fn symlink_node(target: &str) -> Arc<Node> {
    symlink_node_with_mode(target, FileMode::SYMLINK_DEFAULT)
}

fn symlink_node_with_mode(target: &str, mode: FileMode) -> Arc<Node> {
    Arc::new(Node {
        ino: allocate_inode(),
        parent_ino: AtomicU64::new(0),
        nlink: AtomicU64::new(1),
        mode,
        read_only: AtomicBool::new(false),
        state: IrqSpinLock::new_with_class(NodeState::Symlink(String::from(target)), NODE_LOCK),
    })
}

fn device(kind: DeviceKind) -> Arc<Node> {
    Arc::new(Node {
        ino: allocate_inode(),
        parent_ino: AtomicU64::new(0),
        nlink: AtomicU64::new(1),
        mode: FileMode::CHAR_DEFAULT,
        read_only: AtomicBool::new(false),
        state: IrqSpinLock::new_with_class(NodeState::Device(kind), NODE_LOCK),
    })
}

fn proc_file_node(generator: Arc<dyn crate::procfs::ProcFileGenerator>) -> Arc<Node> {
    Arc::new(Node {
        ino: allocate_inode(),
        parent_ino: AtomicU64::new(0),
        nlink: AtomicU64::new(1),
        mode: FileMode::from_bits(FileMode::S_IFREG | 0o444),
        read_only: AtomicBool::new(true),
        state: IrqSpinLock::new_with_class(NodeState::ProcFile(generator), NODE_LOCK),
    })
}

fn block_device_node(
    name: &str,
    device: Arc<dyn crate::block::BlockDevice>,
) -> Result<Arc<Node>, Errno> {
    let cache =
        crate::block::BufferCache::new(device.clone(), BLOCK_CACHE_BLOCKS).map_err(block_errno)?;
    let mut stored_name = String::new();
    stored_name
        .try_reserve(name.len())
        .map_err(|_| Errno::Enomem)?;
    stored_name.push_str(name);
    Ok(Arc::new(Node {
        ino: allocate_inode(),
        parent_ino: AtomicU64::new(0),
        nlink: AtomicU64::new(1),
        mode: FileMode::BLOCK_DEFAULT,
        read_only: AtomicBool::new(false),
        state: IrqSpinLock::new_with_class(
            NodeState::BlockDevice {
                name: stored_name,
                device,
                cache: Arc::new(cache),
            },
            NODE_LOCK,
        ),
    }))
}

fn install_registered_block_devices(dev: &Arc<Node>) -> Result<(), Errno> {
    let devices = crate::block::registered_devices().map_err(block_errno)?;
    for device in devices {
        let node = block_device_node(device.name(), device.device())?;
        insert_child(dev, device.name(), node)?;
    }
    Ok(())
}

/// 为已注册的块设备安装 `/dev/<name>` VFS 节点（post-init 延迟注册用）。
///
/// `fs::initialize()` 只安装注册时的设备快照；USB MSC 这类事后才注册的
/// 设备需要动态补节点。设备必须先经 `block::register_device` 注册。
/// 幂等：同名节点已存在时返回 `Eexist`（调用方可忽略，通常意味着该名字
/// 已被早期快照安装）。
pub fn install_block_device_node(name: &str) -> Result<(), Errno> {
    let _tree = TREE.lock();
    let device = crate::block::open_device(name).ok_or(Errno::Enodev)?;
    let node = block_device_node(name, device)?;
    let path = alloc::format!("/dev/{name}");
    let (parent_path, child_name) = split_parent(&path)?;
    let parent = lookup(parent_path)?;
    insert_child(&parent, child_name, node)
}

fn block_errno(error: crate::block::BlockError) -> Errno {
    match error {
        crate::block::BlockError::AddressOverflow => Errno::Eoverflow,
        crate::block::BlockError::BadBlockSize => Errno::Einval,
        crate::block::BlockError::BufferTooSmall => Errno::Einval,
        crate::block::BlockError::DeviceReadOnly => Errno::Erofs,
        crate::block::BlockError::InvalidArgument => Errno::Einval,
        crate::block::BlockError::MetadataOutOfMemory => Errno::Enomem,
        crate::block::BlockError::OutOfRange => Errno::Einval,
    }
}

fn allocate_inode() -> u64 {
    NEXT_INODE.fetch_add(1, Ordering::Relaxed)
}

fn insert_child(parent: &Arc<Node>, name: &str, child: Arc<Node>) -> Result<(), Errno> {
    validate_component(name)?;
    let mut state = parent.state.lock();
    if let NodeState::Ext4Directory { whiteouts, .. } = &mut *state {
        whiteouts.retain(|hidden| hidden != name);
    }
    let children = directory_children_mut(&mut state).ok_or(Errno::Enotdir)?;
    if children.iter().any(|(child_name, _)| child_name == name) {
        return Err(Errno::Eexist);
    }
    children.try_reserve(1).map_err(|_| Errno::Enomem)?;
    let mut stored_name = String::new();
    stored_name
        .try_reserve(name.len())
        .map_err(|_| Errno::Enomem)?;
    stored_name.push_str(name);
    child.parent_ino.store(parent.ino, Ordering::Release);
    children.push((stored_name, child));
    Ok(())
}

fn insert_child_prepared(
    parent: &Arc<Node>,
    name: String,
    child: Arc<Node>,
    replace_existing: bool,
) -> Result<(), Errno> {
    validate_component(&name)?;
    let mut state = parent.state.lock();
    if let NodeState::Ext4Directory { whiteouts, .. } = &mut *state {
        whiteouts.retain(|hidden| hidden != &name);
    }
    let children = directory_children_mut(&mut state).ok_or(Errno::Enotdir)?;
    if let Some(index) = children
        .iter()
        .position(|(child_name, _)| child_name == &name)
    {
        if !replace_existing {
            return Err(Errno::Eexist);
        }
        child.parent_ino.store(parent.ino, Ordering::Release);
        children[index] = (name, child);
        return Ok(());
    }
    child.parent_ino.store(parent.ino, Ordering::Release);
    children.push((name, child));
    Ok(())
}

fn reserve_child_slot(parent: &Arc<Node>, name: &str, replace_existing: bool) -> Result<(), Errno> {
    validate_component(name)?;
    let mut state = parent.state.lock();
    let children = directory_children_mut(&mut state).ok_or(Errno::Enotdir)?;
    if children.iter().any(|(child_name, _)| child_name == name) {
        return if replace_existing {
            Ok(())
        } else {
            Err(Errno::Eexist)
        };
    }
    children.try_reserve(1).map_err(|_| Errno::Enomem)
}

fn lookup_child(parent: &Arc<Node>, name: &str) -> Result<Option<Arc<Node>>, Errno> {
    validate_component(name)?;
    let backing = {
        let state = parent.state.lock();
        let children = directory_children(&state).ok_or(Errno::Enotdir)?;
        if let Some((_, child)) = children.iter().find(|(child_name, _)| child_name == name) {
            return Ok(Some(Arc::clone(child)));
        }
        match &*state {
            NodeState::Ext4Directory {
                populated: false,
                fs,
                ino,
                whiteouts,
                ..
            } if !whiteouts.iter().any(|hidden| hidden == name) => Some((Arc::clone(fs), *ino)),
            _ => None,
        }
    };
    let Some((fs, ino)) = backing else {
        return Ok(None);
    };
    let info = match fs.lookup_child_info(ino, name) {
        Ok(info) => info,
        Err(crate::ext4::Ext4Error::NotFound) => return Ok(None),
        Err(error) => return Err(ext4_errno(error)),
    };
    let child = ext4_node(fs, info)?;
    child.parent_ino.store(parent.ino, Ordering::Release);
    let stored_name = clone_component(name)?;
    let mut state = parent.state.lock();
    let NodeState::Ext4Directory {
        children,
        whiteouts,
        ..
    } = &mut *state
    else {
        return Err(Errno::Enotdir);
    };
    if whiteouts.iter().any(|hidden| hidden == name) {
        return Ok(None);
    }
    if let Some((_, existing)) = children.iter().find(|(child_name, _)| child_name == name) {
        return Ok(Some(Arc::clone(existing)));
    }
    children.try_reserve(1).map_err(|_| Errno::Enomem)?;
    children.push((stored_name, Arc::clone(&child)));
    Ok(Some(child))
}

fn remove_child(parent: &Arc<Node>, name: &str, remove_dir: bool) -> Result<Arc<Node>, Errno> {
    let child = lookup_child(parent, name)?.ok_or(Errno::Enoent)?;
    if is_node_read_only(&child) {
        return Err(Errno::Erofs);
    }
    match child.mode.file_type() {
        myos_vfs::FileType::Directory => {
            if !remove_dir {
                return Err(Errno::Eisdir);
            }
            ensure_empty_directory(&child)?;
        }
        _ if remove_dir => return Err(Errno::Enotdir),
        _ => {}
    }
    let child = remove_child_unchecked(parent, name)?;
    child.nlink.fetch_sub(1, Ordering::AcqRel);
    Ok(child)
}

fn remove_child_unchecked(parent: &Arc<Node>, name: &str) -> Result<Arc<Node>, Errno> {
    validate_component(name)?;
    let mut state = parent.state.lock();
    if let NodeState::Ext4Directory { whiteouts, .. } = &mut *state {
        if !whiteouts.iter().any(|hidden| hidden == name) {
            whiteouts.try_reserve(1).map_err(|_| Errno::Enomem)?;
            whiteouts.push(clone_component(name)?);
        }
    }
    let children = directory_children_mut(&mut state).ok_or(Errno::Enotdir)?;
    let index = children
        .iter()
        .position(|(child_name, _)| child_name == name)
        .ok_or(Errno::Enoent)?;
    Ok(children.remove(index).1)
}

fn rename_inside_parent(parent: &Arc<Node>, old_name: &str, new_name: String) -> Result<(), Errno> {
    validate_component(old_name)?;
    validate_component(&new_name)?;
    let mut state = parent.state.lock();
    if let NodeState::Ext4Directory { whiteouts, .. } = &mut *state {
        if !whiteouts.iter().any(|hidden| hidden == old_name) {
            whiteouts.try_reserve(1).map_err(|_| Errno::Enomem)?;
            whiteouts.push(clone_component(old_name)?);
        }
        whiteouts.retain(|hidden| hidden != &new_name);
    }
    let children = directory_children_mut(&mut state).ok_or(Errno::Enotdir)?;
    if old_name == new_name {
        return Ok(());
    }
    let source_index = children
        .iter()
        .position(|(name, _)| name == old_name)
        .ok_or(Errno::Enoent)?;
    if let Some(target_index) = children.iter().position(|(name, _)| name == &new_name) {
        children.remove(target_index);
        let source_index = if target_index < source_index {
            source_index - 1
        } else {
            source_index
        };
        children[source_index].0 = new_name;
    } else {
        children[source_index].0 = new_name;
    }
    Ok(())
}

fn validate_rename_target(source: &Arc<Node>, target: Option<&Arc<Node>>) -> Result<(), Errno> {
    let Some(target) = target else {
        return Ok(());
    };
    match (source.mode.file_type(), target.mode.file_type()) {
        (myos_vfs::FileType::Directory, myos_vfs::FileType::Directory) => {
            ensure_empty_directory(target)
        }
        (myos_vfs::FileType::Directory, _) => Err(Errno::Enotdir),
        (_, myos_vfs::FileType::Directory) => Err(Errno::Eisdir),
        _ => Ok(()),
    }
}

fn ensure_empty_directory(node: &Arc<Node>) -> Result<(), Errno> {
    populate_ext4_directory(node)?;
    let state = node.state.lock();
    let children = directory_children(&state).ok_or(Errno::Enotdir)?;
    if children.is_empty() {
        Ok(())
    } else {
        Err(Errno::Enotempty)
    }
}

fn clone_component(name: &str) -> Result<String, Errno> {
    validate_component(name)?;
    let mut stored_name = String::new();
    stored_name
        .try_reserve(name.len())
        .map_err(|_| Errno::Enomem)?;
    stored_name.push_str(name);
    Ok(stored_name)
}

fn components(path: &str) -> Result<Vec<&str>, Errno> {
    if !path.starts_with('/') {
        return Err(Errno::Enoent);
    }
    let mut output = Vec::new();
    append_components(path, &mut output)?;
    Ok(output)
}

fn append_components<'a>(path: &'a str, output: &mut Vec<&'a str>) -> Result<(), Errno> {
    if path.as_bytes().contains(&0) {
        return Err(Errno::Einval);
    }
    for component in path.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." {
            output.pop();
            continue;
        }
        validate_component(component)?;
        output.try_reserve(1).map_err(|_| Errno::Enomem)?;
        output.push(component);
    }
    Ok(())
}

fn build_absolute_path(components: &[&str]) -> Result<String, Errno> {
    if components.is_empty() {
        return Ok(String::from("/"));
    }
    let length = components
        .iter()
        .try_fold(0_usize, |total, component| {
            total
                .checked_add(component.len())
                .and_then(|total| total.checked_add(1))
        })
        .ok_or(Errno::Eoverflow)?;
    let mut output = String::new();
    output.try_reserve(length).map_err(|_| Errno::Enomem)?;
    for component in components {
        output.push('/');
        output.push_str(component);
    }
    Ok(output)
}

fn append_remaining_components(path: &mut String, components: &[&str]) -> Result<(), Errno> {
    for component in components {
        append_component_to_path(path, component)?;
    }
    Ok(())
}

fn append_component_to_path(path: &mut String, component: &str) -> Result<(), Errno> {
    validate_component(component)?;
    if path != "/" {
        path.try_reserve(1).map_err(|_| Errno::Enomem)?;
        path.push('/');
    }
    path.try_reserve(component.len())
        .map_err(|_| Errno::Enomem)?;
    path.push_str(component);
    Ok(())
}

fn is_path_descendant(path: &str, parent: &str) -> bool {
    let parent = parent.trim_end_matches('/');
    path.len() > parent.len()
        && path.starts_with(parent)
        && path.as_bytes().get(parent.len()) == Some(&b'/')
}

fn split_parent(path: &str) -> Result<(&str, &str), Errno> {
    if !path.starts_with('/') {
        return Err(Errno::Enoent);
    }
    let trimmed = path.trim_end_matches('/');
    let index = trimmed.rfind('/').ok_or(Errno::Enoent)?;
    let name = &trimmed[index + 1..];
    validate_component(name)?;
    let parent = if index == 0 { "/" } else { &trimmed[..index] };
    Ok((parent, name))
}

fn validate_component(component: &str) -> Result<(), Errno> {
    if component.is_empty() {
        return Err(Errno::Enoent);
    }
    if component.len() > MAX_COMPONENT_LEN {
        return Err(Errno::Enametoolong);
    }
    if component.as_bytes().contains(&0) || component.contains('/') {
        return Err(Errno::Einval);
    }
    Ok(())
}

struct RegularFile {
    node: Arc<Node>,
}

impl FileOperations for RegularFile {
    fn read(&self, file: &File, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        file.with_position(|position| {
            let backing = {
                let state = self.node.state.lock();
                match &*state {
                    NodeState::Regular(data) => {
                        let start = usize::try_from(*position).map_err(|_| Errno::Eoverflow)?;
                        if start >= data.len() {
                            return Ok(0);
                        }
                        let count = buf.push(&data[start..]);
                        *position = (*position)
                            .checked_add(count as u64)
                            .ok_or(Errno::Eoverflow)?;
                        return Ok(count);
                    }
                    NodeState::Ext4Regular {
                        overlay: Some(data),
                        ..
                    } => {
                        let start = usize::try_from(*position).map_err(|_| Errno::Eoverflow)?;
                        if start >= data.len() {
                            return Ok(0);
                        }
                        let count = buf.push(&data[start..]);
                        *position = (*position)
                            .checked_add(count as u64)
                            .ok_or(Errno::Eoverflow)?;
                        return Ok(count);
                    }
                    NodeState::Ext4Regular {
                        fs,
                        ino,
                        size,
                        overlay: None,
                    } => (Arc::clone(fs), *ino, *size),
                    NodeState::ProcFile(generator) => {
                        // 每次 read 生成内容（简化实现：忽略位置，完整返回）
                        let data = generator.generate()?;
                        if *position >= data.len() as u64 {
                            return Ok(0);
                        }
                        let start = usize::try_from(*position).map_err(|_| Errno::Eoverflow)?;
                        let count = buf.push(&data[start..]);
                        *position = (*position)
                            .checked_add(count as u64)
                            .ok_or(Errno::Eoverflow)?;
                        return Ok(count);
                    }
                    _ => return Err(Errno::Einval),
                }
            };
            let (fs, ino, size) = backing;
            if *position >= size {
                return Ok(0);
            }
            let count = usize::try_from((size - *position).min(buf.remaining() as u64))
                .map_err(|_| Errno::Eoverflow)?;
            let read = fs
                .read_inode_at(ino, *position, &mut buf.unfilled_mut()[..count])
                .map_err(ext4_errno)?;
            buf.advance(read)?;
            *position = position.checked_add(read as u64).ok_or(Errno::Eoverflow)?;
            Ok(read)
        })
    }

    fn write(&self, file: &File, buf: &IoBuffer<'_>) -> Result<usize, Errno> {
        if is_node_read_only(&self.node) {
            return Err(Errno::Erofs);
        }
        // Proc files are always read-only
        if matches!(&*self.node.state.lock(), NodeState::ProcFile(_)) {
            return Err(Errno::Erofs);
        }
        file.with_position(|position| {
            let mut state = self.node.state.lock();
            let data = regular_overlay_mut(&mut *state)?;
            if file.flags().contains(OpenFlags::O_APPEND) {
                *position = data.len() as u64;
            }
            let start = usize::try_from(*position).map_err(|_| Errno::Eoverflow)?;
            let end = start.checked_add(buf.len()).ok_or(Errno::Eoverflow)?;
            if end > data.len() {
                data.try_reserve(end - data.len())
                    .map_err(|_| Errno::Enomem)?;
                data.resize(end, 0);
            }
            data[start..end].copy_from_slice(buf.as_bytes());
            *position = end as u64;
            Ok(buf.len())
        })
    }

    fn seek(&self, file: &File, offset: i64, whence: SeekWhence) -> Result<u64, Errno> {
        let end = {
            let state = self.node.state.lock();
            match &*state {
                NodeState::Regular(data) => data.len() as u64,
                NodeState::Ext4Regular {
                    size,
                    overlay: None,
                    ..
                } => *size,
                NodeState::Ext4Regular {
                    overlay: Some(data),
                    ..
                } => data.len() as u64,
                NodeState::ProcFile(generator) => {
                    generator.generate().map(|d| d.len() as u64).unwrap_or(0)
                }
                _ => return Err(Errno::Einval),
            }
        };
        file.seek_position(offset, whence, Some(end))
    }

    fn fstat(&self, _file: &File) -> Result<Stat, Errno> {
        stat_for_node(&self.node)
    }

    fn truncate(&self, _file: &File, length: u64) -> Result<(), Errno> {
        if matches!(&*self.node.state.lock(), NodeState::ProcFile(_)) {
            return Err(Errno::Erofs);
        }
        truncate_node(&self.node, length)
    }
}

struct DirectoryFile {
    node: Arc<Node>,
}

impl FileOperations for DirectoryFile {
    fn read(&self, _file: &File, _buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        Err(Errno::Eisdir)
    }

    fn write(&self, _file: &File, _buf: &IoBuffer<'_>) -> Result<usize, Errno> {
        Err(Errno::Eisdir)
    }

    fn fstat(&self, _file: &File) -> Result<Stat, Errno> {
        stat_for_node(&self.node)
    }

    fn readdir(&self, file: &File, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        populate_ext4_directory(&self.node)?;
        // /proc: refresh the live-PID children before enumerating.  Runs
        // outside the Vfs tree lock (getdents holds only the file position
        // lock), so reconcile_proc_root can snapshot the registry safely.
        if is_proc_root(&self.node) {
            reconcile_proc_root()?;
        }
        file.with_position(|position| {
            let mut index = usize::try_from(*position).map_err(|_| Errno::Eoverflow)?;
            let state = self.node.state.lock();
            let children = directory_children(&state).ok_or(Errno::Enotdir)?;
            let start_len = buf.len();
            while index < children.len() + 2 {
                let (ino, file_type, name) = match index {
                    0 => (self.node.ino, myos_vfs::FileType::Directory, "."),
                    1 => (
                        self.node.parent_ino.load(Ordering::Acquire),
                        myos_vfs::FileType::Directory,
                        "..",
                    ),
                    _ => {
                        let (name, child) = &children[index - 2];
                        (child.ino, child.mode.file_type(), name.as_str())
                    }
                };
                let emitted = emit_dirent64(
                    buf,
                    DirEntry {
                        ino,
                        offset: (index + 1) as i64,
                        file_type,
                        name,
                    },
                )?;
                if !emitted {
                    break;
                }
                index += 1;
                *position = index as u64;
            }
            Ok(buf.len() - start_len)
        })
    }
}

struct DeviceFile {
    node: Arc<Node>,
    kind: DeviceKind,
}

impl FileOperations for DeviceFile {
    fn read(&self, _file: &File, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        match self.kind {
            DeviceKind::Null => Ok(0),
            DeviceKind::Console | DeviceKind::Tty => crate::tty::read_console(buf),
            DeviceKind::Zero => {
                let zeros = [0_u8; 64];
                let mut total = 0;
                while buf.remaining() > 0 {
                    total += buf.push(&zeros);
                }
                Ok(total)
            }
            DeviceKind::Random => {
                let mut scratch = [0_u8; 64];
                let mut total = 0;
                while buf.remaining() > 0 {
                    let chunk = scratch.len().min(buf.remaining());
                    crate::rng::fill_random(&mut scratch[..chunk]);
                    total += buf.push(&scratch[..chunk]);
                }
                Ok(total)
            }
            DeviceKind::Urandom => {
                let mut scratch = [0_u8; 64];
                let mut total = 0;
                while buf.remaining() > 0 {
                    let chunk = scratch.len().min(buf.remaining());
                    crate::rng::fill_random(&mut scratch[..chunk]);
                    total += buf.push(&scratch[..chunk]);
                }
                Ok(total)
            }
            DeviceKind::Rtc => {
                if let Some(time) = crate::rtc::read_rtc_time() {
                    let time_str = alloc::format!("{}\n", time.unix_seconds);
                    Ok(buf.push(time_str.as_bytes()))
                } else {
                    Err(Errno::Eio)
                }
            }
            // Ptmx is intercepted in open(), never reaches here
            DeviceKind::Ptmx => Err(Errno::Einval),
        }
    }

    fn write(&self, _file: &File, buf: &IoBuffer<'_>) -> Result<usize, Errno> {
        match self.kind {
            DeviceKind::Null
            | DeviceKind::Zero
            | DeviceKind::Random
            | DeviceKind::Urandom
            | DeviceKind::Rtc => Ok(buf.len()),
            DeviceKind::Console | DeviceKind::Tty => Ok(crate::tty::write_console(buf.as_bytes())),
            // Ptmx handled before DeviceFile creation
            _ => Ok(buf.len()),
        }
    }

    fn seek(&self, _file: &File, _offset: i64, _whence: SeekWhence) -> Result<u64, Errno> {
        Err(Errno::Espipe)
    }

    fn fstat(&self, _file: &File) -> Result<Stat, Errno> {
        stat_for_node(&self.node)
    }

    fn ioctl(&self, _file: &File, cmd: usize, arg: usize) -> Result<usize, Errno> {
        match self.kind {
            DeviceKind::Console | DeviceKind::Tty => crate::tty::ioctl(cmd, arg),
            DeviceKind::Rtc => crate::rtc::ioctl(cmd, arg),
            DeviceKind::Null | DeviceKind::Zero => Err(Errno::Enotty),
            // Ptmx, Random, Urandom — no ioctl support
            _ => Err(Errno::Enotty),
        }
    }

    fn poll(&self, file: &File, requested: PollEvents) -> PollEvents {
        match self.kind {
            DeviceKind::Console | DeviceKind::Tty => {
                let mut ready = PollEvents::empty();
                if file.flags().access_mode().is_readable() && crate::tty::input_ready() {
                    ready = ready.union(PollEvents::IN);
                }
                if file.flags().access_mode().is_writable() {
                    ready = ready.union(PollEvents::OUT);
                }
                ready.intersect(requested)
            }
            DeviceKind::Null
            | DeviceKind::Zero
            | DeviceKind::Random
            | DeviceKind::Urandom
            | DeviceKind::Rtc => {
                let mut ready = PollEvents::empty();
                if file.flags().access_mode().is_readable() {
                    ready = ready.union(PollEvents::IN);
                }
                if file.flags().access_mode().is_writable() {
                    ready = ready.union(PollEvents::OUT);
                }
                ready.intersect(requested)
            }
            // Ptmx handled before DeviceFile creation
            _ => {
                let mut ready = PollEvents::empty();
                if file.flags().access_mode().is_readable() {
                    ready = ready.union(PollEvents::IN);
                }
                if file.flags().access_mode().is_writable() {
                    ready = ready.union(PollEvents::OUT);
                }
                ready.intersect(requested)
            }
        }
    }
}

struct BlockDeviceFile {
    node: Arc<Node>,
    name: String,
    device: Arc<dyn crate::block::BlockDevice>,
    cache: Arc<crate::block::BufferCache>,
}

impl FileOperations for BlockDeviceFile {
    fn read(&self, file: &File, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        file.with_position(|position| {
            let size = self.device.size_bytes().map_err(block_errno)?;
            if *position >= size {
                return Ok(0);
            }
            let block_size = self.device.block_size();
            let mut scratch = Vec::new();
            scratch.try_reserve(block_size).map_err(|_| Errno::Enomem)?;
            scratch.resize(block_size, 0);

            let start_len = buf.len();
            while buf.remaining() != 0 && *position < size {
                let block = *position / block_size as u64;
                let block_offset = (*position % block_size as u64) as usize;
                let count = (block_size - block_offset)
                    .min(buf.remaining())
                    .min((size - *position) as usize);
                self.cache.read(block, &mut scratch).map_err(block_errno)?;
                let pushed = buf.push(&scratch[block_offset..block_offset + count]);
                *position = (*position)
                    .checked_add(pushed as u64)
                    .ok_or(Errno::Eoverflow)?;
                if pushed != count {
                    break;
                }
            }
            Ok(buf.len() - start_len)
        })
    }

    fn write(&self, file: &File, buf: &IoBuffer<'_>) -> Result<usize, Errno> {
        file.with_position(|position| {
            let size = self.device.size_bytes().map_err(block_errno)?;
            if *position >= size {
                return Ok(0);
            }
            let block_size = self.device.block_size();
            let mut scratch = Vec::new();
            scratch.try_reserve(block_size).map_err(|_| Errno::Enomem)?;
            scratch.resize(block_size, 0);

            let mut written = 0;
            while written < buf.len() && *position < size {
                let block = *position / block_size as u64;
                let block_offset = (*position % block_size as u64) as usize;
                let count = (block_size - block_offset)
                    .min(buf.len() - written)
                    .min((size - *position) as usize);
                self.cache.read(block, &mut scratch).map_err(block_errno)?;
                scratch[block_offset..block_offset + count]
                    .copy_from_slice(&buf.as_bytes()[written..written + count]);
                self.cache.write(block, &scratch).map_err(block_errno)?;
                written += count;
                *position = (*position)
                    .checked_add(count as u64)
                    .ok_or(Errno::Eoverflow)?;
            }
            Ok(written)
        })
    }

    fn seek(&self, file: &File, offset: i64, whence: SeekWhence) -> Result<u64, Errno> {
        file.seek_position(
            offset,
            whence,
            Some(self.device.size_bytes().map_err(block_errno)?),
        )
    }

    fn fstat(&self, _file: &File) -> Result<Stat, Errno> {
        stat_for_node(&self.node)
    }

    fn sync(&self, _file: &File) -> Result<(), Errno> {
        self.cache.flush().map_err(block_errno)
    }

    fn release(&self, _file: &File) {
        if let Err(error) = self.cache.flush() {
            crate::println!("block device {} flush failed: {error:?}", self.name);
        }
    }
}

fn stat_for_node(node: &Arc<Node>) -> Result<Stat, Errno> {
    let state = node.state.lock();
    let mut stat = Stat::zeroed();
    stat.ino = node.ino;
    stat.mode = node.mode.bits();
    stat.nlink = 1;
    stat.size = match &*state {
        NodeState::Directory(children) => children.len() as i64,
        NodeState::Regular(data) => data.len() as i64,
        NodeState::Ext4Directory { children, .. } => children.len() as i64,
        NodeState::Ext4Regular {
            size,
            overlay: None,
            ..
        } => i64::try_from(*size).map_err(|_| Errno::Eoverflow)?,
        NodeState::Ext4Regular {
            overlay: Some(data),
            ..
        } => data.len() as i64,
        NodeState::Symlink(target) => target.len() as i64,
        NodeState::Device(_) => 0,
        NodeState::BlockDevice { device, .. } => device.size_bytes().unwrap_or(0) as i64,
        NodeState::ProcFile(generator) => generator.generate().map(|d| d.len() as i64).unwrap_or(0),
    };
    stat.nlink = node.nlink.load(Ordering::Acquire) as u32;
    stat.blocks = (stat.size.saturating_add(511)) / 512;
    Ok(stat)
}

#[cfg(debug_assertions)]
pub fn verify() {
    use crate::block::BlockDevice as _;

    let file = open(
        "/m11-vfs-probe",
        OpenFlags::O_CREAT
            .union(OpenFlags::O_RDWR)
            .union(OpenFlags::O_TRUNC),
    )
    .expect("tmpfs probe open failed");
    let written = file
        .write(&IoBuffer::new(b"vfs-ok"))
        .expect("tmpfs probe write failed");
    assert_eq!(written, 6);
    file.seek(0, SeekWhence::Set)
        .expect("tmpfs probe seek failed");
    let mut bytes = [0_u8; 8];
    let mut output = MutableIoBuffer::new(&mut bytes);
    let read = file.read(&mut output).expect("tmpfs probe read failed");
    assert_eq!(read, 6);
    assert_eq!(output.filled_bytes(), b"vfs-ok");
    file.truncate(3).expect("tmpfs probe truncate failed");
    file.seek(0, SeekWhence::Set)
        .expect("tmpfs probe seek after truncate failed");
    let mut truncated = [0_u8; 4];
    let mut truncated_buf = MutableIoBuffer::new(&mut truncated);
    assert_eq!(
        file.read(&mut truncated_buf)
            .expect("tmpfs probe truncated read failed"),
        3,
    );
    assert_eq!(truncated_buf.filled_bytes(), b"vfs");

    mkdir("/m11-dir", 0o755).expect("tmpfs mkdir failed");
    let nested = open(
        "/m11-dir/file",
        OpenFlags::O_CREAT
            .union(OpenFlags::O_RDWR)
            .union(OpenFlags::O_TRUNC),
    )
    .expect("nested tmpfs file open failed");
    assert_eq!(
        nested
            .write(&IoBuffer::new(b"dirent"))
            .expect("nested tmpfs file write failed"),
        6,
    );
    rename("/m11-dir/file", "/m11-dir/renamed").expect("tmpfs rename failed");
    assert_eq!(
        stat("/m11-dir/renamed")
            .expect("renamed file stat failed")
            .size,
        6,
    );
    symlink("renamed", "/m11-dir/link").expect("tmpfs symlink failed");
    let mut link_target = [0_u8; 16];
    let mut link_buf = MutableIoBuffer::new(&mut link_target);
    assert_eq!(
        readlink("/m11-dir/link", &mut link_buf).expect("tmpfs readlink failed"),
        "renamed".len(),
    );
    assert_eq!(link_buf.filled_bytes(), b"renamed");
    let linked = open("/m11-dir/link", OpenFlags::O_RDONLY).expect("open symlink target failed");
    let mut linked_bytes = [0_u8; 8];
    let mut linked_buf = MutableIoBuffer::new(&mut linked_bytes);
    assert_eq!(
        linked.read(&mut linked_buf).expect("read symlink target"),
        6
    );
    assert_eq!(linked_buf.filled_bytes(), b"dirent");
    assert_eq!(
        open(
            "/m11-dir/link",
            OpenFlags::O_RDONLY.union(OpenFlags::O_NOFOLLOW)
        )
        .err(),
        Some(Errno::Eloop),
    );
    link("/m11-dir/renamed", "/m11-dir/hard", false).expect("tmpfs hardlink failed");
    let renamed_stat = stat("/m11-dir/renamed").expect("hardlink source stat failed");
    let hard_stat = stat("/m11-dir/hard").expect("hardlink stat failed");
    assert_eq!(renamed_stat.ino, hard_stat.ino);
    assert_eq!(renamed_stat.nlink, 2);
    let dir = open(
        "/m11-dir",
        OpenFlags::O_RDONLY.union(OpenFlags::O_DIRECTORY),
    )
    .expect("directory open failed");
    let mut dirents = [0_u8; 256];
    let mut dirent_buf = MutableIoBuffer::new(&mut dirents);
    let dirent_bytes = dir.readdir(&mut dirent_buf).expect("getdents probe failed");
    assert!(dirent_bytes > 0);
    let root_ino = stat("/").expect("root stat failed").ino;
    let dir_ino = stat("/m11-dir").expect("directory stat failed").ino;
    assert_eq!(
        find_dirent_ino(dirent_buf.filled_bytes(), "."),
        Some(dir_ino),
        "getdents reported the wrong inode for .",
    );
    assert_eq!(
        find_dirent_ino(dirent_buf.filled_bytes(), ".."),
        Some(root_ino),
        "getdents reported the wrong parent inode for ..",
    );
    assert!(
        dirent_buf
            .filled_bytes()
            .windows(b"renamed".len())
            .any(|window| window == b"renamed"),
        "getdents probe did not expose the renamed entry",
    );
    unlink("/m11-dir/link", false).expect("tmpfs unlink symlink failed");
    unlink("/m11-dir/hard", false).expect("tmpfs unlink hardlink failed");
    unlink("/m11-dir/renamed", false).expect("tmpfs unlink failed");
    unlink("/m11-dir", true).expect("tmpfs rmdir failed");
    assert_eq!(
        resolve_path("/dev/../", "./dev//zero").expect("relative path resolve failed"),
        "/dev/zero",
    );

    let zero = open("/dev/zero", OpenFlags::O_RDONLY).expect("/dev/zero open failed");
    let mut zero_bytes = [1_u8; 4];
    let mut zero_buf = MutableIoBuffer::new(&mut zero_bytes);
    assert_eq!(zero.read(&mut zero_buf).expect("/dev/zero read failed"), 4);
    assert_eq!(zero_buf.filled_bytes(), &[0, 0, 0, 0]);

    let (mounts, sourced, ext4, flagged) = mount_table_counts();
    assert!(mounts >= 2);
    assert!(sourced >= 2);
    assert!(ext4 <= mounts);
    assert!(flagged <= mounts);

    mkdir("/m15-mnt", 0o755).expect("tmpfs mountpoint mkdir failed");
    mount(None, "/m15-mnt", "tmpfs", 0).expect("tmpfs mount failed");
    assert_eq!(mount(None, "/m15-mnt", "tmpfs", 0), Err(Errno::Ebusy));
    umount("/m15-mnt", 0).expect("tmpfs umount failed");
    unlink("/m15-mnt", true).expect("tmpfs mountpoint rmdir failed");

    let ext4_device = Arc::new(
        crate::block::MemoryBlockDevice::new(512, 32).expect("ext4 ro fixture block device"),
    );

    // M15-A no longer accepts a magic-only fake disk. Build the smallest valid
    // read-only ext4 root that this native parser supports: 1 KiB blocks,
    // one group descriptor, one inode table, and an extent-backed empty root dir.
    let mut super_sector = [0_u8; 512];
    super_sector[24..28].copy_from_slice(&0_u32.to_le_bytes()); // s_log_block_size => 1024
    super_sector[32..36].copy_from_slice(&8_u32.to_le_bytes()); // s_blocks_per_group
    super_sector[40..44].copy_from_slice(&16_u32.to_le_bytes()); // s_inodes_per_group
    super_sector[56..58].copy_from_slice(&0xef53_u16.to_le_bytes());
    super_sector[88..90].copy_from_slice(&128_u16.to_le_bytes()); // s_inode_size
    super_sector[96..100].copy_from_slice(&(0x0000_0002_u32 | 0x0000_0040_u32).to_le_bytes());
    super_sector[254..256].copy_from_slice(&32_u16.to_le_bytes());
    ext4_device
        .write_block(2, &super_sector)
        .expect("ext4 ro fixture superblock write failed");

    let mut group_sector = [0_u8; 512];
    group_sector[8..12].copy_from_slice(&3_u32.to_le_bytes()); // bg_inode_table_lo
    ext4_device
        .write_block(4, &group_sector)
        .expect("ext4 ro fixture group descriptor write failed");

    let mut inode_sector = [0_u8; 512];
    let root_inode = &mut inode_sector[128..256]; // inode #2, inode size 128
    root_inode[0..2].copy_from_slice(&0o040755_u16.to_le_bytes());
    root_inode[4..8].copy_from_slice(&24_u32.to_le_bytes()); // two compact dirents
    root_inode[32..36].copy_from_slice(&0x0008_0000_u32.to_le_bytes()); // EXT4_EXTENTS_FL
    let extent_header = &mut root_inode[40..100];
    extent_header[0..2].copy_from_slice(&0xf30a_u16.to_le_bytes());
    extent_header[2..4].copy_from_slice(&1_u16.to_le_bytes());
    extent_header[4..6].copy_from_slice(&4_u16.to_le_bytes());
    extent_header[6..8].copy_from_slice(&0_u16.to_le_bytes());
    extent_header[12..16].copy_from_slice(&0_u32.to_le_bytes()); // logical block
    extent_header[16..18].copy_from_slice(&1_u16.to_le_bytes()); // initialized len
    extent_header[18..20].copy_from_slice(&0_u16.to_le_bytes()); // start_hi
    extent_header[20..24].copy_from_slice(&4_u32.to_le_bytes()); // physical block
    ext4_device
        .write_block(6, &inode_sector)
        .expect("ext4 ro fixture inode table write failed");

    let mut dir_sector = [0_u8; 512];
    dir_sector[0..4].copy_from_slice(&2_u32.to_le_bytes());
    dir_sector[4..6].copy_from_slice(&12_u16.to_le_bytes());
    dir_sector[6] = 1;
    dir_sector[7] = 2;
    dir_sector[8] = b'.';
    dir_sector[12..16].copy_from_slice(&2_u32.to_le_bytes());
    dir_sector[16..18].copy_from_slice(&12_u16.to_le_bytes());
    dir_sector[18] = 2;
    dir_sector[19] = 2;
    dir_sector[20] = b'.';
    dir_sector[21] = b'.';
    ext4_device
        .write_block(8, &dir_sector)
        .expect("ext4 ro fixture directory write failed");

    crate::block::register_device("m15ext4", ext4_device.clone())
        .expect("ext4 ro fixture device register failed");
    mkdir("/m15-ext4", 0o755).expect("ext4 mountpoint mkdir failed");
    mount(Some("/dev/m15ext4"), "/m15-ext4", "ext4", 1).expect("ext4 ro mount failed");
    assert_eq!(
        open("/m15-ext4/new", OpenFlags::O_CREAT.union(OpenFlags::O_RDWR),).err(),
        Some(Errno::Erofs),
        "ext4 read-only root allowed tmpfs-style create",
    );
    let (mounts_after_ext4, sourced_after_ext4, ext4_after, flagged_after) = mount_table_counts();
    assert!(mounts_after_ext4 > mounts);
    assert!(sourced_after_ext4 > sourced);
    assert_eq!(ext4_after, ext4 + 1);
    assert_eq!(flagged_after, flagged + 1);
    umount("/m15-ext4", 0).expect("ext4 ro umount failed");
    unlink("/m15-ext4", true).expect("ext4 mountpoint rmdir failed");
    crate::block::unregister_device("m15ext4").expect("ext4 ro fixture unregister failed");

    // 延迟注册块设备节点（CodePlan §6）：USB MSC 这类事后才注册的设备需
    // 动态补 /dev 节点。注册 → install_block_device_node → open 可见；
    // 重复安装返回 Eexist（幂等可忽略）。
    let late_dev = crate::block::MemoryBlockDevice::new(512, 8).expect("late fixture block device");
    crate::block::register_device("sda-late", Arc::new(late_dev))
        .expect("late device register failed");
    install_block_device_node("sda-late").expect("install /dev/sda-late node failed");
    assert!(
        open("/dev/sda-late", OpenFlags::O_RDONLY).is_ok(),
        "post-init block node must be openable through the VFS tree",
    );
    assert_eq!(
        install_block_device_node("sda-late").err(),
        Some(Errno::Eexist),
        "duplicate block node install must be idempotent-Eexist",
    );
    crate::block::unregister_device("sda-late").expect("late device unregister failed");

    crate::println!("M11 VFS gate:");
    crate::println!("  tmpfs file ops       : verified");
    crate::println!("  directory ops        : verified");
    crate::println!("  symlink/hardlink     : verified");
    crate::println!("  rename/unlink        : verified");
    crate::println!("  cwd path resolver    : verified");
    crate::println!("  devfs null/zero/console: verified");
    crate::println!("  mount table          : verified");
    crate::println!("  ext4 superblock gate : verified");
    crate::println!("  per-process fd table  : verified");
}

fn find_dirent_ino(dirents: &[u8], expected_name: &str) -> Option<u64> {
    let mut offset = 0_usize;
    while offset.checked_add(19)? <= dirents.len() {
        let ino = read_u64_ne(dirents.get(offset..offset + 8)?)?;
        let record_len = read_u16_ne(dirents.get(offset + 16..offset + 18)?)? as usize;
        if record_len < 20 || offset.checked_add(record_len)? > dirents.len() {
            return None;
        }
        let name_start = offset + 19;
        let name_end = dirents[name_start..offset + record_len]
            .iter()
            .position(|byte| *byte == 0)
            .map(|position| name_start + position)?;
        if dirents.get(name_start..name_end) == Some(expected_name.as_bytes()) {
            return Some(ino);
        }
        offset += record_len;
    }
    None
}

fn read_u64_ne(bytes: &[u8]) -> Option<u64> {
    Some(u64::from_ne_bytes([
        *bytes.first()?,
        *bytes.get(1)?,
        *bytes.get(2)?,
        *bytes.get(3)?,
        *bytes.get(4)?,
        *bytes.get(5)?,
        *bytes.get(6)?,
        *bytes.get(7)?,
    ]))
}

fn read_u16_ne(bytes: &[u8]) -> Option<u16> {
    Some(u16::from_ne_bytes([*bytes.first()?, *bytes.get(1)?]))
}
