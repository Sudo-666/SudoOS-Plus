// SUDOOS_M15A_EXT4_RO_PATCH_V1
use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::block::{self, BlockDevice};
use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};

const EXT4_METADATA_LOCK: LockClass = LockClass::new("ext4.metadata", LockRank::Vfs, 20);

const EXT4_SUPER_MAGIC: u16 = 0xef53;
const EXT4_SUPER_OFFSET: u64 = 1024;
const EXT4_SUPER_SIZE: usize = 1024;
const EXT4_ROOT_INO: u32 = 2;
const EXT4_N_BLOCKS_BYTES: usize = 60;
const EXT4_EXTENTS_FL: u32 = 0x0008_0000;
const EXT4_EXTENT_MAGIC: u16 = 0xf30a;
const EXT4_FEATURE_INCOMPAT_FILETYPE: u32 = 0x0000_0002;
const EXT4_FEATURE_INCOMPAT_EXTENTS: u32 = 0x0000_0040;
const EXT4_FEATURE_INCOMPAT_64BIT: u32 = 0x0000_0080;
const EXT4_FEATURE_INCOMPAT_FLEX_BG: u32 = 0x0000_0200;
const EXT4_SUPPORTED_INCOMPAT: u32 = EXT4_FEATURE_INCOMPAT_FILETYPE
    | EXT4_FEATURE_INCOMPAT_EXTENTS
    | EXT4_FEATURE_INCOMPAT_64BIT
    | EXT4_FEATURE_INCOMPAT_FLEX_BG;
// CLOUD_IMAGE_EXT4_COMPAT_V1
//
// Newer e2fsprogs/cloud images may set incompat feature bits that do not
// change the on-disk data path used by this read-only contest reader.
// Keep layout-changing features explicit, print the complete superblock
// fingerprint once, and support META_BG descriptor placement.
const EXT4_FEATURE_INCOMPAT_COMPRESSION: u32 = 0x0000_0001;
const EXT4_FEATURE_INCOMPAT_RECOVER: u32 = 0x0000_0004;
const EXT4_FEATURE_INCOMPAT_JOURNAL_DEV: u32 = 0x0000_0008;
const EXT4_FEATURE_INCOMPAT_META_BG: u32 = 0x0000_0010;
const EXT4_FEATURE_INCOMPAT_MMP: u32 = 0x0000_0100;
const EXT4_FEATURE_INCOMPAT_EA_INODE: u32 = 0x0000_0400;
const EXT4_FEATURE_INCOMPAT_DIRDATA: u32 = 0x0000_1000;
const EXT4_FEATURE_INCOMPAT_CSUM_SEED: u32 = 0x0000_2000;
const EXT4_FEATURE_INCOMPAT_LARGEDIR: u32 = 0x0000_4000;
const EXT4_FEATURE_INCOMPAT_INLINE_DATA: u32 = 0x0000_8000;
const EXT4_FEATURE_INCOMPAT_ENCRYPT: u32 = 0x0001_0000;
const EXT4_FEATURE_INCOMPAT_CASEFOLD: u32 = 0x0002_0000;

const EXT4_FEATURE_COMPAT_SPARSE_SUPER2: u32 = 0x0000_0200;
const EXT4_FEATURE_RO_COMPAT_SPARSE_SUPER: u32 = 0x0000_0001;
const EXT4_FEATURE_RO_COMPAT_BIGALLOC: u32 = 0x0000_0200;

const EXT4_READONLY_IGNORABLE_INCOMPAT: u32 = EXT4_FEATURE_INCOMPAT_RECOVER
    | EXT4_FEATURE_INCOMPAT_MMP
    | EXT4_FEATURE_INCOMPAT_EA_INODE
    | EXT4_FEATURE_INCOMPAT_CSUM_SEED
    | EXT4_FEATURE_INCOMPAT_LARGEDIR
    | EXT4_FEATURE_INCOMPAT_ENCRYPT
    | EXT4_FEATURE_INCOMPAT_CASEFOLD;

const EXT4_LAYOUT_SUPPORTED_INCOMPAT: u32 = EXT4_SUPPORTED_INCOMPAT | EXT4_FEATURE_INCOMPAT_META_BG;

const EXT4_KNOWN_HARD_INCOMPAT: u32 = EXT4_FEATURE_INCOMPAT_COMPRESSION
    | EXT4_FEATURE_INCOMPAT_JOURNAL_DEV
    | EXT4_FEATURE_INCOMPAT_DIRDATA
    | EXT4_FEATURE_INCOMPAT_INLINE_DATA;

const EXT4_KNOWN_INCOMPAT: u32 =
    EXT4_LAYOUT_SUPPORTED_INCOMPAT | EXT4_READONLY_IGNORABLE_INCOMPAT | EXT4_KNOWN_HARD_INCOMPAT;

const EXT4_ENCRYPT_FL: u32 = 0x0000_0800;
const EXT4_INLINE_DATA_FL: u32 = 0x1000_0000;

static EXT4_SUPER_DIAG_PRINTED: AtomicBool = AtomicBool::new(false);
const EXT4_S_IFMT: u16 = 0o170000;
const EXT4_S_IFREG: u16 = 0o100000;
const EXT4_S_IFDIR: u16 = 0o040000;
const EXT4_S_IFLNK: u16 = 0o120000;
const MAX_EXT4_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_EXT4_NODES: usize = 65536;
const MAX_EXT4_DEPTH: usize = 16;
const MAX_EXTENT_TREE_DEPTH: usize = 5;
// BUILDSTORM_EXT4_CACHE_V16
const EXT4_DATA_CACHE_CHUNK_SIZE: usize = 1024 * 1024;
const EXT4_DATA_CACHE_CAPACITY_BYTES: usize = 2 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ext4Error {
    AddressOverflow,
    BadBlockSize,
    BadDirectory,
    BadExtentTree,
    BadGroupDescriptor,
    BadInode,
    BlockIo(block::BlockError),
    FileTooLarge,
    InvalidFeatureSet,
    InvalidSuperblock,
    NotFound,
    OutOfMemory,
    Unsupported,
}

