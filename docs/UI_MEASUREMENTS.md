# Interface refinement measurements

Local Windows 11 Pro build 26200, Intel i7-13700K, RTX 4080 SUPER,
driver 32.0.16.1656, 100% display scale, 1080 × 740 window, Rust 1.95.0,
release build, FemtoVG. Recorded 2026-09-06 with the existing generated fixtures.
These are small before/after samples on one development machine, not cold-start
benchmarks or a general performance guarantee.

## Startup

Five launches per build, JPEG 1920 × 1080. The internal measurement is process
main to the first completed render containing the image. OS caches were not
cleared. Working set/private bytes were sampled when the measurement was observed.

| Measurement | Previous UI | Refined UI |
| --- | ---: | ---: |
| First-render median | 225.262 ms | 217.616 ms |
| First-render range | 207.750–234.389 ms | 206.274–219.910 ms |
| Median observed peak working set | 109.48 MiB | 114.70 MiB |
| Median observed private bytes | 123.09 MiB | 128.76 MiB |
| Executable size | 15,111,168 bytes | 15,907,840 bytes |

Previous raw times: 234.389, 208.078, 228.370, 225.262, 207.750 ms.
Refined raw times: 206.274, 219.910, 217.616, 219.013, 215.085 ms.

No startup slowdown was observed in this sample. The refined UI costs roughly
5–6 MiB more process memory at this observation point and 0.76 MiB of executable
size. This includes the richer controls, icon resources and additional text sizes;
the experiment does not isolate individual allocators or GPU memory.

## Animation

Generated 400 × 240, 12-frame GIF; ten seconds per playing, paused and minimized
phase. CPU percentage is relative to **one logical CPU**, not the whole machine.

The previous UI consumed 156.250 and 140.625 ms CPU in two playing phases
(1.562% and 1.405%). The first refined implementation consumed 406.250 and
343.750 ms (4.060% and 3.436%). This repeatable increase motivated caching only
the unchanged header/control bar during animation playback.

The final implementation consumed 93.750 ms over 10,012.506 ms (0.936%) in its
first playing phase. Paused and minimized CPU increments were 0 ms at the process
counter's resolution. Private bytes during playback were 121,364,480; working
set was 111,632,384. This is not a hard process-memory limit or a leak test.

A second final-build playing phase consumed 171.875 ms over 10,011.354 ms
(1.717%); paused was 0 ms and minimized 15.625 ms (0.156%). The final playing
range, 0.936–1.717%, is in the same small-CPU-cost range as the original build.
These short samples support removing the repeatable intermediate regression,
not a claim of universally faster playback or literally zero idle CPU.

The two chrome caches are enabled only for animated images. They do not change
the decode cache or copy/cache the image itself. Hover, focus, playback state and
resize still invalidate the relevant chrome as needed. No continuous UI animation
is used while idle.

## Reproduction

```powershell
.\scripts\measure.ps1 -Image artifacts\fixtures\image1.jpg -Runs 5
.\scripts\runtime-measure.ps1 -Animation artifacts\fixtures\image3.gif -SecondsPerPhase 10
```

Both scripts accept `-Executable` for comparing a retained previous build.
Record its identity rather than replacing the current executable. CSV decimal
separators follow the Windows locale; the values above use decimal points.

Previous executable SHA-256:
`f4c29aeb899f86e30ae767a7c033308624d990c2df5372210d2db2a40214ab18`
(repository baseline `bf31e0426f17c5245f24e20ccf77dbb122791ddf`).

Refined executable SHA-256:
`7d4c79f4e2fed45767946ba2982c730f5c017e1ce63c096bc5ab3fc80eed5bf4`.

Further validation should cover integrated GPUs, high DPI, long sessions and
physical multi-monitor transitions. See [DESIGN.md](DESIGN.md) for visual rules
and [PERFORMANCE.md](PERFORMANCE.md) for the broader measurement protocol.
