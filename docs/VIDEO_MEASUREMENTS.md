# Viewer and video measurements

## FFmpeg worker pacing, 2026-10-23

These checks used Rust 1.95.0's Windows x64 MSVC **debug** build and the bundled
FFmpeg 8.1.3/dav1d 1.5.3 playback backend. The headless `video_probe --pacing`
decoded a synthetic silent H.264 clip, 64 x 48 at 60 fps, 1.20 seconds long
(72 frames). Its delivery callback slept for a requested 10 ms on each frame to
simulate renderer/upload work. Compilation was complete before each playback
measurement; no other project build ran during playback. The final policy was
repeated three times. These short runs did not record a fresh hardware, driver
or power-plan inventory, so the historical machine details below must not be
treated as current measurement metadata.

The worker previously waited another 16 ms after decode, scaling and delivery.
Subtracting that work from the interval still performed poorly with the default
Windows timer resolution. The controlled comparison retained the same scoped
1 ms timer request and changed only the wait policy:

| Worker wait policy | Timer request | Delivered / source frames |
| --- | --- | ---: |
| Fixed 16 ms after work, initial check | Default | 38 / 72 |
| Remaining interval after work, initial check | Default | 39 / 72 |
| Fixed 16 ms after work, controlled baseline | Scoped 1 ms | 45 / 72 |
| Remaining interval after work, final | Scoped 1 ms | 68 / 72 in each of three runs |

In the controlled baseline, the simulated upload averaged 10.320 ms, with a
10.961 ms maximum. In the final runs the means were 10.405, 10.384 and 10.373 ms;
the maxima were 10.803, 10.605 and 10.969 ms. Probe playback wall time was
approximately 1.21-1.22 seconds. The two initial default-timer checks preceded
the callback-timing instrumentation; the 45-to-68 comparison uses the same
instrumented probe and timer policy.

A separate 50-sample Rust timing diagnostic requested a 6.667 ms condition-variable
wait. Its observed median was 21.014 ms (17.635-23.127 ms) with the default timer
resolution and 7.333 ms (6.823-8.977 ms) with a 1 ms request. A requested 10 ms
thread sleep had medians of 10.460 and 10.462 ms respectively. This diagnosed
coarse condition-variable waits on this host; it does not establish a universal
Windows timeout value.

The final worker also delivered **345 / 360 frames** from a generated silent
1920 x 1080, 60 fps, six-second H.264 clip with the same simulated upload work.
Probe playback wall time was 6.01 seconds; the callback averaged 10.264 ms and
peaked at 11.183 ms. This was one run, with no retained pre-change 1080p run.

These figures measure frames delivered by the playback worker to a synthetic
callback. They do not measure Slint texture upload, GPU presentation, monitor
refresh or vsync, audible synchronization, release-build throughput, battery
consumption, long-session behavior or every file/codec. The final worker still
dropped some source frames in these checks. Host scheduling and other load can
change the result; no stutter-free or all-files guarantee follows from it.

The scoped timer guard requests precision only during active, visible video and
releases it for pause, hidden playback, EOF, Stop, source replacement, error and
shutdown. Audio-only playback uses its existing clock without this request.
Successful requests are balanced even during worker teardown; no permanent
timer change or busy wait is introduced. The additional wakeup precision can
cost power while playing, which these checks did not measure. See
[VIDEO.md](VIDEO.md) for Windows-version behavior and the primary API reference.

### Installed release runtime sample

The complete optimized package was installed with its six decoder DLLs and
matching binary hashes. A separate short sample used the generated silent
1080p60 clip above, looping with isolated settings, default GPU renderer and
125% display scaling. `scripts/runtime-measure.ps1` sampled five seconds per
phase after startup. CPU is normalized to one logical CPU, not the whole host.

| Phase | CPU ms | One-core CPU | Private bytes | Working set bytes |
| --- | ---: | ---: | ---: | ---: |
| Playing | 4,093.750 | 81.801% | 285,130,752 | 275,447,808 |
| Paused | 0 | 0% at counter resolution | 275,767,296 | 266,170,368 |
| Minimized | 0 | 0% at counter resolution | 201,007,104 | 198,729,728 |

Observed peak working set was 279,523,328 bytes. No pre-change sample of this
same workload was retained, so this is an absolute activity/idle check rather
than a CPU improvement claim. Zero sampled increments do not prove literal zero
work; this is not a long-session memory, battery or clean-machine result.

### Reproduce the pacing checks

Use a Developer PowerShell or the MSVC wrapper, with the bundled SDK already
built. The fixture generator needs a developer-installed FFmpeg or
`imageio-ffmpeg`; the application never invokes that encoder.

```powershell
python scripts/video-fixtures.py --tests
.\scripts\cargo-msvc.ps1 build --locked --example video_probe
.\target\debug\examples\video_probe.exe --pacing
.\target\debug\examples\video_probe.exe --pacing
.\target\debug\examples\video_probe.exe --pacing
ffmpeg -hide_banner -loglevel error -y -f lavfi -i testsrc2=size=1920x1080:rate=60 -t 6 -an -c:v libx264 -preset ultrafast -crf 28 -threads 2 -pix_fmt yuv420p artifacts/video-fixtures/pacing-1080p60.mp4
.\target\debug\examples\video_probe.exe --pacing artifacts/video-fixtures/pacing-1080p60.mp4
```