#[derive(Clone, Debug)]
pub struct Ext4SnapshotNode {
    pub ino: u64,
    pub mode: u32,
    pub size: u64,
    pub kind: Ext4SnapshotKind,
}

#[derive(Clone, Debug)]
pub enum Ext4SnapshotKind {
    Directory(Vec<Ext4SnapshotDirEntry>),
    Regular(Vec<u8>),
    Symlink(String),
}

#[derive(Clone, Debug)]
pub struct Ext4SnapshotDirEntry {
    pub name: String,
    pub node: Ext4SnapshotNode,
}

/// 轻量级列出 ext4 根目录条目（仅名字和 inode 号，不递归）。
pub struct Ext4DirEntry {
    pub name: String,
    pub ino: u32,
    pub file_type: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ext4NodeKind {
    Directory,
    Regular,
    Symlink,
}

#[derive(Clone, Copy, Debug)]
pub struct Ext4NodeInfo {
    pub ino: u32,
    pub mode: u32,
    pub size: u64,
    pub kind: Ext4NodeKind,
}

pub fn list_root_directory(
    device: Arc<dyn BlockDevice>,
) -> Result<alloc::vec::Vec<Ext4DirEntry>, Ext4Error> {
    let fs = Ext4FileSystem::open(device)?;
    let mut entries = alloc::vec::Vec::new();
    fs.read_root_dir_entries(&mut entries)?;
    Ok(entries)
}
pub fn list_directory(
    device: Arc<dyn BlockDevice>,
    path: &str,
) -> Result<Vec<Ext4DirEntry>, Ext4Error> {
    let fs = Ext4FileSystem::open(device)?;
    let ino = fs.lookup_path_ino(path)?;
    let mut entries = alloc::vec::Vec::new();
    fs.read_dir_entries_for_ino(ino, &mut entries)?;
    Ok(entries)
}

pub fn load_root_snapshot(device: Arc<dyn BlockDevice>) -> Result<Ext4SnapshotNode, Ext4Error> {
    let fs = Ext4FileSystem::open(device)?;
    let mut budget = NodeBudget::new(MAX_EXT4_NODES);
    fs.load_inode_tree(EXT4_ROOT_INO, 0, &mut budget)
}

pub fn load_path_snapshot(
    device: Arc<dyn BlockDevice>,
    path: &str,
) -> Result<Ext4SnapshotNode, Ext4Error> {
    let fs = Ext4FileSystem::open(device)?;
    fs.load_path(path)
}

pub struct Ext4FileSystem {
    device: Arc<dyn BlockDevice>,
    block_size: u64,
    inode_size: u16,
    inodes_per_group: u32,
    blocks_per_group: u32,
    group_desc_size: u16,
    first_data_block: u32,
    feature_compat: u32,
    feature_incompat: u32,
    feature_ro_compat: u32,
    first_meta_bg: u32,
    backup_bgs: [u32; 2],
    metadata: IrqSpinLock<Ext4MetadataCache>,
}

struct Ext4MetadataCache {
    inodes: BTreeMap<u32, Ext4Inode>,
    inode_tables: BTreeMap<u32, u64>,
    data_chunks: BTreeMap<(u32, u64), Arc<[u8]>>,
    data_bytes: usize,
}

#[derive(Clone)]
pub struct Ext4Inode {
    ino: u32,
    mode: u16,
    size: u64,
    flags: u32,
    block: [u8; EXT4_N_BLOCKS_BYTES],
}

pub struct Ext4File {
    data: Vec<u8>,
}

struct NodeBudget {
    remaining: usize,
}

impl NodeBudget {
    const fn new(limit: usize) -> Self {
        Self { remaining: limit }
    }

