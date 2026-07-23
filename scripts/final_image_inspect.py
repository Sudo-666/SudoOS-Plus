#!/usr/bin/env python3
"""Read-only final image inspector for SudoOS final-2026 bring-up."""

import argparse
import os
import struct
import sys

EXT4_SUPER_OFFSET = 1024
EXT4_SUPER_SIZE = 1024
EXT4_MAGIC = 0xEF53


def le16(data, off):
    return struct.unpack_from("<H", data, off)[0]


def le32(data, off):
    return struct.unpack_from("<I", data, off)[0]


def read_super(path):
    with open(path, "rb") as image:
        image.seek(EXT4_SUPER_OFFSET)
        superblock = image.read(EXT4_SUPER_SIZE)
    if len(superblock) != EXT4_SUPER_SIZE:
        raise RuntimeError("short ext4 superblock")
    if le16(superblock, 56) != EXT4_MAGIC:
        raise RuntimeError("bad ext4 magic")
    return superblock


def uuid_text(raw):
    # ext4 stores UUID bytes in RFC4122 wire order.
    return (
        f"{raw[0:4].hex()}-{raw[4:6].hex()}-{raw[6:8].hex()}-"
        f"{raw[8:10].hex()}-{raw[10:16].hex()}"
    )


def main():
    parser = argparse.ArgumentParser(description="inspect an ext4 final image")
    parser.add_argument("image")
    args = parser.parse_args()

    sb = read_super(args.image)
    block_size = 1024 << le32(sb, 24)
    blocks_lo = le32(sb, 4)
    blocks_hi = le32(sb, 336) if len(sb) >= 340 else 0
    free_blocks_lo = le32(sb, 12)
    free_blocks_hi = le32(sb, 340) if len(sb) >= 344 else 0
    blocks = blocks_lo | (blocks_hi << 32)
    free_blocks = free_blocks_lo | (free_blocks_hi << 32)
    inodes = le32(sb, 0)
    free_inodes = le32(sb, 16)
    inode_size = max(le16(sb, 88), 128)
    volume = sb[120:136].split(b"\0", 1)[0].decode("utf-8", "replace")
    features_compat = le32(sb, 92)
    features_incompat = le32(sb, 96)
    features_ro_compat = le32(sb, 100)

    print(f"image={args.image}")
    print(f"size_bytes={os.path.getsize(args.image)}")
    print(f"uuid={uuid_text(sb[104:120])}")
    print(f"volume={volume}")
    print(f"block_size={block_size}")
    print(f"blocks={blocks}")
    print(f"free_blocks={free_blocks}")
    print(f"inodes={inodes}")
    print(f"free_inodes={free_inodes}")
    print(f"inode_size={inode_size}")
    print(f"blocks_per_group={le32(sb, 32)}")
    print(f"inodes_per_group={le32(sb, 40)}")
    print(f"feature_compat=0x{features_compat:08x}")
    print(f"feature_incompat=0x{features_incompat:08x}")
    print(f"feature_ro_compat=0x{features_ro_compat:08x}")


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"final_image_inspect.py: {exc}", file=sys.stderr)
        sys.exit(1)
