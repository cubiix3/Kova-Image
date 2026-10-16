# Contributing

## Requirements

Windows 10/11 x64, Rust 1.95.0 with the MSVC target, and the
**Desktop development with C++** workload (including a Windows SDK) from
Visual Studio or the standalone Visual Studio Build Tools.

## Build and test

Run from a Developer PowerShell, or use `scripts/cargo-msvc.ps1` to discover
the installed Visual Studio environment:

First build the bundled video decoders with `scripts/build-ffmpeg.ps1` and a
workspace-local MSYS2 installation (make, nasm and pkgconf). Python is required
for the isolated Meson/Ninja build tools. See [VIDEO.md](docs/VIDEO.md). The
source archives are pinned and checked by SHA-256; Cargo does not download codecs.

```powershell
cargo build --locked
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo build --locked --release --bin kova-image
cargo build --locked --release -p kova-thumbnails
```

Use `cargo fmt --all` before submitting. Add meaningful tests for changes to
decoding, navigation, cancellation, memory limits or animation semantics.
Fixtures should be generated, small and free to redistribute. The image fixtures
come from `scripts/image-fixtures.py` (Pillow and a developer-installed ffmpeg;
the application and CI never need an encoder) and the HEVC reference corpus from
`scripts/hevc-reference.py`. Never commit
build outputs, real personal images, private paths, credentials or machine state.

Large animation checks use `scripts/animation-fixtures.py` (Pillow). Regenerate
the JPEG XL regression fixture with `--tests --ffmpeg <developer-ffmpeg>` using
FFmpeg 8.1.3 with `libjxl_anim`; the encoder is not shipped in Kova Image.

## Pull requests

Keep changes focused. Explain the user-visible behavior, relevant risks and
what you tested. Discuss substantial dependencies or format additions first;
include license, memory and failure behavior. Preserve the viewer-only scope.
Windows CI must pass. Contributions are accepted under MIT OR Apache-2.0.

## Security reports

Follow [SECURITY.md](SECURITY.md) for private reporting. Do not attach sensitive
security samples to a public issue or pull request.
