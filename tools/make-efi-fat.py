#!/usr/bin/env python3
"""Create the dependency-free FAT16 El Torito image used by Genesis."""

import pathlib
import struct
import sys

SECTOR = 512
TOTAL_SECTORS = 65536
RESERVED = 1
FATS = 2
FAT_SECTORS = 128
ROOT_ENTRIES = 512
ROOT_SECTORS = ROOT_ENTRIES * 32 // SECTOR
SECTORS_PER_CLUSTER = 2
DATA_START = RESERVED + FATS * FAT_SECTORS + ROOT_SECTORS


def put16(image: bytearray, offset: int, value: int) -> None:
    struct.pack_into("<H", image, offset, value)


def put32(image: bytearray, offset: int, value: int) -> None:
    struct.pack_into("<I", image, offset, value)


def directory_entry(
    image: bytearray,
    offset: int,
    name: bytes,
    attributes: int,
    cluster: int,
    size: int = 0,
) -> None:
    if len(name) != 11:
        raise ValueError("FAT short name must contain exactly 11 bytes")
    image[offset : offset + 11] = name
    image[offset + 11] = attributes
    put16(image, offset + 26, cluster)
    put32(image, offset + 28, size)


def cluster_offset(cluster: int) -> int:
    return (DATA_START + (cluster - 2) * SECTORS_PER_CLUSTER) * SECTOR


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: make-efi-fat.py BOOTX64.EFI OUTPUT.img", file=sys.stderr)
        return 2
    app = pathlib.Path(sys.argv[1]).read_bytes()
    cluster_bytes = SECTORS_PER_CLUSTER * SECTOR
    file_clusters = (len(app) + cluster_bytes - 1) // cluster_bytes
    max_clusters = (TOTAL_SECTORS - DATA_START) // SECTORS_PER_CLUSTER
    if file_clusters + 3 > max_clusters:
        raise SystemExit("BOOTX64.EFI does not fit in the Genesis EFI image")

    image = bytearray(TOTAL_SECTORS * SECTOR)
    image[0:3] = b"\xEB\x3C\x90"
    image[3:11] = b"EXPOS   "
    put16(image, 11, SECTOR)
    image[13] = SECTORS_PER_CLUSTER
    put16(image, 14, RESERVED)
    image[16] = FATS
    put16(image, 17, ROOT_ENTRIES)
    put16(image, 19, 0)
    image[21] = 0xF8
    put16(image, 22, FAT_SECTORS)
    put16(image, 24, 32)
    put16(image, 26, 64)
    put32(image, 32, TOTAL_SECTORS)
    image[36] = 0x80
    image[38] = 0x29
    put32(image, 39, 0x4558504F)
    image[43:54] = b"EXPOS BOOT "
    image[54:62] = b"FAT16   "
    image[510:512] = b"\x55\xAA"

    fat = bytearray(FAT_SECTORS * SECTOR)
    put16(fat, 0, 0xFFF8)
    put16(fat, 2, 0xFFFF)
    for cluster in (2, 3):
        put16(fat, cluster * 2, 0xFFFF)
    for cluster in range(4, 4 + file_clusters):
        put16(fat, cluster * 2, 0xFFFF if cluster + 1 == 4 + file_clusters else cluster + 1)
    for index in range(FATS):
        start = (RESERVED + index * FAT_SECTORS) * SECTOR
        image[start : start + len(fat)] = fat

    root = (RESERVED + FATS * FAT_SECTORS) * SECTOR
    directory_entry(image, root, b"EFI        ", 0x10, 2)
    efi = cluster_offset(2)
    directory_entry(image, efi, b".          ", 0x10, 2)
    directory_entry(image, efi + 32, b"..         ", 0x10, 0)
    directory_entry(image, efi + 64, b"BOOT       ", 0x10, 3)
    boot = cluster_offset(3)
    directory_entry(image, boot, b".          ", 0x10, 3)
    directory_entry(image, boot + 32, b"..         ", 0x10, 2)
    directory_entry(image, boot + 64, b"BOOTX64 EFI", 0x20, 4, len(app))
    app_offset = cluster_offset(4)
    image[app_offset : app_offset + len(app)] = app
    pathlib.Path(sys.argv[2]).write_bytes(image)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
