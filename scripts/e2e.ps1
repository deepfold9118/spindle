<#
.SYNOPSIS
  End-to-end smoke test for Spindle against the real build library.

.DESCRIPTION
  Builds the workspace, runs unit tests, then checks against real Blender builds:
    1. shared profile redirect (BLENDER_USER_RESOURCES) on every build
    2. launch -> bridge heartbeat -> `spindle instances` and spindle-mcp see it -> UI screenshots
       of every view (checks/ui) -> close -> cleared
    3. Superhive list + sync into the add-on library (skipped if no token is stored)
    4. install a library add-on into a profile with one build, confirm another build sees it enabled
  Everything runs in a throwaway profile ("e2e-<random>") that is deleted at the end;
  the default profile is never touched.

.EXAMPLE
  pwsh scripts/e2e.ps1
  pwsh scripts/e2e.ps1 -Addon powersave -SkipLaunch
#>
param(
    [string]$Addon = "powersave",
    [switch]$SkipLaunch,
    [int]$HeartbeatTimeout = 90
)

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$spindle = Join-Path $root "target\debug\spindle.exe"
$mcp = Join-Path $root "target\debug\spindle-mcp.exe"
$testProfile = "e2e-" + [guid]::NewGuid().ToString("N").Substring(0, 8)
$results = [System.Collections.Generic.List[object]]::new()

function Add-Result($name, $ok, $detail = "") {
    $results.Add([pscustomobject]@{ Check = $name; Result = $(if ($ok -eq $null) { "SKIP" } elseif ($ok) { "PASS" } else { "FAIL" }); Detail = $detail })
    $color = if ($ok -eq $null) { "Yellow" } elseif ($ok) { "Green" } else { "Red" }
    Write-Host ("[{0}] {1} {2}" -f $results[-1].Result, $name, $detail) -ForegroundColor $color
}

$appExe = Join-Path $root "target\debug\spindle_app.exe"

# A screenshot counts as drawn when it exists and isn't a flat colour.
function Test-PictureDrawn($path) {
    if (-not (Test-Path $path)) { return $false }
    Add-Type -AssemblyName System.Drawing
    $bmp = [System.Drawing.Bitmap]::FromFile($path)
    try {
        $colors = [System.Collections.Generic.HashSet[int]]::new()
        for ($y = 0; $y -lt $bmp.Height; $y += 37) {
            for ($x = 0; $x -lt $bmp.Width; $x += 41) { [void]$colors.Add($bmp.GetPixel($x, $y).ToArgb()) }
        }
        return $colors.Count -ge 8
    } finally { $bmp.Dispose() }
}

function Invoke-BlenderPython($exe, $testProfileDir, $code) {
    $file = New-TemporaryFile
    $py = "$($file.FullName).py"
    Move-Item $file.FullName $py
    Set-Content $py $code
    $old = $env:BLENDER_USER_RESOURCES
    try {
        $env:BLENDER_USER_RESOURCES = $testProfileDir
        & $exe -b --python $py 2>$null | Where-Object { $_ -like "E2E *" }
    } finally {
        $env:BLENDER_USER_RESOURCES = $old
        Remove-Item $py -ErrorAction SilentlyContinue
    }
}

