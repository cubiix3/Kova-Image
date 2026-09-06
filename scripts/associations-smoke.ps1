#Requires -Version 5.1
# Opt-in integration test: registers the supplied executable for the current user.
# Never changes Windows UserChoice defaults. Use a permanent executable path.
param([Parameter(Mandatory=$true)][string]$Executable)
$ErrorActionPreference='Stop'
$exe=(Resolve-Path -LiteralPath $Executable).Path
$images=@('jpg','jpeg','jpe','png','apng','gif','webp','bmp','tif','tiff','ico')
$videos=@('mp4','m4v','mov','webm','mkv')
function Read-Choices {
    $values=[ordered]@{}
    foreach ($extension in ($images+$videos)) {
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
foreach ($kind in @('Image','Video')) {
    $key=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Software\Classes\KovaImage.$kind\shell\open\command")
    try {if (!$key -or $key.GetValue('') -ne $expected) {throw "Incorrect $kind command."}}
    finally {if ($key) {$key.Dispose()}}
}
$caps=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Kova\Image\Capabilities\FileAssociations')
try {
    foreach ($extension in ($images+$videos)) {
        $kind=if ($images -contains $extension) {'Image'} else {'Video'}
        if ($caps.GetValue(".$extension") -ne "KovaImage.$kind") {throw "Missing .$extension capability."}
        $key=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Software\Classes\.$extension\OpenWithProgids")
        try {if (!$key -or $key.GetValueNames() -notcontains "KovaImage.$kind") {throw "Missing .$extension Open with entry."}}
        finally {if ($key) {$key.Dispose()}}
    }
    if ($caps.GetValueNames() -contains '.avif') {throw 'Unimplemented AVIF must not be advertised.'}
} finally {if ($caps) {$caps.Dispose()}}
$registered=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\RegisteredApplications')
try {if ($registered.GetValue('Kova Image') -ne 'Software\Kova\Image\Capabilities') {throw 'Missing RegisteredApplications entry.'}}
finally {$registered.Dispose()}
if ((Read-Choices) -ne $before) {throw 'Windows default choices unexpectedly changed.'}
Write-Output 'PASS: 16 format capabilities, quoted commands, Open with, RegisteredApplications; UserChoice unchanged.'
