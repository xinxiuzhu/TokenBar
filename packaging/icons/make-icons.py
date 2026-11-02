#!/usr/bin/env python3
"""Rasterize the locked TokenBar mark into platform icon files.

Source of truth is assets/logo.svg (navy #0B1B3A, white ring, yellow #F5C518).
This script only scales that SVG. It does not redraw the mark.

Outputs:
  assets/AppIcon.icns   macOS icon (PNG chunks, including @2x sizes)
  assets/tokenbar.ico   Windows multi-size icon (PNG-in-ICO)
  assets/tokenbar.png   512px PNG for the Linux desktop file

Regenerate (needs rsvg-convert):
  python3 packaging/icons/make-icons.py

Check the committed files, Info.plist wiring, and Cargo bundle keys:
  python3 packaging/icons/make-icons.py --verify
"""

from __future__ import annotations

import hashlib
import struct
import subprocess
import sys
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SVG = ROOT / "assets" / "logo.svg"
ICNS_PATH = ROOT / "assets" / "AppIcon.icns"
ICO_PATH = ROOT / "assets" / "tokenbar.ico"
PNG_PATH = ROOT / "assets" / "tokenbar.png"

# Pin the locked mark. A design change has to update this hash and the binaries together.
SVG_SHA256 = "e43cbdcdd66ab116634115f334b019820f52e589c8cd5c3c27c66ecee3886494"

# OSType -> pixel size. @2x entries repeat a size under the Retina type code.
ICNS_CHUNKS = (
    ("icp4", 16),    # 16
    ("ic11", 32),    # 16@2x
    ("icp5", 32),    # 32
    ("ic12", 64),    # 32@2x
    ("icp6", 64),    # 64
    ("ic07", 128),   # 128
    ("ic13", 256),   # 128@2x
    ("ic08", 256),   # 256
    ("ic14", 512),   # 256@2x
    ("ic09", 512),   # 512
    ("ic10", 1024),  # 512@2x
)

ICO_SIZES = (16, 24, 32, 48, 64, 128, 256)
LINUX_PNG_SIZE = 512

NAVY = (0x0B, 0x1B, 0x3A, 255)
WHITE = (255, 255, 255, 255)
YELLOW = (0xF5, 0xC5, 0x18, 255)


def rasterize(size: int) -> bytes:
    proc = subprocess.run(
        ["rsvg-convert", "-w", str(size), "-h", str(size), str(SVG)],
        check=True,
        capture_output=True,
    )
    blob = proc.stdout
    if not blob.startswith(b"\x89PNG\r\n\x1a\n"):
        raise SystemExit(f"rsvg-convert did not return a PNG for {size}px")
    return blob


def pack_icns(pngs: dict[int, bytes]) -> bytes:
    chunks = []
    for ostype, size in ICNS_CHUNKS:
        blob = pngs[size]
        chunks.append(ostype.encode("ascii") + struct.pack(">I", 8 + len(blob)) + blob)
    body = b"".join(chunks)
    return b"icns" + struct.pack(">I", 8 + len(body)) + body


def pack_ico(pngs: dict[int, bytes]) -> bytes:
    count = len(ICO_SIZES)
    header = struct.pack("<HHH", 0, 1, count)
    offset = 6 + 16 * count
    entries = []
    blobs = []
    for size in ICO_SIZES:
        blob = pngs[size]
        dim = 0 if size >= 256 else size
        entries.append(struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(blob), offset))
        blobs.append(blob)
        offset += len(blob)
    return header + b"".join(entries) + b"".join(blobs)


def paeth(left: int, up: int, upper_left: int) -> int:
    estimate = left + up - upper_left
    pa = abs(estimate - left)
    pb = abs(estimate - up)
    pc = abs(estimate - upper_left)
    if pa <= pb and pa <= pc:
        return left
    if pb <= pc:
        return up
    return upper_left