Push-Location $root
try {
    # --- build + unit tests -------------------------------------------------
    cargo build -q 2>&1 | Out-Host
    Add-Result "cargo build" ($LASTEXITCODE -eq 0)
    cargo test -q 2>&1 | Out-Null
    Add-Result "cargo test" ($LASTEXITCODE -eq 0)

    $info = & $spindle info
    $library = (($info | Select-String '^library:') -replace '^library:\s*', '').Trim()
    $testProfileDir = Join-Path $library "profiles\$testProfile"
    $builds = & $spindle builds list --json | ConvertFrom-Json
    Add-Result "builds found" ($builds.Count -gt 0) (($builds | ForEach-Object name) -join ", ")
    if ($builds.Count -eq 0) { throw "no builds in $library" }

    # Profile folder only; `spindle launch`/`addons install` add the bridge when they open it.
    $null = New-Item -ItemType Directory -Force (Join-Path $testProfileDir "config")

    # --- 1. profile redirect on every build ----------------------------------
    foreach ($b in $builds) {
        $out = Invoke-BlenderPython $b.executable $testProfileDir "import bpy`nprint('E2E', bpy.utils.user_resource('CONFIG'))"
        $got = ($out -replace '^E2E\s+', '').Trim()
        Add-Result "profile redirect ($($b.name))" ($got -eq (Join-Path $testProfileDir "config")) $got
    }

    # --- 2. launch -> heartbeat -> MCP -> close ------------------------------
    if ($SkipLaunch) {
        Add-Result "launch + heartbeat" $null "skipped"
    } else {
        $build = $builds[0]
        $line = & $spindle launch $build.name --profile $testProfile
        $launchedPid = [int]([regex]::Match($line, 'pid (\d+)').Groups[1].Value)
        $deadline = (Get-Date).AddSeconds($HeartbeatTimeout)
        do {
            Start-Sleep -Seconds 2
            $inst = & $spindle instances --json | ConvertFrom-Json | Where-Object pid -eq $launchedPid
        } while (-not $inst -and (Get-Date) -lt $deadline)
        Add-Result "launch + heartbeat" ([bool]$inst) "pid $launchedPid, profile $($inst.profile)"

        $requests = @(
            '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"e2e","version":"0"}}}',
            '{"jsonrpc":"2.0","method":"notifications/initialized"}',
            '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_running_blenders","arguments":{}}}'
        )
        $responses = $requests | & $mcp | ForEach-Object { $_ | ConvertFrom-Json }
        $seen = $responses | Where-Object id -eq 2 | ForEach-Object { $_.result.structuredContent.instances } | Where-Object pid -eq $launchedPid
        Add-Result "spindle-mcp sees instance" ([bool]$seen)

        # UI pictures (the Fontabulous check): every view while a Blender is running, plus the light theme.
        $shots = Join-Path $root "checks\ui"
        $null = New-Item -ItemType Directory -Force $shots
        foreach ($shot in @(@("builds", "dark"), @("running", "dark"), @("addons", "dark"), @("profiles", "dark"), @("builds", "light"))) {
            $png = Join-Path $shots "$($shot[0])-$($shot[1]).png"
            Remove-Item $png -ErrorAction SilentlyContinue
            & $appExe --screenshot $png --view $shot[0] --theme $shot[1] | Out-Null
            Add-Result "ui screenshot $($shot[0]) ($($shot[1]))" (Test-PictureDrawn $png) $png
        }

        taskkill /PID $launchedPid 2>&1 | Out-Null
        $deadline = (Get-Date).AddSeconds(30)
        while ((Get-Process -Id $launchedPid -ErrorAction SilentlyContinue) -and (Get-Date) -lt $deadline) { Start-Sleep -Seconds 1 }
        if (Get-Process -Id $launchedPid -ErrorAction SilentlyContinue) {
            taskkill /F /PID $launchedPid 2>&1 | Out-Null
            Start-Sleep -Seconds 22   # forced kill skips atexit; wait out the stale window
        }
        $still = & $spindle instances --json | ConvertFrom-Json | Where-Object pid -eq $launchedPid
        Add-Result "instance cleared after exit" (-not $still)
    }

    # --- 3. Superhive ---------------------------------------------------------
    if ($info -match 'token stored') {
        $list = & $spindle superhive list
        Add-Result "superhive list" ($LASTEXITCODE -eq 0 -and $list.Count -gt 0) "$($list.Count) purchases"
        $sync = & $spindle superhive sync
        Add-Result "superhive sync" ($LASTEXITCODE -eq 0) (($sync | ForEach-Object { ($_ -split '\s{2,}')[0] } | Group-Object | ForEach-Object { "$($_.Count) $($_.Name)" }) -join ", ")
    } else {
        Add-Result "superhive" $null "no token stored (spindle superhive login)"
    }

    # --- 4. install from library with one build, visible in another -----------
    $haveAddon = & $spindle addons list | Select-String -Pattern "^$Addon\s"
    if ($haveAddon) {
        $installer = $builds[0]
        & $spindle addons install $Addon --build $installer.name --profile $testProfile | Out-Null
        Add-Result "addon install ($Addon via $($installer.name))" ($LASTEXITCODE -eq 0)
        $checker = $builds[-1]
        $out = Invoke-BlenderPython $checker.executable $testProfileDir "import bpy`nprint('E2E', [a.module for a in bpy.context.preferences.addons if a.module.endswith('.$Addon')])"
        Add-Result "addon enabled in $($checker.name)" ($out -match $Addon) ($out -replace '^E2E\s+', '')
    } else {
        Add-Result "addon install" $null "'$Addon' not in the add-on library"
    }
} finally {
    if ($testProfileDir -and (Test-Path $testProfileDir)) { Remove-Item $testProfileDir -Recurse -Force }
    Pop-Location
}

$results | Format-Table -AutoSize | Out-Host
$failed = @($results | Where-Object Result -eq "FAIL").Count
if ($failed) { Write-Host "$failed check(s) failed" -ForegroundColor Red; exit 1 }
Write-Host "all checks passed" -ForegroundColor Green
