# screendoc installer for Windows (PowerShell 5.1+).
#
# Downloads the prebuilt Windows binary from the GitHub releases, verifies
# the SHA-256, and installs it to %LOCALAPPDATA%\screendoc (added to the
# user PATH). Prerequisites are checked and installed when a supported
# package manager (winget or choco) is present:
#   - ffmpeg (optional: video renders; cast/web/import work without it)
#   - Git    (provides the Git Bash shell the recording wrapper uses)
#
# tmux does not exist on Windows. For `record --tmux`, run screendoc inside
# WSL instead (Linux build, full feature set).
#
# Usage:
#   .\install.ps1                 # latest release
#   .\install.ps1 -Version v0.1.0
#   .\install.ps1 -NoPath         # skip the user PATH modification

[CmdletBinding()]
param(
    [string]$Version = "latest",
    [switch]$NoPath
)

$ErrorActionPreference = "Stop"
$Repo = "GlobalTechInfo/screendoc"
$Target = "x86_64-pc-windows-msvc"
$Artifact = "screendoc-$Target.zip"
$InstallDir = Join-Path $env:LOCALAPPDATA "screendoc"
$Bin = Join-Path $InstallDir "screendoc.exe"

function Say($msg)  { Write-Host "==> $msg" }
function Warn2($m)  { Write-Warning $m }

# ---------------- prerequisites ----------------
function Have-Command($name) {
    try { $null = Get-Command $name -ErrorAction Stop; return $true } catch { return $false }
}

$pm = $null
if (Have-Command "winget") { $pm = "winget" }
elseif (Have-Command "choco") { $pm = "choco" }

function Install-Tool($name, $wingetId, $chocoId) {
    if (Have-Command $name) { return }
    if ($null -eq $pm) {
        Warn2 "$name not found and no winget/choco available — install it manually (video/shell features will be limited)."
        return
    }
    Say "installing $name via $pm"
    if ($pm -eq "winget") {
        # winget >= 1.11 takes the package as a positional query.
        winget install -e --silent $wingetId --accept-source-agreements --accept-package-agreements | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "winget install failed for $wingetId" }
    } else {
        choco install $chocoId -y --no-progress | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "choco install failed for $chocoId" }
    }
}

# ffmpeg (optional but recommended: mp4/webm/gif renders)
Install-Tool "ffmpeg" "Gyan.FFmpeg" "ffmpeg"
if (Have-Command "ffmpeg") { Say "ffmpeg available" }

# Git (provides Git Bash, which the recording wrapper launches as the shell)
Install-Tool "git" "Git.Git" "git"
if (Have-Command "bash") { Say "Git Bash available" }
else {
    Warn2 "bash not found on PATH. The wrapper needs a shell: install Git for Windows"
    Warn2 "(winget install Git.Git). tmux is not available on Windows — use WSL for `record --tmux`."
}

# ---------------- fetch ----------------
$apiVersion = $Version
if ($Version -eq "latest") {
    Say "resolving the latest release tag"
    $rel = Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest" -TimeoutSec 20
    $apiVersion = $rel.tag_name
}
elseif (-not $Version.StartsWith("v")) { $apiVersion = "v$Version" }

$url = "https://github.com/$Repo/releases/download/$apiVersion/$Artifact"
$shaUrl = "$url.sha256"
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("screendoc-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    Say "downloading $Artifact ($apiVersion, $Target)"
    $zip = Join-Path $tmp $Artifact
    Invoke-WebRequest -Uri $url -OutFile $zip -TimeoutSec 600
    try {
        $sha = Invoke-WebRequest -Uri $shaUrl -OutFile (Join-Path $tmp "$Artifact.sha256") -TimeoutSec 30
        $expected = (Get-Content (Join-Path $tmp "$Artifact.sha256") -Raw).Trim().Split('\s+')[0]
        $actual = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
        if ($expected -and $actual -ne $expected) {
            throw "sha256 mismatch: expected $expected, got $actual"
        }
        Say "sha256 verified"
    } catch [System.Net.WebException] {
        Write-Verbose "no .sha256 published yet — skipping checksum verification"
    }
    Expand-Archive -Path $zip -DestinationPath $tmp -Force
}
finally {
    if (Test-Path $tmp) { Remove-Item $tmp -Recurse -Force }
}

# ---------------- install ----------------
if (-not (Test-Path $InstallDir)) { New-Item -ItemType Directory -Path $InstallDir | Out-Null }
$extracted = Get-ChildItem -Path $tmp -Filter "screendoc.exe" -Recurse | Select-Object -First 1
if (-not $extracted) {
    Warn2 "download failed for $apiVersion (artifact not available yet) — use `cargo build --release` from a checkout."
    exit 1
}
Copy-Item $extracted.FullName $Bin -Force
Say "installed $Bin ($apiVersion)"

if (-not $NoPath) {
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if (($userPath -split [IO.Path]::PathSeparator) -notcontains $InstallDir) {
        [Environment]::SetEnvironmentVariable("Path", "$userPath;$InstallDir", "User")
        Say "added $InstallDir to the user PATH (new terminals only)"
    }
}

Say "done. Open a new terminal and verify with: screendoc doctor"
