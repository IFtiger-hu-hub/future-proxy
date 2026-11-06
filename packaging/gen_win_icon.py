#!/usr/bin/env python3
"""Generate 100% Microsoft-standard multi-size Windows .ico (BMP DIB for <=128px, PNG for 256px) and 128x128 raw RGBA."""
import io
import struct
from pathlib import Path
from PIL import Image, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parent


def make_master_icon(size: int = 1024) -> Image.Image:
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))

    # 1. Soft drop shadow under squircle
    shadow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    sdraw = ImageDraw.Draw(shadow)
    pad = int(size * 0.075)
    radius = int(size * 0.225)
    sdraw.rounded_rectangle(
        [pad, pad + int(size * 0.025), size - pad, size - pad + int(size * 0.025)],
        radius=radius,
        fill=(10, 35, 90, 115),
    )
    shadow = shadow.filter(ImageFilter.GaussianBlur(int(size * 0.028)))
    img = Image.alpha_composite(img, shadow)

    # 2. Gradient squircle body (#3898FF -> #0A84FF -> #1258E4)
    grad = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    gpx = grad.load()
    for y in range(pad, size - pad):
        ty = (y - pad) / max(1, (size - 2 * pad))
        for x in range(pad, size - pad):
            tx = (x - pad) / max(1, (size - 2 * pad))
            t = 0.65 * ty + 0.35 * tx
            r = int(56 * (1.0 - t) + 18 * t)
            g = int(152 * (1.0 - t) + 88 * t)
            b = int(255 * (1.0 - t) + 228 * t)
            gpx[x, y] = (r, g, b, 255)

    mask = Image.new("L", (size, size), 0)
    mdraw = ImageDraw.Draw(mask)
    mdraw.rounded_rectangle([pad, pad, size - pad, size - pad], radius=radius, fill=255)

    squircle = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    squircle.paste(grad, (0, 0), mask)

    # Subtle inner border highlight
    bdraw = ImageDraw.Draw(squircle)
    bdraw.rounded_rectangle(
        [
            pad + int(size * 0.006),
            pad + int(size * 0.006),
            size - pad - int(size * 0.006),
            size - pad - int(size * 0.006),
        ],
        radius=radius - int(size * 0.006),
        outline=(255, 255, 255, 78),
        width=max(2, int(size * 0.012)),
    )
    img = Image.alpha_composite(img, squircle)

    # 3. Subtle lightning glow
    bolt_pts = [
        (size * 0.565, size * 0.195),
        (size * 0.275, size * 0.545),
        (size * 0.485, size * 0.545),
        (size * 0.435, size * 0.805),
        (size * 0.725, size * 0.455),
        (size * 0.515, size * 0.455),
    ]
    bolt_shadow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    bsdraw = ImageDraw.Draw(bolt_shadow)
    shifted_pts = [(x, y + size * 0.018) for (x, y) in bolt_pts]
    bsdraw.polygon(shifted_pts, fill=(8, 35, 110, 125))
    bolt_shadow = bolt_shadow.filter(ImageFilter.GaussianBlur(int(size * 0.02)))
    img = Image.alpha_composite(img, bolt_shadow)

    # 4. Crisp white lightning bolt
    bolt_layer = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    ldraw = ImageDraw.Draw(bolt_layer)
    ldraw.polygon(bolt_pts, fill=(255, 255, 255, 255))
    img = Image.alpha_composite(img, bolt_layer)

    return img


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
    master = make_master_icon(1024)

    ico_path = ROOT / "future-proxy.ico"
    build_standard_ico(master, ico_path)

    icon_128 = master.resize((128, 128), Image.Resampling.LANCZOS)
    rgba_path = ROOT / "icon_128.rgba"
    rgba_path.write_bytes(icon_128.tobytes("raw", "RGBA"))

    print(f"Generated {ico_path} ({ico_path.stat().st_size} bytes) and {rgba_path} ({rgba_path.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
