# Windows build and release preparation

Version: 0.1.0, Early Development. The executable embeds ProductName, version,
description and the Kova icon. The manifest declares `asInvoker`, long-path
awareness and PerMonitorV2 DPI. No elevation or automatic association changes.

```powershell
.\scripts\cargo-msvc.ps1 build --locked --release --bin kova-image
.\target\release\kova-image.exe "C:\Pictures\example.png"
```

Rust uses the MSVC toolchain. Windows 10/11 x64 and the Microsoft Visual C++
Redistributable runtime may be required on a clean machine. A developer-machine
smoke test does not prove clean-machine installation readiness.

## Portable package preparation

```powershell
.\scripts\package.ps1
```

The script creates a fresh staging directory under ignored `dist/`, builds the
release executable, includes documentation and exact dependency license files,
and produces a ZIP plus SHA-256 only when license-file collection completes.
It intentionally fails closed if a registry package has no license file. Review
Slint attribution, required runtime DLLs and clean-machine behavior before
distributing. No GitHub release is published and existing packages are not deleted.

The repository CI also compiles the release EXE, but never publishes a release.
Future GitHub Releases can attach the reviewed ZIP/checksum generated here.
The release executable omits the debug-only renderer capture hook.

## Installer strategy

Evaluate an Inno Setup per-user installer, consistent with Kova File, once the
portable package passes clean Windows VM tests. Install under LocalAppData,
create a Start menu shortcut and uninstall entry, and optionally register Kova
Image under Open with. File associations must be opt-in and use supported
Windows default-app flows; never overwrite UserChoice hashes.

Code signing, SmartScreen reputation, ARM64 support, redistributable licensing,
upgrade/uninstall behavior and clean-machine file associations need more validation. There
is no MSI/MSIX/Inno installer or stable release in this initial repository.

The package script has been run successfully on the development machine. The
result is a local evaluation artifact, not a validated clean-machine installer.

Per-user registration and a Default Apps settings helper are implemented; see
[FILE_ASSOCIATIONS.md](FILE_ASSOCIATIONS.md). Portable packaging itself does not
register or change defaults. Videos require Windows Media Foundation and a
compatible installed codec; no codec engine or pack is redistributed.