    fn charge(&mut self) -> Result<(), Ext4Error> {
        if self.remaining == 0 {
            return Err(Ext4Error::Unsupported);
        }
        self.remaining -= 1;
        Ok(())
    }
}

fn ext4_print_incompat_features(mask: u32) {
    let features = [
        (EXT4_FEATURE_INCOMPAT_COMPRESSION, "compression"),
        (EXT4_FEATURE_INCOMPAT_FILETYPE, "filetype"),
        (EXT4_FEATURE_INCOMPAT_RECOVER, "recover"),
        (EXT4_FEATURE_INCOMPAT_JOURNAL_DEV, "journal_dev"),
        (EXT4_FEATURE_INCOMPAT_META_BG, "meta_bg"),
        (EXT4_FEATURE_INCOMPAT_EXTENTS, "extents"),
        (EXT4_FEATURE_INCOMPAT_64BIT, "64bit"),
        (EXT4_FEATURE_INCOMPAT_MMP, "mmp"),
        (EXT4_FEATURE_INCOMPAT_FLEX_BG, "flex_bg"),
        (EXT4_FEATURE_INCOMPAT_EA_INODE, "ea_inode"),
        (EXT4_FEATURE_INCOMPAT_DIRDATA, "dirdata"),
        (EXT4_FEATURE_INCOMPAT_CSUM_SEED, "csum_seed"),
        (EXT4_FEATURE_INCOMPAT_LARGEDIR, "largedir"),
        (EXT4_FEATURE_INCOMPAT_INLINE_DATA, "inline_data"),
        (EXT4_FEATURE_INCOMPAT_ENCRYPT, "encrypt"),
        (EXT4_FEATURE_INCOMPAT_CASEFOLD, "casefold"),
    ];

    for (bit, name) in features {
        if mask & bit != 0 {
            crate::println!("ext4-super: incompat-feature {name} bit={bit:#010x}");
        }
    }

    let unknown = mask & !EXT4_KNOWN_INCOMPAT;
    if unknown != 0 {
        crate::println!("ext4-super: incompat-feature unknown mask={unknown:#010x}");
    }
}

fn ext4_exact_power(mut value: u32, base: u32) -> bool {
    if value < 1 {
        return false;
    }
    while value > 1 {
        if value % base != 0 {
            return false;
        }
        value /= base;
    }
    true
}

impl Ext4FileSystem {
    pub fn open(device: Arc<dyn BlockDevice>) -> Result<Self, Ext4Error> {
        let logical_block_size = device.block_size();
        if logical_block_size == 0
            || !logical_block_size.is_power_of_two()
            || logical_block_size > myos_mm::PAGE_SIZE
        {
            return Err(Ext4Error::BadBlockSize);
        }

        let mut superblock = [0_u8; EXT4_SUPER_SIZE];
        read_exact(&device, EXT4_SUPER_OFFSET, &mut superblock)?;
        if le_u16(&superblock, 56)? != EXT4_SUPER_MAGIC {
            return Err(Ext4Error::InvalidSuperblock);
        }

        let log_block_size = le_u32(&superblock, 24)?;
        if log_block_size > 16 {
            return Err(Ext4Error::InvalidSuperblock);
        }
        let block_size = 1024_u64
            .checked_shl(log_block_size)
            .ok_or(Ext4Error::InvalidSuperblock)?;
        if block_size == 0
            || block_size > myos_mm::PAGE_SIZE as u64
            || !block_size.is_power_of_two()
        {
            return Err(Ext4Error::BadBlockSize);
        }

        let blocks_per_group = le_u32(&superblock, 32)?;
        let inodes_per_group = le_u32(&superblock, 40)?;
        if blocks_per_group == 0 || inodes_per_group == 0 {
            return Err(Ext4Error::InvalidSuperblock);
        }

        let feature_compat = le_u32(&superblock, 92)?;
        let feature_incompat = le_u32(&superblock, 96)?;
        let feature_ro_compat = le_u32(&superblock, 100)?;
        let first_data_block = le_u32(&superblock, 20)?;
        let filesystem_state = le_u16(&superblock, 58)?;
        let errors_behavior = le_u16(&superblock, 60)?;
        let journal_inode = le_u32(&superblock, 224).unwrap_or(0);
        let last_orphan = le_u32(&superblock, 232).unwrap_or(0);
        let first_meta_bg = le_u32(&superblock, 260).unwrap_or(0);
        let backup_bgs = [
            le_u32(&superblock, 588).unwrap_or(0),
            le_u32(&superblock, 592).unwrap_or(0),
        ];

        let hard_incompat = feature_incompat & EXT4_KNOWN_HARD_INCOMPAT;
        let unknown_incompat = feature_incompat & !EXT4_KNOWN_INCOMPAT;
        let readonly_ignored = feature_incompat & EXT4_READONLY_IGNORABLE_INCOMPAT;
        let dangerous_ro = feature_ro_compat & EXT4_FEATURE_RO_COMPAT_BIGALLOC;

        let first_diagnostic = !EXT4_SUPER_DIAG_PRINTED.swap(true, Ordering::AcqRel);
        if first_diagnostic {
            let uuid0 = le_u32(&superblock, 104).unwrap_or(0);
            let uuid1 = le_u32(&superblock, 108).unwrap_or(0);
            let uuid2 = le_u32(&superblock, 112).unwrap_or(0);
            let uuid3 = le_u32(&superblock, 116).unwrap_or(0);
            crate::println!(
                "ext4-super: magic={:#06x} logical_block={} fs_block={} first_data={} inode_size_raw={} desc_size_raw={}",
                EXT4_SUPER_MAGIC,
                logical_block_size,
                block_size,
                first_data_block,
                le_u16(&superblock, 88).unwrap_or(0),
                le_u16(&superblock, 254).unwrap_or(0),
            );
            crate::println!(
                "ext4-super: compat={:#010x} incompat={:#010x} ro_compat={:#010x} ignored={:#010x} hard={:#010x} unknown={:#010x}",
                feature_compat,
                feature_incompat,
                feature_ro_compat,
                readonly_ignored,
                hard_incompat,
                unknown_incompat,
            );
            crate::println!(
                "ext4-super: state={:#06x} errors={:#06x} journal_ino={} last_orphan={} first_meta_bg={} backup_bgs=[{},{}]",
                filesystem_state,
                errors_behavior,
                journal_inode,
                last_orphan,
                first_meta_bg,
                backup_bgs[0],
                backup_bgs[1],
            );
            crate::println!(
                "ext4-super: uuid={:08x}-{:08x}-{:08x}-{:08x}",
                uuid0,
                uuid1,
                uuid2,
                uuid3,
            );
            ext4_print_incompat_features(feature_incompat);

            if readonly_ignored != 0 {
                crate::println!(
                    "ext4-super: read-only compatibility mode enabled mask={readonly_ignored:#010x}"
                );
            }
            if feature_incompat & EXT4_FEATURE_INCOMPAT_RECOVER != 0 {
                crate::println!(
                    "ext4-super: WARNING journal recovery requested; contest reader proceeds read-only"
                );
            }
            if feature_incompat & EXT4_FEATURE_INCOMPAT_META_BG != 0 {
                crate::println!("ext4-super: META_BG descriptor addressing enabled");
            }
        }

        if hard_incompat != 0 || unknown_incompat != 0 || dangerous_ro != 0 {
            if first_diagnostic {
                crate::println!(
                    "ext4-super: rejected hard={hard_incompat:#010x} unknown={unknown_incompat:#010x} dangerous_ro={dangerous_ro:#010x}"
                );
            }
            return Err(Ext4Error::InvalidFeatureSet);
        }

        let inode_size = le_u16(&superblock, 88)?.max(128);
        if inode_size < 128 || !u32::from(inode_size).is_power_of_two() {
            return Err(Ext4Error::InvalidSuperblock);
        }

        let raw_desc_size = le_u16(&superblock, 254).unwrap_or(32);
        let group_desc_size = if feature_incompat & EXT4_FEATURE_INCOMPAT_64BIT != 0 {
            raw_desc_size.max(32)
        } else {
            32
        };
        if !(32..=1024).contains(&group_desc_size) || group_desc_size % 8 != 0 {
            return Err(Ext4Error::InvalidSuperblock);
        }

        Ok(Self {
            device,
            block_size,
            inode_size,
            inodes_per_group,
            blocks_per_group,
            group_desc_size,
            first_data_block,
            feature_compat,
            feature_incompat,
            feature_ro_compat,
            first_meta_bg,
            backup_bgs,
            metadata: IrqSpinLock::new_with_class(
                Ext4MetadataCache {
                    inodes: BTreeMap::new(),
                    inode_tables: BTreeMap::new(),
                    data_chunks: BTreeMap::new(),
                    data_bytes: 0,
                },
                EXT4_METADATA_LOCK,
            ),
        })
    }

