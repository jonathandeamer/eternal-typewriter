#!/usr/bin/env python3
"""Read a scroll disk image, emit the prose. Validates exactly like the
kernel: magic, seq==LBA, length, CRC-32 (zlib's crc32 is the same IEEE
polynomial as scroll-core's). Separator marker bytes (0x1E) are stripped."""
import struct
import sys
import zlib

MAGIC = b"ETYP"
PAYLOAD_CAPACITY = 494


def payloads(path):
    with open(path, "rb") as f:
        lba = 0
        while True:
            sector = f.read(512)
            if len(sector) < 512 or sector[:4] != MAGIC:
                return
            (seq,) = struct.unpack_from("<Q", sector, 4)
            (length,) = struct.unpack_from("<H", sector, 12)
            (crc,) = struct.unpack_from("<I", sector, 14)
            # An invalid record marks the end of the valid prefix — exactly
            # how the kernel's record::decode treats it. A torn tail sector
            # (bad CRC after power loss) must end the stream gracefully, not
            # discard all the good prose written before it.
            if seq != lba or length > PAYLOAD_CAPACITY:
                return
            payload = sector[18 : 18 + length]
            if zlib.crc32(payload) & 0xFFFFFFFF != crc:
                return
            yield payload
            lba += 1


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: extract.py scroll.img")
    try:
        data = b"".join(payloads(sys.argv[1])).replace(b"\x1e", b"")
        sys.stdout.buffer.write(data)
    except Exception as e:
        sys.exit(f"Error: {e}")


if __name__ == "__main__":
    main()
