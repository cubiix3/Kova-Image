# Contributing

## Requirements

Windows 10/11 x64, Rust 1.95.0 with the MSVC target, and the
**Desktop development with C++** workload (including a Windows SDK) from
Visual Studio or the standalone Visual Studio Build Tools.

## Build and test

Run from a Developer PowerShell, or use `scripts/cargo-msvc.ps1` to discover
the installed Visual Studio environment:

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

## Pull requests

Keep changes focused. Explain the user-visible behavior, relevant risks and
what you tested. Discuss substantial dependencies or format additions first;
include license, memory and failure behavior. Preserve the viewer-only scope.
Windows CI must pass. Contributions are accepted under MIT OR Apache-2.0.

## Security reports

Follow [SECURITY.md](SECURITY.md) for private reporting. Do not attach sensitive
security samples to a public issue or pull request.
