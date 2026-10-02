"""One sample file per extension Kova Image knows, plus some videos, in a fresh folder.

    python scripts/preview-samples.py <folder>

Needs Pillow; ffmpeg adds QOI, EXR and the videos. Check the result with
`scripts/preview-check.ps1 -Folder <folder>`. The pictures are synthetic, and the camera RAW
files are a TIFF-style container around a JPEG preview, as the viewer reads them.
"""
import gzip
import shutil
import struct
import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageDraw

if len(sys.argv) != 2:
    raise SystemExit("usage: preview-samples.py <folder>  (the folder is emptied first)")
out = Path(sys.argv[1])
shutil.rmtree(out, ignore_errors=True)
out.mkdir(parents=True)
repo = Path(__file__).resolve().parent.parent
ffmpeg = shutil.which("ffmpeg")

W, H = 160, 120


def picture(alpha=False):
    image = Image.new("RGBA", (W, H), (30, 60, 120, 255))
    d = ImageDraw.Draw(image)
    d.ellipse((12, 12, 80, 80), fill=(230, 60, 40, 255))
    d.rectangle((70, 50, 150, 110), fill=(40, 200, 90, 255 if not alpha else 180))
    d.rectangle((0, 0, 20, 20), fill=(255, 255, 255, 255))
    return image if alpha else image.convert("RGB")


rgb, rgba = picture(), picture(True)
rgb.save(out / "t.jpg", quality=90)
shutil.copy(out / "t.jpg", out / "t.jpeg")
shutil.copy(out / "t.jpg", out / "t.jpe")
rgba.save(out / "t.png")
frames = [picture().rotate(a) for a in (0, 30, 60)]
frames[0].save(out / "t.apng", format="PNG", save_all=True, append_images=frames[1:], duration=100, loop=0)
frames[0].save(out / "t.gif", save_all=True, append_images=frames[1:], duration=100, loop=0)
rgb.save(out / "t.bmp")
rgb.save(out / "t.tif")
shutil.copy(out / "t.tif", out / "t.tiff")
rgba.resize((64, 64)).save(out / "t.ico", sizes=[(64, 64), (32, 32)])
rgba.save(out / "t.webp", lossless=True)
rgba.save(out / "t-anim.webp", save_all=True, append_images=[f.convert("RGBA") for f in frames[1:]], duration=100, loop=0)
rgb.save(out / "t.tga")
rgba.save(out / "t-rle.tga", compression="tga_rle")
rgb.save(out / "t.ppm")
rgb.convert("L").save(out / "t.pgm")
rgb.convert("1").save(out / "t.pbm")
shutil.copy(out / "t.ppm", out / "t.pnm")
(out / "t.pam").write_bytes(
    f"P7\nWIDTH {W}\nHEIGHT {H}\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n".encode() + rgba.tobytes()
)
for name, fmt in (("dxt1", "DXT1"), ("dxt3", "DXT3"), ("dxt5", "DXT5")):
    rgba.save(out / f"t-{name}.dds", pixel_format=fmt)
rgba.save(out / "t-argb.dds")
rgb.save(out / "t-rgb.dds")
# Radiance HDR, flat RGBE.
pixels = bytearray()
for r, g, b in rgb.get_flattened_data() if hasattr(rgb, 'get_flattened_data') else rgb.getdata():
    pixels += bytes((r, g, b, 129))
(out / "t.hdr").write_bytes(f"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y {H} +X {W}\n".encode() + bytes(pixels))
# farbfeld
ff = bytearray(b"farbfeld" + struct.pack(">II", W, H))
for r, g, b, a in rgba.get_flattened_data() if hasattr(rgba, 'get_flattened_data') else rgba.getdata():
    ff += struct.pack(">HHHH", r * 257, g * 257, b * 257, a * 257)
(out / "t.ff").write_bytes(bytes(ff))
svg = (
    f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">'
    '<rect width="160" height="120" fill="#1e3c78"/><circle cx="46" cy="46" r="34" fill="#e63c28"/>'
    '<rect x="70" y="50" width="80" height="60" fill="#28c85a"/><text x="10" y="112" font-size="14" fill="white">Kova</text></svg>'
)
(out / "t.svg").write_text(svg)
(out / "t.svgz").write_bytes(gzip.compress(svg.encode()))

if ffmpeg:
    src = out / "_src.png"
    rgb.save(src)

    def run(*args):
        subprocess.run([ffmpeg, "-hide_banner", "-loglevel", "error", "-y", *args], check=True)

    run("-i", str(src), "-frames:v", "1", "-c:v", "qoi", str(out / "t.qoi"))
    run("-i", str(src), "-frames:v", "1", "-c:v", "exr", "-pix_fmt", "gbrpf32le", str(out / "t.exr"))
    src.unlink()

# Real-size and tiny samples of the container formats from the repository.
fixtures = repo / "tests/fixtures"
for name, target in (("rgb420.heic", "t.heic"), ("rgb420.heic", "t.heif"), ("rgba.avif", "t.avif"), ("lossy.jxl", "t.jxl")):
    shutil.copy(fixtures / name, out / target)
bench = repo / "artifacts/bench-formats"
for name in ("photo.heic", "photo.avif", "photo.jxl", "photo.webp", "texture-bc7.dds", "texture-dxt5.dds", "drawing.svg"):
    if (bench / name).exists():
        shutil.copy(bench / name, out / f"big-{name}")

# RAW: a TIFF-style container with the camera's JPEG preview inside.
import io

jpeg = io.BytesIO()
picture().save(jpeg, format="JPEG", quality=85)
raw = bytearray(b"II*\0" + struct.pack("<I", 8) + struct.pack("<H", 1))
raw += struct.pack("<HHII", 0x0112, 3, 1, 1) + struct.pack("<I", 0)
raw += bytes([0x5A]) * 4000 + jpeg.getvalue() + bytes([0x11]) * 200
for ext in "3fr ari arw cr2 cr3 crw dcr dng erf iiq kdc mef mrw nef nrw orf pef raf rw2 rwl sr2 srf srw x3f".split():
    (out / f"t.{ext}").write_bytes(bytes(raw))

# Videos.
if ffmpeg:
    def video(name, *codec):
        run("-f", "lavfi", "-i", "testsrc=size=320x240:rate=25:duration=2", *codec, str(out / name))

    video("t.mp4", "-c:v", "libx264", "-pix_fmt", "yuv420p")
    video("t.m4v", "-c:v", "libx264", "-pix_fmt", "yuv420p")
    video("t.mov", "-c:v", "libx264", "-pix_fmt", "yuv420p")
    video("t.mkv", "-c:v", "libx264", "-pix_fmt", "yuv420p")
    video("t.webm", "-c:v", "libvpx-vp9", "-pix_fmt", "yuv420p")
    video("t-hevc.mp4", "-c:v", "libx265", "-pix_fmt", "yuv420p", "-tag:v", "hvc1")
    video("t-vp8.webm", "-c:v", "libvpx")

print(len(list(out.iterdir())), "files in", out)
