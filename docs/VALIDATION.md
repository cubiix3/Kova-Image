# Initial validation

The initial Windows implementation is undergoing local and GitHub CI validation.
This file records actual checks and remaining coverage gaps; it is not a stable
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

## Local GUI coverage

The debug viewer is exercised with a repeatable Python/Win32 smoke harness.
Renderer snapshots are reviewed for fit, rotation, fullscreen, image information
and a 660 × 460 window. The harness verifies next/first/last, natural order,
actual size, zoom, rotation, horizontal flip, fullscreen, GIF pause/resume and
stable paused frame index. Synthetic input supplies Windows character messages
as well as key messages so Winit sees actual logical character keys.

## Remaining validation and implementation gaps

- Native drag/drop, clipboard, Recycle Bin, picker, Explorer/Open with, touchpad,
  mouse Back/Forward and physical multi-monitor DPI need broader hands-on testing.
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
