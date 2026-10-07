#Requires -Version 5.1
# Opt-in integration test: registers the supplied executable for the current user.
# Never changes Windows UserChoice defaults. Use a permanent executable path.
param([Parameter(Mandatory=$true)][string]$Executable)
$ErrorActionPreference='Stop'
$exe=(Resolve-Path -LiteralPath $Executable).Path
# Read the viewer's lists so newly supported extensions cannot escape this check.
function Read-Extensions([string]$Source, [string]$Name) {
    $text=Get-Content -LiteralPath (Join-Path $PSScriptRoot "../src/$Source") -Raw
    $list=[regex]::Match($text, ('(?s)pub const '+$Name+': &\[&str\] = &\[(.*?)\];'))
    if (!$list.Success) { throw "Cannot read $Name from $Source." }
    $extensions=@([regex]::Matches($list.Groups[1].Value, '"([a-z0-9]+)"') | ForEach-Object { $_.Groups[1].Value })
    if ($extensions.Count -eq 0) { throw "Empty $Name in $Source." }
    return $extensions
}
$images=@(Read-Extensions 'format.rs' 'IMAGE_EXTENSIONS')
$videos=@(Read-Extensions 'media.rs' 'VIDEO_EXTENSIONS')
$audio=@(Read-Extensions 'media.rs' 'AUDIO_EXTENSIONS')
function Read-Choices {
    $values=[ordered]@{}
    foreach ($extension in ($images+$videos+$audio)) {
        $key=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.$extension\UserChoice")
        try { $values[$extension]=if ($key) { @($key.GetValue('ProgId'),$key.GetValue('Hash')) } else { @() } }
        finally { if ($key) { $key.Dispose() } }
    }
    return ($values | ConvertTo-Json -Compress)
}
$before=Read-Choices
$process=Start-Process -FilePath $exe -ArgumentList '--register-file-associations' -WindowStyle Hidden -Wait -PassThru
if ($process.ExitCode -ne 0) { throw 'Registration failed.' }
$expected='"'+$exe+'" -- "%1"'
foreach ($kind in @('Image','Video','Audio')) {
    $key=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Software\Classes\KovaImage.$kind\shell\open\command")
    try {if (!$key -or $key.GetValue('') -ne $expected) {throw "Incorrect $kind command."}}
    finally {if ($key) {$key.Dispose()}}
}
$app=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Classes\Applications\kova-image.exe\shell\open\command')
try {if (!$app -or $app.GetValue('') -ne $expected) {throw 'Incorrect application command.'}}
finally {if ($app) {$app.Dispose()}}
$types=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Classes\Applications\kova-image.exe\SupportedTypes')
$caps=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Kova\Image\Capabilities\FileAssociations')
try {
    if (!$caps -or !$types) {throw 'Missing capabilities or SupportedTypes.'}
    foreach ($extension in ($images+$videos+$audio)) {
        $kind=if ($images -contains $extension) {'Image'} elseif ($videos -contains $extension) {'Video'} else {'Audio'}
        if ($caps.GetValue(".$extension") -ne "KovaImage.$kind") {throw "Missing .$extension capability."}
        if ($types.GetValueNames() -notcontains ".$extension") {throw "Missing .$extension SupportedTypes entry."}
        $key=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Software\Classes\.$extension\OpenWithProgids")
        try {if (!$key -or $key.GetValueNames() -notcontains "KovaImage.$kind") {throw "Missing .$extension Open with entry."}}
        finally {if ($key) {$key.Dispose()}}
    }
} finally {
    if ($caps) {$caps.Dispose()}
    if ($types) {$types.Dispose()}
}
$registered=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\RegisteredApplications')
try {if ($registered.GetValue('Kova Image') -ne 'Software\Kova\Image\Capabilities') {throw 'Missing RegisteredApplications entry.'}}
finally {$registered.Dispose()}
if ((Read-Choices) -ne $before) {throw 'Windows default choices unexpectedly changed.'}
Write-Output "PASS: $(($images+$videos+$audio).Count) format capabilities, SupportedTypes, quoted commands, Open with, RegisteredApplications; UserChoice unchanged."
