# Reproducible performance measurements

Use release builds and the same image corpus, window size, renderer, power plan
and machine. Record Windows build, CPU/GPU, driver, display scale and memory.
Generated fixtures contain no private images and are reproducible from source.

```powershell
.\scripts\cargo-msvc.ps1 build --locked --release --bins --examples
.\target\release\examples\fixtures.exe artifacts\fixtures
.\target\release\kova-bench.exe artifacts\fixtures\image1.jpg artifacts\fixtures\image2.png artifacts\fixtures\image10.webp artifacts\fixtures\image20-large.png
.\scripts\measure.ps1 -Image artifacts\fixtures\image1.jpg -Runs 10
.\scripts\measure.ps1 -Image artifacts\fixtures\image1.jpg -Runs 10 -Software
.\scripts\runtime-measure.ps1 -Animation artifacts\fixtures\image3.gif -SecondsPerPhase 10
```

`kova-bench` reports file-open + decode wall time, dimensions, frame count and
retained RGBA bytes. It does not measure rendering or OS-cold reads.
`--measure <output.json>` records process-main to the first completed render
whose frame started with a loaded image. `measure.ps1` adds external launch-to-
observation latency, peak working set and private bytes. The external value
includes 20 ms observation granularity and process-launch overhead. No per-frame
logging or measurement runs during ordinary release use.

Slint's software renderer does not expose the rendering notifier. In that mode,
the JSON explicitly reports `first_render_ms: null` and `image_ready_ms` (decoded
image submitted to the UI). Do not compare this value to hardware first-render
completion as if they measured the same event.

The first process after building is not necessarily disk-cold. Use a fresh boot
or a controlled Windows VM for a cold-start experiment; document how the OS cache
was handled. Do not label repeated launches "cold startup".

## Navigation and animation protocol

1. Open the generated corpus and wait for next/previous preloads.
2. Measure repeated next/previous operations separately from uncached navigation.
3. Navigate a larger corpus repeatedly; sample private bytes and working set.
4. Record peak RAM for `image20-large.png`, including UI texture upload.
5. Measure process CPU seconds over 30 seconds of GIF playback, then the same
   paused and minimized. Divide CPU time by elapsed time; state whether reporting
   one logical CPU or machine-wide percentage.
6. Try rapid navigation during a large uncached load; assert the requested file
   is the one displayed and record latency outliers, not just an average.

Current automated tooling covers per-format decode, first-render/startup/RAM,
and GIF CPU/private-memory sampling while playing, paused and minimized.
Cached-vs-uncached end-to-end navigation, long-session RAM and truly
cold startup still need a dedicated benchmark harness. AVIF cannot be measured
until its decoder is implemented. No performance superiority is claimed.

## Visual/interaction verification

```powershell
.\scripts\cargo-msvc.ps1 build --locked --bin kova-image
python -m pip install Pillow
python scripts\ui-smoke.py
```

The local smoke harness uses original generated fixtures, sends window messages
to its own viewer, and captures Slint's renderer through a debug-only hook.
It does not capture the user's desktop. Outputs go to ignored `artifacts/`.
It covers navigation, natural order, fit/zoom, transforms, fullscreen, animation,
information UI and small-window resize. This is a local test, not an unattended
desktop test on GitHub's hosted runner.

The redesigned interface also checks Tab/Space activation, active fit modes,
fullscreen auto-hide/wake, popup dismissal without leaving fullscreen, and the
640 × 420 minimum layout. The harness keeps its window in the background and
uses only its own HWND. Additional visual states can be captured with
`--state=empty`, `--state=missing`, `--state=corrupted` and `--state=long-name`.
Each checks the expected state and keyboard focus at minimum size.

Pass `--software` to repeat GUI checks on the fallback. Pass `--clipboard` only
when willing to replace the clipboard with a generated fixture; that opt-in
tests native Copy Path and Copy Image without reading previous clipboard data.

See [the initial measurement record](MEASUREMENTS.md) for the observed local
values and their limits.
