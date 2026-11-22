<#
.SYNOPSIS
  Installs (or uninstalls) Spindle for the current user on Windows.

.DESCRIPTION
  Copies the Spindle programs to a per-user folder (no administrator rights needed), adds a
  Start menu shortcut, puts the folder on your user PATH and starts Spindle. Run it from an
  unpacked release folder to install that release; run it on its own to download the latest
  release from GitHub, checked against the release's SHA256SUMS.

  Upgrading is the same command: a running Spindle is asked to quit first. Your library,
  profiles and settings live elsewhere and are never touched, not even by -Uninstall.

.EXAMPLE
  & ([scriptblock]::Create((irm https://github.com/deepfold9118/spindle/releases/latest/download/install.ps1)))

.EXAMPLE
  .\install.ps1 -StartAtSignIn -OpenBlendFiles

.EXAMPLE
  .\install.ps1 -Uninstall
#>
param(
    [string]$Dir = (Join-Path $env:LOCALAPPDATA "Programs\Spindle"),
    # A release tag such as "v0.1.1" to download instead of the latest.
    [string]$Version = "latest",
    [switch]$NoPath,
    [switch]$NoShortcut,
    [switch]$NoStart,
    # Start Spindle in the tray when you sign in.
    [switch]$StartAtSignIn,
    # Offer Spindle under "Open with" for .blend files.
    [switch]$OpenBlendFiles,
    [switch]$Uninstall
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$Repo = "deepfold9118/spindle"
$Programs = "spindle.exe", "spindle_app.exe", "spindle-mcp.exe"
$Docs = "LICENSE", "NOTICE", "THIRD-PARTY-LICENSES.html", "README.md", "install.ps1"
$Shortcut = Join-Path ([Environment]::GetFolderPath("Programs")) "Spindle.lnk"
$Dir = [IO.Path]::GetFullPath($Dir)
$App = Join-Path $Dir "spindle_app.exe"
$Cli = Join-Path $Dir "spindle.exe"

function Stop-Spindle {
    $running = @(Get-Process spindle_app, spindle-mcp -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($Dir, [StringComparison]::OrdinalIgnoreCase) })
    if (-not $running) { return }
    if (Test-Path $App) { & $App --quit }
    $deadline = (Get-Date).AddSeconds(15)
    while ((Get-Date) -lt $deadline -and @($running | Where-Object { -not $_.HasExited }).Count) { Start-Sleep -Milliseconds 300 }
}

# A file in use (an MCP client keeping spindle-mcp open) can be renamed, not replaced.
function Copy-Over($from, $to) {
    if (Test-Path $to) {
        try { Remove-Item $to -Force }
        catch {
            $old = "$to.old"
            Remove-Item $old -Force -ErrorAction SilentlyContinue
            Rename-Item $to (Split-Path $old -Leaf)
        }
    }
    Copy-Item $from $to
}

# Registrations (sign-in, .blend) that point into this folder.
function Get-OwnRegistrations {
    if (-not (Test-Path $Cli)) { return @() }
    $status = & $Cli desktop --app $App 2>$null
    $own = @()
    foreach ($line in $status) {
        if ($line -match '^(start at sign-in|open \.blend files):\s+on \((.+)\)$' -and $Matches[2].StartsWith($Dir, [StringComparison]::OrdinalIgnoreCase)) {
            $own += $(if ($Matches[1] -eq "start at sign-in") { "--autostart" } else { "--open-blend" })
        }
    }
    return $own
}

if ($Uninstall) {
    Stop-Spindle
    if (Test-Path $Cli) {
        $off = @("--path", "off")
        foreach ($r in Get-OwnRegistrations) { $off += @($r, "off") }
        & $Cli desktop --app $App @off | Out-Null
    }
    if ((Test-Path $Shortcut) -and ((New-Object -ComObject WScript.Shell).CreateShortcut($Shortcut).TargetPath -like "$Dir\*")) {
        Remove-Item $Shortcut -Force
    }
    foreach ($f in $Programs + $Docs) {
        Remove-Item (Join-Path $Dir $f), (Join-Path $Dir "$f.old") -Force -ErrorAction SilentlyContinue
    }
    if ((Test-Path $Dir) -and -not (Get-ChildItem $Dir -Force)) { Remove-Item $Dir -Force }
    Write-Host "Spindle is uninstalled. Your library, profiles and settings were left alone."
    return
}

# --- the release to install: next to this script, or downloaded --------------------------
$work = $null
$here = if ($PSScriptRoot) { $PSScriptRoot } else { "" }
if ($here -and (Test-Path (Join-Path $here "spindle_app.exe"))) {
    $src = $here
} else {
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    $api = if ($Version -eq "latest") { "https://api.github.com/repos/$Repo/releases/latest" } else { "https://api.github.com/repos/$Repo/releases/tags/$Version" }
    $release = Invoke-RestMethod $api -Headers @{ "User-Agent" = "spindle-install" }
    $zip = $release.assets | Where-Object { $_.name -like "spindle-*-windows-x64.zip" } | Select-Object -First 1
    $sums = $release.assets | Where-Object { $_.name -eq "SHA256SUMS" } | Select-Object -First 1
    if (-not $zip -or -not $sums) { throw "release $($release.tag_name) has no Windows archive or SHA256SUMS" }
    $work = Join-Path ([IO.Path]::GetTempPath()) ("spindle-install-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
    $null = New-Item -ItemType Directory -Force $work
    Write-Host "Downloading Spindle $($release.tag_name)..."
    $archive = Join-Path $work $zip.name
    Invoke-WebRequest $zip.browser_download_url -OutFile $archive -UseBasicParsing
    $sumText = (Invoke-WebRequest $sums.browser_download_url -UseBasicParsing).Content
    if ($sumText -is [byte[]]) { $sumText = [Text.Encoding]::UTF8.GetString($sumText) }
    $want = ($sumText -split "`n" | Where-Object { $_ -match [regex]::Escape($zip.name) } | Select-Object -First 1) -replace '\s.*$', ''
    $got = (Get-FileHash $archive -Algorithm SHA256).Hash
    if (-not $want -or $want.Trim() -ne $got.ToLower()) { throw "SHA-256 mismatch for $($zip.name)" }
    Expand-Archive $archive (Join-Path $work "x")
    $src = (Get-ChildItem (Join-Path $work "x") -Recurse -Filter spindle_app.exe | Select-Object -First 1).DirectoryName
}

try {
    # --- copy ---------------------------------------------------------------------------
    $null = New-Item -ItemType Directory -Force $Dir
    if ([IO.Path]::GetFullPath($src).TrimEnd('\') -ne $Dir.TrimEnd('\')) {
        Stop-Spindle
        foreach ($f in $Programs + $Docs) {
            $from = Join-Path $src $f
            if (Test-Path $from) { Copy-Over $from (Join-Path $Dir $f) }
        }
        # Running from a download: keep this script with the programs for -Uninstall.
        if (-not (Test-Path (Join-Path $src "install.ps1")) -and $PSCommandPath) { Copy-Item $PSCommandPath (Join-Path $Dir "install.ps1") -Force }
    }
    if ($PSCommandPath -and -not (Test-Path (Join-Path $Dir "install.ps1"))) { Copy-Item $PSCommandPath (Join-Path $Dir "install.ps1") }

    # --- shortcut, PATH, registrations -------------------------------------------------------
    if (-not $NoShortcut) {
        $s = (New-Object -ComObject WScript.Shell).CreateShortcut($Shortcut)
        $s.TargetPath = $App
        $s.WorkingDirectory = $Dir
        $s.Description = "Blender build, profile and add-on manager"
        $s.Save()
    }
    $desk = @()
    if (-not $NoPath) { $desk += @("--path", "on") }
    if ($StartAtSignIn) { $desk += @("--autostart", "on") }
    if ($OpenBlendFiles) { $desk += @("--open-blend", "on") }
    if ($desk) { & $Cli desktop --app $App @desk | Out-Null }

    $version = (& $Cli --version) -replace '^spindle\s+', ''
    Write-Host "Spindle $version is installed in $Dir"
    if (-not $NoShortcut) { Write-Host "  Start menu: Spindle" }
    if (-not $NoPath) { Write-Host "  spindle, spindle_app and spindle-mcp work in new terminals" }
    if ($StartAtSignIn) { Write-Host "  starts in the tray when you sign in" }
    if ($OpenBlendFiles) { Write-Host "  offered under Open with for .blend files" }
    if (-not $NoStart) { Start-Process $App -WorkingDirectory $Dir }
} finally {
    if ($work) { Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue }
}