The optional path must identify a 60 fps test clip: the probe compares delivery
against that expected rate and requires at least 50 worker frame deliveries per
second of clip duration. Its
real-time threshold is opt-in and does not run in CI. Run on an otherwise quiet
host, record its hardware/power configuration, and keep clip, build profile,
viewport and simulated callback work unchanged for a comparison.

For the controlled baseline in a temporary checkout, replace the remaining-budget
wait for active video in `src/video/mod.rs` with a fixed 16 ms wait while retaining
the scoped timer guard and slower inactive waits. Build and run the same probe,
then restore the final source and
rebuild before measuring it. The original default-timer policies additionally
left the precision guard inactive. These temporary baseline edits were restored;
they are not runtime settings in the product.

## Historical Media Foundation/D3D11 measurements, 2026-09-06

Historical Media Foundation/D3D11 measurements. Video moved to bundled software
decoders on 2026-10-05; these figures do not measure the new playback backend.

Recorded 2026-09-06, Windows 11 Pro build 26200, Intel i7-13700K,
RTX 4080 SUPER, driver 32.0.16.1656, 100% DPI, 1080 x 740 window.
Rust 1.95.0 release build, FemtoVG. Synthetic local files, no cleared OS caches.
These are short development-machine checks, not cold-start or superiority claims.

### Image startup and binary cost

The metric is process main to first completed render containing the 1920 x 1080
JPEG fixture. Five initial old-build runs had a 213.981 ms median; five new-build
runs had a 230.388 ms median. A follow-up alternating old/new experiment checked
whether the apparent slowdown was repeatable:

| Measurement, five alternating pairs | Previous image-only build | Compact UI + video |
| --- | ---: | ---: |
| First-render median | 208.198 ms | 200.079 ms |
| First-render range | 204.015–340.630 ms | 198.843–222.688 ms |
| Median observed peak working set | 114.77 MiB | 115.55 MiB |
| Executable size | 15,907,840 bytes | 16,336,384 bytes |

Old raw times: 340.630, 205.566, 208.198, 216.893, 204.015 ms.
New raw times: 199.701, 222.688, 202.558, 200.079, 198.843 ms.

The initial apparent latency increase did not repeat in the alternating sample.
This supports no consistent image-startup regression in these runs, not a claim
that every machine is faster. Observed image working-set cost increased about
0.8 MiB; the executable grew 428,544 bytes (about 0.41 MiB). No Cargo package
was added. The native playback engine is not initialized by image startup.

### Video runtime

A silent synthetic H.264 video, 1920 x 1080 at 30 fps, 30 seconds long.
After startup and a two-second settle, each phase lasts about five seconds.
CPU is normalized to **one logical CPU**, not the whole machine.

| Phase | CPU ms | One-core CPU | Private bytes | Working set bytes |
| --- | ---: | ---: | ---: | ---: |
| Playing | 734.375 | 14.680% | 266,256,384 | 211,509,248 |
| Paused | 0 | 0% at counter resolution | 248,786,944 | 194,437,120 |
| Minimized | 0 | 0% at counter resolution | 237,723,648 | 194,072,576 |

Observed peak working set was 221,605,888 bytes. Native decode surfaces, GPU
memory, D3D readback and Slint uploads are additional to image-cache accounting.
Zero sampled CPU increments do not mean literal zero work or prove leak freedom.
No long-session or integrated-GPU conclusion is drawn from this short run.

### Reproduction

Use `scripts/measure.ps1 -Image artifacts/fixtures/image1.jpg -Runs 5`, optionally
with `-Executable` pointing to a retained previous build. Alternate one-run
invocations for an interleaved comparison. CSV decimal separators follow Windows
locale. Keep the machine idle during measurement.

Generate the runtime clip with developer FFmpeg:

```text
ffmpeg -f lavfi -i testsrc2=size=1920x1080:rate=30 -t 30 -an -c:v libx264 -preset ultrafast -crf 30 -pix_fmt yuv420p 1080p30.mp4
```

Then run `scripts/runtime-measure.ps1 -Animation 1080p30.mp4 -SecondsPerPhase 5`.
The existing `Animation` parameter also accepts video; the script only opens,
pauses and minimizes its own viewer instance. It does not install codecs.

Previous executable SHA-256:
`7d4c79f4e2fed45767946ba2982c730f5c017e1ce63c096bc5ab3fc80eed5bf4`
(repository baseline `cf7e0cbeba1da6ad7b3f990cb67e86433b1ef209`).

Measured compact/video executable SHA-256:
`ac2d9e2f375abef9e568d9fbc1520e762b0537f3a62cd64b8e5c6b8d854800e9`.

The final Settings scroll-extent correction followed these measurements; it did
not change startup or playback code. Final executable SHA-256:
`b19ee7cad23427db5280b956ff3da5088cd8fe508c6252f0c9cbd12d61ba6fe1`.
