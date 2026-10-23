"""Generate local synthetic clips for opt-in playback checks.

Requires a developer-installed ffmpeg on PATH or the imageio-ffmpeg Python
package. The fixture encoder executable is not shipped or invoked by the application.
"""
from pathlib import Path
import argparse
import shutil
import subprocess

root = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser()
parser.add_argument("--codecs", action="store_true", help="generate the bundled-decoder regression clips")
parser.add_argument("--tests", action="store_true", help="generate the small redistributable Cargo fixtures")
args = parser.parse_args()
output = root / ("artifacts/video-codecs" if args.codecs else "artifacts/video-fixtures")
if args.tests:
    output = root / "tests/fixtures"
output.mkdir(parents=True, exist_ok=True)
ffmpeg = shutil.which("ffmpeg")
if not ffmpeg:
    import imageio_ffmpeg
    ffmpeg = imageio_ffmpeg.get_ffmpeg_exe()

clips = [
    ("clip1.mp4", "libx264", "aac"),
    ("clip2.mov", "libx264", "aac"),
    ("clip3.webm", "libvpx-vp9", "libopus"),
    ("clip4.mkv", "libx264", "aac"),
    ("vp8.webm", "libvpx", "libvorbis"),
]
if args.codecs:
    clips = [
        ("h264-ac3.mkv", "libx264", "ac3"),
        ("hevc-aac.mkv", "libx265", "aac"),
        ("hevc-eac3.mkv", "libx265", "eac3"),
        ("h264-dts.mkv", "libx264", "dca"),
        ("vp9-opus.mkv", "libvpx-vp9", "libopus"),
        ("av1-opus.mkv", "libaom-av1", "libopus"),
    ]
if args.tests:
    clips = [("codec-hevc.mkv", "libx265", "ac3"),
             ("codec-av1.mkv", "libaom-av1", "libopus"),
             ("codec-vp9.webm", "libvpx-vp9", "libvorbis"),
             ("codec-resample.mp4", "libx264", "aac")]
for name, video, audio in clips:
    extra = []
    if video == "libx265":
        extra = ["-preset", "ultrafast", "-x265-params", "log-level=error:pools=2"]
    elif video == "libaom-av1":
        extra = ["-cpu-used", "8", "-row-mt", "1", "-threads", "2"]
    subprocess.run([
        ffmpeg, "-hide_banner", "-loglevel", "error", "-y",
        "-f", "lavfi", "-i", "testsrc2=size=64x48:rate=10" if args.tests else "testsrc2=size=640x360:rate=30",
        "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=" + ("44100" if name == "codec-resample.mp4" else "48000"),
        "-t", "1.2" if args.tests else "6", "-c:v", video, "-pix_fmt", "yuv420p", "-c:a", audio,
        "-strict", "-2", *extra,
        str(output / name),
    ], check=True)
print(output)
if args.tests:
    # No sound device is needed for the opt-in 60 fps worker/upload pacing check.
    subprocess.run([
        ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i",
        "testsrc2=size=64x48:rate=60", "-t", "1.2", "-an", "-c:v", "libx264",
        "-pix_fmt", "yuv420p", str(output / "codec-pacing.mp4"),
    ], check=True)
    subprocess.run([
        ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i",
        "testsrc2=size=64x48:rate=10", "-t", "1.2", "-an", "-c:v", "libx264",
        "-vf", "scale=out_color_matrix=bt709:out_range=full,setsar=4/3",
        "-color_range", "pc", "-colorspace", "bt709", "-color_primaries", "bt709",
        "-color_trc", "bt709", str(output / "codec-color.mp4"),
    ], check=True)
    subprocess.run([
        ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-i", str(output / "codec-color.mp4"),
        "-frames:v", "1", "-vf", "scale=85:48:flags=bilinear", str(output / "codec-color.png"),
    ], check=True)
    subprocess.run([
        ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i",
        "testsrc2=size=64x48:rate=10", "-t", "1.2", "-an", "-c:v", "libx264",
        str(output / "codec-landscape.mp4"),
    ], check=True)
    subprocess.run([
        ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-display_rotation", "90",
        "-i", str(output / "codec-landscape.mp4"),
        "-c", "copy", str(output / "codec-rotated.mp4"),
    ], check=True)
    subprocess.run([
        ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-i", str(output / "codec-rotated.mp4"),
        "-frames:v", "1", str(output / "codec-rotated.png"),
    ], check=True)
    (output / "codec-landscape.mp4").unlink()
