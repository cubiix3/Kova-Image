# Security architecture and limits

## Enforced application limits

| Resource | Limit |
| --- | --- |
| Compressed image file | 128 MiB |
| Width or height | 32,768 pixels |
| Pixel count | 33,554,432 pixels |
| Decoder allocation allowance | 256 MiB, where the codec honors image-rs Limits |
| Stored animation RGBA | 128 MiB, with space reserved for the next frame |
| Animation frame count | Fewer than 2,000 frames |
| Retained cache pixel data | 192 MiB / at most 32 entries |
| Decode threads | 1 |
| Pending decode requests | 1 (newest replaces previous) |
| Pending image/folder results | 4 |
| Speculative neighbors | Next, then previous |
| Folder entries | Fewer than 100,000 supported entries |
| Retained folder path names | 16 MiB |
| Frame duration | 10 ms through 60 seconds |
| Settings read | 8 KiB |

Zero dimensions and overflowing pixel/byte arithmetic are rejected. Dimensions
are checked before allocating output buffers. Decoder constructors still parse
metadata, and image-rs allocation limits have codec-dependent coverage. An image
at the dimension boundary can therefore still require significant scratch space.
These numbers are admission limits, not a measured upper bound on all process RAM.

## File and execution boundary

Images use compiled-in image-rs format adapters; magic bytes choose among
them. Videos use the separate native boundary documented below. No Shell thumbnail codecs, user plugins, downloaded codecs or browser
rendering are invoked. SVG and other unsupported data are rejected. Slint's
SVG machinery renders the trusted built-in Kova logo only.

On Windows, file handles deny concurrent writes/deletion, open reparse points
without following them, and reject reparse-point image handles. Length and
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

## Native video boundary

Videos are limited to 32 GiB local regular files, 16,777,216 native pixels,
8,192 pixels per side and seven days of finite duration. Presentation buffers
are capped at 1920 x 1080. The Media Engine can parse/allocate before reporting
native dimensions: this is not an OS decoder allocation cap or a sandbox.

A retained read-only file handle denies writes/deletion during playback. The
engine receives an IStream-backed byte stream and a synthetic container hint,
never a user-supplied URL. Calling Load after SetSourceFromByteStream is avoided:
it would start a new URL load instead of retaining the admitted stream.

MP4/MOV box traversal has depth, entry-count and length limits; media payloads
are skipped by seeking. External data references, reference movies and compressed
movie headers are rejected. UNC/mapped network video paths and leaf reparse
points are rejected. Ancestor reparse races remain a limitation, as for images.
Locally registered Media Foundation plugins are disabled. Windows system codecs
still parse hostile bytes in process; native faults are not caught by Rust.

One coalesced desired state, one playback worker and one pending UI update bound
application-level work. Delete awaits engine shutdown on the Shell worker before
rechecking the file stamp. No protected default-app UserChoice value is changed.
