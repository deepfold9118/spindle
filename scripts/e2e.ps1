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
    5. build downloads: every channel lists builds, update check runs
    6. profile compatibility: builds older than 4.5 are flagged as unable to read 5.x preferences,
       newer ones aren't; spindle-mcp refuses to launch an old build onto such a profile
    7. removal: an external build is unregistered and its files are left alone
    8. tray: close hides to the tray, the tray shows it again, a second start hands over,
       `--quit` exits (skipped while a Spindle is already running)
    9. favourites: a starred build is listed first and is the default build; unstarred after
   10. desktop integration: start at sign-in and the .blend "Open with" registration are written
       and removed again (skipped if Spindle is already registered, so a real setup isn't undone)
   11. opening .blend files: plain and zstd-compressed files saved by two builds each open in
       the build of their series; `spindle_app file.blend` launches Blender with the file
   12. self-update: the newest GitHub release is readable (skipped before the first release)
  Everything runs in a throwaway profile ("e2e-<random>") that is deleted at the end;
  the default profile is never touched. Downloading a whole build is not repeated here
  (400 MB each); run `spindle builds download <version>` to exercise it.

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

# (major, minor) as a comparable number: 4.5 -> 405.
function Get-Series($version) {
    if ($version -match '^(\d+)\.(\d+)') { return [int]$Matches[1] * 100 + [int]$Matches[2] }
    return 0
}

function Invoke-BlenderPython($build, $testProfileDir, $code) {
    $file = New-TemporaryFile
    $py = "$($file.FullName).py"
    Move-Item $file.FullName $py
    Set-Content $py $code
    $names = "BLENDER_USER_RESOURCES", "BLENDER_USER_CONFIG", "BLENDER_USER_SCRIPTS", "BLENDER_USER_DATAFILES"
    $old = @{}; foreach ($n in $names) { $old[$n] = [Environment]::GetEnvironmentVariable($n) }
    try {
        $env:BLENDER_USER_RESOURCES = $testProfileDir
        if ((Get-Series $build.version) -lt 306) {
            # Spindle sets these for builds older than 3.6, which ignore BLENDER_USER_RESOURCES.
            $env:BLENDER_USER_CONFIG = Join-Path $testProfileDir "config"
            $env:BLENDER_USER_SCRIPTS = Join-Path $testProfileDir "scripts"
            $env:BLENDER_USER_DATAFILES = Join-Path $testProfileDir "datafiles"
        }
        & $build.executable -b --python $py 2>$null | Where-Object { $_ -like "E2E *" }
    } finally {
        foreach ($n in $names) { [Environment]::SetEnvironmentVariable($n, $old[$n]) }
        Remove-Item $py -ErrorAction SilentlyContinue
    }
}

function Invoke-Mcp($calls) {
    $requests = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"e2e","version":"0"}}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}'
    ) + $calls
    $requests | & $mcp | ForEach-Object { $_ | ConvertFrom-Json }
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
        $out = Invoke-BlenderPython $b $testProfileDir "import bpy`nprint('E2E', bpy.utils.user_resource('CONFIG'))"
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

        $responses = Invoke-Mcp @('{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_running_blenders","arguments":{}}}')
        $seen = $responses | Where-Object id -eq 2 | ForEach-Object { $_.result.structuredContent.instances } | Where-Object pid -eq $launchedPid
        Add-Result "spindle-mcp sees instance" ([bool]$seen)

        # UI pictures (the Fontabulous check): every view while a Blender is running, plus the light theme.
        $shots = Join-Path $root "checks\ui"
        $null = New-Item -ItemType Directory -Force $shots
        foreach ($shot in @(@("builds", "dark"), @("running", "dark"), @("addons", "dark"), @("profiles", "dark"), @("downloads", "dark"), @("builds", "light"))) {
            $png = Join-Path $shots "$($shot[0])-$($shot[1]).png"
            Remove-Item $png -ErrorAction SilentlyContinue
            & $appExe --screenshot $png --view $shot[0] --theme $shot[1] | Out-Null
            Add-Result "ui screenshot $($shot[0]) ($($shot[1]))" (Test-PictureDrawn $png) $png
        }
        # The compatibility dialog for the oldest build.
        $png = Join-Path $shots "compat-dark.png"
        Remove-Item $png -ErrorAction SilentlyContinue
        & $appExe --screenshot $png --view builds --show-compat $builds[-1].name | Out-Null
        Add-Result "ui screenshot compatibility dialog" (Test-PictureDrawn $png) $png

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
        # The oldest other build that can read the installer's preferences (5.x format: 4.5+).
        $checker = @($builds | Where-Object { $_.name -ne $installer.name -and (Get-Series $_.version) -ge 405 })[-1]
        if (-not $checker) { $checker = $installer }
        $out = Invoke-BlenderPython $checker $testProfileDir "import bpy`nprint('E2E', [a.module for a in bpy.context.preferences.addons if a.module.endswith('.$Addon')])"
        Add-Result "addon enabled in $($checker.name)" ($out -match $Addon) ($out -replace '^E2E\s+', '')
    } else {
        Add-Result "addon install" $null "'$Addon' not in the add-on library"
    }

    # --- 5. build downloads ------------------------------------------------------
    $avail = & $spindle builds available --refresh --json | ConvertFrom-Json
    foreach ($ch in "lts", "stable", "daily", "experimental") {
        $n = @($avail | Where-Object channel -eq $ch).Count
        Add-Result "available $ch builds" ($n -gt 0) "$n listed"
    }
    $ups = & $spindle builds updates
    Add-Result "update check" ($LASTEXITCODE -eq 0) (($ups | Select-Object -First 3) -join "; ")

    # --- 6. profile compatibility ---------------------------------------------------
    $prefs = Join-Path $testProfileDir "config\userpref.blend"
    if (-not (Test-Path $prefs)) {
        # Header of a preferences file saved by Blender 5.3 (the 5.0+ .blend format).
        [IO.File]::WriteAllBytes($prefs, [Text.Encoding]::ASCII.GetBytes("BLENDER17-01v0503" + ("`0" * 64)))
    }
    foreach ($b in $builds) {
        $r = & $spindle check $b.name --profile $testProfile --json | ConvertFrom-Json
        $blocked = @($r.issues | Where-Object severity -eq "blocker").Count -gt 0
        $expect = (Get-Series $b.version) -lt 405
        Add-Result "compat check ($($b.name))" ($blocked -eq $expect) $(if ($blocked) { "blocker: $($r.issues[0].title)" } else { "$(@($r.issues).Count) issue(s), no blocker" })
    }
    $old = @($builds | Where-Object { (Get-Series $_.version) -lt 405 })[-1]
    if ($old) {
        $arg = @{ name = "launch_blender"; arguments = @{ build = $old.name; profile = $testProfile } } | ConvertTo-Json -Compress
        $resp = Invoke-Mcp @("{""jsonrpc"":""2.0"",""id"":3,""method"":""tools/call"",""params"":$arg}") | Where-Object id -eq 3
        $text = ($resp.result.content | ForEach-Object text) -join " "
        Add-Result "spindle-mcp refuses old build" ($resp.result.isError -and $text -match "not launched") $old.name
    } else {
        Add-Result "spindle-mcp refuses old build" $null "no build older than 4.5 installed"
    }

    # --- 7. removal (external build: unregistered, files kept) ---------------------
    $alias = "e2e-external-" + $testProfile.Substring(4)
    & $spindle builds add $alias $builds[0].executable | Out-Null
    $out = & $spindle builds remove $alias
    $gone = -not (& $spindle builds list --json | ConvertFrom-Json | Where-Object name -eq $alias)
    Add-Result "remove external build" ($gone -and (Test-Path $builds[0].executable) -and "$out" -match "unregistered") "$out"

    # --- 8. tray ----------------------------------------------------------------
    if (Get-Process spindle_app -ErrorAction SilentlyContinue) {
        Add-Result "tray" $null "Spindle is running; close it to test the tray"
    } else {
        Add-Type -Namespace E2E -Name U -MemberDefinition @'
[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowW(string cls, string title);
[DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
[DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
'@
        function Find-Tray { [E2E.U]::FindWindowW("SpindleTrayWindow", [NullString]::Value) }
        function Wait-Until([scriptblock]$cond, $secs = 10) {
            $t = [Diagnostics.Stopwatch]::StartNew()
            while ($t.Elapsed.TotalSeconds -lt $secs) { if (& $cond) { return $true }; Start-Sleep -Milliseconds 200 }
            return $false
        }
        $app = Start-Process $appExe -PassThru
        $null = Wait-Until { (Find-Tray) -ne [IntPtr]::Zero } 20
        $null = Wait-Until { $app.Refresh(); $app.MainWindowHandle -ne [IntPtr]::Zero } 20
        $main = $app.MainWindowHandle
        $second = Start-Process $appExe -PassThru
        Add-Result "tray: second start hands over" ($second.WaitForExit(8000) -and @(Get-Process spindle_app).Count -eq 1)
        [void][E2E.U]::PostMessageW($main, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)   # WM_CLOSE
        Add-Result "tray: close hides to tray" ((Wait-Until { -not [E2E.U]::IsWindowVisible($main) } 5) -and -not $app.HasExited)
        [void][E2E.U]::PostMessageW((Find-Tray), 0x8003, [IntPtr]::Zero, [IntPtr]::Zero)   # WM_SHOW_MAIN
        Add-Result "tray: shows the window again" (Wait-Until { [E2E.U]::IsWindowVisible($main) } 5)
        & $appExe --quit
        Add-Result "tray: --quit exits and removes the icon" ($app.WaitForExit(15000) -and (Find-Tray) -eq [IntPtr]::Zero)
    }

    # --- 9. favourites -------------------------------------------------------
    $before = & $spindle builds list --json | ConvertFrom-Json
    $pick = $before | Where-Object { -not $_.favourite } | Select-Object -Last 1
    if ($pick) {
        & $spindle builds favourite $pick.name | Out-Null
        $after = & $spindle builds list --json | ConvertFrom-Json
        $favs = @($before | Where-Object favourite)
        # Favourites keep their order; a newly starred build goes after the existing ones.
        Add-Result "favourite: listed with favourites first" (($after[$favs.Count].name -eq $pick.name) -and $after[$favs.Count].favourite) $pick.name
        $dry = (& $spindle builds list | Select-Object -First 1)
        Add-Result "favourite: marked in the list" ($dry -match '^\*' -or $favs.Count -gt 0) $dry
        & $spindle builds favourite $pick.name --remove | Out-Null
        $restored = & $spindle builds list --json | ConvertFrom-Json
        Add-Result "favourite: unstarred again" (-not ($restored | Where-Object { $_.name -eq $pick.name }).favourite)
    }

    # --- 10. desktop integration (registry round trip) ------------------------
    $status = & $spindle desktop --app $appExe
    if ($status -match ': on') {
        Add-Result "desktop: register + unregister" $null "skipped: Spindle is registered already ($($status -join '; '))"
    } else {
        $on = & $spindle desktop --app $appExe --autostart on --open-blend on
        $run = (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -ErrorAction SilentlyContinue).Spindle
        $open = (Get-ItemProperty 'HKCU:\Software\Classes\Spindle.blend\shell\open\command' -ErrorAction SilentlyContinue).'(default)'
        Add-Result "desktop: start at sign-in written" ($run -eq "`"$appExe`" --tray") $run
        Add-Result "desktop: .blend Open with written" ($open -eq "`"$appExe`" `"%1`"") $open
        $off = & $spindle desktop --app $appExe --autostart off --open-blend off
        $gone = -not (Test-Path 'HKCU:\Software\Classes\Spindle.blend') -and -not (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name Spindle -ErrorAction SilentlyContinue)
        Add-Result "desktop: removed again" ($gone -and ($off -join ' ') -notmatch ': on')
    }

    # --- 11. opening .blend files ---------------------------------------------
    $blendDir = Join-Path $env:TEMP "spindle-e2e-blends-$testProfile"
    $null = New-Item -ItemType Directory -Force $blendDir
    try {
        # The newest and the oldest build save a file each.
        $bySeries = $builds | Sort-Object { Get-Series $_.version }
        $savers = @($bySeries[-1], $bySeries[0]) | Sort-Object name -Unique
        foreach ($b in $savers) {
            $tag = Get-Series $b.version
            $plain = (Join-Path $blendDir "saved-$tag.blend").Replace('\', '/')
            $packed = (Join-Path $blendDir "saved-$tag-zstd.blend").Replace('\', '/')
            $null = Invoke-BlenderPython $b $testProfileDir "import bpy`nbpy.ops.wm.save_as_mainfile(filepath='$plain')`nbpy.ops.wm.save_as_mainfile(filepath='$packed', compress=True)"
            # A build the profile blocks (older than its preferences) hands the file to a newer one.
            $blocked = @((& $spindle check $b.name --profile $testProfile --json | ConvertFrom-Json).issues | Where-Object severity -eq 'blocker').Count -gt 0
            foreach ($f in @($plain, $packed)) {
                $out = & $spindle open $f --profile $testProfile --dry-run 2>&1
                $chosen = ([regex]::Match(($out -join "`n"), 'would open in (\S+)')).Groups[1].Value
                $series = Get-Series (($builds | Where-Object name -eq $chosen).version)
                $want = if ($blocked) { $series -gt (Get-Series $b.version) } else { $series -eq (Get-Series $b.version) }
                $what = if ($blocked) { "a newer build (its own is blocked by the profile)" } else { "build of its series" }
                Add-Result "open: $(Split-Path $f -Leaf) -> $what" $want $chosen
            }
        }
        if (-not $SkipLaunch) {
            # The app opens a file in the build that saved it and exits without a window.
            $f = Get-ChildItem $blendDir -Filter 'saved-*.blend' | Where-Object Name -notmatch 'zstd' | Select-Object -First 1
            $app = Start-Process $appExe -ArgumentList "`"$($f.FullName)`"" -PassThru
            $exited = $app.WaitForExit(20000)
            $deadline = (Get-Date).AddSeconds($HeartbeatTimeout)
            do {
                Start-Sleep -Seconds 2
                $inst = & $spindle instances --json | ConvertFrom-Json | Where-Object { $_.file -like "*$($f.Name)" }
            } while (-not $inst -and (Get-Date) -lt $deadline)
            Add-Result "open: spindle_app file.blend launches it" ($exited -and [bool]$inst) "Blender $($inst.blender_version), $($inst.file)"
            if ($inst) { Stop-Process -Id $inst.pid -Force -ErrorAction SilentlyContinue }
        }
    } finally {
        Remove-Item $blendDir -Recurse -Force -ErrorAction SilentlyContinue
    }

    # --- 12. self-update ------------------------------------------------------
    $su = & $spindle self-update --check 2>&1
    if (($su -join ' ') -match 'no Spindle release published yet') {
        Add-Result "self-update: newest release readable" $null "skipped: no release yet"
    } else {
        Add-Result "self-update: newest release readable" (($su -join ' ') -match 'newest:\s+\d') ($su -join '; ')
    }
} finally {
    if ($testProfileDir -and (Test-Path $testProfileDir)) { Remove-Item $testProfileDir -Recurse -Force }
    Pop-Location
}

$results | Format-Table -AutoSize | Out-Host
$failed = @($results | Where-Object Result -eq "FAIL").Count
if ($failed) { Write-Host "$failed check(s) failed" -ForegroundColor Red; exit 1 }
Write-Host "all checks passed" -ForegroundColor Green
exit 0
