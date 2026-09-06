# Initial validation

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
  The current suite contains **23 passing tests**, with no ignored tests.
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
- No installer, signing, automatic associations or single-instance reuse.
- No external comparative benchmarks or cold-start claims.

The initial code is suitable for evaluation and iteration, not a claim that every
V1 objective or every hostile image is already handled.
