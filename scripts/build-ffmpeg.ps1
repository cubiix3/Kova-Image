#Requires -Version 5.1
param([string]$MsysRoot = "$PSScriptRoot/../artifacts/ffmpeg/msys64")
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/msvc-env.ps1"
$projectRoot = Split-Path -Parent $PSScriptRoot
$nativeRoot = Join-Path $projectRoot 'artifacts/ffmpeg'
$sdk = Join-Path $nativeRoot 'sdk'
$bash = Join-Path $MsysRoot 'usr/bin/bash.exe'
if (-not (Test-Path -LiteralPath $bash)) { throw 'Provide a workspace-local MSYS2 installation with -MsysRoot; see docs/VIDEO.md.' }
$null = New-Item -ItemType Directory -Force -Path $nativeRoot

function Get-Source($Name, $Url, $Hash) {
    $archive = Join-Path $nativeRoot $Name
    if (-not (Test-Path -LiteralPath $archive)) { Invoke-WebRequest -UseBasicParsing $Url -OutFile $archive }
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash -ne $Hash) { throw "Source checksum mismatch: $Name" }
    $directory = Join-Path $nativeRoot ($Name -replace '\.tar\.xz$', '')
    if (-not (Test-Path -LiteralPath $directory)) {
        & tar.exe -xf $archive -C $nativeRoot
        if ($LASTEXITCODE -ne 0) { throw "Cannot extract $Name" }
    }
    return $archive
}
$ffmpegSource = Get-Source 'ffmpeg-8.1.3.tar.xz' 'https://ffmpeg.org/releases/ffmpeg-8.1.3.tar.xz' '7138d28c96d9d3e3af4ee3d8cad72741f8ffb40da90c1112235dea3ecd3178a3'
$dav1dSource = Get-Source 'dav1d-1.5.3.tar.xz' 'https://downloads.videolan.org/pub/videolan/dav1d/1.5.3/dav1d-1.5.3.tar.xz' '732010aa5ef461fa93355ed2c6c5fedb48ddc4b74e697eaabe8907eaeb943011'
$recipeHash = ((Get-FileHash -LiteralPath $PSCommandPath,"$PSScriptRoot/build-ffmpeg.sh","$PSScriptRoot/msvc-env.ps1" -Algorithm SHA256).Hash -join '')
$stamp = Join-Path $sdk 'recipe.sha256'
$required = @('include/libavcodec/avcodec.h', 'bin/avcodec-62.dll', 'bin/avformat-62.dll',
    'bin/avutil-60.dll', 'bin/swscale-9.dll', 'bin/swresample-6.dll', 'bin/dav1d.dll',
    'sources/ffmpeg-8.1.3.tar.xz', 'sources/dav1d-1.5.3.tar.xz',
    'sources/COPYING.LGPLv2.1', 'sources/dav1d-COPYING')
if ((Test-Path -LiteralPath $stamp) -and (Get-Content -LiteralPath $stamp -Raw).Trim() -eq $recipeHash -and
    @($required | Where-Object { -not (Test-Path -LiteralPath (Join-Path $sdk $_)) }).Count -eq 0) {
    Write-Output "FFmpeg SDK is current: $sdk"
    return
}
$ErrorActionPreference = 'Continue'
& $bash -lc 'pacman -S --noconfirm --needed make nasm pkgconf' 2>&1 | ForEach-Object { $_.ToString() }
if ($LASTEXITCODE -ne 0) { throw 'MSYS2 build tools are missing; initialize its package database first.' }
$tools = Join-Path $nativeRoot 'tools'
if (-not (Test-Path -LiteralPath "$tools/Scripts/python.exe")) { & python -m venv $tools }
& "$tools/Scripts/python.exe" -m pip install --disable-pip-version-check meson==1.9.1 ninja==1.13.0
if ($LASTEXITCODE -ne 0) { throw 'Native build tools could not be prepared.' }
$env:PATH = "$tools/Scripts;$env:PATH;$MsysRoot/usr/bin"
$meson = Join-Path $tools 'Scripts/meson.exe'
$dav1dBuild = Join-Path $nativeRoot 'dav1d-build'
$setupArgs = @('setup', $dav1dBuild, (Join-Path $nativeRoot 'dav1d-1.5.3'), '--prefix', $sdk, '--buildtype', 'release', '--default-library', 'shared', '-Denable_tools=false', '-Denable_tests=false')
if (Test-Path -LiteralPath "$dav1dBuild/meson-private/coredata.dat") { $setupArgs += '--reconfigure' }
& $meson @setupArgs
if ($LASTEXITCODE -ne 0) { throw 'AV1 decoder configure failed.' }
& $meson compile -C $dav1dBuild -j 4
if ($LASTEXITCODE -ne 0) { throw 'AV1 decoder build failed.' }
& $meson install -C $dav1dBuild
if ($LASTEXITCODE -ne 0) { throw 'AV1 decoder install failed.' }
$env:MSYS2_PATH_TYPE = 'inherit'
$env:KOVA_NATIVE_ROOT = $nativeRoot
& $bash "$PSScriptRoot/build-ffmpeg.sh"
if ($LASTEXITCODE -ne 0) { throw 'FFmpeg build failed.' }
$ErrorActionPreference = 'Stop'
$sourceDir = Join-Path $sdk 'sources'
$null = New-Item -ItemType Directory -Force -Path $sourceDir
Copy-Item -LiteralPath $ffmpegSource,$dav1dSource,$PSCommandPath,"$PSScriptRoot/build-ffmpeg.sh","$PSScriptRoot/msvc-env.ps1" -Destination $sourceDir
Copy-Item -LiteralPath "$nativeRoot/ffmpeg-8.1.3/COPYING.LGPLv2.1" -Destination $sourceDir
Copy-Item -LiteralPath "$nativeRoot/dav1d-1.5.3/COPYING" -Destination (Join-Path $sourceDir 'dav1d-COPYING')
$recipeHash | Set-Content -Encoding ascii -LiteralPath $stamp
Write-Output "FFmpeg SDK built: $sdk"
