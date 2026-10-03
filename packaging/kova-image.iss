; Per-user installer for Kova Image.
;
; Build it with scripts\installer.ps1, which first produces the reviewed
; portable staging folder (license collection fails closed there) and then
; passes that folder in as StageDir. Compiling this file on its own is
; deliberately an error: an installer built from an unreviewed folder could
; ship without the dependency license texts.

#ifndef StageDir
  #error StageDir is not defined. Build the installer with scripts\installer.ps1.
#endif
#ifndef AppVersion
  #error AppVersion is not defined. Build the installer with scripts\installer.ps1.
#endif

#define AppName "Kova Image"
#define Publisher "Kova Contributors"
#define RepositoryUrl "https://github.com/cubiix3/Kova-Image"

[Setup]
; A stable AppId keeps upgrades and the uninstall entry consistent. Never
; change it; a new value would install a second, parallel copy.
AppId={{6E2F5A17-4D3B-4A9E-9C2B-0F7A1D8C5E34}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#Publisher}
AppPublisherURL={#RepositoryUrl}
AppSupportURL={#RepositoryUrl}/issues
AppUpdatesURL={#RepositoryUrl}/releases
VersionInfoVersion={#AppVersion}
; The viewer needs no elevation: it installs under the user profile, writes
; only HKCU when the user opts in to Open with, and never touches machine state.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
UninstallDisplayName={#AppName}
UninstallDisplayIcon={app}\kova-image.exe
LicenseFile={#StageDir}\LICENSE
OutputBaseFilename=Kova-Image-{#AppVersion}-x64-setup
SetupIconFile={#SourcePath}\..\assets\kova.ico
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; An open viewer holds its own executable. Ask to close it rather than
; failing the install or leaving a half-replaced folder behind.
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "german"; MessagesFile: "compiler:Languages\German.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
; File associations stay opt-in, and registration only *adds* Kova Image to the
; Open with list. Windows still owns which application is the default.
Name: "associations"; Description: "Register {#AppName} for Open with (Windows keeps your current defaults)"; Flags: unchecked
; Previews in Explorer and the file dialogs for the formats Windows cannot preview
; itself (WebP, AVIF, SVG, RAW...). Per user, and only where no other program
; already provides previews for the extension.
Name: "thumbnails"; Description: "Show previews in Explorer for more image formats (WebP, AVIF, HEIC, SVG, RAW, TGA, DDS...)"

[Files]
Source: "{#StageDir}\*"; DestDir: "{app}"; Excludes: "kova_thumbnails.dll"; Flags: ignoreversion recursesubdirs createallsubdirs
; Explorer keeps a loaded preview provider in memory for a while, so replacing or
; removing it may have to wait for a restart.
Source: "{#StageDir}\kova_thumbnails.dll"; DestDir: "{app}"; Flags: ignoreversion restartreplace uninsrestartdelete

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\kova-image.exe"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\kova-image.exe"; Tasks: desktopicon

[Run]
; Registration is idempotent and records the final install path, so it has to
; run after the files are in place.
Filename: "{app}\kova-image.exe"; Parameters: "--register-file-associations"; StatusMsg: "Registering {#AppName} for Open with..."; Flags: runhidden waituntilterminated; Tasks: associations
Filename: "{app}\kova-image.exe"; Parameters: "--register-thumbnails"; StatusMsg: "Enabling Explorer previews..."; Flags: runhidden waituntilterminated; Tasks: thumbnails
Filename: "{app}\kova-image.exe"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Takes the preview provider out of the registry again before its files go.
Filename: "{app}\kova-image.exe"; Parameters: "--unregister-thumbnails"; RunOnceId: "KovaImageThumbnails"; Flags: runhidden

[Registry]
; Uninstall cleanup for the keys the application writes when the user opts in.
; dontcreatekey keeps the installer itself from registering anything: a user who
; skips the task must end up with no registration at all. Protected UserChoice
; values belong to Windows and are never read or written here.
Root: HKCU; Subkey: "Software\Kova\Image"; Flags: uninsdeletekey dontcreatekey
Root: HKCU; Subkey: "Software\Kova"; Flags: uninsdeletekeyifempty dontcreatekey
Root: HKCU; Subkey: "Software\Classes\KovaImage.Image"; Flags: uninsdeletekey dontcreatekey
Root: HKCU; Subkey: "Software\Classes\KovaImage.Video"; Flags: uninsdeletekey dontcreatekey
Root: HKCU; Subkey: "Software\Classes\KovaImage.Audio"; Flags: uninsdeletekey dontcreatekey
Root: HKCU; Subkey: "Software\Classes\Applications\kova-image.exe"; Flags: uninsdeletekey dontcreatekey
Root: HKCU; Subkey: "Software\RegisteredApplications"; ValueType: none; ValueName: "{#AppName}"; Flags: uninsdeletevalue dontcreatekey
; Only Kova's own ProgID value is removed from each extension's Open with list.
Root: HKCU; Subkey: "Software\Classes\.jpg\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.jpeg\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.jpe\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.png\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.apng\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.gif\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.webp\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.bmp\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.tif\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.tiff\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.ico\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.tga\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.pbm\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.pgm\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.ppm\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.pnm\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.pam\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.qoi\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.dds\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.hdr\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.exr\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.ff\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.jxl\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.avif\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.heic\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.heif\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.svg\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.svgz\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.3fr\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.ari\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.arw\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.cr2\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.cr3\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.crw\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.dcr\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.dng\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.erf\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.iiq\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.kdc\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.mef\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.mrw\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.nef\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.nrw\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.orf\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.pef\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.raf\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.rw2\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.rwl\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.sr2\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.srf\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.srw\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.x3f\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Image"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.mp4\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Video"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.m4v\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Video"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.mov\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Video"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.webm\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Video"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.mkv\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Video"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.mp3\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Audio"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.m4a\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Audio"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.m4b\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Audio"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.aac\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Audio"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.wav\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Audio"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.flac\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Audio"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.ogg\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Audio"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.oga\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Audio"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.opus\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Audio"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.wma\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Audio"; Flags: uninsdeletevalue dontcreatekey

[UninstallDelete]
; Inno removes every installed file, but leaves the nested payload folders
; behind as empty directories. These four are created by this installer and
; hold nothing the user authored, so an uninstall should take them with it.
Type: filesandordirs; Name: "{app}\assets"
Type: filesandordirs; Name: "{app}\docs"
Type: filesandordirs; Name: "{app}\licenses"
Type: filesandordirs; Name: "{app}\third-party-licenses"
Type: dirifempty; Name: "{app}"
