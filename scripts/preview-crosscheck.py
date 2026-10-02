"""Compare thumbnails from the Windows Shell with an independent decoder (Pillow).

    python scripts/preview-crosscheck.py <picture folder> <thumbnail folder> [ext ...]

Create the thumbnails with `scripts/preview-check.ps1 -SaveTo <thumbnail folder>`. Each one
is compared with Pillow's rendering of the same file (embedded ICC profiles applied,
transparency over black, as the Shell hands over premultiplied pixels): the mean difference
of an 8 x 8 grid of cell averages must stay small. Pillow cannot read every format Kova Image
reads, so those files are skipped; thumbnails that Windows made itself (PNG, JPEG, ...) differ
in how they treat transparency and are not what this checks.
"""
import sys
from pathlib import Path

from PIL import Image

Image.MAX_IMAGE_PIXELS = None
source, thumbs = Path(sys.argv[1]), Path(sys.argv[2])
wanted = {e.lower().lstrip(".") for e in sys.argv[3:]}


def to_srgb(image):
    """Applies an embedded ICC profile like a colour-managed viewer would."""
    profile = image.info.get("icc_profile")
    if not profile:
        return image
    try:
        import io

        from PIL import ImageCms

        rgba = image.convert("RGBA")
        converted = ImageCms.profileToProfile(
            rgba.convert("RGB"), ImageCms.ImageCmsProfile(io.BytesIO(profile)),
            ImageCms.createProfile("sRGB"), renderingIntent=1,
        )
        converted.putalpha(rgba.getchannel("A"))
        return converted
    except Exception:
        return image


def premultiplied_on_black(image):
    image = to_srgb(image).convert("RGBA")
    background = Image.new("RGBA", image.size, (0, 0, 0, 255))
    # The Shell hands over premultiplied pixels, which a plain bitmap read shows over black.
    return Image.alpha_composite(background, image).convert("RGB")


def grid(image, n=8):
    small = image.resize((n, n), Image.BOX)
    return list(small.getdata())


results = {"ok": 0, "skipped": 0}
bad = []
worst = {}
for path in sorted(source.rglob("*")):
    if not path.is_file():
        continue
    ext = path.suffix.lower().lstrip(".")
    if wanted and ext not in wanted:
        continue
    rel = str(path.relative_to(source))
    thumb = thumbs / (rel.replace("\\", "__").replace("/", "__") + ".png")
    if not thumb.exists():
        results["skipped"] += 1
        continue
    try:
        reference = Image.open(path)
        reference.load()
    except Exception:
        results["skipped"] += 1  # Pillow cannot read it either
        continue
    shown = Image.open(thumb).convert("RGB")
    expected = premultiplied_on_black(reference)
    # Thumbnails keep the aspect ratio and are never larger than the picture.
    ratio = max(shown.size) / max(expected.size)
    if abs(shown.width - expected.width * ratio) > 1.5 or abs(shown.height - expected.height * ratio) > 1.5:
        bad.append((rel, f"shape {shown.size} vs {expected.size}"))
        continue
    expected_fit = expected if shown.size == expected.size else expected.resize(shown.size, Image.LANCZOS)
    a, b = grid(shown), grid(expected_fit)
    # Mean absolute difference over the 8 x 8 cell averages and the three channels.
    difference = round(sum(abs(x - y) for p, q in zip(a, b) for x, y in zip(p, q)) / (len(a) * 3), 2)
    worst[ext] = max(worst.get(ext, 0), difference)
    tolerance = 2 if shown.size == expected.size else 6
    if difference > tolerance:
        bad.append((rel, f"mean cell difference {difference} (limit {tolerance})"))
    else:
        results["ok"] += 1

print("ok:", results["ok"], "skipped (Pillow cannot read):", results["skipped"], "mismatches:", len(bad))
print("largest difference per extension:", worst)
for rel, why in bad[:40]:
    print("  ", rel, why)
