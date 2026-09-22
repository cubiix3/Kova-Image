<p align="center">
  <img src="docs/images/banner.svg" width="100%" alt="Kova Image - Open a file. See it immediately. Move on.">
</p>

<h1 align="center">Kova Image</h1>

<p align="center">
  <b>A fast, quiet image and video viewer for Windows.</b><br>
  Native Rust. No accounts, no cloud, no telemetry. Just your files.
</p>

<p align="center">
  <a href="https://github.com/cubiix3/Kova-Image/releases/latest"><img src="https://img.shields.io/github/v/release/cubiix3/Kova-Image?label=download&color=86d5f4&labelColor=101214" alt="Latest release"></a>
  <a href="https://github.com/cubiix3/Kova-Image/actions/workflows/ci.yml"><img src="https://github.com/cubiix3/Kova-Image/actions/workflows/ci.yml/badge.svg" alt="Windows CI"></a>
  <a href="https://github.com/cubiix3/Kova-Image/actions/workflows/security.yml"><img src="https://github.com/cubiix3/Kova-Image/actions/workflows/security.yml/badge.svg" alt="Dependency audit"></a>
  <img src="https://img.shields.io/badge/Windows-10%20%7C%2011%20x64-253e4b?labelColor=101214" alt="Windows 10 and 11, x64">
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-253e4b?labelColor=101214" alt="MIT OR Apache-2.0"></a>
</p>

<p align="center">
  <a href="#download">Download</a> &nbsp;&middot;&nbsp;
  <a href="#highlights">Highlights</a> &nbsp;&middot;&nbsp;
  <a href="#supported-formats">Formats</a> &nbsp;&middot;&nbsp;
  <a href="#keyboard-and-mouse">Shortcuts</a> &nbsp;&middot;&nbsp;
  <a href="#building-from-source">Build</a> &nbsp;&middot;&nbsp;
  <a href="docs/README.md">Docs</a>
</p>

<p align="center">
  <img src="docs/images/viewer.png" width="900" alt="Kova Image showing a photo with floating navigation, zoom and fit controls">
</p>

---

## Why Kova Image?

Opening a picture should take a moment, not a splash screen. Kova Image opens
a file, shows it, and lets you move through its folder with the arrow keys.
The interface stays out of the way: a compact titlebar and floating controls
fade out after two seconds so the image is all you see.

