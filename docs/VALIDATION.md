# Validation record

Validated locally on Windows 11 Pro (build 26200), x64, Rust 1.95.0. The Windows
GitHub CI has also completed successfully, including the release executable.
This records actual checks and remaining coverage gaps; it is not a stable
release acceptance claim.

## Automated checks

- Cargo unit/integration tests exercise natural sorting, extension/content
  handling, navigation boundaries, cache budget/LRU eviction, dimension/overflow
  rejection, stale tickets, missing/corrupted/disappearing files, Unicode and
  extended Windows paths, and animation timing/loops/disposal.
- Actual generated JPEG, PNG, WebP, BMP, TIFF, ICO, GIF, APNG and animated WebP
  files pass through the production decoder in integration tests.
- Windows CI runs formatting, check, strict Clippy, tests and release compilation.
- A separate cargo-audit workflow leaves maintenance warnings visible.
- Local formatting, check, strict Clippy, tests and release compilation pass.
  The current suite contains **29 passing tests**, with no ignored tests.
- The dependency graph is populated; GitHub's SBOM endpoint returns packages.
- Dependabot alerts/security updates, secret scanning, push protection and private
  vulnerability reporting were enabled and read back from GitHub's API.

## Local GUI coverage

The debug viewer is exercised with a repeatable Python/Win32 smoke harness.
Renderer snapshots are reviewed for fit, rotation, fullscreen, image information
and a 660 × 460 window. The harness verifies next/first/last, natural order,
actual size, zoom, rotation, horizontal flip, fullscreen, GIF pause/resume and
stable paused frame index. Synthetic input supplies Windows character messages
as well as key messages so Winit sees actual logical character keys.

The same harness passes with the software renderer. Mouse Back/Forward, wheel
zoom and panning are verified through actual window messages. A drag of 100 × 50
logical pixels changes pan by exactly that amount. Native Copy Image and Copy
Path return success when invoked from the running viewer with generated fixtures.
The manual `windows_smoke` example successfully recycles a newly generated PNG
whose path includes Unicode; no original user image is used for that test.

The settings and More panels were visually reviewed. The README screenshot is
captured from Slint's renderer with the original fixture generator; it contains
no desktop capture, personal files or mock application chrome.

The local portable-package script produces an unsigned ZIP and SHA-256, with
upstream license texts. A first packaging attempt identified missing upstream
workspace license files; version-specific source texts now supplement those
archives. ZIP-incompatible Cargo source timestamps are normalized in staging.

## Interface refinement

The redesigned Kova chrome was checked with real Slint renderer captures using
the hardware and software paths. The local GUI harness verifies active view
modes, Tab/Space control activation, image focus restoration, fullscreen
auto-hide/wake, Escape dismissing a popup while preserving fullscreen, outside
click dismissal and the 640 × 420 minimum window. Empty, missing, corrupted and
long-filename states were also captured and visually reviewed.

Menus align labels and shortcuts separately; short windows scroll popovers.
Settings and image information share the same panel surfaces. Paths in debug
captures are local test fixtures; only the path-free viewer and empty-state
captures are published in documentation. See [DESIGN.md](DESIGN.md).

## Remaining validation and implementation gaps

- Native OLE drag/drop, the picker, Explorer/Open with, clipboard interoperability
  with other applications, touchpad and physical multi-monitor DPI need broader
  hands-on testing. They are not marked as manually validated by the smoke test.
- Full-resolution decoding and full bounded animation collection remain the
  initial policy. No progressive/scaled decode or animated-image streaming yet.
- AVIF, HEIC/HEIF, JPEG XL, SVG and RAW are absent.
- Color-managed output/HDR, complex format corpus coverage, fuzzing and decoder
  process isolation are future work.
- Cache budgets do not include every decoder scratch allocation, copied renderer
  frame or GPU texture. There is no hard total-process memory guarantee.
- Shell operations have path-based race limitations; see security architecture.
- No installer, signing, automatic default takeover or single-instance reuse.
- No external comparative benchmarks or cold-start claims.

The initial code is suitable for evaluation and iteration, not a claim that every
V1 objective or every hostile image is already handled.

## Compact UI and local video

The earlier compact layout used a 50 px header and a 60 px bottom bar, with
32 px controls and rounded groups. Real renderer captures cover the empty state,
image view, video transport, short windows and popovers. The mixed navigation GUI check switches
image -> MP4 -> MOV -> WebM -> MKV and verifies that late results cannot replace
the newest file. Timeline seeking, pause/clock stability, mute, file information
and fullscreen hide/wake are checked against the running native player.

The muted `video_probe` checks H.264/AAC MP4, MOV and MKV: non-empty RGBA frames,
pause/seek, end-of-file, looping and a writable file handle after acknowledged
Stop. VP8/Vorbis and VP9/Opus WebM files return a codec error on this machine;
successful WebM playback is not claimed. Deterministic Cargo tests cover mixed
navigation, cancelled admission, read locks, Unicode/long paths, missing files,
external references, compressed/reference movie rejection, box lengths and depth.
See [VIDEO.md](VIDEO.md) for repeatable commands and native limitations.

Per-user registration was run from a permanent local Programs folder and read
back with `scripts/associations-smoke.ps1`: all 16 capabilities, quoted ProgID
commands, OpenWith entries and RegisteredApplications were present. Existing
UserChoice ProgID/hash values stayed unchanged. This verifies registration on
the development machine, not every Explorer/default-picker behavior on clean
Windows installations. The user still chooses defaults in Windows Settings.

The software rendering path and saved video-autoplay=false first-open path also
pass GUI checks. Image startup and 1080p video CPU/RAM samples are recorded in
[VIDEO_MEASUREMENTS.md](VIDEO_MEASUREMENTS.md). The release executable and local
unsigned portable package build successfully; no stable release was published.

The settings scroll extent was corrected so Windows registration/default-app
helpers remain reachable. The bottom section was captured after real wheel
input; it is not merely present outside the visible panel.

## Floating controls and action feedback — 2026-09-10

The 40 px titlebar and floating viewing controls were checked locally with
Slint's default GPU renderer and its software fallback. The windowed titlebar
remains available when controls hide. The dedicated `--state=chrome` check
verifies unchanged viewport, image dimensions and pan, and compares rendered
image pixels before and after hiding. It also verifies feedback and expiry
while controls are hidden, Tab recovery, focus/menu/held-drag retention, panning
beside the controls, and suppression of canvas pan/zoom over the visible bar.

The existing image/animation workflow passes on both renderers, including
Tab/Space activation and the 640 × 420 layout. Native video checks pass on both
renderers, including a held seek outside the timeline without an early seek or
hidden controls, mute feedback while controls are hidden, pause/clock stability,
mixed-media navigation and fullscreen hide/wake.

The disabled-auto-hide setting retains controls in both window modes. Empty,
missing, corrupted and long-filename states pass the state/focus checks; the
new image, video, empty and compact layouts were visually reviewed.

The harness starts without requesting activation and resets isolated test
preferences on each run. GPU visual captures receive a second render; timed
wake assertions sample once because saving large fullscreen PNGs can exceed
the inactivity timeout. Captures are generated test content, not desktop images.

Local formatting, all-target check, strict Clippy, all 29 Cargo tests and the
optimized release build pass. This refinement adds no runtime dependencies.
