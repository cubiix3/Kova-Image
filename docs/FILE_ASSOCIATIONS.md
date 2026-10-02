# Windows file associations

Keep the portable application in a permanent folder before registering it.
Settings provides **Register Kova Image for Open with** and **Choose default
viewer in Windows Settings**. Registration can also run without starting the UI:

```powershell
.\kova-image.exe --register-file-associations
```

Registration is explicit, idempotent and per-user. It requires no administrator
rights and calls the documented Windows Registry and Shell APIs:

| HKCU location | Purpose |
| --- | --- |
| `Software\Kova\Image\Capabilities` | Application metadata and supported file associations |
| `Software\RegisteredApplications`, value `Kova Image` | Windows Default Apps discovery |
| `Software\Classes\KovaImage.Image` / `KovaImage.Video` | Descriptions, icon and quoted open command |
| `Software\Classes\Applications\kova-image.exe` | Friendly name, executable command and SupportedTypes |
| `Software\Classes\.<extension>\OpenWithProgids` | Add only Kova's own ProgID value |

The open command quotes the executable and `%1`, with `--` before the file path.
Spaces, Unicode, long paths and filenames resembling switches remain file
arguments. Each activation starts an independent window; there is no fragile
single-instance IPC. The native file picker and file drop use the same loader.

Images: JPG/JPEG/JPE, PNG/APNG, GIF, WebP, BMP, TIF/TIFF, ICO, TGA, PBM/PGM/PPM/PNM/PAM,
QOI, DDS, HDR, EXR, farbfeld (FF), JXL, AVIF, HEIC/HEIF, SVG/SVGZ and the camera RAW
extensions listed in `src/format.rs`.
Videos: MP4/M4V, MOV, WebM and MKV, subject to installed Windows codecs.
Only formats Kova Image can decode are advertised.

No extension default value, protected `UserChoice`, hash or another program's
registration is overwritten. Registration adds a choice; it does not make Kova
the default. The Settings button opens the official
`ms-settings:defaultapps?registeredAppUser=Kova%20Image` URI. Windows controls
which formats the user selects; older Windows versions may show the general
Default Apps page.

Moving/removing the executable can invalidate registration. Register again from
its new permanent location. The [per-user installer](WINDOWS_RELEASE.md) keeps a
permanent folder for you, offers registration as an unchecked task and removes
these keys again on uninstall. Registration itself is still not an automatic
default takeover, and clean-machine association tests remain future work.

## Explorer previews

Previews are separate from associations. Windows previews JPEG, PNG, GIF, BMP,
TIFF and ICO, but it has no WebP, AVIF, HEIC, JPEG XL, SVG, RAW, TGA, APNG, QOI,
DDS, HDR or EXR decoder of its own. `kova_thumbnails.dll`, next to the executable,
is a thumbnail provider (`IThumbnailProvider` with `IInitializeWithStream`) that
decodes these through the same code as the viewer, with the same limits, and
returns a premultiplied BGRA bitmap. Explorer runs it in its isolated
preview host, so a damaged file takes down only that host process.

The provider is registered for every image extension Kova Image opens. For the six
formats that Windows reads itself, it looks at the first bytes of the file: when
the content matches the extension (a real PNG named `.png`), it hands the file to
Windows' own photo thumbnail provider (`PhotoMetadataHandler.dll`) and returns
that bitmap unchanged, so those previews do not change. When the content is
something else (game folders are full of TGA files named `.png`) or Windows fails,
Kova decodes the file itself.

```powershell
.\kova-image.exe --register-thumbnails
.\kova-image.exe --unregister-thumbnails
```

Both are also in the menu (**Show previews in Explorer**, **Remove Explorer
previews**) and the installer offers registration as a task. Like the Open with
registration it is per user and idempotent and writes only Kova's own keys:

| HKCU location | Purpose |
| --- | --- |
| `Software\Classes\CLSID\{7C1D3A52-9E84-4B6F-8A07-52F0D9C3B1E6}` | The provider class and the path of the DLL (`InprocServer32`, `ThreadingModel` Both) |
| `Software\Classes\.<extension>\ShellEx\{E357FCCD-A995-4576-B01F-234630154E96}` | The preview handler of one extension |

An extension is taken only when no other program provides a working preview;
Windows' generic image handler does not count (it is used through the provider
as described above), and neither does a handler whose DLL no longer
exists (for example "SumatraPDF Preview" left behind for `.tga` after an
uninstall: Explorer cannot load it and shows only the file icon). The exception
is `.dds`: texture tools register their own provider, and Kova's reads the same
formats, so it is replaced for the current user (HKCU wins over the machine-wide
key, and removing the previews brings the other provider back). Unregistering removes the class and exactly the
extension keys that point at it. After registering, Explorer may need `F5` or a
larger icon size before the thumbnails show; the thumbnail cache is Windows'.
Moving the executable requires registering again, and the DLL cannot be
replaced while Explorer holds it (the installer schedules the replacement for
the next restart in that case).

References: [Default Programs registration](https://learn.microsoft.com/en-us/windows/win32/shell/default-programs),
[launch Default Apps Settings](https://learn.microsoft.com/en-us/windows/apps/develop/launch/launch-default-apps-settings).
