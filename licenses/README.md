# Supplemental upstream license texts

Some crates omit workspace-level license files from their published archive.
These files were retrieved from each package's upstream repository at the exact
commit recorded in that archive's `.cargo_vcs_info.json`. `SOURCES.json` records
the URLs and SHA-256 hashes. No license choice has been changed.

`scripts/package.ps1` supplements the registry's files from these version-specific
directories, and uses `cargo tree` to exclude inactive optional packages.
Build-only procedural macro binaries are not distributed; their declared license
is retained in the generated index even if an upstream license text is absent.
In the current graph this exception applies to `simd_helpers` 0.1.0 (MIT).
