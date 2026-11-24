#!/usr/bin/env python3
"""Generate 100% Microsoft-standard multi-size Windows .ico (BMP DIB for <=128px, PNG for 256px), 128x128 raw RGBA, 256x256 PNG, and SVG from packaging/brand-icon.png."""
import base64
import io
import struct
from pathlib import Path
from PIL import Image

ROOT = Path(__file__).resolve().parent


def load_master_icon(size: int = 1024) -> Image.Image:
    master_path = ROOT / "brand-icon.png"
    master = Image.open(master_path).convert("RGBA")
    if master.size != (size, size):
        master = master.resize((size, size), Image.Resampling.LANCZOS)
    return master


def encode_dib_frame(im: Image.Image) -> bytes:
    """Encode an RGBA image into standard Windows ICO 32-bpp BGRA BITMAPINFOHEADER + XOR + AND mask."""
    w, h = im.size
    rgba = im.convert("RGBA")
    px = rgba.load()

    # BITMAPINFOHEADER (40 bytes): height is 2*h (XOR + AND mask)
    bih = struct.pack(
        "<IIIHHIIIIII",
        40,          # biSize
        w,           # biWidth
        h * 2,       # biHeight (XOR + AND combined)
        1,           # biPlanes
        32,          # biBitCount
        0,           # biCompression (BI_RGB)
        w * h * 4,   # biSizeImage
        0,           # biXPelsPerMeter
        0,           # biYPelsPerMeter
        0,           # biClrUsed
        0,           # biClrImportant
    )

    # XOR mask: bottom-up BGRA rows
    xor_bytes = bytearray(w * h * 4)
    # AND mask: bottom-up 1-bpp rows padded to 32-bit (4-byte) boundary
    and_row_stride = ((w + 31) // 32) * 4
    and_bytes = bytearray(and_row_stride * h)

    for y in range(h):
        src_y = h - 1 - y  # bottom-up
        xor_row_off = y * w * 4
        and_row_off = y * and_row_stride
        for x in range(w):
            r, g, b, a = px[x, src_y]
            off = xor_row_off + x * 4
            xor_bytes[off] = b
            xor_bytes[off + 1] = g
            xor_bytes[off + 2] = r
            xor_bytes[off + 3] = a
            if a < 16:
                and_bytes[and_row_off + (x >> 3)] |= 1 << (7 - (x & 7))

    return bih + bytes(xor_bytes) + bytes(and_bytes)


def build_standard_ico(master: Image.Image, out_path: Path) -> None:
    sizes = [16, 24, 32, 48, 64, 128, 256]
    frames = []
    for s in sizes:
        resized = master.resize((s, s), Image.Resampling.LANCZOS)
        if s == 256:
            buf = io.BytesIO()
            resized.save(buf, format="PNG", optimize=True)
            data = buf.getvalue()
        else:
            data = encode_dib_frame(resized)
        frames.append((s, data))

    # ICONDIR header: reserved=0, type=1 (ICO), count=len(frames)
    header = struct.pack("<HHH", 0, 1, len(frames))
    offset = 6 + 16 * len(frames)
    dir_entries = bytearray()
    payloads = bytearray()

    for s, data in frames:
        b_w = 0 if s == 256 else s
        b_h = 0 if s == 256 else s
        entry = struct.pack(
            "<BBBBHHII",
            b_w,         # bWidth
            b_h,         # bHeight
            0,           # bColorCount
            0,           # bReserved
            1,           # wPlanes
            32,          # wBitCount
            len(data),   # dwBytesInRes
            offset,      # dwImageOffset
        )
        dir_entries.extend(entry)
        payloads.extend(data)
        offset += len(data)

    out_path.write_bytes(header + bytes(dir_entries) + bytes(payloads))


def main() -> None:
    master = load_master_icon(1024)

    ico_path = ROOT / "future-proxy.ico"
    build_standard_ico(master, ico_path)

    icon_128 = master.resize((128, 128), Image.Resampling.LANCZOS)
    rgba_path = ROOT / "icon_128.rgba"
    rgba_path.write_bytes(icon_128.tobytes("raw", "RGBA"))

    png_256 = master.resize((256, 256), Image.Resampling.LANCZOS)
    png_path = ROOT / "future-proxy.png"
    png_256.save(png_path, format="PNG", optimize=True)

    b64_png = base64.b64encode(png_path.read_bytes()).decode("ascii")
    svg_path = ROOT / "future-proxy.svg"
    svg_path.write_text(
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="256" height="256">\n'
        f'  <image width="256" height="256" href="data:image/png;base64,{b64_png}"/>\n'
        '</svg>\n',
        encoding="utf-8",
    )

    print(
        f"Generated {ico_path} ({ico_path.stat().st_size} bytes), "
        f"{rgba_path} ({rgba_path.stat().st_size} bytes), "
        f"{png_path} ({png_path.stat().st_size} bytes), "
        f"{svg_path} ({svg_path.stat().st_size} bytes)"
    )


if __name__ == "__main__":
    main()
