"""Generate original large animations for viewer checks. Requires Pillow.

--jxl also needs a developer FFmpeg with the libjxl_anim encoder.
--tests regenerates tests/fixtures/animated-budget.jxl with that encoder.
"""
import argparse
from pathlib import Path
import shutil
import subprocess

from PIL import Image, ImageDraw

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--jxl", action="store_true")
parser.add_argument("--tests", action="store_true")
parser.add_argument("--ffmpeg", help="developer FFmpeg executable with libjxl_anim")
args = parser.parse_args()
root = Path(__file__).resolve().parent.parent
output = root / "artifacts/animation-fixtures"
output.mkdir(parents=True, exist_ok=True)
frames = []
for index in range(25 if args.tests else 48):
    frame = Image.new("P", (1600, 900), 0)
    frame.putpalette([20, 30, 45, 240, 100, 30, 30, 200, 160] + [0, 0, 0] * 253)
    draw = ImageDraw.Draw(frame)
    draw.rectangle((index * 24, 150, index * 24 + 220, 650), fill=1)
    draw.rectangle((80, 50, 260, 120), fill=2)
    frames.append(frame)
frames[0].save(output / "large.gif", save_all=True, append_images=frames[1:],
               duration=60, loop=0, disposal=2, optimize=False)
if not args.tests:
    rgba = [frame.convert("RGBA") for frame in frames]
    for extension in ("apng", "webp"):
        rgba[0].save(output / ("large." + extension), save_all=True,
                     append_images=rgba[1:], duration=60, loop=0, lossless=True)
if args.jxl or args.tests:
    ffmpeg = args.ffmpeg or shutil.which("ffmpeg")
    if not ffmpeg:
        raise SystemExit("Pass --ffmpeg with a developer executable that includes libjxl_anim.")
    destination = root / "tests/fixtures/animated-budget.jxl" if args.tests else output / "large.jxl"
    subprocess.run([ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-i",
                    str(output / "large.gif"), "-c:v", "libjxl_anim", "-pix_fmt", "rgb24", "-effort", "1",
                    "-distance", "0", "-threads", "2", "-f", "image2pipe", str(destination)], check=True)
print(output)
