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
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked --release --bin kova-image
```

Use `cargo fmt --all` before submitting. Add meaningful tests for changes to
decoding, navigation, cancellation, memory limits or animation semantics.
Fixtures should be generated, small and free to redistribute. Never commit
build outputs, real personal images, private paths, credentials or machine state.

## Pull requests

Keep changes focused. Explain the user-visible behavior, relevant risks and
what you tested. Discuss substantial dependencies or format additions first;
include license, memory and failure behavior. Preserve the viewer-only scope.
Windows CI must pass. Contributions are accepted under MIT OR Apache-2.0.

## Security reports

Follow [SECURITY.md](SECURITY.md) for private reporting. Do not attach sensitive
security samples to a public issue or pull request.
