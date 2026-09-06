<p align="center">
  <img src="assets/kova.svg" width="88" height="88" alt="Kova logo">
</p>

<h1 align="center">Kova Image</h1>
<p align="center">A fast, lightweight and secure image viewer for Windows.</p>

<p align="center">
  <a href="https://github.com/cubiix3/Kova-Image/actions/workflows/ci.yml"><img src="https://github.com/cubiix3/Kova-Image/actions/workflows/ci.yml/badge.svg" alt="Windows CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue" alt="MIT or Apache-2.0"></a>
</p>

> **Status: Early Development — 0.1.0.** The initial viewer is implemented.
> There is no stable release or installer yet. Build from source to try it.
> See [verification and limitations](docs/VALIDATION.md).

<p align="center"><a href="https://slint.dev"><img src="assets/made-with-slint.png" width="150" alt="Made with Slint"></a></p>

## What is Kova Image?

A standalone, native Windows image and animation viewer in the Kova product
family. Built with **Rust, Slint and official Windows APIs**. Open an image,
see it, and move through its folder. Kova Image is a viewer, not an editor.

![Kova Image running with an original generated test image](docs/images/viewer.png)

## Goals

- Keep opening and navigation responsive, including while decoding fails.
- Bound image memory, cache retention, speculative work and queued requests.
- Keep the image central, with a compact, dark interface.
- No accounts, cloud, telemetry, gallery database or background services.

## Current status

Windows 10/11 x64 is the target. The 0.1.0 source provides the viewer workflow,
animation, folder navigation and Windows actions below. This is a first
implementation, with no stability or performance guarantees. Hardware diversity,
color management, accessibility and hostile-file coverage need more validation.

## Features

- Open by CLI path, native file picker, or file drop.
- Previous/next/first/last, with natural sorting in the current folder.
- Fit, fit width, 100%, cursor-centered wheel zoom and drag to pan.
- Fullscreen with auto-hiding controls; rotation and horizontal/vertical flips.
- GIF, animated WebP and APNG playback with pause and bounded frame storage.
- Copy the original decoded image/current animation frame or Unicode path.
- Move a loaded file to the Windows Recycle Bin, reveal it in Explorer, or
  open the native **Open with** dialog.
- Small information and settings panels, local settings and sharp-pixel mode.
- Asynchronous decoding, latest-request priority, stale-result rejection and
  next/previous preloading through a bounded, weighted LRU cache.

Rotation and flips affect the view only. Copy Image copies the decoded frame
before view transforms. Original files are never rewritten. Each launch opens
an independent window; there is no single-instance IPC service.

## Supported formats

| Format | Current implementation |
| --- | --- |
| JPEG / JPG | Still image; EXIF orientation applied |
| PNG | Still image |
| GIF | Animation, timing, loops, transparency and disposal |
| WebP | Still and animated |
| APNG | Animation, timing, loops, blending and disposal |
| BMP | Still image |
| TIFF | First image/page |
| ICO | Decoder-selected icon image |
| AVIF | Planned; no decoder shipped yet |
| HEIC/HEIF, JPEG XL, SVG, RAW | Not supported; evaluation remains on the roadmap |

File contents determine the decoder. Extensions are used only to filter folder
navigation and the file picker. An explicitly opened supported image can have
an unusual extension. SVG and external resources are never rendered.

## Installation / Running

There is no published installer or stable binary. After building:

```powershell
.\target\release\kova-image.exe
.\target\release\kova-image.exe "C:\Pictures\example.png"
.\target\release\kova-image.exe --software "C:\Pictures\example.png"
```

`--software` selects the rendering fallback. The default uses OpenGL through
Slint's FemtoVG renderer. Keep any packaged runtime DLLs alongside the executable.
See [Windows builds and packaging](docs/WINDOWS_RELEASE.md).

## Building from source

Install Rust and Visual Studio's **Desktop development with C++** workload,
including a Windows SDK. The pinned toolchain is Rust 1.95.0 (MSVC).

```powershell
git clone https://github.com/cubiix3/Kova-Image.git
cd Kova-Image
.\scripts\cargo-msvc.ps1 build --locked --release --bin kova-image
```