    pub fn root_info(&self) -> Result<Ext4NodeInfo, Ext4Error> {
        self.inode_info(EXT4_ROOT_INO)
    }

    pub fn lookup_child_info(&self, parent: u32, name: &str) -> Result<Ext4NodeInfo, Ext4Error> {
        let ino = self.lookup_child_ino(parent, name)?;
        self.inode_info(ino)
    }

    pub fn list_directory_info(&self, ino: u32) -> Result<Vec<(String, Ext4NodeInfo)>, Ext4Error> {
        let mut entries = Vec::new();
        let mut raw = Vec::new();
        self.read_dir_entries_for_ino(ino, &mut raw)?;
        entries
            .try_reserve(raw.len())
            .map_err(|_| Ext4Error::OutOfMemory)?;
        for entry in raw {
            entries.push((entry.name, self.inode_info(entry.ino)?));
        }
        Ok(entries)
    }

    pub fn read_inode_at(
        &self,
        ino: u32,
        offset: u64,
        output: &mut [u8],
    ) -> Result<usize, Ext4Error> {
        let inode = self.read_inode(ino)?;
        if inode.file_type() != EXT4_S_IFREG {
            return Err(Ext4Error::BadInode);
        }
        if offset >= inode.size || output.is_empty() {
            return Ok(0);
        }
        let count = usize::try_from((inode.size - offset).min(output.len() as u64))
            .map_err(|_| Ext4Error::AddressOverflow)?;
        output[..count].fill(0);
        if inode.flags & EXT4_EXTENTS_FL == 0 {
            return Err(Ext4Error::Unsupported);
        }
        let mut copied = 0_usize;
        while copied < count {
            let current = offset
                .checked_add(copied as u64)
                .ok_or(Ext4Error::AddressOverflow)?;
            let chunk_index = current / EXT4_DATA_CACHE_CHUNK_SIZE as u64;
            let chunk_start = chunk_index
                .checked_mul(EXT4_DATA_CACHE_CHUNK_SIZE as u64)
                .ok_or(Ext4Error::AddressOverflow)?;
            let in_chunk =
                usize::try_from(current - chunk_start).map_err(|_| Ext4Error::AddressOverflow)?;
            let chunk_len =
                usize::try_from((inode.size - chunk_start).min(EXT4_DATA_CACHE_CHUNK_SIZE as u64))
                    .map_err(|_| Ext4Error::AddressOverflow)?;
            let copy_len = (count - copied).min(chunk_len - in_chunk);

            let Some(chunk) =
                self.read_data_chunk_cached(&inode, chunk_index, chunk_start, chunk_len)?
            else {
                // A full cache or memory pressure must not turn a valid read
                // into ENOMEM.  Preserve the original direct, bulk-I/O path
                // for this request and all remaining bytes.
                self.read_extent_range(&inode.block, current, &mut output[copied..count], 0)?;
                return Ok(count);
            };
            output[copied..copied + copy_len]
                .copy_from_slice(&chunk[in_chunk..in_chunk + copy_len]);
            copied += copy_len;
        }
        Ok(count)
    }

    fn read_data_chunk_cached(
        &self,
        inode: &Ext4Inode,
        chunk_index: u64,
        chunk_start: u64,
        chunk_len: usize,
    ) -> Result<Option<Arc<[u8]>>, Ext4Error> {
        let key = (inode.ino, chunk_index);
        {
            let metadata = self.metadata.lock();
            if let Some(chunk) = metadata.data_chunks.get(&key) {
                return Ok(Some(Arc::clone(chunk)));
            }
            if metadata
                .data_bytes
                .checked_add(chunk_len)
                .is_none_or(|bytes| bytes > EXT4_DATA_CACHE_CAPACITY_BYTES)
            {
                return Ok(None);
            }
        }

        let mut data = Vec::new();
        if data.try_reserve(chunk_len).is_err() {
            return Ok(None);
        }
        data.resize(chunk_len, 0);
        self.read_extent_range(&inode.block, chunk_start, &mut data, 0)?;
        let loaded: Arc<[u8]> = Arc::from(data.into_boxed_slice());

        let mut metadata = self.metadata.lock();
        if let Some(chunk) = metadata.data_chunks.get(&key) {
            return Ok(Some(Arc::clone(chunk)));
        }
        if metadata
            .data_bytes
            .checked_add(chunk_len)
            .is_some_and(|bytes| bytes <= EXT4_DATA_CACHE_CAPACITY_BYTES)
        {
            metadata.data_bytes += chunk_len;
            metadata.data_chunks.insert(key, Arc::clone(&loaded));
        }
        Ok(Some(loaded))
    }

    pub fn read_symlink_inode(&self, ino: u32) -> Result<String, Ext4Error> {
        let inode = self.read_inode(ino)?;
        if inode.file_type() != EXT4_S_IFLNK {
            return Err(Ext4Error::BadInode);
        }
        self.read_symlink(&inode)
    }

    fn inode_info(&self, ino: u32) -> Result<Ext4NodeInfo, Ext4Error> {
        let inode = self.read_inode(ino)?;
        let kind = match inode.file_type() {
            EXT4_S_IFDIR => Ext4NodeKind::Directory,
            EXT4_S_IFREG => Ext4NodeKind::Regular,
            EXT4_S_IFLNK => Ext4NodeKind::Symlink,
            _ => return Err(Ext4Error::Unsupported),
        };
        Ok(Ext4NodeInfo {
            ino,
            mode: linux_mode(inode.mode),
            size: inode.size,
            kind,
        })
    }

