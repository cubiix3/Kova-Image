# Kova Image project instructions

Kova Image is a Windows image viewer with bounded local video playback, written in Rust and Slint.
Keep the viewer-only scope. The nested `Kova-Screen/` checkout is an independent project; these viewer rules apply only to Kova Image.

## Context and paths

- `README.md`, `CONTRIBUTING.md`: product scope, build and contribution expectations.
- `docs/ARCHITECTURE.md`, `docs/SECURITY_ARCHITECTURE.md`: concurrency, decoding and safety.
- `src/`: coordination, decoders, animation, cache, navigation, Windows integration and video.
- `ui/`: Slint interface. `tests/`: regression fixtures/tests.
- `scripts/cargo-msvc.ps1`: Visual Studio environment wrapper.
- Use the pinned toolchain in `rust-toolchain.toml` and Windows MSVC with a Windows SDK.

## Architecture and invariants

- Decoders do not import Slint; the UI does not parse image files.
- Retain one decode worker, a latest-request slot, bounded results and a weighted cache.
- Only the current generation updates the view. Cancellation is cooperative; never kill a codec thread.
- Keep the previous image visible while loading; destructive/copy actions require displayed and requested paths to match.
- Folder scans read names/types, not full image content or speculative thumbnails.
- Animation uses a single-shot frame timer; no idle/paused polling.
- Video uses the existing Media Foundation/D3D11 worker and latest-frame slot.
- Never preload videos or cache entire video frame sequences. Stop/release video resources before recycling.
- Shell objects stay on their STA worker; shutdown must not block the UI on a non-cooperative decoder.
- No daemon, networking or single-instance protocol. File associations respect Windows-owned default selection.

## Verification and delivery

- Follow `CONTRIBUTING.md`: locked build/check/test, fmt, all-target Clippy with warnings denied, and release build of `kova-image`.
- Use small generated redistributable fixtures for decode, navigation, cancellation, bounds and animation changes.
- Do not commit personal images, private paths, credentials, machine state or build output.
- Consult `docs/PERFORMANCE.md` and the relevant measurement report for comparable workloads.
- Use `docs/VALIDATION.md` and `docs/WINDOWS_RELEASE.md` for runtime/release verification.
- Discuss substantial dependencies and format additions with their license, memory and failure implications.
- Windows CI must pass; contributions are MIT OR Apache-2.0.
