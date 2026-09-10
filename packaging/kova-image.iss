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

[Files]
Source: "{#StageDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\kova-image.exe"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\kova-image.exe"; Tasks: desktopicon

[Run]
; Registration is idempotent and records the final install path, so it has to
; run after the files are in place.
Filename: "{app}\kova-image.exe"; Parameters: "--register-file-associations"; StatusMsg: "Registering {#AppName} for Open with..."; Flags: runhidden waituntilterminated; Tasks: associations
Filename: "{app}\kova-image.exe"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Registry]
; Uninstall cleanup for the keys the application writes when the user opts in.
; dontcreatekey keeps the installer itself from registering anything: a user who
; skips the task must end up with no registration at all. Protected UserChoice
; values belong to Windows and are never read or written here.
Root: HKCU; Subkey: "Software\Kova\Image"; Flags: uninsdeletekey dontcreatekey
Root: HKCU; Subkey: "Software\Kova"; Flags: uninsdeletekeyifempty dontcreatekey
Root: HKCU; Subkey: "Software\Classes\KovaImage.Image"; Flags: uninsdeletekey dontcreatekey
Root: HKCU; Subkey: "Software\Classes\KovaImage.Video"; Flags: uninsdeletekey dontcreatekey
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
Root: HKCU; Subkey: "Software\Classes\.mp4\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Video"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.m4v\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Video"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.mov\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Video"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.webm\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Video"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\.mkv\OpenWithProgids"; ValueType: none; ValueName: "KovaImage.Video"; Flags: uninsdeletevalue dontcreatekey

[UninstallDelete]
; Inno removes every installed file, but leaves the nested payload folders
; behind as empty directories. These four are created by this installer and
; hold nothing the user authored, so an uninstall should take them with it.
Type: filesandordirs; Name: "{app}\assets"
Type: filesandordirs; Name: "{app}\docs"
Type: filesandordirs; Name: "{app}\licenses"
Type: filesandordirs; Name: "{app}\third-party-licenses"
Type: dirifempty; Name: "{app}"
