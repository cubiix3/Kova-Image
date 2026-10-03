<#
.SYNOPSIS
Asks the Windows Shell for a thumbnail of every file in a folder, the way Explorer does.

.DESCRIPTION
The thumbnail cache is bypassed (WTS_FORCEEXTRACTION | WTS_EXTRACTDONOTCACHE), so the
registered handler is really asked, and nothing is written to the cache or the registry.
For each file the script reports who handles the extension (Kova Image, Windows or another
program), the size of the thumbnail or the error, and how long it took. A thumbnail of one
colour is marked FLAT: right for a plain texture, suspicious for anything else.

.PARAMETER Folder      Folder to check.
.PARAMETER Recurse     Include sub folders.
.PARAMETER Extensions  Only these extensions (without the dot).
.PARAMETER SaveTo      Save every thumbnail as PNG there, named after the file with '__' for
                       folder separators; scripts/preview-crosscheck.py compares them with an
                       independent decoder.
.PARAMETER FailuresOnly  List only the files without a thumbnail.

.EXAMPLE
.\scripts\preview-check.ps1 -Folder C:\Pictures -Recurse -FailuresOnly
#>
param(
    [Parameter(Mandatory = $true)][string]$Folder,
    [string]$SaveTo,
    [int]$Size = 256,
    [switch]$Recurse,
    [string[]]$Extensions,
    [switch]$FailuresOnly
)
$ErrorActionPreference = 'Stop'
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Drawing;
using System.Runtime.InteropServices;

[StructLayout(LayoutKind.Sequential)] public struct WtsId { [MarshalAs(UnmanagedType.ByValArray, SizeConst = 16)] public byte[] key; }