    fn load_inode_tree(
        &self,
        ino: u32,
        depth: usize,
        budget: &mut NodeBudget,
    ) -> Result<Ext4SnapshotNode, Ext4Error> {
        budget.charge()?;
        if depth > MAX_EXT4_DEPTH {
            return Err(Ext4Error::Unsupported);
        }
        let inode = self.read_inode(ino)?;
        let mode = linux_mode(inode.mode);
        let kind = match inode.file_type() {
            EXT4_S_IFDIR => {
                let bytes = self.read_inode_bytes(&inode)?;
                let children = self.read_directory(&bytes, depth, budget)?;
                Ext4SnapshotKind::Directory(children)
            }
            EXT4_S_IFREG => {
                let file = Ext4File {
                    data: self.read_inode_bytes(&inode)?,
                };
                Ext4SnapshotKind::Regular(file.data)
            }
            EXT4_S_IFLNK => {
                let target = self.read_symlink(&inode)?;
                Ext4SnapshotKind::Symlink(target)
            }
            _ => return Err(Ext4Error::Unsupported),
        };
        Ok(Ext4SnapshotNode {
            ino: u64::from(inode.ino),
            mode,
            size: inode.size,
            kind,
        })
    }

    fn read_directory(
        &self,
        data: &[u8],
        depth: usize,
        budget: &mut NodeBudget,
    ) -> Result<Vec<Ext4SnapshotDirEntry>, Ext4Error> {
        let mut entries = Vec::new();
        let mut offset = 0_usize;
        while offset < data.len() {
            if data.len() - offset < 8 {
                break;
            }
            let ino = le_u32(data, offset)?;
            let rec_len = usize::from(le_u16(data, offset + 4)?);
            let name_len = usize::from(*data.get(offset + 6).ok_or(Ext4Error::BadDirectory)?);
            if rec_len == 0 || rec_len < 8 || rec_len % 4 != 0 || offset + rec_len > data.len() {
                return Err(Ext4Error::BadDirectory);
            }
            if name_len > rec_len - 8 {
                return Err(Ext4Error::BadDirectory);
            }
            if ino != 0 {
                let name_bytes = &data[offset + 8..offset + 8 + name_len];
                if name_bytes != b"."
                    && name_bytes != b".."
                    && !name_bytes.contains(&0)
                    && !name_bytes.contains(&b'/')
                {
                    let name = String::from_utf8(name_bytes.to_vec())
                        .map_err(|_| Ext4Error::BadDirectory)?;
                    let node = self.load_inode_tree(ino, depth + 1, budget)?;
                    entries.try_reserve(1).map_err(|_| Ext4Error::OutOfMemory)?;
                    entries.push(Ext4SnapshotDirEntry { name, node });
                }
            }
            offset += rec_len;
        }
        Ok(entries)
    }

    fn lookup_path_ino(&self, path: &str) -> Result<u32, Ext4Error> {
        if path.is_empty() || path == "/" {
            return Ok(EXT4_ROOT_INO);
        }
        let mut current = EXT4_ROOT_INO;
        let mut saw_component = false;
        for component in path.split('/') {
            if component.is_empty() {
                continue;
            }
            if component == "." || component == ".." || component.as_bytes().contains(&0) {
                return Err(Ext4Error::BadDirectory);
            }
            saw_component = true;
            current = self.lookup_child_ino(current, component)?;
        }
        if saw_component {
            Ok(current)
        } else {
            Ok(EXT4_ROOT_INO)
        }
    }

    fn read_dir_entries_for_ino(
        &self,
        ino: u32,
        output: &mut alloc::vec::Vec<Ext4DirEntry>,
    ) -> Result<(), Ext4Error> {
        let inode = self.read_inode(ino)?;
        if inode.file_type() != EXT4_S_IFDIR {
            return Err(Ext4Error::BadDirectory);
        }
        let data = self.read_inode_bytes(&inode)?;
        let mut offset = 0_usize;
        while offset < data.len() {
            if data.len() - offset < 8 {
                break;
            }
            let child_ino = le_u32(&data, offset)?;
            let rec_len = usize::from(le_u16(&data, offset + 4)?);
            let name_len_raw = *data.get(offset + 6).ok_or(Ext4Error::BadDirectory)?;
            let file_type_raw = *data.get(offset + 7).ok_or(Ext4Error::BadDirectory)?;
            if rec_len == 0 || rec_len < 8 || offset + rec_len > data.len() {
                return Err(Ext4Error::BadDirectory);
            }
            let name_len = usize::from(name_len_raw);
            if name_len > rec_len - 8 {
                return Err(Ext4Error::BadDirectory);
            }
            if child_ino != 0 {
                let name_bytes = &data[offset + 8..offset + 8 + name_len];
                if name_bytes != b"."
                    && name_bytes != b".."
                    && !name_bytes.contains(&0)
                    && !name_bytes.contains(&b'/')
                {
                    let name = String::from_utf8(name_bytes.to_vec())
                        .map_err(|_| Ext4Error::BadDirectory)?;
                    output.try_reserve(1).map_err(|_| Ext4Error::OutOfMemory)?;
                    output.push(Ext4DirEntry {
                        name,
                        ino: child_ino,
                        file_type: u16::from(file_type_raw),
                    });
                }
            }
            offset += rec_len;
        }
        Ok(())
    }
    /// 读取根目录条目列表（仅名字、inode 号和文件类型，不递归加载）。
    fn read_root_dir_entries(
        &self,
        output: &mut alloc::vec::Vec<Ext4DirEntry>,
    ) -> Result<(), Ext4Error> {
        let inode = self.read_inode(EXT4_ROOT_INO)?;
        if inode.file_type() != EXT4_S_IFDIR {
            return Err(Ext4Error::BadDirectory);
        }
        let data = self.read_inode_bytes(&inode)?;
        let mut offset = 0_usize;
        while offset < data.len() {
            if data.len() - offset < 8 {
                break;
            }
            let ino = le_u32(&data, offset)?;
            let rec_len = usize::from(le_u16(&data, offset + 4)?);
            let name_len_raw = *data.get(offset + 6).ok_or(Ext4Error::BadDirectory)?;
            let file_type_raw = *data.get(offset + 7).ok_or(Ext4Error::BadDirectory)?;
            if rec_len == 0 || rec_len < 8 || offset + rec_len > data.len() {
                return Err(Ext4Error::BadDirectory);
            }
            let name_len = usize::from(name_len_raw);
            if name_len > rec_len - 8 {
                return Err(Ext4Error::BadDirectory);
            }
            if ino != 0 {
                let name_bytes = &data[offset + 8..offset + 8 + name_len];
                if name_bytes != b"."
                    && name_bytes != b".."
                    && !name_bytes.contains(&0)
                    && !name_bytes.contains(&b'/')
                {
                    let name = String::from_utf8(name_bytes.to_vec())
                        .map_err(|_| Ext4Error::BadDirectory)?;
                    output.try_reserve(1).map_err(|_| Ext4Error::OutOfMemory)?;
                    output.push(Ext4DirEntry {
                        name,
                        ino,
                        file_type: u16::from(file_type_raw),
                    });
                }
            }
            offset += rec_len;
        }
        Ok(())
    }