def decode_png(data: bytes) -> tuple[int, int, bytes]:
    if not data.startswith(b"\x89PNG\r\n\x1a\n"):
        raise ValueError("not a png")
    pos = 8
    width = height = bit_depth = color_type = None
    idat = []
    while pos + 8 <= len(data):
        length = struct.unpack(">I", data[pos : pos + 4])[0]
        ctype = data[pos + 4 : pos + 8]
        chunk = data[pos + 8 : pos + 8 + length]
        pos += 12 + length
        if ctype == b"IHDR":
            width, height, bit_depth, color_type, comp, filt, interlace = struct.unpack(
                ">IIBBBBB", chunk
            )
            if (bit_depth, comp, filt, interlace) != (8, 0, 0, 0):
                raise ValueError(f"unsupported png header {bit_depth=} {comp=} {filt=} {interlace=}")
            if color_type not in (2, 6):
                raise ValueError(f"unsupported png color type {color_type}")
        elif ctype == b"IDAT":
            idat.append(chunk)
        elif ctype == b"IEND":
            break
    if width is None or height is None or color_type is None:
        raise ValueError("png missing IHDR")
    raw = zlib.decompress(b"".join(idat))
    channels = 4 if color_type == 6 else 3
    stride = width * channels
    rows = []
    index = 0
    prev = bytearray(stride)
    for _y in range(height):
        filter_type = raw[index]
        index += 1
        scan = bytearray(raw[index : index + stride])
        index += stride
        recon = bytearray(stride)
        for x, value in enumerate(scan):
            left = recon[x - channels] if x >= channels else 0
            up = prev[x]
            upper_left = prev[x - channels] if x >= channels else 0
            if filter_type == 1:
                value = (value + left) & 255
            elif filter_type == 2:
                value = (value + up) & 255
            elif filter_type == 3:
                value = (value + ((left + up) // 2)) & 255
            elif filter_type == 4:
                value = (value + paeth(left, up, upper_left)) & 255
            elif filter_type != 0:
                raise ValueError(f"bad png filter {filter_type}")
            recon[x] = value
        prev = recon
        rows.append(recon)
    rgba = bytearray(width * height * 4)
    out = 0
    for row in rows:
        if channels == 4:
            rgba[out : out + stride] = row
            out += stride
        else:
            for x in range(width):
                src = x * 3
                rgba[out : out + 4] = bytes((row[src], row[src + 1], row[src + 2], 255))
                out += 4
    return width, height, bytes(rgba)


def pixel(rgba: bytes, width: int, x: int, y: int) -> tuple[int, int, int, int]:
    i = (y * width + x) * 4
    return tuple(rgba[i : i + 4])  # type: ignore[return-value]


def assert_close(got: tuple[int, int, int, int], want: tuple[int, int, int, int], label: str) -> None:
    if any(abs(a - b) > 8 for a, b in zip(got, want)):
        raise SystemExit(f"{label}: pixel {got} != {want}")


def assert_mark(png: bytes, label: str) -> None:
    """Sample the locked geometry: navy field, white ring, yellow bar, clear corner."""
    width, height, rgba = decode_png(png)
    if width != height:
        raise SystemExit(f"{label}: expected a square, got {width}x{height}")
    if width < 128:
        return
    scale = width / 32.0

    def at(vx: float, vy: float) -> tuple[int, int, int, int]:
        x = min(width - 1, max(0, round(vx * scale)))
        y = min(height - 1, max(0, round(vy * scale)))
        return pixel(rgba, width, x, y)

    assert_close(at(16, 1.2), NAVY, f"{label} navy")
    assert_close(at(16, 4.4), WHITE, f"{label} ring")
    assert_close(at(16, 16), YELLOW, f"{label} bar")
    corner = pixel(rgba, width, 1, 1)
    if corner[3] > 8:
        raise SystemExit(f"{label}: corner should be transparent, got {corner}")


def iter_icns(data: bytes):
    if data[:4] != b"icns" or len(data) < 8:
        raise SystemExit("AppIcon.icns missing icns magic")
    total = struct.unpack(">I", data[4:8])[0]
    if total != len(data):
        raise SystemExit(f"AppIcon.icns length {len(data)} != header {total}")
    pos = 8
    while pos + 8 <= len(data):
        ostype = data[pos : pos + 4].decode("ascii")
        chunk_len = struct.unpack(">I", data[pos + 4 : pos + 8])[0]
        if chunk_len < 8 or pos + chunk_len > len(data):
            raise SystemExit(f"bad icns chunk {ostype}")
        yield ostype, data[pos + 8 : pos + chunk_len]
        pos += chunk_len
    if pos != len(data):
        raise SystemExit("AppIcon.icns has trailing bytes")


def iter_ico(data: bytes):
    if len(data) < 6:
        raise SystemExit("tokenbar.ico is too small")
    reserved, kind, count = struct.unpack_from("<HHH", data, 0)
    if (reserved, kind) != (0, 1) or count < 1:
        raise SystemExit(f"tokenbar.ico header {(reserved, kind, count)} is not an icon")
    entries = []
    for index in range(count):
        w, h, _colors, _res, planes, bitcount, nbytes, offset = struct.unpack_from(
            "<BBBBHHII", data, 6 + 16 * index
        )
        width = 256 if w == 0 else w
        height = 256 if h == 0 else h
        blob = data[offset : offset + nbytes]
        entries.append((width, height, planes, bitcount, blob))
    return entries


def must_contain(path: Path, needles: tuple[str, ...]) -> None:
    text = path.read_text(encoding="utf-8")
    for needle in needles:
        if needle not in text:
            raise SystemExit(f"{path.relative_to(ROOT)} does not reference {needle!r}")


def verify() -> None:
    digest = hashlib.sha256(SVG.read_bytes()).hexdigest()
    if digest != SVG_SHA256:
        raise SystemExit("assets/logo.svg changed; regenerate icons with make-icons.py")

    found = {ostype: blob for ostype, blob in iter_icns(ICNS_PATH.read_bytes())}
    for ostype, size in ICNS_CHUNKS:
        blob = found.get(ostype)
        if blob is None:
            raise SystemExit(f"AppIcon.icns missing {ostype}")
        width, height, _rgba = decode_png(blob)
        if (width, height) != (size, size):
            raise SystemExit(f"{ostype} is {width}x{height}, expected {size}")
    assert_mark(found["ic10"], "ic10")

    entries = iter_ico(ICO_PATH.read_bytes())
    got_sizes = []
    for width, height, planes, bitcount, blob in entries:
        if planes != 1 or bitcount != 32:
            raise SystemExit(f"ico {width}x{height} planes/bitcount {planes}/{bitcount}")
        decoded_w, decoded_h, _rgba = decode_png(blob)
        if (decoded_w, decoded_h) != (width, height):
            raise SystemExit(f"ico entry {width} claims png {decoded_w}x{decoded_h}")
        got_sizes.append(width)
    if tuple(got_sizes) != ICO_SIZES:
        raise SystemExit(f"ico sizes {got_sizes} != {list(ICO_SIZES)}")
    largest = max(entries, key=lambda item: item[0])
    assert_mark(largest[4], "ico")

    width, height, _rgba = decode_png(PNG_PATH.read_bytes())
    if (width, height) != (LINUX_PNG_SIZE, LINUX_PNG_SIZE):
        raise SystemExit(f"tokenbar.png is {width}x{height}")
    assert_mark(PNG_PATH.read_bytes(), "tokenbar.png")

    must_contain(
        ROOT / "packaging" / "macos" / "package.sh",
        (
            "assets/AppIcon.icns",
            "Contents/Resources/AppIcon.icns",
            "<key>CFBundleIconFile</key>",
            "<string>AppIcon</string>",
        ),
    )
    must_contain(
        ROOT / "crates" / "tokenbar" / "Cargo.toml",
        (
            "[package.metadata.bundle]",
            "../../assets/AppIcon.icns",
            "../../assets/tokenbar.ico",
            "../../assets/tokenbar.png",
            "[package.metadata.wix]",
            'product-icon = "../../assets/tokenbar.ico"',
        ),
    )
    must_contain(
        ROOT / "crates" / "tokenbar" / "build.rs",
        ("tokenbar.ico", "set_icon"),
    )
    must_contain(
        ROOT / ".github" / "workflows" / "release.yml",
        ("assets/tokenbar.png", "assets/tokenbar.ico", "AppIcon.icns"),
    )
    must_contain(
        ROOT / "packaging" / "linux" / "app.tokenbar.TokenBar.desktop",
        ("Icon=tokenbar", "Name=TokenBar"),
    )
    print("icons ok")
    print(f"  {ICNS_PATH.relative_to(ROOT)} ({ICNS_PATH.stat().st_size} bytes)")
    print(f"  {ICO_PATH.relative_to(ROOT)} ({ICO_PATH.stat().st_size} bytes)")
    print(f"  {PNG_PATH.relative_to(ROOT)} ({PNG_PATH.stat().st_size} bytes)")


def generate() -> None:
    digest = hashlib.sha256(SVG.read_bytes()).hexdigest()
    if digest != SVG_SHA256:
        raise SystemExit(
            f"assets/logo.svg sha256 is {digest}, expected the locked mark {SVG_SHA256}"
        )
    sizes = sorted(set([size for _kind, size in ICNS_CHUNKS] + list(ICO_SIZES) + [LINUX_PNG_SIZE]))
    pngs = {size: rasterize(size) for size in sizes}
    assert_mark(pngs[1024], "raster 1024")
    ICNS_PATH.write_bytes(pack_icns(pngs))
    ICO_PATH.write_bytes(pack_ico(pngs))
    PNG_PATH.write_bytes(pngs[LINUX_PNG_SIZE])
    verify()


def main() -> None:
    if "--verify" in sys.argv:
        verify()
    else:
        generate()


if __name__ == "__main__":
    main()
