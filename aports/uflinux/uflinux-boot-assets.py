#!/usr/bin/env python3
"""Derive the Plymouth boot assets from the pinned original UFLinux splash.

Only the standard library is used so packaging needs no image toolkit: the
imported original is an unmodified 8-bit RGBA PNG, which is decoded directly.
The white artwork is kept pixel-for-pixel and the black background becomes
transparency, so the same files compose over a firmware logo or a solid colour.
"""
import argparse
import struct
import sys
import zlib
from pathlib import Path

# Content boxes measured on the pinned original: the leaf mark and the logotype,
# separated by the empty column band between them.
LEAF = (37, 10, 174, 225)
WORDMARK = (271, 90, 331, 79)


def read_png(path):
    data = path.read_bytes()
    if not data.startswith(b"\x89PNG\r\n\x1a\n"):
        raise SystemExit(f"{path}: not a PNG file")
    pos, width, height, channels, scanlines = 8, None, None, None, bytearray()
    while pos + 8 <= len(data):
        length, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + length]
        if struct.unpack(">I", data[pos + 8 + length:pos + 12 + length])[0] != zlib.crc32(kind + body):
            raise SystemExit(f"{path}: corrupt chunk {kind!r}")
        if kind == b"IHDR":
            width, height, depth, colour, compression, filter_method, interlace = struct.unpack(">IIBBBBB", body)
            if (depth, colour, compression, filter_method, interlace) != (8, 6, 0, 0, 0):
                raise SystemExit(f"{path}: expected an unmodified 8-bit RGBA PNG")
            channels = 4
        elif kind == b"IDAT":
            scanlines += body
        elif kind == b"IEND":
            break
        pos += 12 + length
    else:
        raise SystemExit(f"{path}: PNG has no IEND chunk")
    raw = zlib.decompress(bytes(scanlines))
    stride = width * channels
    rows, previous, offset = [], bytearray(stride), 0
    for y in range(height):
        filter_type, row = raw[offset], bytearray(raw[offset + 1:offset + 1 + stride])
        offset += 1 + stride
        if filter_type == 0:
            pass
        elif filter_type == 1:
            for i in range(channels, stride):
                row[i] = (row[i] + row[i - channels]) & 0xFF
        elif filter_type == 2:
            for i in range(stride):
                row[i] = (row[i] + previous[i]) & 0xFF
        elif filter_type == 3:
            for i in range(stride):
                left = row[i - channels] if i >= channels else 0
                row[i] = (row[i] + ((left + previous[i]) >> 1)) & 0xFF
        elif filter_type == 4:
            for i in range(stride):
                a = row[i - channels] if i >= channels else 0
                b = previous[i]
                c = previous[i - channels] if i >= channels else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                predictor = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                row[i] = (row[i] + predictor) & 0xFF
        else:
            raise SystemExit(f"{path}: unknown PNG filter {filter_type}")
        rows.append(bytes(row))
        previous = row
    return width, height, rows


def write_png(path, width, height, pixels):
    stride = width * 4
    raw = b"".join(b"\x00" + pixels[y * stride:(y + 1) * stride] for y in range(height))

    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

    path.write_bytes(b"\x89PNG\r\n\x1a\n"
                    + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
                    + chunk(b"IDAT", zlib.compress(raw, 9))
                    + chunk(b"IEND", b""))


def cut_to_alpha(source, box, destination):
    """Copy a box of the original, turning its black background transparent."""
    width, height, rows = read_png(source)
    x, y, box_width, box_height = box
    if width < x + box_width or height < y + box_height:
        raise SystemExit(f"{source}: crop box {box} falls outside {width}x{height}")
    pixels = bytearray()
    for row in range(y, y + box_height):
        for column in range(x, x + box_width):
            offset = column * 4
            red, green, blue = rows[row][offset:offset + 3]
            # The original artwork is white on black, so its brightness is coverage.
            coverage = (red * 299 + green * 587 + blue * 114 + 500) // 1000
            pixels += bytes((255, 255, 255, coverage))
    write_png(destination, box_width, box_height, pixels)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("original", type=Path, help="pinned original splash PNG")
    parser.add_argument("--outdir", type=Path, required=True)
    arguments = parser.parse_args()
    arguments.outdir.mkdir(parents=True, exist_ok=True)
    cut_to_alpha(arguments.original, LEAF, arguments.outdir / "bgrt-fallback.png")
    cut_to_alpha(arguments.original, WORDMARK, arguments.outdir / "watermark.png")
    return 0


if __name__ == "__main__":
    sys.exit(main())