    fn load_path(&self, path: &str) -> Result<Ext4SnapshotNode, Ext4Error> {
        if path.is_empty() {
            return Err(Ext4Error::BadDirectory);
        }
        let mut current = EXT4_ROOT_INO;
        let mut saw_component = false;
        for component in path.split('/') {
            if component.is_empty() {
                continue;
            }
            if component == "." || component == ".." || component.as_bytes().contains(&0) {
                return Err(Ext4Error::BadDirectory);
            }
            saw_component = true;
            current = self.lookup_child_ino(current, component)?;
        }
        if !saw_component {
            return self.load_inode_tree_limited(EXT4_ROOT_INO);
        }
        self.load_inode_tree_limited(current)
    }

    fn load_inode_tree_limited(&self, ino: u32) -> Result<Ext4SnapshotNode, Ext4Error> {
        let mut budget = NodeBudget::new(MAX_EXT4_NODES);
        self.load_inode_tree(ino, 0, &mut budget)
    }

    fn lookup_child_ino(&self, parent: u32, name: &str) -> Result<u32, Ext4Error> {
        let inode = self.read_inode(parent)?;
        if inode.file_type() != EXT4_S_IFDIR {
            return Err(Ext4Error::BadDirectory);
        }
        let data = self.read_inode_bytes(&inode)?;
        let mut offset = 0_usize;
        while offset < data.len() {
            if data.len() - offset < 8 {
                break;
            }
            let ino = le_u32(&data, offset)?;
            let rec_len = usize::from(le_u16(&data, offset + 4)?);
            let name_len = usize::from(*data.get(offset + 6).ok_or(Ext4Error::BadDirectory)?);
            if rec_len == 0 || rec_len < 8 || rec_len % 4 != 0 || offset + rec_len > data.len() {
                return Err(Ext4Error::BadDirectory);
            }
            if name_len > rec_len - 8 {
                return Err(Ext4Error::BadDirectory);
            }
            let name_bytes = &data[offset + 8..offset + 8 + name_len];
            if ino != 0 && name_bytes == name.as_bytes() {
                return Ok(ino);
            }
            offset += rec_len;
        }
        Err(Ext4Error::NotFound)
    }

    fn read_symlink(&self, inode: &Ext4Inode) -> Result<String, Ext4Error> {
        if inode.size <= EXT4_N_BLOCKS_BYTES as u64 && inode.flags & EXT4_EXTENTS_FL == 0 {
            let end = usize::try_from(inode.size).map_err(|_| Ext4Error::AddressOverflow)?;
            return String::from_utf8(inode.block[..end].to_vec()).map_err(|_| Ext4Error::BadInode);
        }
        let bytes = self.read_inode_bytes(inode)?;
        String::from_utf8(bytes).map_err(|_| Ext4Error::BadInode)
    }

    fn read_inode_bytes(&self, inode: &Ext4Inode) -> Result<Vec<u8>, Ext4Error> {
        if inode.size > MAX_EXT4_FILE_BYTES {
            return Err(Ext4Error::FileTooLarge);
        }
        let length = usize::try_from(inode.size).map_err(|_| Ext4Error::AddressOverflow)?;
        let mut data = Vec::new();
        data.try_reserve(length)
            .map_err(|_| Ext4Error::OutOfMemory)?;
        data.resize(length, 0);
        if length == 0 {
            return Ok(data);
        }
        if inode.flags & EXT4_EXTENTS_FL == 0 {
            return Err(Ext4Error::Unsupported);
        }
        self.read_extent_bytes(&inode.block, &mut data, 0)?;
        Ok(data)
    }

    fn read_extent_bytes(
        &self,
        node: &[u8],
        output: &mut [u8],
        depth_seen: usize,
    ) -> Result<(), Ext4Error> {
        if depth_seen > MAX_EXTENT_TREE_DEPTH {
            return Err(Ext4Error::BadExtentTree);
        }
        if node.len() < 12 || le_u16(node, 0)? != EXT4_EXTENT_MAGIC {
            return Err(Ext4Error::BadExtentTree);
        }
        let entries = usize::from(le_u16(node, 2)?);
        let max_entries = usize::from(le_u16(node, 4)?);
        let depth = usize::from(le_u16(node, 6)?);
        if entries > max_entries || 12 + entries * 12 > node.len() {
            return Err(Ext4Error::BadExtentTree);
        }
        if depth == 0 {
            for index in 0..entries {
                let entry = 12 + index * 12;
                let logical = u64::from(le_u32(node, entry)?);
                let raw_len = le_u16(node, entry + 4)?;
                let (initialized_len, unwritten) = Self::decode_extent_len(raw_len);
                let start_hi = u64::from(le_u16(node, entry + 6)?);
                let start_lo = u64::from(le_u32(node, entry + 8)?);
                let physical = (start_hi << 32) | start_lo;
                if initialized_len == 0 || unwritten {
                    continue;
                }
                self.copy_extent(logical, initialized_len, physical, output)?;
            }
            return Ok(());
        }
        for index in 0..entries {
            let entry = 12 + index * 12;
            let leaf_lo = u64::from(le_u32(node, entry + 4)?);
            let leaf_hi = u64::from(le_u16(node, entry + 8)?);
            let leaf = (leaf_hi << 32) | leaf_lo;
            let block = self.read_block(leaf)?;
            self.read_extent_bytes(&block, output, depth_seen + 1)?;
        }
        Ok(())
    }

