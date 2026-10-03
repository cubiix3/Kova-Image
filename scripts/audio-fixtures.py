"""Generate the small audio fixtures in tests/fixtures.

Requires Pillow and a developer-installed ffmpeg with libmp3lame, libvorbis, libopus
and the AAC, FLAC and WMA encoders on PATH. Neither is linked, bundled or invoked by
the application or by the tests: the generated files are committed, so CI needs no
encoder.

Every file holds 0.8 seconds of a 440 Hz sine, mono (8 kHz; 22.05 kHz for WMA; one second
at 11.025 kHz for FLAC). The `song.*` files
also carry a title, an artist, an album and a 32 x 24 cover picture; the `tone.*`
files carry nothing, so the formats are also read without a tag.
"""
from pathlib import Path
import shutil
import subprocess
import tempfile

from PIL import Image

root = Path(__file__).resolve().parent.parent
output = root / "tests/fixtures"
output.mkdir(parents=True, exist_ok=True)
ffmpeg = shutil.which("ffmpeg")
if not ffmpeg:
    raise SystemExit("ffmpeg is required to encode the fixtures")

SINE = ["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=8000:duration=0.8"]
TAGS = ["-metadata", "title=Kova Song", "-metadata", "artist=Kova Band",
        "-metadata", "album=Kova Album"]


def strip_id3v2(name):
    """ffmpeg adds an ID3v2 tag with the encoder name; the untagged files must have none."""
    path = output / name
    data = path.read_bytes()
    if data[:3] == b"ID3":
        size = 0
        for byte in data[6:10]:
            size = size << 7 | byte
        path.write_bytes(data[10 + size:])


def encode(*args, name):
    subprocess.run(
        [ffmpeg, "-hide_banner", "-loglevel", "error", "-y", *args, str(output / name)],
        check=True,
    )


with tempfile.TemporaryDirectory() as scratch:
    cover = Path(scratch) / "cover.png"
    image = Image.new("RGB", (32, 24))
    image.putdata([(x * 8, y * 10, 96) for y in range(24) for x in range(32)])
    image.save(cover)

    # Without tags: MP3 frames from the first byte, raw AAC, WAV, Ogg, Opus, WMA.
    encode(*SINE, "-ac", "1", "-c:a", "libmp3lame", "-b:a", "16k", name="tone.mp3")
    strip_id3v2("tone.mp3")
    encode(*SINE, "-ac", "1", "-c:a", "aac", "-b:a", "16k", "-f", "adts", name="tone.aac")
    encode(*SINE, "-ac", "1", "-c:a", "pcm_s16le", name="tone.wav")
    # A sound effect of 50 ms: over before a clock can be sampled (see the player's reports).
    encode("-f", "lavfi", "-i", "sine=frequency=880:sample_rate=8000:duration=0.05", "-ac", "1",
           "-c:a", "pcm_s16le", name="click.wav")
    encode(*SINE, "-ac", "1", "-c:a", "libvorbis", "-q:a", "0", name="tone.ogg")
    encode(*SINE, "-ac", "1", "-c:a", "libopus", "-b:a", "12k", name="tone.opus")
    # The WMA encoder wants a higher sample rate and bit rate than the others.
    encode(*SINE, "-ac", "1", "-ar", "22050", "-c:a", "wmav2", "-b:a", "32k", name="tone.wma")

    # With tags and cover: ID3v2.3 in MP3, iTunes atoms in M4A, FLAC blocks.
    picture = ["-i", str(cover), "-map", "0:a", "-map", "1:v", "-c:v", "copy",
               "-metadata:s:v", "title=Album cover", "-metadata:s:v", "comment=Cover (front)",
               "-disposition:v", "attached_pic"]
    encode(*SINE, *picture, "-ac", "1", "-c:a", "libmp3lame", "-b:a", "16k",
           "-id3v2_version", "3", *TAGS, name="song.mp3")
    encode(*SINE, *picture, "-ac", "1", "-c:a", "aac", "-b:a", "16k", "-f", "ipod", *TAGS,
           name="song.m4a")
    # Windows reports no duration for a FLAC shorter than a second, so this one
    # is one second long.
    flac_sine = ["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=11025:duration=1.0"]
    encode(*flac_sine, *picture, "-ac", "1", "-c:a", "flac", *TAGS, name="song.flac")

for path in sorted(output.glob("tone.*")) + sorted(output.glob("song.*")) + [output / "click.wav"]:
    print(path.name, path.stat().st_size)