From a Visual Studio Developer PowerShell:

```powershell
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

The first build downloads Rust dependencies. Application startup performs no
update check or dependency download. [Dependency decisions](docs/DEPENDENCIES.md)
distinguish build tooling from shipped code.

## Keyboard / Mouse controls

| Action | Control |
| --- | --- |
| Open image | `Ctrl+O` or drop a file |
| Previous / next | `Left` / `Right`, mouse Back / Forward |
| First / last | `Home` / `End` |
| Zoom in / out | `+` / `-`, mouse wheel |
| Fit / fit width / 100% | `0` / `W` / `1` |
| Pan | Drag the image with the left mouse button |
| Fullscreen / exit fullscreen | `F11` / `Esc` |
| Pause / resume animation | `Space` |
| Rotate right / left | `R` / `Shift+R` |
| Flip horizontal / vertical | `H` / `V` |
| Image information | `I` |
| Copy image / path | `Ctrl+C` / `Ctrl+Shift+C` |
| Move to Recycle Bin | `Delete` |

The More panel contains Windows actions and settings. Shortcuts are defined in
`src/input.rs`. Wheel navigation can replace wheel zoom in Settings.
Settings live in `%LOCALAPPDATA%\Kova Image\settings.conf`.

## Performance philosophy

No splash screen, database, thumbnails for every file or startup indexing.
The current image wins over preloads. A single decode worker replaces pending
requests, checks cancellation on file reads and frame boundaries, and preloads
only the next and previous image. Codecs cannot all be interrupted during their
internal CPU work. Folder scanning and Shell operations stay off the UI thread.

Decoded frames share storage with the cache. The cache retains at most 192 MiB
of pixel data and 32 entries. A displayed image, decoder scratch space, Slint
frame upload and GPU textures have additional costs; this is not a process-RAM
cap. Large images and animations may be rejected before exhausting these budgets.

Current decoders produce full-resolution images. Scaled decode and streaming
animation are future work. Animations currently decode into a bounded collection
before playback. Timers sleep between frames and stop when paused/minimized.
See [reproducible measurements](docs/PERFORMANCE.md); no benchmark superiority
is claimed.

## Security philosophy

Every image is untrusted input. Validate dimensions with overflow-safe arithmetic,
limit file size and decoded storage, reject unsupported formats, and keep decoder
errors away from the UI thread. Reparse-point files are rejected on Windows;
open image handles deny writes and deletion while decoding. There is no image-
initiated network access. Recycle operations veto permanent-deletion fallback.

Resource limits and panic recovery are not a sandbox. Native faults, allocator
aborts, renderer faults and codec-internal allocations need further defense.
Read [SECURITY.md](SECURITY.md) and the
[security architecture](docs/SECURITY_ARCHITECTURE.md). Report sensitive problems
privately through GitHub Security Advisories.

## Roadmap

- [AVIF decoding](https://github.com/cubiix3/Kova-Image/issues/3) with an acceptable license, bounded memory and repeatable builds.
- [Progressive/scaled decode and streaming animations](https://github.com/cubiix3/Kova-Image/issues/4), with measured navigation tuning.
- [Fuzzing and stronger file identity checks](https://github.com/cubiix3/Kova-Image/issues/6), including decoder isolation evaluation.
- [Validated portable packages and an installer](https://github.com/cubiix3/Kova-Image/issues/5), with signing and opt-in file associations.
- Broader GPU/DPI/accessibility validation and color-management evaluation.
- Evaluate HEIC/HEIF and JPEG XL; hardened SVG and RAW only if justified.

No image editing, albums, tags, cloud, AI, video or PDF support is planned.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Keep changes focused on the viewer.
Architecture and current tradeoffs are described in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## License

MIT OR Apache-2.0, matching [Kova File](https://github.com/cubiix3/Kova-File-Manager).
See [LICENSE](LICENSE), [LICENSE-MIT](LICENSE-MIT), and
[LICENSE-APACHE](LICENSE-APACHE). Dependencies retain their own licenses.

Made with [Slint](https://slint.dev). See
[third-party notices](THIRD_PARTY_NOTICES.md) for licensing and Kova asset provenance.
