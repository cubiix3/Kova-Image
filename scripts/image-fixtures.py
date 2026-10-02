"""Generate the small image fixtures in tests/fixtures.

Requires Pillow and a developer-installed ffmpeg with libjxl, libaom and libx265
on PATH (`python scripts/image-fixtures.py dds` needs only Pillow). Neither is linked, bundled or invoked by the application or by the
tests: the generated files are committed, so CI needs no encoder.

Every fixture shows the same synthetic 32 x 24 picture: a red and green
gradient over a blue base, a white square in the top-left corner and a black
square in the bottom-right corner, so a swapped channel, a wrong rotation or a
mirrored image is visible in the pixels. The alpha variants fade in from left
to right.
"""
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile

from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent))
from heif_writer import aux_alpha, box, heif, hevc_picture, ispe, nclx

W, H = 32, 24
root = Path(__file__).resolve().parent.parent
output = root / "tests/fixtures"
output.mkdir(parents=True, exist_ok=True)
ffmpeg = shutil.which("ffmpeg")
only_dds = sys.argv[1:] == ["dds"]
if not ffmpeg and not only_dds:
    raise SystemExit("ffmpeg is required to encode the fixtures")


def pixel(x, y, alpha):
    if x < 6 and y < 6:
        color = (255, 255, 255)
    elif x >= W - 6 and y >= H - 6:
        color = (0, 0, 0)
    else:
        color = (x * 8, y * 10, 96)
    return color + ((x * 255 // (W - 1),) if alpha else ())


def picture(alpha):
    image = Image.new("RGBA" if alpha else "RGB", (W, H))
    image.putdata([pixel(x, y, alpha) for y in range(H) for x in range(W)])
    return image


def write_dds():
    """DDS textures from Pillow's own encoder: block compressed (BC1, BC3) and
    uncompressed, as a second opinion on the block decoders."""
    picture(True).save(output / "rgba.dds", pixel_format="DXT5")
    picture(False).save(output / "rgb.dds", pixel_format="DXT1")
    picture(True).save(output / "rgba-raw.dds")


if only_dds:
    write_dds()
    print(output)
    raise SystemExit


def encode(source, name, *args):
    subprocess.run(
        [ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-i", str(source), *args,
         str(output / name)],
        check=True,
    )


with tempfile.TemporaryDirectory() as scratch:
    opaque = Path(scratch) / "opaque.png"
    faded = Path(scratch) / "faded.png"
    picture(False).save(opaque)
    picture(True).save(faded)
    # Lossless, so the pixels can be compared exactly.
    encode(opaque, "rgb.jxl", "-c:v", "libjxl", "-distance", "0", "-frames:v", "1")
    encode(faded, "rgba.jxl", "-c:v", "libjxl", "-distance", "0", "-frames:v", "1")
    # Lossy (VarDCT with XYB), which takes the other decoding path.
    encode(opaque, "lossy.jxl", "-c:v", "libjxl", "-distance", "1", "-frames:v", "1")
    # AV1 in an AVIF container: lossy 4:2:0 with limited range, lossless 4:4:4 in
    # the identity (GBR) matrix, and a colour picture with a separate alpha picture.
    still = ["-c:v", "libaom-av1", "-cpu-used", "8", "-still-picture", "1", "-frames:v", "1"]
    encode(opaque, "rgb420.avif", *still, "-crf", "10", "-pix_fmt", "yuv420p")
    encode(opaque, "rgb444.avif", *still, "-aom-params", "lossless=1", "-pix_fmt", "gbrp")
    # The alpha plane is monochrome and full range, and must not inherit the
    # identity matrix that an RGB source would give it.
    encode(
        faded, "rgba.avif", *still, "-crf", "10",
        "-filter_complex",
        "[0:v]split[a][b];[a]alphaextract,setparams=colorspace=bt709:range=pc[al];"
        "[b]format=rgb24,scale=in_range=full:out_range=limited:out_color_matrix=bt709,"
        "format=yuv420p[c]",
        "-map", "[c]", "-map", "[al]", "-pix_fmt:1", "gray",
    )

    # HEIC: HEVC pictures (4:2:0, limited range, BT.709) in a HEIF container.
    bt709 = "scale=out_color_matrix=bt709:out_range=limited"

    def hevc(source, pix_fmt="yuv420p", params="crf=12", video_filter=bt709):
        return hevc_picture(ffmpeg, source, pix_fmt, params, video_filter)

    config, data = hevc(opaque)
    (output / "rgb420.heic").write_bytes(heif(
        [(1, b"hvc1", data, [1, 2, 3])], 1, [box(b"hvcC", config), ispe(W, H), nclx(1)]))
    config, data = hevc(opaque, "yuv420p10le")
    (output / "rgb10.heic").write_bytes(heif(
        [(1, b"hvc1", data, [1, 2, 3])], 1, [box(b"hvcC", config), ispe(W, H), nclx(1)]))
    # The alpha picture is a separate, monochrome picture that points at the colour one.
    faded_rgb = Path(scratch) / "faded-rgb.png"
    Image.open(faded).convert("RGB").save(faded_rgb)
    alpha_png = Path(scratch) / "alpha.png"
    Image.open(faded).getchannel("A").save(alpha_png)
    colour, colour_data = hevc(faded_rgb)
    alpha_config, alpha_data = hevc(alpha_png, "gray", "crf=8", "scale=in_range=full:out_range=full")
    (output / "rgba.heic").write_bytes(heif(
        [(1, b"hvc1", colour_data, [1, 3, 4]), (2, b"hvc1", alpha_data, [2, 3, 5])], 1,
        [box(b"hvcC", colour), box(b"hvcC", alpha_config), ispe(W, H), nclx(1), aux_alpha()],
        references=[(b"auxl", 2, [1])]))
    # A 2x2 grid of 32x24 tiles that make up the 64x48 picture.
    tiles = []
    for index in range(4):
        tile = Image.new("RGB", (W, H))
        tile.putdata([pixel((x + index % 2 * W) // 2, (y + index // 2 * H) // 2, False)
                      for y in range(H) for x in range(W)])
        path = Path(scratch) / f"tile{index}.png"
        tile.save(path)
        tiles.append(hevc(path))
    grid = struct.pack(">BBBBHH", 0, 0, 1, 1, 2 * W, 2 * H)
    items = [(1, b"grid", grid, [2, 3])] + [
        (2 + i, b"hvc1", tiles[i][1], [1, 4]) for i in range(4)]
    (output / "grid.heic").write_bytes(heif(
        items, 1, [box(b"hvcC", tiles[0][0]), ispe(2 * W, 2 * H), nclx(1), ispe(W, H)],
        references=[(b"dimg", 1, [2, 3, 4, 5])]))
    # Turned by a quarter turn: the file holds the picture as the sensor saw it
    # and `irot` says how to turn it counter-clockwise.
    sensor = Path(scratch) / "sensor.png"
    Image.open(opaque).rotate(-90, expand=True).save(sensor)
    config, data = hevc(sensor)
    (output / "turned.heic").write_bytes(heif(
        [(1, b"hvc1", data, [1, 2, 3, 4])], 1,
        [box(b"hvcC", config), ispe(H, W), nclx(1), box(b"irot", bytes([1]))]))
write_dds()
print(output)
