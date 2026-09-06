#Requires -Version 5.1
param([string]$Destination = "dist")
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
& "$PSScriptRoot/cargo-msvc.ps1" -CargoArgs @('build', '--locked', '--release', '--bin', 'kova-image')
if ($LASTEXITCODE -ne 0) { throw "Release build failed." }
$metadataText = & cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc
if ($LASTEXITCODE -ne 0) { throw "Dependency metadata failed." }
$metadata = $metadataText | ConvertFrom-Json
$tree = & cargo tree --locked --target x86_64-pc-windows-msvc --edges normal,build --prefix none --format '{p}'
if ($LASTEXITCODE -ne 0) { throw "Active dependency graph failed." }
$active = @{}
foreach ($line in $tree) {
    if ($line -match '^([A-Za-z0-9_-]+) v([^\s]+)') { $active[$matches[1] + '-' + $matches[2]] = $true }
}
$package = $metadata.packages | Where-Object { $_.name -eq 'kova-image' }
$version = $package.version
$null = New-Item -ItemType Directory -Force -Path $Destination
$destinationRoot = (Resolve-Path -LiteralPath $Destination).Path
$stage = Join-Path $destinationRoot ("Kova-Image-" + $version + "-x64-" + [guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $stage
Copy-Item -LiteralPath target/release/kova-image.exe -Destination $stage
foreach ($file in @('README.md', 'LICENSE', 'LICENSE-MIT', 'LICENSE-APACHE', 'THIRD_PARTY_NOTICES.md', 'SECURITY.md', 'Cargo.lock')) {
    Copy-Item -LiteralPath $file -Destination $stage
}
Copy-Item -LiteralPath docs -Destination (Join-Path $stage 'docs') -Recurse
Copy-Item -LiteralPath assets -Destination (Join-Path $stage 'assets') -Recurse
Copy-Item -LiteralPath licenses -Destination (Join-Path $stage 'licenses') -Recurse
$licenseRoot = Join-Path $stage 'third-party-licenses'
$null = New-Item -ItemType Directory -Path $licenseRoot
$index = @()
$missing = @()
# Cargo metadata also lists inactive optional packages. Cargo tree selects the
# actual Windows build. Preserve a conservative superset of runtime libraries.
foreach ($dependency in $metadata.packages | Where-Object { $_.source -like 'registry+*' -and $active.ContainsKey($_.name + '-' + $_.version) }) {
    $source = Split-Path -Parent $dependency.manifest_path
    $files = @(Get-ChildItem -LiteralPath $source -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE|UNLICENSE)' })
    $folders = @(Get-ChildItem -LiteralPath $source -Directory | Where-Object { $_.Name -match '^(LICENSES|LICENCES)$' })
    $supplement = Join-Path $root ('licenses/' + $dependency.name + '-' + $dependency.version)
    if ($files.Count -eq 0 -and $folders.Count -eq 0 -and (Test-Path -LiteralPath $supplement)) {
        $files = @(Get-ChildItem -LiteralPath $supplement -File)
    }
    if ($files.Count -eq 0 -and $folders.Count -eq 0) {
        # Procedural macro executables run only inside the compiler; they are not
        # redistributed in the viewer. Retain their declaration in the index.
        $macroOnly = @($dependency.targets | Where-Object { $_.kind -contains 'proc-macro' }).Count -gt 0
        if ($macroOnly) {
            $index += [pscustomobject]@{ name=$dependency.name; version=$dependency.version; license=$dependency.license; repository=$dependency.repository }
            continue
        }
        $missing += "$($dependency.name) $($dependency.version)"; continue
    }
    $dest = Join-Path $licenseRoot ($dependency.name + '-' + $dependency.version)
    $null = New-Item -ItemType Directory -Path $dest
    foreach ($file in $files) { Copy-Item -LiteralPath $file.FullName -Destination $dest }
    foreach ($folder in $folders) { Copy-Item -LiteralPath $folder.FullName -Destination $dest -Recurse }
    $index += [pscustomobject]@{ name=$dependency.name; version=$dependency.version; license=$dependency.license; repository=$dependency.repository }
}
$index | Export-Csv -NoTypeInformation -LiteralPath (Join-Path $licenseRoot 'index.csv')
if ($missing.Count -gt 0) {
    $missing | Set-Content -LiteralPath (Join-Path $stage 'LICENSE-REVIEW-REQUIRED.txt')
    throw "License texts missing for $($missing.Count) crates. Review the staging folder before distributing; no ZIP produced."
}
@'
Kova Image 0.1.0 - Early Development

Windows 10/11 x64. Run kova-image.exe, or drop a local image or video into its window.
The Microsoft Visual C++ 2015-2022 Redistributable (x64) may be required.
This package is unsigned and experimental. No installer or associations are applied.
If hardware rendering fails, run kova-image.exe --software.
Video codecs are supplied by Windows; not every container/codec combination plays.
Use Settings to opt in to Open with registration after choosing a permanent folder.
No stable release is created by this script.
'@ | Set-Content -Encoding utf8 -LiteralPath (Join-Path $stage 'RUNNING.txt')
$zip = $stage + '.zip'
# Registry archives can carry 1970 timestamps, outside ZIP's supported range.
# Normalize only files in this freshly created package staging directory.
Get-ChildItem -LiteralPath $stage -Recurse -File | ForEach-Object { $_.LastWriteTimeUtc = [datetime]'2000-01-01T00:00:00Z' }
Add-Type -AssemblyName System.IO.Compression.FileSystem
[IO.Compression.ZipFile]::CreateFromDirectory($stage, $zip)
$hash = Get-FileHash -Algorithm SHA256 -LiteralPath $zip
($hash.Hash.ToLowerInvariant() + '  ' + [IO.Path]::GetFileName($zip)) | Set-Content -Encoding ascii -LiteralPath ($zip + '.sha256')
Write-Output $zip
