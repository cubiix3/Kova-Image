"""Generate local synthetic clips for opt-in playback checks.

Requires a developer-installed ffmpeg on PATH or the imageio-ffmpeg Python
package. Neither is linked, bundled or invoked by the application.
"""
from pathlib import Path
import shutil
import subprocess

root = Path(__file__).resolve().parent.parent
output = root / "artifacts/video-fixtures"
output.mkdir(parents=True, exist_ok=True)
ffmpeg = shutil.which("ffmpeg")
if not ffmpeg:
    import imageio_ffmpeg
    ffmpeg = imageio_ffmpeg.get_ffmpeg_exe()

for name, video, audio in [
    ("clip1.mp4", "libx264", "aac"),
    ("clip2.mov", "libx264", "aac"),
    ("clip3.webm", "libvpx-vp9", "libopus"),
    ("clip4.mkv", "libx264", "aac"),
    ("vp8.webm", "libvpx", "libvorbis"),
]:
    subprocess.run([
        ffmpeg, "-hide_banner", "-loglevel", "error", "-y",
        "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30",
        "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000",
        "-t", "6", "-c:v", video, "-pix_fmt", "yuv420p", "-c:a", audio,
        str(output / name),
    ], check=True)
print(output)
