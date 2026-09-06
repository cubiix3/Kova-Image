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
$licenseRoot = Join-Path $stage 'third-party-licenses'
$null = New-Item -ItemType Directory -Path $licenseRoot
$index = @()
$missing = @()
# Include a conservative superset (build-time packages too), with exact license
# files from verified Cargo registry sources. Never include local registry paths.
foreach ($dependency in $metadata.packages | Where-Object { $_.source -like 'registry+*' }) {
    $source = Split-Path -Parent $dependency.manifest_path
    $files = @(Get-ChildItem -LiteralPath $source -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE|UNLICENSE)' })
    $folders = @(Get-ChildItem -LiteralPath $source -Directory | Where-Object { $_.Name -match '^(LICENSES|LICENCES)$' })
    if ($files.Count -eq 0 -and $folders.Count -eq 0) { $missing += "$($dependency.name) $($dependency.version)"; continue }
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
Kova Image 0.1.0 — Early Development

Windows 10/11 x64. Run kova-image.exe, or drop an image into its window.
The Microsoft Visual C++ 2015–2022 Redistributable (x64) may be required.
This package is unsigned and experimental. No installer or associations are applied.
If hardware rendering fails, run kova-image.exe --software.
No stable release is created by this script.
'@ | Set-Content -Encoding utf8 -LiteralPath (Join-Path $stage 'RUNNING.txt')
$zip = $stage + '.zip'
Compress-Archive -LiteralPath $stage -DestinationPath $zip
$hash = Get-FileHash -Algorithm SHA256 -LiteralPath $zip
($hash.Hash.ToLowerInvariant() + '  ' + [IO.Path]::GetFileName($zip)) | Set-Content -Encoding ascii -LiteralPath ($zip + '.sha256')
Write-Output $zip