[ComImport, Guid("43826d1e-e718-42ee-bc55-a1e261c37bfe"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IShellItemStub { }

[ComImport, Guid("091162a4-bc96-411f-aae8-c5122cd03363"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface ISharedBitmap {
    [PreserveSig] int GetSharedBitmap(out IntPtr bitmap);
    [PreserveSig] int GetSize(out long size);
    [PreserveSig] int GetFormat(out int format);
    [PreserveSig] int InitializeBitmap(IntPtr bitmap, int format);
    [PreserveSig] int Detach(out IntPtr bitmap);
}

[ComImport, Guid("F676C15D-596A-4ce2-8234-33996F445DB1"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IThumbnailCache {
    [PreserveSig] int GetThumbnail(IShellItemStub item, uint size, uint flags, out ISharedBitmap bitmap, out uint outFlags, out WtsId id);
}

public static class ThumbCheck {
    [DllImport("shell32.dll", CharSet = CharSet.Unicode, PreserveSig = false)]
    static extern void SHCreateItemFromParsingName(string path, IntPtr context, ref Guid iid,
        [MarshalAs(UnmanagedType.Interface)] out IShellItemStub item);
    [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr handle);

    static IThumbnailCache cache;

    // Returns "WxH" or an error text. Saves the bitmap when pngPath is given.
    public static string Fetch(string path, uint size, string pngPath) {
        if (cache == null) {
            Type type = Type.GetTypeFromCLSID(new Guid("50EF4544-AC9F-4A8E-B21B-8A26180DB13F"));
            cache = (IThumbnailCache)Activator.CreateInstance(type);
        }
        Guid iid = new Guid("43826d1e-e718-42ee-bc55-a1e261c37bfe");
        IShellItemStub item;
        SHCreateItemFromParsingName(path, IntPtr.Zero, ref iid, out item);
        ISharedBitmap shared; uint outFlags; WtsId id;
        // 0x4 force extraction, 0x20 do not cache the result.
        int hr = cache.GetThumbnail(item, size, 0x24, out shared, out outFlags, out id);
        if (hr != 0) return "FAIL 0x" + hr.ToString("X8");
        IntPtr hbitmap;
        hr = shared.GetSharedBitmap(out hbitmap);
        if (hr != 0 || hbitmap == IntPtr.Zero) return "FAIL bitmap 0x" + hr.ToString("X8");
        using (Bitmap bitmap = Image.FromHbitmap(hbitmap)) {
            if (!string.IsNullOrEmpty(pngPath)) bitmap.Save(pngPath, System.Drawing.Imaging.ImageFormat.Png);
            // A thumbnail that is one flat colour is as good as none.
            int flat = 1; int first = bitmap.GetPixel(0, 0).ToArgb();
            for (int y = 0; y < bitmap.Height && flat == 1; y += 4)
                for (int x = 0; x < bitmap.Width; x += 4)
                    if (bitmap.GetPixel(x, y).ToArgb() != first) { flat = 0; break; }
            return bitmap.Width + "x" + bitmap.Height + (flat == 1 ? " (FLAT)" : "");
        }
    }
}
"@

$handler = '{E357FCCD-A995-4576-B01F-234630154E96}'
$ours = '{7C1D3A52-9E84-4B6F-8A07-52F0D9C3B1E6}'
$whoCache = @{}
function Who($ext) {
    if ($whoCache.ContainsKey($ext)) { return $whoCache[$ext] }
    $whoCache[$ext] = Resolve-Who $ext
    return $whoCache[$ext]
}
function Resolve-Who($ext) {
    foreach ($base in "Registry::HKEY_CLASSES_ROOT\.$ext\ShellEx\$handler",
                      "Registry::HKEY_CLASSES_ROOT\SystemFileAssociations\.$ext\ShellEx\$handler") {
        $key = Get-Item -LiteralPath $base -ErrorAction SilentlyContinue
        if ($key) {
            $value = $key.GetValue('')
            if ($value -ieq $ours) { return 'Kova' }
            if ($value) {
                # A packaged (store app) provider has no CLSID key; show its id then.
                $class = Get-Item -LiteralPath "Registry::HKEY_CLASSES_ROOT\CLSID\$value" -ErrorAction SilentlyContinue
                $name = if ($class -and $class.GetValue('')) { $class.GetValue('') } else { $value }
                return "other: $name"
            }
        }
    }
    return 'Windows'
}
if ($SaveTo) { New-Item -ItemType Directory -Force -Path $SaveTo | Out-Null }
$Folder = (Resolve-Path -LiteralPath $Folder).Path
$rootLength = $Folder.TrimEnd([char]92).Length + 1
$files = Get-ChildItem -LiteralPath $Folder -File -Recurse:$Recurse
if ($Extensions) { $files = $files | Where-Object { $Extensions -contains $_.Extension.TrimStart('.').ToLower() } }
$rows = foreach ($file in $files | Sort-Object Extension, FullName) {
    $ext = $file.Extension.TrimStart('.').ToLower()
    $relative = $file.FullName.Substring($rootLength)
    $png = if ($SaveTo) { Join-Path $SaveTo (($relative -replace '[\\/]', '__') + '.png') } else { '' }
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $result = try { [ThumbCheck]::Fetch($file.FullName, [uint32]$Size, $png) } catch { "ERROR $($_.Exception.Message)" }
    $watch.Stop()
    [pscustomobject]@{ File = $relative; Handler = (Who $ext); Result = $result; Ms = [int]$watch.ElapsedMilliseconds }
}
# FLAT is a thumbnail of one colour: right for a plain texture, wrong for a broken decode.
$bad = @($rows | Where-Object { $_.Result -notmatch '^\d+x\d+( \(FLAT\))?$' })
$flat = @($rows | Where-Object { $_.Result -match 'FLAT' })
if ($FailuresOnly) { $bad | Format-Table -AutoSize | Out-String -Width 250 } else { $rows | Format-Table -AutoSize | Out-String -Width 250 }
$rows | Group-Object { ($_.File -replace '^.*\.', '').ToLower() } | ForEach-Object {
    $failed = @($_.Group | Where-Object { $_.Result -notmatch '^\d+x\d+( \(FLAT\))?$' }).Count
    '.{0,-6} {1,5} files, {2,4} failed, handler {3}, slowest {4} ms' -f $_.Name, $_.Count, $failed, $_.Group[0].Handler, ($_.Group | Measure-Object Ms -Maximum).Maximum
}
"{0} files, {1} without a thumbnail, {2} with a single-colour thumbnail" -f @($rows).Count, $bad.Count, $flat.Count
