"""A minimal HEIF writer for the HEIC fixtures: HEVC pictures encoded with x265
(through ffmpeg, which has no HEIF muxer) in a still-image container with
properties, item references, grids and auxiliary pictures. Used by
image-fixtures.py; it writes only what the fixtures need.
"""
from pathlib import Path
import struct
import subprocess
import tempfile


def box(kind, payload=b""):
    return struct.pack(">I4s", 8 + len(payload), kind) + payload


def full_box(kind, payload=b"", version=0, flags=0):
    return box(kind, struct.pack(">I", version << 24 | flags) + payload)


def split_annex_b(stream):
    """The NAL units (header included) of an Annex B stream."""
    units, start, i = [], None, 0
    while i + 3 <= len(stream):
        if stream[i:i + 3] == b"\x00\x00\x01":
            if start is not None:
                units.append(stream[start:i].rstrip(b"\x00"))
            start = i + 3
            i += 3
        else:
            i += 1
    units.append(stream[start:].rstrip(b"\x00"))
    return units


def unescape(data):
    out, zeros = bytearray(), 0
    for byte in data:
        if zeros >= 2 and byte == 3:
            zeros = 0
            continue
        zeros = zeros + 1 if byte == 0 else 0
        out.append(byte)
    return bytes(out)


def hevc_picture(ffmpeg, source, pix_fmt, params, video_filter=""):
    """Encodes `source` with x265. Returns the `hvcC` payload and the item data
    (length-prefixed slice NAL units)."""
    with tempfile.TemporaryDirectory() as scratch:
        stream = Path(scratch) / "picture.hevc"
        filters = ["-vf", video_filter] if video_filter else []
        subprocess.run(
            [ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-i", str(source), *filters,
             "-pix_fmt", pix_fmt, "-c:v", "libx265", "-x265-params",
             "keyint=1:min-keyint=1:log-level=error:" + params, "-frames:v", "1", "-f", "hevc",
             str(stream)],
            check=True,
        )
        units = split_annex_b(stream.read_bytes())
    parameter_sets = {32: [], 33: [], 34: []}
    data = b""
    for unit in units:
        kind = unit[0] >> 1 & 0x3F
        if kind in parameter_sets:
            parameter_sets[kind].append(unit)
        elif kind <= 21:
            data += struct.pack(">I", len(unit)) + unit
    sps = unescape(parameter_sets[33][0][2:])
    profile = sps[1:13]  # profile_tier_level up to and including the level
    name = pix_fmt.rstrip("0123456789le")
    chroma = {"gray": 0, "yuv420p": 1, "yuv422p": 2, "yuv444p": 3}[name]
    depth = 10 if "10" in pix_fmt else 8
    config = bytes([1]) + profile + struct.pack(
        ">HBBBBHB", 0xF000, 0xFC, 0xFC | chroma, 0xF8 | depth - 8, 0xF8 | depth - 8, 0, 0x0F)
    config += bytes([3])
    for kind in (32, 33, 34):
        config += bytes([0x80 | kind]) + struct.pack(">H", len(parameter_sets[kind]))
        for unit in parameter_sets[kind]:
            config += struct.pack(">H", len(unit)) + unit
    return config, data


def heif(items, primary, properties, references=(), brand=b"heic"):
    """A HEIF file. `items`: (id, type, data, [property indices]); `properties`:
    ready-made property boxes (indices count from 1); `references`:
    (type, from item, [to items])."""
    def meta(offsets):
        iloc = full_box(b"iloc", struct.pack(">HH", 0x4400, len(items)) + b"".join(
            struct.pack(">HHHII", item[0], 0, 1, offset, len(item[2]))
            for item, offset in zip(items, offsets)))
        iinf = full_box(b"iinf", struct.pack(">H", len(items)) + b"".join(
            full_box(b"infe", struct.pack(">HH4s", item[0], 0, item[1]) + b"\x00", version=2)
            for item in items))
        ipma = full_box(b"ipma", struct.pack(">I", len(items)) + b"".join(
            struct.pack(">HB", item[0], len(item[3])) + bytes(0x80 | i for i in item[3])
            for item in items))
        iref = full_box(b"iref", b"".join(
            box(kind, struct.pack(">HH", source, len(targets))
                + b"".join(struct.pack(">H", t) for t in targets))
            for kind, source, targets in references)) if references else b""
        return full_box(
            b"meta",
            full_box(b"hdlr", struct.pack(">I4s12s", 0, b"pict", b"") + b"\x00")
            + full_box(b"pitm", struct.pack(">H", primary)) + iloc + iinf + iref
            + box(b"iprp", box(b"ipco", b"".join(properties)) + ipma))
    ftyp = box(b"ftyp", brand + b"\x00\x00\x00\x00" + b"mif1" + brand)
    size = len(meta([0] * len(items)))
    offsets, at = [], len(ftyp) + size + 8
    for item in items:
        offsets.append(at)
        at += len(item[2])
    return ftyp + meta(offsets) + box(b"mdat", b"".join(item[2] for item in items))


def ispe(width, height):
    return full_box(b"ispe", struct.pack(">II", width, height))


def nclx(matrix, full_range=0, primaries=1, transfer=13):
    return box(b"colr", b"nclx" + struct.pack(">HHHB", primaries, transfer, matrix, full_range << 7))


def aux_alpha():
    return full_box(b"auxC", b"urn:mpeg:hevc:2015:auxid:1\x00")
