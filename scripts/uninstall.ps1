# Uninstall the screendoc binary installed by scripts/install.ps1.
#
# Data in $env:USERPROFILE\.screendoc (sessions, config, encryption key) is
# kept by default. -Purge removes it too (destructive: encrypted sessions
# become unreadable once the key is gone).

[CmdletBinding()]
param([switch]$Purge)

$InstallDir = Join-Path $env:LOCALAPPDATA "screendoc"
$Bin = Join-Path $InstallDir "screendoc.exe"

if (Test-Path $Bin) {
    Remove-Item $Bin -Force
    if (-not (Get-ChildItem $InstallDir -ErrorAction SilentlyContinue)) {
        Remove-Item $InstallDir -Force
    }
    Write-Host "removed $Bin"
} else {
    Write-Host "nothing to remove: $Bin not found"
}

if ($Purge) {
    $data = Join-Path $env:USERPROFILE ".screendoc"
    if (Test-Path $data) {
        $ans = Read-Host "delete $data (sessions, config, encryption key)? 'yes' to confirm"
        if ($ans -eq "yes") {
            Remove-Item $data -Recurse -Force
            Write-Host "purged $data"
        } else {
            Write-Host "aborted: $data kept"
        }
    }
} else {
    $data = Join-Path $env:USERPROFILE ".screendoc"
    Write-Host "recorded sessions remain in $data — run this script with -Purge to delete them too"
}
