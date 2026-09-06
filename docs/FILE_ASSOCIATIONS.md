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

Images: JPG/JPEG/JPE, PNG/APNG, GIF, WebP, BMP, TIF/TIFF and ICO.
Videos: MP4/M4V, MOV, WebM and MKV, subject to installed Windows codecs.
AVIF and other unimplemented image formats are deliberately not advertised.

No extension default value, protected `UserChoice`, hash or another program's
registration is overwritten. Registration adds a choice; it does not make Kova
the default. The Settings button opens the official
`ms-settings:defaultapps?registeredAppUser=Kova%20Image` URI. Windows controls
which formats the user selects; older Windows versions may show the general
Default Apps page.

Moving/removing the executable can invalidate registration. Register again from
its new permanent location. A complete installer, Start menu shortcut,
upgrade/uninstall cleanup and clean-machine association tests are future work;
this is not an installer or an automatic default takeover.

References: [Default Programs registration](https://learn.microsoft.com/en-us/windows/win32/shell/default-programs),
[launch Default Apps Settings](https://learn.microsoft.com/en-us/windows/apps/develop/launch/launch-default-apps-settings).
