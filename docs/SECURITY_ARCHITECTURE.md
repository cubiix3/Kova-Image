# Security architecture and limits

## Enforced application limits

| Resource | Limit |
| --- | --- |
| Compressed image file | 128 MiB |
| Width or height | 32,768 pixels |
| Pixel count | 33,554,432 pixels; JPEG up to 67,108,864 (3 bytes per pixel, shrunk before widening to RGBA). No retained bitmap exceeds 33,554,432 pixels |
| Decoder allocation allowance | 256 MiB, where the codec honors image-rs Limits. JPEG XL has its own 512 MiB allowance (32-bit float planes); a larger picture ends in a memory error |
| Camera RAW file | 1 GiB, read through seeks for the embedded preview only |
| SVG | 16 MiB (also after gunzip); drawn at most 8,192 pixels a side; embedded `data:` images 64 Mi pixels in total |
| Stored animation RGBA | 128 MiB, with space reserved for the next frame |
| Animation frame count | Fewer than 2,000 frames |
| Retained cache pixel data | 192 MiB / at most 32 entries |
| Decode threads | 1 |
| Pending decode requests | 1 (newest replaces previous) |
| Pending loader results | One image, one video and one folder event, coalesced by request id |
| Speculative neighbors | Next two in the direction of travel; next and previous before the first move; only after 150 ms without a new request |
| Folder entries | Fewer than 100,000 supported entries |
| Retained folder path names | 16 MiB |
| Frame duration | 10 ms through 60 seconds; GIF frames of 10 ms or less show for 100 ms |
| Settings read | 8 KiB |

Zero dimensions and overflowing pixel/byte arithmetic are rejected. Dimensions
are checked before allocating output buffers. Decoder constructors still parse
metadata, and image-rs allocation limits have codec-dependent coverage. An image
at the dimension boundary can therefore still require significant scratch space.
These numbers are admission limits, not a measured upper bound on all process RAM.

## File and execution boundary

Images use compiled-in decoders, which are the image-rs format adapters and
the modules in `src/codecs` (JPEG XL, SVG, camera RAW, AVIF and HEIC); content
(`src/format.rs`) chooses among them, and the extension decides only for formats
without a signature (TGA, SVG, RAW). Videos use the separate native boundary
documented below. No Shell thumbnail codecs, user plugins, downloaded codecs or
browser rendering are invoked.

- **SVG** is drawn by resvg, never by Slint (whose SVG machinery renders only the
  trusted built-in Kova logo). Only `data:` images are loaded; links to files or
  URLs, external stylesheets, scripts and animation are not followed or run, so
  opening an SVG cannot read another file or touch the network.
- **Camera RAW** is never developed. The file is scanned for JPEG streams, whose
  structure is checked before decoding (baseline and progressive only), and the
  largest one is shown.
- **AVIF** runs rav1d on a dedicated thread with a frame size limit. The vendored
  copy declares its C ABI `C-unwind`, so a panic is caught, the decoder is
  abandoned and the request ends in an error; the process survives.
- **HEIC** is decoded by Kova Image's own HEVC intra decoder
  (`src/codecs/hevc`). Every read is bounds checked; picture dimensions are
  checked against the pixel limits and the decode budget, and the NAL unit count
  is limited, before pictures are allocated; unsupported tools end in an error. A test flips bits in a stream corpus and demands error rather than
  panic. This is hand-written parsing of untrusted data in safe Rust, but it has
  had no external fuzzing.
- **JPEG XL** runs inside jxl-oxide's allocation tracker.

## Explorer thumbnail provider

`kova_thumbnails.dll` is loaded by Explorer, the file dialogs and other programs
that ask the Shell for thumbnails, only if the user registered it. It holds the
same decoders and limits as the viewer, receives the file as an `IStream`
(never a path), decodes at the size asked for (at most 4,096 pixels) and runs
each request inside `catch_unwind`. It is loaded into the Shell's isolated
thumbnail host, not into Explorer itself, so a crash affects thumbnails only.
It performs no network access and reads no file beyond the stream it is given,
except that an SVG with text makes it load the installed system fonts. It
writes no cache of its own. For PNG, JPEG, GIF, BMP, TIFF and ICO files whose content matches their
extension it loads Windows' own photo thumbnail provider in the same host and
passes the stream on; the decoding of such a file is then Windows' and not Kova's.
This is a deliberate exception to "everything is decoded inside Kova Image", made
for the previews Windows already produces (and which would otherwise change for
every photo in Explorer); it is limited to the thumbnail DLL, to files whose
content matches the extension, and the viewer never does it. Those files were
decoded by the same Windows code before Kova's provider was registered, so the
exposure is not new.
It adds a second, separate attack surface compared with
the viewer: untrusted files are decoded by every program that shows previews
after registration, without the user opening them. `--unregister-thumbnails`
removes it.

