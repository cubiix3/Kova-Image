"""Build a corpus of HEVC still pictures with reference decodings, to check the
HEVC decoder in src/codecs/hevc bit by bit.

For every case an intra picture is encoded with x265 (many tool combinations)
and decoded again with ffmpeg's own HEVC decoder. The corpus is written as

    <name>.hevc   the Annex B stream
    <name>.yuv    the reference decoding (planar, 8 or 16 bits per sample)
    <name>.json   width, height, pixel format

and checked by `cargo test -p kova-image --lib hevc -- --ignored` with the
environment variable KOVA_HEVC_REFERENCE set to the output folder.

Requires Pillow and a developer-installed ffmpeg with libx265 on PATH. Neither
is used by the application or by the normal tests.
"""
from pathlib import Path
import json
import random
import shutil
import subprocess
import sys

from PIL import Image, ImageDraw, ImageFilter

root = Path(__file__).resolve().parent.parent
output = Path(sys.argv[1]) if len(sys.argv) > 1 else root / "artifacts/hevc-reference"
only = set(sys.argv[2:])
output.mkdir(parents=True, exist_ok=True)
ffmpeg = shutil.which("ffmpeg")
if not ffmpeg:
    raise SystemExit("ffmpeg is required")


def picture(width, height, seed):
    """Something between a photograph and a test chart: smooth areas, texture,
    hard edges and noise, so that every prediction mode and transform size occurs."""
    rng = random.Random(seed)
    base = Image.effect_noise((width, height), 70).convert("RGB")
    base = base.filter(ImageFilter.GaussianBlur(rng.uniform(2, 6)))
    layer = Image.new("RGB", (width, height))
    pixels = layer.load()
    for y in range(height):
        for x in range(width):
            pixels[x, y] = (
                (x * 255 // max(1, width - 1)),
                (y * 255 // max(1, height - 1)),
                int(128 + 100 * ((x * y) % 97) / 97),
            )
    image = Image.blend(base, layer, 0.55)
    draw = ImageDraw.Draw(image)
    for _ in range(14):
        x0, y0 = rng.randrange(width), rng.randrange(height)
        x1, y1 = x0 + rng.randrange(4, width // 2 + 5), y0 + rng.randrange(4, height // 2 + 5)
        color = tuple(rng.randrange(256) for _ in range(3))
        if rng.random() < 0.5:
            draw.rectangle([x0, y0, x1, y1], fill=color)
        else:
            draw.line([x0, y0, x1, y1], fill=color, width=rng.randrange(1, 4))
    for _ in range(width * height // 40):
        x, y = rng.randrange(width), rng.randrange(height)
        pixels = image.load()
        r, g, b = pixels[x, y]
        d = rng.randrange(-30, 30)
        pixels[x, y] = (max(0, min(255, r + d)), max(0, min(255, g + d)), max(0, min(255, b + d)))
    return image


# name, size, pixel format, x265 parameters
CASES = [
    ("main420", (128, 96), "yuv420p", "qp=24"),
    ("main420-q10", (128, 96), "yuv420p", "qp=10"),
    ("main420-q40", (128, 96), "yuv420p", "qp=40"),
    ("crop420", (122, 90), "yuv420p", "qp=26"),
    ("ctu16", (96, 64), "yuv420p", "qp=24:ctu=16:min-cu-size=8"),
    ("ctu32", (160, 96), "yuv420p", "qp=24:ctu=32:min-cu-size=8"),
    ("ctu64-cu16", (192, 128), "yuv420p", "qp=24:ctu=64:min-cu-size=16"),
    ("tu-depth4", (128, 96), "yuv420p", "qp=24:tu-intra-depth=4:max-tu-size=32"),
    ("small-tu", (128, 96), "yuv420p", "qp=24:max-tu-size=8:tu-intra-depth=2"),
    ("no-sao", (128, 96), "yuv420p", "qp=28:no-sao=1"),
    ("no-deblock", (128, 96), "yuv420p", "qp=28:no-deblock=1"),
    ("no-filters", (128, 96), "yuv420p", "qp=28:no-sao=1:no-deblock=1"),
    ("deblock-offsets", (128, 96), "yuv420p", "qp=30:deblock=3,-2"),
    ("no-signhide", (128, 96), "yuv420p", "qp=24:no-signhide=1"),
    ("tskip", (128, 96), "yuv420p", "qp=22:tskip=1:tskip-fast=0"),
    ("no-strong", (128, 96), "yuv420p", "qp=24:no-strong-intra-smoothing=1"),
    ("no-wpp", (128, 96), "yuv420p", "qp=24:no-wpp=1"),
    ("wpp", (192, 128), "yuv420p", "qp=24:wpp=1:ctu=16:min-cu-size=8"),
    ("slices", (192, 128), "yuv420p", "qp=24:slices=3"),
    ("aq", (160, 96), "yuv420p", "crf=26:aq-mode=2:qg-size=16"),
    ("aq-qg8", (160, 96), "yuv420p", "crf=26:aq-mode=3:qg-size=16:ctu=32"),
    ("rd6", (128, 96), "yuv420p", "qp=24:rd=6"),
    ("lossless", (96, 64), "yuv420p", "lossless=1"),
    # "cu-lossless" with both SAO and deblocking is left out: ffmpeg restores the
    # samples of lossless blocks after SAO with the chroma width where the luma
    # width is meant, so it filters lossless chroma samples in the right half of
    # a CTB that the standard says stay as they are.
    ("scaling", (128, 96), "yuv420p", "qp=24:scaling-list=default"),
    ("main10", (128, 96), "yuv420p10le", "qp=24"),
    ("main10-wpp", (192, 128), "yuv420p10le", "qp=20:wpp=1"),
    ("mono", (128, 96), "gray", "qp=24"),
    ("mono10", (96, 64), "gray10le", "qp=24"),
    ("yuv422", (128, 96), "yuv422p", "qp=24"),
    ("yuv422-10", (128, 96), "yuv422p10le", "qp=22"),
    ("yuv422-10-no-sao", (128, 96), "yuv422p10le", "qp=22:no-sao=1"),
    ("yuv422-10-no-deblock", (128, 96), "yuv422p10le", "qp=22:no-deblock=1"),
    ("yuv422-10-no-filters", (128, 96), "yuv422p10le", "qp=22:no-sao=1:no-deblock=1"),
    ("cu-lossless-no-sao", (128, 96), "yuv420p", "qp=26:cu-lossless=1:no-sao=1"),
    ("cu-lossless-no-deblock", (128, 96), "yuv420p", "qp=26:cu-lossless=1:no-deblock=1"),
    ("cu-lossless-no-filters", (128, 96), "yuv420p", "qp=26:cu-lossless=1:no-sao=1:no-deblock=1"),
    ("yuv422-q10", (128, 96), "yuv422p", "qp=10:no-sao=1:no-deblock=1"),
    ("yuv422-q36", (128, 96), "yuv422p", "qp=36:no-sao=1:no-deblock=1"),
    ("yuv422-ctu16", (128, 96), "yuv422p", "qp=24:ctu=16:min-cu-size=8:no-sao=1:no-deblock=1"),
    ("yuv422-10-q10", (128, 96), "yuv422p10le", "qp=10:no-sao=1:no-deblock=1"),
    ("yuv422-10-q36", (128, 96), "yuv422p10le", "qp=36:no-sao=1:no-deblock=1"),
    ("yuv422-10-ctu16", (128, 96), "yuv422p10le", "qp=24:ctu=16:min-cu-size=8:no-sao=1:no-deblock=1"),
    ("yuv422-10-notu", (128, 96), "yuv422p10le", "qp=24:max-tu-size=8:tu-intra-depth=1:no-sao=1:no-deblock=1"),
    ("yuv422-lossless", (96, 64), "yuv422p", "lossless=1"),
    ("yuv422-ctu16-lossless", (96, 64), "yuv422p", "lossless=1:ctu=16:min-cu-size=8"),
    ("yuv444", (128, 96), "yuv444p", "qp=24"),
    ("yuv444-10", (128, 96), "yuv444p10le", "qp=22"),
    ("yuv444-tskip", (128, 96), "yuv444p", "qp=22:tskip=1:tskip-fast=0"),
    ("yuv444-lossless", (96, 64), "yuv444p", "lossless=1"),
    ("big", (640, 480), "yuv420p", "crf=24:aq-mode=2"),
    ("odd-size", (200, 120), "yuv420p", "qp=26:ctu=32"),
    ("12bit", (96, 64), "yuv420p12le", "qp=24"),
]

planes = {"gray": (1, 1, 1, 8), "gray10le": (1, 1, 1, 10)}


def run(*command):
    subprocess.run([str(c) for c in command], check=True, capture_output=True)


for index, (name, (width, height), pix, params) in enumerate(CASES):
    if only and name not in only:
        continue
    source = output / f"{name}.png"
    picture(width, height, index).save(source)
    stream = output / f"{name}.hevc"
    reference = output / f"{name}.yuv"
    try:
        run(ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-i", source, "-pix_fmt", pix,
            "-c:v", "libx265", "-x265-params", f"keyint=1:min-keyint=1:log-level=error:{params}",
            "-frames:v", "1", "-f", "hevc", stream)
        run(ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-i", stream, "-pix_fmt", pix,
            "-f", "rawvideo", reference)
    except subprocess.CalledProcessError as error:
        print(f"{name}: skipped ({error.stderr.decode(errors='replace').strip()[:200]})")
        for path in (stream, reference):
            path.unlink(missing_ok=True)
        continue
    (output / f"{name}.json").write_text(
        json.dumps({"width": width, "height": height, "pix_fmt": pix}), encoding="utf-8"
    )
    source.unlink()
    print(f"{name}: {stream.stat().st_size} bytes")
print(output)
