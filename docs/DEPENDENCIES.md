# Dependency decisions

Reviewed for the initial 0.1.0 implementation on 2026-09-06. Versions are locked
in Cargo.lock. A small direct dependency list does not imply a tiny transitive
graph: native UI/font support has a substantial build footprint.

| Dependency | Decision and license | Cost / security considerations |
| --- | --- | --- |
| Slint 1.17.1 | Active native toolkit; use its Royalty-free 2.0 desktop option with attribution | Winit + FemtoVG/OpenGL and software fallback only; no Qt, Skia, WebView, system-tray feature or runtime image downloading. GUI, text, font and SVG support remain transitive costs. Minor version pinned because the winit adapter is unstable. |
| Slint accessibility / AccessKit | Existing Slint feature; AccessKit core, consumer and Windows crates are MIT OR Apache-2.0, `accesskit_winit` is Apache-2.0 | Exposes the existing Slint controls through Windows UI Automation. Adds no service or network path. The local release EXE grew from 16,745,984 to 17,354,752 bytes; runtime tree allocation is not separately measured. Upstream license texts are included in packaging. |
| image 0.25.10 | Actively maintained image-rs project; MIT OR Apache-2.0 | Explicit JPEG/PNG/GIF/WebP/BMP/TIFF/ICO/TGA/PNM/QOI/HDR/EXR/farbfeld features, no default-format bundle and no Rayon. Limits, content detection, orientation and composited animation. Format-specific limits are not a universal allocator cap. DDS is read by `src/codecs/dds.rs` instead (below). |
| jxl-oxide 0.12.6 (and its `jxl-*` crates) | Pure Rust JPEG XL decoder; MIT OR Apache-2.0 | `default-features = false` with the moxcms colour engine, so no lcms2 and no C code. Its allocation tracker is capped at `JXL_BUDGET` (512 MiB), because the decoder works on 32-bit float planes: a 12 MP photo peaks at about 270 MiB tracked and 330 MB working set, and decodes in about 460 ms. Larger pictures end in a memory error instead of an out-of-memory crash. |
| resvg 0.47 (usvg, tiny-skia 0.11/0.12) | Rust SVG renderer; Apache-2.0 OR MIT, tiny-skia BSD-3-Clause | Already in the lockfile through Slint, now also used on untrusted files. Drawn at the size the view needs (at most 8192 px a side, bounded pixels); only `data:` images up to 64 Mi pixels are loaded, so an SVG cannot read other files or reach the network, and there is no scripting. System fonts are loaded only when the file contains text. SVG text goes through rustybuzz and ttf-parser (both flagged unmaintained below), which parse the installed fonts, not fonts from the file. |
| flate2 1.1 | MIT OR Apache-2.0 | Unpacks `.svgz`, with the output capped at 16 MiB. Pure Rust backend. |
| rav1d 1.1.0, vendored in `vendor/rav1d` | Rust port of dav1d; BSD-2-Clause | AV1 for AVIF. No assembly (it would need nasm in the locked MSVC build), so it is slower than dav1d: a 12 MP 4:2:0 picture decodes in about 320 ms on one thread. The vendored copy differs from the published crate only in declaring `extern "C"` as `extern "C-unwind"`, so a panic inside the decoder is caught instead of aborting the process (see `vendor/rav1d/KOVA.md` and `scripts/vendor-rav1d.py`). Decoding runs on its own thread with a frame size limit. |
| moxcms 0.8 | BSD-3-Clause OR Apache-2.0; already used by image-rs | Direct use converts an embedded ICC profile to 8-bit sRGB. A missing or unreadable profile leaves the decoded pixels unchanged. No network and no monitor profile. |
| kamadak-exif 0.6.1 | BSD-2-Clause | Reads the Exif block image-rs already returns. Used for the date, camera and exposure rows. No thumbnail extraction. |
| windows / windows-core 0.62 | Microsoft bindings; MIT OR Apache-2.0 | Selected Win32/Shell/COM/clipboard/Registry/DWM and Media Foundation/D3D11 APIs, shared version with Slint. Native video initializes on demand; no bundled multimedia engine or IPC. |
| raw-window-handle 0.6 | MIT OR Apache-2.0 | Already in the GUI graph; obtains an owner HWND for native dialogs/clipboard. |
| slint-build | Same licensing family as Slint | Build-only UI compiler; its image features include encoders and formats not shipped as viewer decoders. |
| winresource 0.1 | MIT | Build-only Windows resource compiler wrapper for icon, version and manifest. |
| png / gif (dev) | MIT OR Apache-2.0 / MIT | Generate small test animations; no new runtime format stack. |

Slint's build-time image dependency enables AVIF **encoding**, EXR and other
formats in the build graph. Those encoders are not part of the viewer. Inspect
`cargo tree --edges normal,build` and the release binary, rather than equating
every lockfile entry with code shipped at runtime.

The viewer executable grew from about 17.4 MB to about 23 MB with the formats
below; `kova_thumbnails.dll` (the Explorer preview provider, which carries the
same decoders) is about 14 MB.

## Audit result and maintenance warnings

The initial `cargo audit` run reported **zero known vulnerabilities**, with four
informational unmaintained-package advisories. After the image formats below were
added, a second run on 2026-10-02 (629 crates) reported the same four and no
vulnerability:

- bincode 2.0.1, RUSTSEC-2025-0141 (platform-dependent Slint graph; not the selected Windows dependency tree).
- paste 1.0.15, RUSTSEC-2024-0436 (build-time image/EXR/AVIF-encoder dependencies).
- rustybuzz 0.20.1, RUSTSEC-2026-0206 (Slint and resvg font machinery; resvg shapes SVG text with it).
- ttf-parser 0.25.1, RUSTSEC-2026-0192 (Slint and resvg font machinery; parses installed system fonts).