On Windows, file handles deny concurrent writes/deletion, open reparse points
without following them, and reject reparse-point image handles. The exception is
the cloud-files tag family (`IO_REPARSE_TAG_CLOUD` and its variants), which
OneDrive, Dropbox and iCloud use for placeholders. It is read from the open
handle, never from the path. Such a file is then opened again without
`FILE_FLAG_OPEN_REPARSE_POINT` so reads reach the provider, and the new handle is
accepted only if volume and file index match the first one. Symlinks, junctions and every other tag stay
refused. Reading a dehydrated placeholder makes its provider fetch the content;
Kova Image itself opens no connection. This path has been exercised with symlinks
only, not with a live sync provider. Length and
timestamps are compared after decode and before cache reuse. File identity
checks do not yet constitute a complete defense against deliberately constructed
timestamp-preserving replacements or ancestor-directory reparse races. Strong
file IDs and process-isolated decode are follow-up hardening work.

Cache reads still perform file metadata I/O on the worker. A missing, changed or
corrupted file cannot replace a newer request. Deleted or inaccessible files
remain navigable past. Directory scanning is non-recursive and ignores symlink
entries. No metadata values are interpreted as commands or URLs.

Rust decoder panics unwind into an error on the worker. The release profile
does not use panic=abort. `catch_unwind` cannot catch allocator aborts, access
violations or other native faults. No claim of decoder sandboxing is made.

## User-triggered Windows actions

Rotation and flip never save. Copy Image uses the original decoded frame.
Deleting requires a successfully loaded, current media file and rechecks metadata.
IFileOperation is configured for recycling and a progress sink rejects permanent
deletion. There is no `remove_file` fallback in product code. Restore a recycled
file through Windows Recycle Bin. Network drives and unavailable bins may fail.

The Shell acts on a path after validation, so a narrow malicious replacement
race remains between validation and the operation. This is disclosed rather
than presented as an atomic-by-handle delete guarantee.

Clipboard allocations are overflow-checked, explicitly owned, and transferred
to Windows only after successful publication. COM and HGLOBAL ownership are
kept inside `windows_integration`. Unsafe blocks document API lifetimes locally.

## Project controls

Windows formatting, check, strict Clippy, tests and release compilation run on
GitHub Actions. Workflows have read-only tokens, immutable action references,
bounded timeouts and no automatic release publishing. Dependabot monitors Cargo
and Actions. `cargo audit` runs separately and does not suppress advisories.
Private reporting is documented in the root security policy.

## Native video and audio boundary

Audio files (MP3, M4A/M4B, AAC, WAV, FLAC, Ogg, Opus, WMA) are admitted exactly like
video: regular local files up to 32 GiB, a retained read-only handle, no reparse points
other than cloud placeholders, content identification, and the box checks for M4A.
Kova Image's own parsing of an audio file is limited to its tags (`src/audio.rs`):
ID3v1/v2, MP4 `ilst` atoms and FLAC metadata blocks, read with positional reads, at
most 16 MiB per tag or block and 32 MiB of `moov`, with bounded loops, in safe Rust and
under `catch_unwind`; the cover picture goes through the viewer's image decoders and
their limits. A test flips bytes in fixtures of every format and requires an error or a
plain file, never a panic. WMA (ASF) containers are not inspected by Kova Image; Windows'
own ASF source reads them, and its handling of URLs inside such files is not verified.
The audio player creates no Direct3D device.

Videos are limited to 32 GiB local regular files, 16,777,216 native pixels,
8,192 pixels per side and seven days of finite duration. Presentation buffers
fit the window and are capped at 3840 x 2160. The Media Engine can parse/allocate before reporting
native dimensions: this is not an OS decoder allocation cap or a sandbox.

A retained read-only file handle denies writes/deletion during playback. The
engine receives an IStream-backed byte stream and a synthetic container hint,
never a user-supplied URL. Calling Load after SetSourceFromByteStream is avoided:
it would start a new URL load instead of retaining the admitted stream.

MP4/MOV box traversal has depth, entry-count and length limits; media payloads
are skipped by seeking. External data references, reference movies and compressed
movie headers are rejected. UNC/mapped network video paths and leaf reparse
points other than cloud placeholders are rejected. Ancestor reparse races remain a limitation, as for images.
Locally registered Media Foundation plugins are disabled. Windows system codecs
still parse hostile bytes in process; native faults are not caught by Rust.

One coalesced desired state, one playback worker and one pending UI update bound
application-level work. Delete awaits engine shutdown on the Shell worker before
rechecking the file stamp. No protected default-app UserChoice value is changed.
