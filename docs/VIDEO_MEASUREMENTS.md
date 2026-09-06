# Compact UI and native video measurements

Recorded 2026-09-06, Windows 11 Pro build 26200, Intel i7-13700K,
RTX 4080 SUPER, driver 32.0.16.1656, 100% DPI, 1080 x 740 window.
Rust 1.95.0 release build, FemtoVG. Synthetic local files, no cleared OS caches.
These are short development-machine checks, not cold-start or superiority claims.

## Image startup and binary cost

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

## Video runtime

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

## Reproduction

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