It is a **viewer, not an editor**, built with **Rust, [Slint](https://slint.dev)
and official Windows APIs**. It is a sibling of
[Kova Screen](https://github.com/cubiix3/Kova-Screen).

> [!NOTE]
> **Early development, version 0.1.0.** Kova Image is ready to try, with known
> limitations. Release builds are unsigned, so Windows SmartScreen warns on
> first run. See the [validation record](docs/VALIDATION.md) for what has been
> tested.

## Highlights

|  |  |
| --- | --- |
| ⚡ **Instant navigation** | The current image always wins. The next two files in your direction of travel are decoded in advance. |
| 🎞️ **Animations and video** | GIF, APNG and animated WebP with correct timing and loops. Local MP4, MOV and MKV through Windows Media Foundation. |
| 🎨 **Real color** | Embedded ICC profiles are converted to sRGB. EXIF orientation is applied automatically. |
| 🧠 **Bounded memory** | Images are decoded at the size your view needs, and the cache is capped at 192 MiB of pixels. |
| 🛡️ **Careful with files** | Every file is treated as untrusted. Deleting goes to the Recycle Bin, and `Ctrl+Z` brings it back. |
| 🪟 **At home on Windows** | Per-user install, Open with registration, high contrast and reduced motion follow your system settings. |

## Screenshots

| Start screen | Local video |
| :---: | :---: |
| [![Dark start screen with a compact titlebar and an Open file button](docs/images/empty.png)](docs/images/empty.png) | [![Video playback with timeline, time and volume controls](docs/images/video.png)](docs/images/video.png) |
| Nothing to set up. Open or drop a file. | Play, pause, seek and volume, with controls that hide themselves. |

All screenshots come from the running app with synthetic test files. They are
not mockups and contain no personal media.

## Download

1. Download **`Kova-Image-<version>-x64-setup.exe`** from the
   [latest release](https://github.com/cubiix3/Kova-Image/releases/latest).
2. Run it. It installs for your user only and needs no administrator rights.
3. Optional: in the app, open **More → Settings → Register Kova Image for Open with**,
   then pick Kova Image as your default viewer in Windows Settings.

Prefer not to install? Download the **portable ZIP** from the same release,
unpack it anywhere and start `kova-image.exe`. Uninstall through
**Settings → Apps** like any other app.

<details>
<summary><b>Command line options</b></summary>

```powershell
kova-image.exe                                    # start screen
kova-image.exe "C:\Pictures\example.png"          # open an image
kova-image.exe "C:\Videos\example.mp4"            # open a video
kova-image.exe --software "C:\Pictures\photo.jpg" # CPU renderer instead of OpenGL
kova-image.exe --register-file-associations       # per-user Open with registration
```

Registration never touches the protected `UserChoice` defaults; Windows keeps
the final say. See [file associations](docs/FILE_ASSOCIATIONS.md). Video needs
the Windows Media Foundation components, which Windows N editions may lack.

</details>

## Features

| Area | What you get |
| --- | --- |
| **Open and browse** | Command line, file picker or drag and drop. Natural sorting (`img2` before `img10`), first/last, and folders that mix images and videos. |
| **View** | Fit to window, fit width, 100%, zoom around the cursor, pan, rotate and flip. The view is never written back to the file. |
| **Animation** | Pause and resume, per-frame timing, loop counts, and the first frame appears while the rest decodes. |
| **Video** | Play/pause, timeline, elapsed and total time, volume, mute and optional looping. Video keeps playing while you drag the window. |
| **Windows actions** | Copy image or path, move to the Recycle Bin and undo, Show in Explorer, Open with. |
| **Interface** | Dark, compact chrome, fullscreen, auto-hiding controls, brief on-screen feedback and visible keyboard focus. |

## Supported formats

| Format | Support |
| --- | --- |
| **JPEG** | Still image with EXIF orientation and ICC color |
| **PNG / APNG** | Still images and animation with blending and disposal |
| **GIF** | Animation with timing, loops, transparency and disposal |
| **WebP** | Still and animated |
| **BMP, ICO** | Still image |
| **TIFF** | First page |
| **MP4 / M4V, MOV, MKV** | Through Windows codecs; H.264/AAC is tested |
| **WebM** | Plays if a Windows codec is installed, otherwise shows a clear error |
| AVIF, HEIC/HEIF, JPEG XL, SVG, RAW | Not supported yet ([roadmap](#roadmap)) |

Formats are detected from file content, not only the extension. Videos must be
self-contained local files; streaming, subtitles and DRM are out of scope. See
the [video architecture](docs/VIDEO.md) for details.

## Keyboard and mouse

| Action | Shortcut |
| --- | --- |
| Open a file | `Ctrl+O` or drag a file onto the window |
| Previous / next | `←` / `→`, mouse Back / Forward |
| First / last | `Home` / `End` |
| Zoom in / out | `+` / `-` or the mouse wheel |
| Fit / fit width / 100% | `0` / `W` / `1` |
| Pan | Drag with the left mouse button |
| Fullscreen | `F11` or double-click; `Esc` to leave |
| Pause / play | `Space` |
| Seek video 5 s | `Ctrl+←` / `Ctrl+→` |
| Mute | `M` |
| Rotate right / left | `R` / `Shift+R` |
| Flip horizontal / vertical | `H` / `V` |
| File info | `I` |
| Copy image / copy path | `Ctrl+C` / `Ctrl+Shift+C` |
| Move to Recycle Bin / undo | `Delete` / `Ctrl+Z` |

In Settings you can switch the mouse wheel from zooming to browsing, turn off
auto-hide, and change looping and autoplay. Settings are stored in
`%LOCALAPPDATA%\Kova Image\settings.conf`.

## Building from source

You need Rust and the **Desktop development with C++** workload with a Windows
SDK, from Visual Studio or the standalone Build Tools. The toolchain is pinned
to Rust 1.95.0 (MSVC) in `rust-toolchain.toml`.

```powershell
git clone https://github.com/cubiix3/Kova-Image.git
cd Kova-Image
.\scripts\cargo-msvc.ps1 build --locked --release --bin kova-image
```

`cargo-msvc.ps1` finds Visual Studio and loads its build environment for you.
Before opening a pull request, run the same checks as CI:

```powershell
.\scripts\cargo-msvc.ps1 fmt --all -- --check
.\scripts\cargo-msvc.ps1 clippy --locked --all-targets -- -D warnings
.\scripts\cargo-msvc.ps1 test --locked
```

Packaging and the installer are described in
[Windows builds and packaging](docs/WINDOWS_RELEASE.md).

## Under the hood

<details>
<summary><b>Performance</b></summary>

- One decode worker holds only the latest request, so rapid key presses never
  queue up work. Stale results are dropped before they reach the screen.
- Images are fitted to the pixels the view needs and cached at that size in a
  weighted LRU (192 MiB of pixels, 32 entries). Zooming past it decodes the
  original again.
- Folder scans read names and types only: no thumbnails, no database.
- Video runs on its own worker. Media Foundation keeps audio and video in sync,
  only the latest frame is kept, and presentation is capped at 3840 × 2160.
  Minimizing pauses playback.

Numbers and methodology: [performance protocol](docs/PERFORMANCE.md) ·
[video measurements](docs/VIDEO_MEASUREMENTS.md) · [design notes](docs/DESIGN.md)

</details>

<details>
<summary><b>Security</b></summary>

- Every image and video is untrusted input. Dimensions are checked with
  overflow-safe arithmetic, and file size, decoded pixels and animation frames
  have hard limits.
- Files are opened without following reparse points and locked against
  writes while decoding. MP4/MOV files that reference external data are rejected.
- Decoder panics are caught off the UI thread. Opening a file never triggers
  network access.
- Recycling refuses a permanent-delete fallback. Restoring asks before it
  would replace an existing file.

These limits are not a sandbox. Read [SECURITY.md](SECURITY.md) and the
[security architecture](docs/SECURITY_ARCHITECTURE.md), and report issues
privately through GitHub Security Advisories.

</details>

## Roadmap

- [ ] [AVIF](https://github.com/cubiix3/Kova-Image/issues/3), once it can ship without a system codec or a network build
- [ ] [Downscaling inside the codec](https://github.com/cubiix3/Kova-Image/issues/4) and measurements on real photo folders
- [ ] [Fuzzing and stronger file identity checks](https://github.com/cubiix3/Kova-Image/issues/6)
- [ ] [Signed releases](https://github.com/cubiix3/Kova-Image/issues/5) and a clean-machine install test
- [ ] Wider coverage of monitor color, screen readers, GPUs and DPI setups
- [ ] HEIC/HEIF and JPEG XL after a license and memory review

Out of scope by design: editing, albums, tags, cloud sync, AI features,
streaming and PDF.

## Contributing

Bug reports and focused pull requests are welcome. Read
[CONTRIBUTING.md](CONTRIBUTING.md) first, and see
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for how the pieces fit together.

## License

Kova Image is dual-licensed under [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE), at your option. Dependencies keep their own
licenses; see [third-party notices](THIRD_PARTY_NOTICES.md).

<p align="center">
  <a href="https://slint.dev"><img src="assets/made-with-slint.png" width="140" alt="Made with Slint"></a>
</p>
