# Dependency decisions

Reviewed for the initial 0.1.0 implementation on 2026-09-06. Versions are locked
in Cargo.lock. A small direct dependency list does not imply a tiny transitive
graph: native UI/font support has a substantial build footprint.

| Dependency | Decision and license | Cost / security considerations |
| --- | --- | --- |
| Slint 1.17.1 | Active native toolkit; use its Royalty-free 2.0 desktop option with attribution | Winit + FemtoVG/OpenGL and software fallback only; no Qt, Skia, WebView, system-tray feature or runtime image downloading. GUI, text, font and SVG support remain transitive costs. Minor version pinned because the winit adapter is unstable. |
| image 0.25.10 | Actively maintained image-rs project; MIT OR Apache-2.0 | Explicit JPEG/PNG/GIF/WebP/BMP/TIFF/ICO features, no default-format bundle and no Rayon. Limits, content detection, orientation and composited animation. Format-specific limits are not a universal allocator cap. |
| windows / windows-core 0.62 | Microsoft bindings; MIT OR Apache-2.0 | Selected Win32/Shell/COM/clipboard APIs, shared version with Slint. Local unsafe ownership wrappers. No IPC or codec activation. |
| raw-window-handle 0.6 | MIT OR Apache-2.0 | Already in the GUI graph; obtains an owner HWND for native dialogs/clipboard. |
| slint-build | Same licensing family as Slint | Build-only UI compiler; its image features include encoders and formats not shipped as viewer decoders. |
| winresource 0.1 | MIT | Build-only Windows resource compiler wrapper for icon, version and manifest. |
| png / gif (dev) | MIT OR Apache-2.0 / MIT | Generate small test animations; no new runtime format stack. |

Slint's build-time image dependency enables AVIF **encoding**, EXR and other
formats in the build graph. That does not enable AVIF decoding in Kova Image.
Inspect `cargo tree --edges normal,build` and the release binary, rather than
equating every lockfile entry with code shipped at runtime.

## Audit result and maintenance warnings

The initial `cargo audit` run reported **zero known vulnerabilities**, with four
informational unmaintained-package advisories:

- bincode 2.0.1, RUSTSEC-2025-0141 (platform-dependent Slint graph; not the selected Windows dependency tree).
- paste 1.0.15, RUSTSEC-2024-0436 (build-time image/EXR/AVIF-encoder dependencies).
- rustybuzz 0.20.1, RUSTSEC-2026-0206 (Slint/resvg font machinery).
- ttf-parser 0.25.1, RUSTSEC-2026-0192 (Slint font/SVG machinery).

These warnings are visible in audit output and are not ignored by configuration.
Untrusted SVG is not passed to Slint. Track upstream migration and reassess
before releases; absence of an advisory is not proof of safety.

## Format decisions

AVIF remains deferred. image-rs `avif-native` introduces dav1d and native build/
packaging requirements. The reviewed zenavif 0.1.6 alternative is
AGPL-3.0-only OR commercial, which does not preserve the intended Kova licensing
model without additional choices. No encoder is added merely to claim a decoder.

HEIC/HEIF, JPEG XL and RAW require separate license, memory, maintenance and
distribution evaluation. SVG would require a deliberate bounded rendering and
external-resource policy; using a browser is not an option.

Primary references: [Slint license](https://github.com/slint-ui/slint/blob/cf62c975c311e7036d599ed8ed0b7e6a8386a934/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md),
[Slint winit API](https://docs.rs/slint/1.17.1/slint/winit_030/index.html),
[image limits](https://docs.rs/image/0.25.10/image/struct.Limits.html),
[image source](https://github.com/image-rs/image),
[windows-rs](https://github.com/microsoft/windows-rs),
[RustSec](https://rustsec.org/advisories/),
[zenavif metadata](https://crates.io/crates/zenavif/0.1.6).
