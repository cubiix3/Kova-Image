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
- Windows CI is configured to run formatting, check, strict Clippy, tests,
  release compilation and portable packaging.
- A separate cargo-audit workflow leaves maintenance warnings visible.
- Local formatting, check, strict Clippy, tests and release compilation pass.
  The current suite contains **62 passing tests**, with no ignored tests.
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
- Display-sized retention is implemented after the codec's full canvas decode.
  Codec-level progressive or DCT-scaled decode is not. Animation shows its first
  fitted frame while later frames are still collected.
- AVIF, HEIC/HEIF, JPEG XL, SVG, camera RAW and the rarer formats (TGA, PNM,
  QOI, DDS, HDR, EXR, farbfeld) are decoded by Kova Image itself and tested
  with small generated fixtures (`tests/fixtures`, `scripts/image-fixtures.py`).
  Limits: animated AVIF/HEIC sequences are not shown, HDR (PQ/HLG) pictures
  are shown as stored, RAW shows the embedded JPEG
  preview rather than developed sensor data, and HEIC covers HEVC intra
  pictures only (see [DEPENDENCIES.md](DEPENDENCIES.md#hevc-and-patents)).
  The HEVC decoder was compared bit for bit with ffmpeg on a generated corpus
  and with damaged streams; real phone and camera files have been tried by hand
  only. The Explorer preview provider was checked through the real Shell on
  WebP, SVG, AVIF, JPEG XL and RAW files (`cargo test -p kova-thumbnails --
  --ignored`; it changes the HKCU registration, so run it on a development
  machine and register again afterwards).
- Previews were then checked at scale with `scripts/preview-check.ps1`, which asks
  the Shell for each thumbnail with the cache bypassed, and
  `scripts/preview-crosscheck.py`, which compares each one with an independent
  decoder (Pillow). On the development machine: one generated sample for every
  supported extension, every camera RAW extension and several videos (72 files, all
  with a preview), and 36,000 pictures and videos in the user folders, among them
  about 25,000 TGA and DDS files of game assets and 10,000 PNG: all had a preview.
  Of 1,507 TGA and DDS files in one asset folder, 1,506 matched Pillow
  (embedded ICC profiles applied) and the one Pillow cannot read, a colour-mapped
  RLE TGA, matched an independent decoder written for the check pixel for pixel.
  The check found and led to fixes for: TGA files named `.dds` or `.png`, 8 bit
  palettized DDS, APNG, and a leftover preview handler of an uninstalled program.
  It does not prove clean-machine behaviour or other users' registry states.
- Audio (MP3, M4A, AAC, WAV, FLAC, Ogg Vorbis, Opus, WMA) was checked with generated
  files (`scripts/audio-fixtures.py`; admission, tags, covers and damaged files in
  `tests/audio.rs`), with the opt-in `examples/audio_probe.rs`, which plays a file through
  Media Foundation (muted) with pause, seek, end, loop and release of the file, and with
  `scripts/ui-smoke.py --state=audio` (cover, panel without cover, file information,
  error state, return to images). Every generated format plays twelve times out of twelve,
  except Ogg Vorbis and Opus, which this Windows refuses without Microsoft's "Web Media
  Extensions" and which show a message that names them.
  The 1,729 audio files on the development machine (1,672 WAV, 43 MP3, 9 FLAC, mostly
  game sounds and music) were read with `examples/audio_scan.rs`: all were admitted except
  five unusual WAV variants (big-endian RIFX, RF64) that Windows does not play either; tags
  came from 10 files and a cover from 1. All 43 MP3 and 24 chosen WAV files, down to a sound
  of 10 milliseconds, played to the end and looped. This found that sounds shorter than the
  state report interval left the display on the old state after being played again; every
  command now causes an immediate report. FLAC files shorter than about one second report
  no duration in Windows and are refused, and some of those play unreliably.
  Not tested: audible output (the checks are muted), Bluetooth or exclusive-mode devices,
  a clean machine, and text in legacy code pages (Korean tags in "Latin-1" fields show as
  Latin-1, as in other players on a Western Windows).
- Embedded ICC profiles are converted to sRGB. Monitor profiles, HDR and a wide
  color corpus are not. Fuzzing and decoder process isolation remain future work.
- Cache budgets do not include every decoder scratch allocation, copied renderer
  frame or GPU texture. There is no hard total-process memory guarantee.
- Shell operations have path-based race limitations; see security architecture.
- No code signing, automatic default takeover or single-instance reuse. The
  per-user installer is unsigned and untested on a clean machine.
- No external comparative benchmarks or cold-start claims.

The initial code is suitable for evaluation and iteration, not a claim that every
V1 objective or every hostile image is already handled.

## Navigation and accessibility, 2026-09-22

The GUI smoke test exposed a startup race: display-size refinement could replace
the first request before its folder scan completed, leaving Previous/Next disabled.
The refinement now carries the pending scan. The hardware- and software-rendered
GUI smoke tests pass navigation and the complete image, animation, focus and
compact-layout path in the final local build. Two earlier software runs failed
at synthetic pointer events in fullscreen or the small-window More button, so
the harness remains timing-sensitive. Both video GUI smoke tests passed.

With Slint's accessibility feature enabled, Windows UI Automation exposed the
video seek and volume controls as sliders with names and live values (observed
`0:01 / 0:06` and `70%`). Narrator behavior and physical high-contrast/DPI changes
remain untested. Generated MP4, MOV and MKV passed the native video probe;
WebM still returned the missing-codec error `0xC00D5212` on this machine.

The local portable package was built with the new AccessKit license texts. Its
SHA-256 matched, extraction contained all four active AccessKit license folders,
and the extracted executable opened a generated JPEG through the software path.
This is still a developer-machine check, not clean-VM acceptance. The new CI
packaging step has not yet been observed on GitHub.

## Compact UI and local video ? historical Media Foundation checks

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
back with `scripts/associations-smoke.ps1` (at that time 16 capabilities):
quoted ProgID commands, OpenWith entries and RegisteredApplications were present. Existing
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

Local formatting, all-target check, strict Clippy, all Cargo tests and the
optimized release build passed at that time. This refinement adds no runtime dependencies.

## Large photos, sorting, slideshow and damage tests, 2026-09-28

Local formatting, all-target check, strict Clippy and all 62 Cargo tests pass on
Windows 11 Pro (build 26200), Rust 1.95.0.

New automated coverage: JPEG orientation, fitting and colour at several target
sizes; the 64 MP JPEG and 32 MP retained-bitmap limits; the resampler against the
`image` crate's Triangle filter; Explorer-order sorting with umlauts, date and
size orders and wrap-around; settings round trips; cloud reparse tag
classification; symlinks refused and plain files opened; content-detected video
and neighbour preload after a pause; EXIF lens and focal length; alpha detection.
`tests/robustness.rs` damages every supported format (truncation, bit flips,
spliced junk; 150 rounds per format at two target sizes) plus a hostile MP4 box
tree, and requires no contained decoder panic, no hang and no crash. It is a
deterministic regression net, not coverage-guided fuzzing.

The GUI smoke harness passes on both renderers for the default scenario and for
the empty, missing, corrupted, long-name, video and pinned states. It found one
real defect: the sharper decode that follows a zoom re-showed hidden controls.
That is fixed, and `--state=chrome` then passed three of three runs on the
default renderer. On the software renderer `--state=chrome` passed six of ten
runs; each failure was a "controls stay visible for 2.5 s while focus or a held
pointer is active" assertion at a different step. The harness documents that
real pointer movement or focus changes invalidate these inactivity timings, and
I did not attribute the failures to a code path.

Not verified: cloud placeholders (no sync provider was available; only the tag
classification and symlink refusal are tested), Narrator, physical high-contrast
and multi-monitor DPI, clean-machine installation, and code signing. Photos were
synthetic JPEGs; real 50 MP photo folders are still unmeasured.


## Bundled MKV/video decoders ? 2026-10-05

The previous Media Foundation probe played H.264/AAC MKV but rejected HEVC/AAC
MKV, H.264/AC-3 MKV and VP9/Opus WebM with missing-codec error `0xC00D5212`.
A reduced shared FFmpeg 8.1.3 build with dav1d 1.5.3 now decodes video from the
retained admitted handle; WASAPI outputs timestamped stereo PCM. Audio-only
files retain Media Foundation. Video decoding is software based.

Local Rust 1.95.0/MSVC checks pass: formatting, locked all-target workspace
check, all-target Clippy with warnings denied, 150 tests, and optimized viewer
and Explorer preview provider builds. Three opt-in tests remain ignored (HEVC
reference/damage corpus and registry-changing Explorer acceptance).

Muted player probes pass for H.264/AAC MP4, MOV and MKV; VP9/Opus and VP8/Vorbis
WebM; HEVC/AAC, HEVC/E-AC-3, H.264/AC-3, H.264/DTS, VP9/Opus and AV1/Opus MKV;
and 44.1 kHz AAC MP4. Each checks non-empty frames, pause/seek, an actual frame
after seeking to exact duration, end-of-file, looping and file-handle release
after acknowledged Stop. The MP3 audio-only probe passes the same clock/resource
checks. Probe files are separate copies, so a GUI holding another clip cannot
invalidate the writable-handle assertion.

Cargo fixtures decode HEVC/AC-3, AV1/Opus and VP9/Vorbis to non-empty RGBA and
non-silent finite PCM without a sound device. Rotation, pixel aspect ratio,
BT.709 and full-range conversion match generated reference PNGs. A 44.1 kHz AAC
fixture verifies continuous PCM timestamps and all 57,958 reference output
frames through resampler EOF. Timestamped audio queue tests cover gaps, late
samples and the two-second bound. Independent playback review found and verified
fixes for resampling delay/tail and out-of-range native seek timestamps.

The actual-window codec scenario passes for HEVC/AAC MKV with the software
renderer and AV1/Opus MKV with the default GPU renderer. It checks the generated
color pattern in the viewport, pause/clock stability, seek, mute and file info.
The full GPU video scenario also passes with saved autoplay=false, held seeking,
compact layout, fullscreen auto-hide and mixed/stale navigation. A renderer
capture after the synthetic mouse press lets the viewer process that event
before the outside-track move; no assertions were removed or relaxed.

The full software video scenario still fails an auto-hide/hover assertion around
a held synthetic drag. A diagnostic run verified held seeking but later failed
a fullscreen auto-hide assertion. This surface is not claimed passing; earlier
software smoke timing limitations are documented above. Targeted software MKV
playback and all native player probes pass.

The software image scenario passes navigation, transforms, animation controls,
fullscreen and compact layout. The audio GUI scenario passes covers/tags,
pause, unsupported-codec error handling, slideshow and return to image files.

DLL import checks find only the six shipped decoder DLLs and Windows/C runtime
libraries. The native configuration disables GPL, nonfree, encoders and network
features. Source archive hashes match the pinned recipes; packages include the
complete corresponding archives and license texts. The portable package passes
the dependency license check. ZIP extraction preserves the executable, preview
DLL, all six decoder DLLs and source archives byte for byte. HEVC and AV1 player
probes run successfully against those extracted DLLs. The extracted optimized
viewer opens HEVC MKV with the software renderer; its accessibility tree reports
640 x 360, six seconds and a paused 0:00 / 0:06 timeline with saved autoplay=false.
The Explorer preview DLL has no FFmpeg imports.

No user-provided failing video was available. Clean-VM playback/installation,
physical audio synchronization, HDR tone mapping, device-loss handling and every
codec/profile combination remain unverified. GitHub CI has the native build step
but has not run for this unpushed change. The existing installed viewer was not
replaced.


## Fitted animation memory accounting, 2026-10-16

Generated 1600 x 900, 48-frame GIF, APNG and WebP files were rejected with
`MemoryBudget` at a 640 x 360 display target. Admission charged every frame at
source resolution even though frames were already fitted before retention.
The sequence needs 44,236,800 stored RGBA bytes at that target, below 128 MiB.
GIF/APNG/WebP collection and JPEG XL now charge the fitted frame size. Source
pixel/dimension limits, codec allocation limits, frame count and the 128 MiB
retained-frame budget are unchanged.

Regression tests generate 72-frame GIF/APNG/WebP files and check all fitted
pixels, frame delays, original dimensions and the first-frame preview. Full-size
requests that exceed the budget still return `MemoryBudget`. A generated
25-frame RGB JPEG XL fixture checks the same storage behavior and full-size
refusal. The larger 48-frame JPEG XL variant still reaches its separate 512 MiB
codec allocation limit, which remains enforced.

Formatting, locked all-target workspace check, strict all-target Clippy and all
154 workspace tests pass (three existing opt-in tests remain ignored). These
include still-image formats, image damage/cancellation, animation disposal and
looping, audio and bundled video regressions. New animation GUI checks verify
changing rendered pixels and frame indices, pause stability and resume for GIF,
APNG, WebP and JPEG XL. Default image/navigation GUI and HEVC MKV GUI checks also
pass. No performance improvement is claimed.

Fixtures come from `scripts/animation-fixtures.py`; JPEG XL was encoded with a
developer FFmpeg 8.1.3 `libjxl_anim` encoder, not the bundled playback-only SDK.
The application launches no encoder. No failing personal GIF was provided, so
verification uses the generated files. Clean-machine acceptance and every file
variant remain outside these checks.


The optimized viewer and Explorer preview provider build successfully. The
portable package passes its license check. The per-user installation was backed
up and updated from that package; the installed viewer, preview DLL and six
video DLL hashes match the staged release. Settings and the existing GIF
UserChoice ProgId/hash remain unchanged. The installed release viewer was
checked with a generated GIF: window captures show changing animation pixels,
identical pixels while paused, and frame changes after resuming. This verifies
the installed application, beyond the debug tests and release build. GitHub CI
is tracked separately from these local checks.

## Default Apps registration refresh, 2026-10-07

The development machine still had the earlier 16-extension registration despite
having the newer executable installed. Running `scripts/associations-smoke.ps1`
against that permanent installation refreshed and verified all 67 currently
advertised extensions: 52 image, 5 video and 10 audio extensions. The script now
reads the extension lists from the Rust sources and checks Capabilities,
SupportedTypes, the quoted application and ProgID commands, OpenWithProgids and
RegisteredApplications. Every existing UserChoice ProgID/hash stayed unchanged.

The viewer now refreshes registration before opening its Default Apps Settings
page, so the list includes formats added since an earlier registration.
Local locked build/check, formatting, strict all-target Clippy, workspace tests
(145 passed, 3 ignored) and the optimized release build passed. The installed
executable was updated to that build and the registration check passed again.
This is a developer-machine registration check, not clean-machine acceptance or
proof that every video/audio codec is
available. The native player still depends on installed Windows codecs.

## Format, playback and window reliability, 2026-10-23

The current `origin/main` video/animation changes were fetched before this work.
Formatting, locked workspace/all-target check, strict all-target Clippy, locked
workspace tests, the optimized viewer and optimized Explorer preview DLL pass.
There are 182 passing tests and three existing opt-in ignored tests. The extra
one-test child-process result isolates the AVIF damaged-input regression and is
already included in that total. No dependency or lockfile changes were needed.

Generated regressions cover DDS row pitch, 16-bit UNORM and alpha modes; TGA
padding/footer/premultiplied alpha; seven extension aliases; RAW signature
precedence and usable preview selection; HEIF aggregate metadata/grid limits;
HEVC NAL count/cancellation; and AVIF packet ownership/decoder-panic containment.
See [FORMATS.md](FORMATS.md) for supported variants and remaining gaps. This does
not establish compatibility with every file carrying a supported extension.

The actual-window resize scenario passes with the default GPU renderer and the
software renderer: all eight edges/corners, anchored minimum size and ordinary
image panning. The developer display uses 125% scaling. Negative coordinates and
DPI-scaled hit zones have unit coverage; a physical drag between monitors with
different DPI settings remains unverified.

The actual-window delayed-video scenario passes with both renderers. It checks
that the previous pixels and zoom remain visible during a five-second open,
pending media cannot be deleted, canceled generations cannot replace the image,
the first valid frame adopts the new file, and a failed open retains the previous
picture with a failure notice. The software test waits for zoom refinement and
the subsequent renderer capture before comparing pixels; it does not compare an
unfinished render. This delay hook is absent from release builds.

The full image/navigation and full video GUI scenarios also pass with both
renderers, including transforms, animation pause/resume, keyboard focus,
fullscreen auto-hide, timeline seeking and compact layout. The image-pan check
allows less than one logical pixel of coordinate rounding at fractional DPI;
the observed vertical delta was 50.4 rather than exactly 50 logical pixels.

Native probes pass H.264 1080p60, HEVC/AC3, AV1/Opus, VP9/Vorbis, color and rotated
video: play, pause, seek, exact EOF, loop and confirmed file-handle release. The
rotation fixture now actually contains a display matrix and has a portrait pixel
reference. Audio-endpoint initialization failure has an injected regression:
video continues silently, while real decode errors still fail.

The controlled debug pacing workload delivers 68/72 frames in each of three final
runs versus 38/72 before this change, with the same simulated 10 ms presentation
work. A generated 1080p60 six-second clip delivers 345/360 frames. See
[VIDEO_MEASUREMENTS.md](VIDEO_MEASUREMENTS.md) for the comparison, timer diagnostic
and limits. These measure delivery to the callback, not display synchronization.

A single-pass source security audit covered 59 deduplicated first-party files
and 17 attack surfaces. Five availability/resource findings were fixed and
source-rechecked; no remaining reportable finding was identified in that reviewed
scope. This is a partial audit, not a claim that the whole program is secure.
Deep HEVC decoding stages, all BC7 paths, native process faults and clean-machine
runtime behavior were not exhaustively verified. No ImageGlass source was copied;
the project retains its own decoders and MIT OR Apache-2.0 contribution policy.

The portable package passes dependency-license collection and the per-user
installer builds successfully. After backing up the previous installation, the
installer updates the viewer, preview DLL and six bundled video DLLs; all eight
installed binary hashes match the staged optimized build. Setup reports no
restart required. File-association verification passes all 74 extensions (59
image, five video, ten audio), including SupportedTypes, Capabilities, quoted
commands and Open with. User settings and the sampled pre-install default
ProgID/hash values remain unchanged; the registration test also checks every
advertised extension's UserChoice before and after its refresh.

The installed optimized viewer opens the generated padded DDS, R16/RGBA16 UNORM
DDS, zero-padding-alpha TGA and TARGA files with both renderers (ten first-render
checks). A separate installed-release 1080p60 runtime sample uses isolated
settings and looping silent video. Over five seconds per phase it records
81.801% of one logical CPU while playing and zero CPU-counter increments while
paused or minimized. This short sample does not prove zero idle work or leak
freedom; current absolute measurements are recorded in VIDEO_MEASUREMENTS.md.
Clean-VM installation, physical audio sync and arbitrary user videos remain
outside this developer-machine validation. GitHub Windows CI is required
separately before merging.