    fn read_extent_range(
        &self,
        node: &[u8],
        file_offset: u64,
        output: &mut [u8],
        depth_seen: usize,
    ) -> Result<(), Ext4Error> {
        if depth_seen > MAX_EXTENT_TREE_DEPTH {
            return Err(Ext4Error::BadExtentTree);
        }
        if node.len() < 12 || le_u16(node, 0)? != EXT4_EXTENT_MAGIC {
            return Err(Ext4Error::BadExtentTree);
        }
        let entries = usize::from(le_u16(node, 2)?);
        let max_entries = usize::from(le_u16(node, 4)?);
        let depth = usize::from(le_u16(node, 6)?);
        if entries > max_entries || 12 + entries * 12 > node.len() {
            return Err(Ext4Error::BadExtentTree);
        }
        if depth != 0 {
            for index in 0..entries {
                let entry = 12 + index * 12;
                let leaf_lo = u64::from(le_u32(node, entry + 4)?);
                let leaf_hi = u64::from(le_u16(node, entry + 8)?);
                let leaf = (leaf_hi << 32) | leaf_lo;
                let block = self.read_block(leaf)?;
                self.read_extent_range(&block, file_offset, output, depth_seen + 1)?;
            }
            return Ok(());
        }

        let request_end = file_offset
            .checked_add(output.len() as u64)
            .ok_or(Ext4Error::AddressOverflow)?;
        for index in 0..entries {
            let entry = 12 + index * 12;
            let logical = u64::from(le_u32(node, entry)?);
            let raw_len = le_u16(node, entry + 4)?;
            let (len_blocks, unwritten) = Self::decode_extent_len(raw_len);
            if unwritten {
                continue;
            }
            if len_blocks == 0 {
                continue;
            }
            let start_hi = u64::from(le_u16(node, entry + 6)?);
            let start_lo = u64::from(le_u32(node, entry + 8)?);
            let physical = (start_hi << 32) | start_lo;
            let extent_start = logical
                .checked_mul(self.block_size)
                .ok_or(Ext4Error::AddressOverflow)?;
            let extent_end = extent_start
                .checked_add(
                    len_blocks
                        .checked_mul(self.block_size)
                        .ok_or(Ext4Error::AddressOverflow)?,
                )
                .ok_or(Ext4Error::AddressOverflow)?;
            let overlap_start = extent_start.max(file_offset);
            let overlap_end = extent_end.min(request_end);
            if overlap_start >= overlap_end {
                continue;
            }
            let disk_offset = physical
                .checked_mul(self.block_size)
                .and_then(|base| base.checked_add(overlap_start - extent_start))
                .ok_or(Ext4Error::AddressOverflow)?;
            let output_start = usize::try_from(overlap_start - file_offset)
                .map_err(|_| Ext4Error::AddressOverflow)?;
            let count = usize::try_from(overlap_end - overlap_start)
                .map_err(|_| Ext4Error::AddressOverflow)?;
            read_exact(
                &self.device,
                disk_offset,
                &mut output[output_start..output_start + count],
            )?;
        }
        Ok(())
    }

    fn decode_extent_len(raw_len: u16) -> (u64, bool) {
        const EXT4_INIT_MAX_LEN: u16 = 0x8000;
        if raw_len <= EXT4_INIT_MAX_LEN {
            (u64::from(raw_len), false)
        } else {
            (u64::from(raw_len - EXT4_INIT_MAX_LEN), true)
        }
    }

    fn copy_extent(
        &self,
        logical: u64,
        len_blocks: u64,
        physical: u64,
        output: &mut [u8],
    ) -> Result<(), Ext4Error> {
        let file_start = logical
            .checked_mul(self.block_size)
            .ok_or(Ext4Error::AddressOverflow)?;
        let bytes = len_blocks
            .checked_mul(self.block_size)
            .ok_or(Ext4Error::AddressOverflow)?;
        let file_end = file_start
            .checked_add(bytes)
            .ok_or(Ext4Error::AddressOverflow)?;
        if file_start >= output.len() as u64 {
            return Ok(());
        }
        let copy_end = file_end.min(output.len() as u64);
        let mut cursor = file_start;
        while cursor < copy_end {
            let block_index = (cursor - file_start) / self.block_size;
            let physical_block = physical
                .checked_add(block_index)
                .ok_or(Ext4Error::AddressOverflow)?;
            let block_offset = (cursor % self.block_size) as usize;
            let count = (self.block_size as usize - block_offset).min((copy_end - cursor) as usize);
            let block = self.read_block(physical_block)?;
            let out_start = usize::try_from(cursor).map_err(|_| Ext4Error::AddressOverflow)?;
            output[out_start..out_start + count]
                .copy_from_slice(&block[block_offset..block_offset + count]);
            cursor = cursor
                .checked_add(count as u64)
                .ok_or(Ext4Error::AddressOverflow)?;
        }
        Ok(())
    }

