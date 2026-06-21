#!/usr/bin/env python3
import argparse
import stat
import struct
import sys


SUPER_OFFSET = 1024
SUPER_SIZE = 1024
EXT4_MAGIC = 0xEF53
ROOT_INO = 2
EXTENTS_FL = 0x00080000
EXTENT_MAGIC = 0xF30A
S_IFMT = 0o170000
S_IFDIR = 0o040000
S_IFREG = 0o100000
S_IFLNK = 0o120000


class Ext4Error(Exception):
    pass


def le16(data, off):
    return struct.unpack_from("<H", data, off)[0]


def le32(data, off):
    return struct.unpack_from("<I", data, off)[0]


class Ext4:
    def __init__(self, path):
        self.image = open(path, "rb")
        sb = self.read_at(SUPER_OFFSET, SUPER_SIZE)
        if le16(sb, 56) != EXT4_MAGIC:
            raise Ext4Error("bad ext4 magic")
        log_block_size = le32(sb, 24)
        self.block_size = 1024 << log_block_size
        self.blocks_per_group = le32(sb, 32)
        self.inodes_per_group = le32(sb, 40)
        self.inode_size = max(le16(sb, 88), 128)
        feature_incompat = le32(sb, 96)
        raw_desc_size = le16(sb, 254)
        self.group_desc_size = max(raw_desc_size, 32) if feature_incompat & 0x80 else 32

    def read_at(self, off, size):
        self.image.seek(off)
        data = self.image.read(size)
        if len(data) != size:
            raise Ext4Error(f"short read at {off:#x}")
        return data

    def read_block(self, block):
        return self.read_at(block * self.block_size, self.block_size)

    def inode_table_block(self, group):
        desc_table_block = 2 if self.block_size == 1024 else 1
        desc = self.read_at(
            desc_table_block * self.block_size + group * self.group_desc_size,
            self.group_desc_size,
        )
        lo = le32(desc, 8)
        hi = le32(desc, 40) if self.group_desc_size >= 44 else 0
        block = lo | (hi << 32)
        if block == 0:
            raise Ext4Error("bad group descriptor")
        return block

    def read_inode(self, ino):
        group = (ino - 1) // self.inodes_per_group
        index = (ino - 1) % self.inodes_per_group
        table = self.inode_table_block(group)
        raw = self.read_at(table * self.block_size + index * self.inode_size, self.inode_size)
        mode = le16(raw, 0)
        size = le32(raw, 4) | (le32(raw, 108) << 32 if len(raw) >= 112 else 0)
        flags = le32(raw, 32)
        block = raw[40:100]
        return {"ino": ino, "mode": mode, "size": size, "flags": flags, "block": block}

    def read_inode_bytes(self, inode):
        size = inode["size"]
        if size == 0:
            return b""
        if inode["flags"] & EXTENTS_FL == 0:
            raise Ext4Error("non-extent inode unsupported")
        out = bytearray(size)
        self.read_extent_bytes(inode["block"], out, 0)
        return bytes(out)

    def read_extent_bytes(self, node, out, depth_seen):
        if len(node) < 12 or le16(node, 0) != EXTENT_MAGIC:
            raise Ext4Error("bad extent tree")
        entries = le16(node, 2)
        max_entries = le16(node, 4)
        depth = le16(node, 6)
        if entries > max_entries or 12 + entries * 12 > len(node):
            raise Ext4Error("bad extent header")
        if depth == 0:
            for idx in range(entries):
                off = 12 + idx * 12
                logical = le32(node, off)
                raw_len = le16(node, off + 4)
                length = raw_len & 0x7FFF
                start = le32(node, off + 8) | (le16(node, off + 6) << 32)
                self.copy_extent(logical, length, start, out)
            return
        for idx in range(entries):
            off = 12 + idx * 12
            leaf = le32(node, off + 4) | (le16(node, off + 8) << 32)
            self.read_extent_bytes(self.read_block(leaf), out, depth_seen + 1)

    def copy_extent(self, logical, length, physical, out):
        file_start = logical * self.block_size
        file_end = min(file_start + length * self.block_size, len(out))
        cursor = file_start
        while cursor < file_end:
            block_index = (cursor - file_start) // self.block_size
            block = self.read_block(physical + block_index)
            block_off = cursor % self.block_size
            count = min(self.block_size - block_off, file_end - cursor)
            out[cursor : cursor + count] = block[block_off : block_off + count]
            cursor += count

    def read_dir_entries(self, ino):
        inode = self.read_inode(ino)
        if inode["mode"] & S_IFMT != S_IFDIR:
            raise Ext4Error("not a directory")
        data = self.read_inode_bytes(inode)
        entries = []
        off = 0
        while off + 8 <= len(data):
            child_ino = le32(data, off)
            rec_len = le16(data, off + 4)
            name_len = data[off + 6]
            if rec_len < 8 or off + rec_len > len(data):
                raise Ext4Error("bad directory entry")
            name = data[off + 8 : off + 8 + name_len].decode("utf-8", "replace")
            if child_ino and name not in (".", ".."):
                entries.append((name, child_ino))
            off += rec_len
        return entries

    def lookup(self, path):
        ino = ROOT_INO
        for part in path.split("/"):
            if not part:
                continue
            for name, child_ino in self.read_dir_entries(ino):
                if name == part:
                    ino = child_ino
                    break
            else:
                raise Ext4Error(f"not found: {path}")
        return ino

    def read_path(self, path):
        inode = self.read_inode(self.lookup(path))
        mode = inode["mode"] & S_IFMT
        if mode == S_IFREG:
            return self.read_inode_bytes(inode)
        if mode == S_IFLNK:
            if inode["size"] <= 60 and inode["flags"] & EXTENTS_FL == 0:
                return inode["block"][: inode["size"]]
            return self.read_inode_bytes(inode)
        raise Ext4Error("path is not a regular file or symlink")

    def list_path(self, path):
        ino = self.lookup(path)
        rows = []
        for name, child_ino in self.read_dir_entries(ino):
            inode = self.read_inode(child_ino)
            mode = inode["mode"]
            rows.append((name, child_ino, mode, inode["size"]))
        return rows


def main():
    parser = argparse.ArgumentParser(description="small read-only ext4 image inspector")
    parser.add_argument("image")
    parser.add_argument("path")
    parser.add_argument("--ls", action="store_true")
    args = parser.parse_args()

    fs = Ext4(args.image)
    if args.ls:
        for name, ino, mode, size in fs.list_path(args.path):
            kind = stat.filemode(mode)
            print(f"{ino:8d} {kind} {size:8d} {name}")
    else:
        sys.stdout.buffer.write(fs.read_path(args.path))


if __name__ == "__main__":
    main()
