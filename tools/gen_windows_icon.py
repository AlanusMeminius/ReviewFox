"""Bake a Windows taskbar ICO with Dock-style rounded-rect corners.

Reads the full-bleed square `assets/icon.ico`, insets each size by 10% and
clips to a rounded rect (corner radius ~22.37% of the artwork — same constants
as `src/ui/app_icon.rs`), then writes `assets/icon-windows.ico`.

Usage, from the repo root:

    python tools/gen_windows_icon.py
"""

from __future__ import annotations

import io
import struct
import sys
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "assets" / "icon.ico"
DST = ROOT / "assets" / "icon-windows.ico"

# Match src/ui/app_icon.rs
INSET_FRAC = 0.10
CORNER_FRAC = 0.2237
MASK_SCALE = 4


def load_ico_pngs(path: Path) -> list[Image.Image]:
    data = path.read_bytes()
    count = struct.unpack_from("<H", data, 4)[0]
    images: list[Image.Image] = []
    off = 6
    for _ in range(count):
        w, h, _ncolors, _res, _planes, _bpp, size, offset = struct.unpack_from(
            "<BBBBHHII", data, off
        )
        w = 256 if w == 0 else w
        h = 256 if h == 0 else h
        chunk = data[offset : offset + size]
        if chunk[:8] != b"\x89PNG\r\n\x1a\n":
            raise SystemExit(f"expected PNG frame in {path}, got non-PNG {w}x{h}")
        im = Image.open(io.BytesIO(chunk)).convert("RGBA")
        if im.size != (w, h):
            im = im.resize((w, h), Image.Resampling.LANCZOS)
        images.append(im)
        off += 16
    return images


def rounded_mask(size: int) -> Image.Image:
    """Antialiased rounded-rect mask matching the Dock inset geometry."""
    big = size * MASK_SCALE
    inset = big * INSET_FRAC
    art = big - 2.0 * inset
    radius = art * CORNER_FRAC
    mask = Image.new("L", (big, big), 0)
    draw = ImageDraw.Draw(mask)
    box = (inset, inset, big - inset - 1, big - inset - 1)
    draw.rounded_rectangle(box, radius=radius, fill=255)
    return mask.resize((size, size), Image.Resampling.LANCZOS)


def dock_style(src: Image.Image) -> Image.Image:
    size = src.size[0]
    assert src.size[0] == src.size[1], src.size
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    inset = size * INSET_FRAC
    art = max(1, round(size - 2.0 * inset))
    x0 = round(inset)
    y0 = round(inset)
    scaled = src.resize((art, art), Image.Resampling.LANCZOS)
    canvas.paste(scaled, (x0, y0), scaled)
    r, g, b, a = canvas.split()
    a = ImageChops.multiply(a, rounded_mask(size))
    return Image.merge("RGBA", (r, g, b, a))


def png_bytes(im: Image.Image) -> bytes:
    buf = io.BytesIO()
    im.save(buf, format="PNG")
    return buf.getvalue()


def write_ico(path: Path, images: list[Image.Image]) -> None:
    """Write a PNG-compressed ICO (same layout as assets/icon.ico)."""
    frames = [png_bytes(im) for im in images]
    count = len(frames)
    # 6-byte header + 16-byte directory entry per image
    offset = 6 + 16 * count
    entries = bytearray()
    entries += struct.pack("<HHH", 0, 1, count)
    body = bytearray()
    for im, png in zip(images, frames):
        w, h = im.size
        entry_w = 0 if w >= 256 else w
        entry_h = 0 if h >= 256 else h
        entries += struct.pack(
            "<BBBBHHII",
            entry_w,
            entry_h,
            0,
            0,
            1,
            32,
            len(png),
            offset + len(body),
        )
        body += png
    path.write_bytes(bytes(entries) + bytes(body))


def main() -> int:
    if not SRC.is_file():
        print(f"missing {SRC}", file=sys.stderr)
        return 1
    frames = [dock_style(im) for im in load_ico_pngs(SRC)]
    write_ico(DST, frames)
    # Spot-check: corners of every size must be fully transparent.
    for im in frames:
        px = im.load()
        w, h = im.size
        for x, y in ((0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)):
            if px[x, y][3] != 0:
                print(
                    f"{w}x{h} corner ({x},{y}) alpha={px[x, y][3]}",
                    file=sys.stderr,
                )
                return 1
    print(f"wrote {DST.relative_to(ROOT)} ({len(frames)} sizes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