    fn read_inode(&self, ino: u32) -> Result<Ext4Inode, Ext4Error> {
        if ino == 0 {
            return Err(Ext4Error::BadInode);
        }
        if let Some(inode) = self.metadata.lock().inodes.get(&ino).cloned() {
            return Ok(inode);
        }
        let group = (ino - 1) / self.inodes_per_group;
        let index = (ino - 1) % self.inodes_per_group;
        let inode_table_block = self.inode_table_block(group)?;
        let inode_offset = inode_table_block
            .checked_mul(self.block_size)
            .and_then(|base| base.checked_add(u64::from(index) * u64::from(self.inode_size)))
            .ok_or(Ext4Error::AddressOverflow)?;
        let mut raw = Vec::new();
        let inode_size = usize::from(self.inode_size);
        raw.try_reserve(inode_size)
            .map_err(|_| Ext4Error::OutOfMemory)?;
        raw.resize(inode_size, 0);
        read_exact(&self.device, inode_offset, &mut raw)?;
        let mode = le_u16(&raw, 0)?;
        let size_lo = u64::from(le_u32(&raw, 4)?);
        let size_hi = if raw.len() >= 112 {
            u64::from(le_u32(&raw, 108)?)
        } else {
            0
        };
        let size = size_lo | (size_hi << 32);
        let flags = le_u32(&raw, 32)?;
        let unsupported_inode_flags = flags & (EXT4_ENCRYPT_FL | EXT4_INLINE_DATA_FL);
        if unsupported_inode_flags != 0 {
            crate::println!(
                "ext4-inode: unsupported ino={} flags={:#010x} mask={:#010x}",
                ino,
                flags,
                unsupported_inode_flags,
            );
            return Err(Ext4Error::Unsupported);
        }
        let mut block = [0_u8; EXT4_N_BLOCKS_BYTES];
        block.copy_from_slice(raw.get(40..100).ok_or(Ext4Error::BadInode)?);
        let inode = Ext4Inode {
            ino,
            mode,
            size,
            flags,
            block,
        };
        self.metadata
            .lock()
            .inodes
            .entry(ino)
            .or_insert_with(|| inode.clone());
        Ok(inode)
    }

    fn group_has_superblock(&self, group: u32) -> bool {
        if self.feature_compat & EXT4_FEATURE_COMPAT_SPARSE_SUPER2 != 0 {
            return group == 0 || group == self.backup_bgs[0] || group == self.backup_bgs[1];
        }

        if self.feature_ro_compat & EXT4_FEATURE_RO_COMPAT_SPARSE_SUPER == 0 {
            return true;
        }

        group == 0
            || group == 1
            || ext4_exact_power(group, 3)
            || ext4_exact_power(group, 5)
            || ext4_exact_power(group, 7)
    }

    fn inode_table_block(&self, group: u32) -> Result<u64, Ext4Error> {
        if self.blocks_per_group == 0 {
            return Err(Ext4Error::InvalidSuperblock);
        }
        if let Some(block) = self.metadata.lock().inode_tables.get(&group).copied() {
            return Ok(block);
        }

        let descriptors_per_block = self
            .block_size
            .checked_div(u64::from(self.group_desc_size))
            .filter(|count| *count != 0)
            .ok_or(Ext4Error::BadGroupDescriptor)?;
        let descriptor_block_index = u64::from(group) / descriptors_per_block;
        let descriptor_index = u64::from(group) % descriptors_per_block;

        let descriptor_table_block = if self.feature_incompat & EXT4_FEATURE_INCOMPAT_META_BG == 0
            || descriptor_block_index < u64::from(self.first_meta_bg)
        {
            u64::from(self.first_data_block)
                .checked_add(1)
                .and_then(|base| base.checked_add(descriptor_block_index))
                .ok_or(Ext4Error::AddressOverflow)?
        } else {
            let meta_group = descriptor_block_index
                .checked_mul(descriptors_per_block)
                .ok_or(Ext4Error::AddressOverflow)?;
            let meta_group_u32 =
                u32::try_from(meta_group).map_err(|_| Ext4Error::AddressOverflow)?;
            let group_first = u64::from(self.first_data_block)
                .checked_add(
                    meta_group
                        .checked_mul(u64::from(self.blocks_per_group))
                        .ok_or(Ext4Error::AddressOverflow)?,
                )
                .ok_or(Ext4Error::AddressOverflow)?;
            group_first
                .checked_add(u64::from(self.group_has_superblock(meta_group_u32)))
                .ok_or(Ext4Error::AddressOverflow)?
        };

        let descriptor_offset = descriptor_table_block
            .checked_mul(self.block_size)
            .and_then(|base| base.checked_add(descriptor_index * u64::from(self.group_desc_size)))
            .ok_or(Ext4Error::AddressOverflow)?;

        let mut descriptor = Vec::new();
        let descriptor_size = usize::from(self.group_desc_size);
        descriptor
            .try_reserve(descriptor_size)
            .map_err(|_| Ext4Error::OutOfMemory)?;
        descriptor.resize(descriptor_size, 0);
        read_exact(&self.device, descriptor_offset, &mut descriptor)?;

        let lo = u64::from(le_u32(&descriptor, 8)?);
        let hi = if descriptor_size >= 44 {
            u64::from(le_u32(&descriptor, 40)?)
        } else {
            0
        };
        let block = lo | (hi << 32);
        if block == 0 {
            crate::println!(
                "ext4-gdt: invalid group={} desc_block={} desc_index={} offset={:#x}",
                group,
                descriptor_table_block,
                descriptor_index,
                descriptor_offset,
            );
            return Err(Ext4Error::BadGroupDescriptor);
        }

        self.metadata
            .lock()
            .inode_tables
            .entry(group)
            .or_insert(block);
        Ok(block)
    }

    fn read_block(&self, block: u64) -> Result<Vec<u8>, Ext4Error> {
        let size = usize::try_from(self.block_size).map_err(|_| Ext4Error::AddressOverflow)?;
        let mut data = Vec::new();
        data.try_reserve(size).map_err(|_| Ext4Error::OutOfMemory)?;
        data.resize(size, 0);
        let offset = block
            .checked_mul(self.block_size)
            .ok_or(Ext4Error::AddressOverflow)?;
        read_exact(&self.device, offset, &mut data)?;
        Ok(data)
    }
}

impl Ext4Inode {
    const fn file_type(&self) -> u16 {
        self.mode & EXT4_S_IFMT
    }
}

fn linux_mode(mode: u16) -> u32 {
    u32::from(mode)
}

fn read_exact(
    device: &Arc<dyn BlockDevice>,
    offset: u64,
    output: &mut [u8],
) -> Result<(), Ext4Error> {
    let read = block::read_at(device, offset, output).map_err(Ext4Error::BlockIo)?;
    if read != output.len() {
        return Err(Ext4Error::BadBlockSize);
    }
    Ok(())
}

fn le_u16(data: &[u8], offset: usize) -> Result<u16, Ext4Error> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or(Ext4Error::AddressOverflow)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn le_u32(data: &[u8], offset: usize) -> Result<u32, Ext4Error> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or(Ext4Error::AddressOverflow)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}