These warnings are visible in audit output and are not ignored by configuration.
Untrusted SVG is not passed to Slint; it is rendered by resvg as described above. Track upstream migration and reassess
before releases; absence of an advisory is not proof of safety.

## Resampling

No resampling crate was added. `src/resample.rs` averages whole blocks and then
applies a bilinear pass. On the 24 MP test photo the display-size decode fell
from about 195 ms to about 118 ms with this and the RGB-first JPEG path (see
[MEASUREMENTS.md](MEASUREMENTS.md)); a SIMD crate such as `fast_image_resize`
remains an option if real photo folders show resampling as the bottleneck.

## Format decisions

Every image format is decoded inside Kova Image. No Windows codec, extension or
system library is required, and nothing is downloaded or built from the network.

**AVIF** uses rav1d. image-rs `avif-native` resolves to dav1d-sys, which needs
pkg-config and either a preinstalled dav1d or a Meson build that clones dav1d
during compilation; that does not fit the locked MSVC build. The reviewed
zenavif 0.1.6 alternative is AGPL-3.0-only OR commercial, which does not
preserve the intended Kova licensing model. rav1d is pure Rust and BSD-2-Clause.
The container (ISOBMFF items, `ispe`/`colr`/`irot`/`imir`/`clap`, grids, alpha
as an auxiliary picture, Exif) is parsed by `src/codecs/isobmff.rs` and
`heif.rs`, and the YUV to RGB conversion by `yuv.rs`. Animated AVIF sequences are
not shown. HDR (PQ/HLG) is shown as stored.

**JPEG XL** uses jxl-oxide (above), including animation.

**SVG** uses resvg with the limits above.

**RAW** is not developed. Kova Image reads the largest JPEG preview that the
camera stored in the file and the Exif block of the container (orientation,
camera). The scan validates the JPEG structure and refuses lossless JPEG and
files over 1 GiB (`MAX_RAW_FILE_BYTES`; the file is read through seeks).

**DDS** is read by own code (`src/codecs/dds.rs`), because the crate's decoder covers only DXT1/3/5 and game textures use more: BC1 to BC5 and BC7, and uncompressed layouts described by channel masks. It reads the data strip by strip, so memory is the RGBA result only. The BC7 partition tables were extracted from [bcdec](https://github.com/iOrange/bcdec) (MIT or Unlicense, Sergii Kudlai), and the decoder is tested against bcdec's output for random blocks of all eight modes (`scripts/bc7-reference.py`, fixtures `tests/fixtures/bc7.*`); bcdec itself is not part of the program. BC6H, floating point and 16-bit formats are not supported.

**HEIC/HEIF** is decoded by own code in `src/codecs/hevc` (about 4,300 lines,
intra pictures only), described in the next section.

## HEVC and patents

No usable HEVC decoder exists under a licence that fits Kova Image: the `heic`
crate is AGPL-3.0, libheif and libde265 are LGPL and would be C libraries in the
build. HEIC stills, as phones and cameras write them, are HEVC *intra* pictures,
so a decoder for just that part is much smaller than a video decoder. It
supports Main, Main 10, Main Still Picture and the range extension profiles for
4:0:0, 4:2:2 and 4:4:4 up to 12 bit, with grids and auxiliary alpha pictures.
It reports a clear "unsupported" error for P and B slices, several tiles,
separate colour planes, cross-component prediction, extended precision
processing, CABAC bypass alignment and bit depths above 12.

It was checked bit for bit against ffmpeg's libx265 and HEVC decoder on a
generated corpus (`scripts/hevc-reference.py`, run through the ignored test
`matches_the_reference_decoder_bit_for_bit` with `KOVA_HEVC_REFERENCE`). One
ffmpeg case, transquant bypass in 4:2:2 chroma, was left out because ffmpeg
itself uses a wrong width there. A second ignored test flips bits in the corpus
and demands an error instead of a panic or a hang.

**Patents.** HEVC is covered by patent pools (Access Advance, Via LA and
others). Writing a decoder from the specification does not grant any licence,
and including one is not legal advice or a statement that none is needed.
Anyone who redistributes Kova Image commercially should check this for their
own case. Windows' own HEVC codec is a paid extension for the same reason,
which is why Kova Image does not rely on it.

Primary references: [Slint license](https://github.com/slint-ui/slint/blob/cf62c975c311e7036d599ed8ed0b7e6a8386a934/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md),
[Slint winit API](https://docs.rs/slint/1.17.1/slint/winit_030/index.html),
[image limits](https://docs.rs/image/0.25.10/image/struct.Limits.html),
[image source](https://github.com/image-rs/image),
[windows-rs](https://github.com/microsoft/windows-rs),
[RustSec](https://rustsec.org/advisories/),
[zenavif metadata](https://crates.io/crates/zenavif/0.1.6).

## Local video addition

No Cargo package was added. Selected features on the existing windows-rs binding
expose Windows Media Foundation Media Engine and D3D11. The OS supplies container
parsers, audio/video clocks and installed codecs; OS servicing supplies fixes.
D3D hardware acceleration is requested, with WARP device fallback. The presence
of a hardware device is not proof that each codec decoded in hardware.

The executable does not link or launch FFmpeg, libVLC, mpv, a browser or a codec
installer. FFmpeg may generate synthetic developer test clips; it is not an app
or package dependency. This saves a bundled codec stack but makes format support
dependent on the Windows installation. See [VIDEO.md](VIDEO.md) for references,
real codec results, native allocation limits and the readback tradeoff.
