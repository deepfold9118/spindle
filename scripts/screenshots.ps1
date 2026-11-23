<#
.SYNOPSIS
  Regenerates the README screenshots (docs/screenshots) from a made-up demo library.

.DESCRIPTION
  Builds a demo library with placeholder build folders, three profiles, a few free add-ons
  from extensions.blender.org and GitHub, a running instance and the public download listing, points
  Spindle at it with SPINDLE_CONFIG, and saves `spindle_app --screenshot` pictures. Nothing
  personal (paths, purchases, tokens) ends up in the pictures. The demo library is deleted
  afterwards; the real config and library are never touched.

.EXAMPLE
  pwsh scripts/screenshots.ps1
#>
param(
    # Shown in the pictures (build paths), so keep it short and neutral.
    [string]$Library = "$env:SystemDrive\Spindle",
    # A cached download listing to show (public data); default: fetch a fresh one.
    [string]$Catalog = ""
)

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$app = Join-Path $root "target\release\spindle_app.exe"
$cli = Join-Path $root "target\release\spindle.exe"
$out = Join-Path $root "docs\screenshots"

if (Test-Path $Library) { throw "$Library exists; pass -Library <empty folder>" }
cargo build --release -q -p spindle_app -p spindle-launcher
if ($LASTEXITCODE) { throw "cargo build failed" }

$cfgDir = Join-Path $Library "config"
$env:SPINDLE_CONFIG = Join-Path $cfgDir "config.toml"
function Write-Json($path, $obj) {
    $null = New-Item -ItemType Directory -Force (Split-Path $path)
    $obj | ConvertTo-Json -Depth 6 | Set-Content -Encoding utf8NoBOM $path
}
$now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()

try {
    # --- builds: placeholder folders with the files Spindle reads -------------------------
    $builds = @(
        @{ name = "blender-5.3.0-stable.02f0899d78e3"; channel = "stable"; version = "5.3.0"; cycle = "stable"; branch = "v53"; hash = "02f0899d78e3"; py = 313; age = 9 },
        @{ name = "blender-5.4.0-alpha+main.3c1f2b9a7d10"; channel = "daily"; version = "5.4.0"; cycle = "alpha"; branch = "main"; hash = "3c1f2b9a7d10"; py = 313; age = 1 },
        @{ name = "blender-5.2.0-lts.fbe6228777e7"; channel = "lts"; version = "5.2.0"; cycle = "lts"; branch = "v52"; hash = "fbe6228777e7"; py = 313; age = 60 },
        @{ name = "blender-5.3.1-candidate+main-PR151234.8e2d4a6c1b90"; channel = "experimental"; version = "5.3.1"; cycle = "candidate"; branch = "main-PR151234"; hash = "8e2d4a6c1b90"; py = 313; age = 2 },
        @{ name = "blender-4.5.16-lts"; channel = "lts"; version = "4.5.16"; cycle = "lts"; branch = "4.5"; hash = ""; py = 311; age = 30 },
        @{ name = "blender-4.1.1-stable"; channel = "stable"; version = "4.1.1"; cycle = "stable"; branch = "4.1"; hash = ""; py = 311; age = 400 }
    )
    foreach ($b in $builds) {
        $dir = Join-Path $Library "builds\$($b.name)"
        $series = ($b.version -split '\.')[0..1] -join '.'
        $null = New-Item -ItemType Directory -Force (Join-Path $dir $series)
        Set-Content (Join-Path $dir "blender.exe") ""
        Set-Content (Join-Path $dir "python$($b.py).dll") ""
        Write-Json (Join-Path $dir "spindle-build.json") ([ordered]@{
            channel = $b.channel; version = $b.version; cycle = $b.cycle; branch = $b.branch; hash = $b.hash
            url = ""; published = $now - $b.age * 86400; installed = $now - $b.age * 86400
        })
    }

    # --- profiles: preferences saved by 5.2 (the 5.0+ file format), two extensions enabled --
    foreach ($p in "default", "studio", "teaching") {
        $null = New-Item -ItemType Directory -Force (Join-Path $Library "profiles\$p\config")
    }
    $prof = Join-Path $Library "profiles\default"
    $exts = @(
        @{ id = "looptools"; name = "LoopTools"; min = "4.2.0"; max = $null },
        @{ id = "node_arrange"; name = "Node Arrange"; min = "4.2.0"; max = $null }
    )
    $modules = foreach ($e in $exts) {
        $d = Join-Path $prof "extensions\user_default\$($e.id)"
        $null = New-Item -ItemType Directory -Force $d
        $toml = "schema_version = `"1.0.0`"`nid = `"$($e.id)`"`nname = `"$($e.name)`"`nversion = `"1.0.0`"`ntype = `"add-on`"`nblender_version_min = `"$($e.min)`"`n"
        if ($e.max) { $toml += "blender_version_max = `"$($e.max)`"`n" }
        Set-Content -Encoding utf8NoBOM (Join-Path $d "blender_manifest.toml") $toml
        "bl_ext.user_default.$($e.id)"
    }
    $bytes = [Text.Encoding]::ASCII.GetBytes("BLENDER17-01v0502" + ("`0" * 15) + (($modules | ForEach-Object { "$_`0" }) -join ""))
    [IO.File]::WriteAllBytes((Join-Path $prof "config\userpref.blend"), $bytes)

    # --- add-on library: free extensions from extensions.blender.org --------------------------
    $addons = @(
        @{ id = "looptools"; name = "LoopTools"; version = "4.7.7" },
        @{ id = "bool_tool"; name = "Bool Tool"; version = "1.1.2" },
        @{ id = "extra_mesh_objects"; name = "Extra Mesh Objects"; version = "0.3.11" },
        @{ id = "copy_attributes_menu"; name = "Copy Attributes Menu"; version = "0.6.5" },
        @{ id = "node_arrange"; name = "Node Arrange"; version = "1.4.0" }
    )
    foreach ($a in $addons) {
        $d = Join-Path $Library "addons\blender_org\$($a.id)\$($a.version)"
        $zip = "$($a.id)-$($a.version).zip"
        $null = New-Item -ItemType Directory -Force $d
        Set-Content (Join-Path $d $zip) ""
        Write-Json (Join-Path $d "addon.json") ([ordered]@{
            id = $a.id; name = $a.name; version = $a.version; kind = "add-on"; source = "blender_org"; archive = $zip
            sha256 = ("0" * 64); blender_version_min = "4.2.0"; blender_version_max = $null; platforms = @(); fetched = $now - 86400
        })
    }
    # GitHub: one from a release asset, one Spindle built from the source (no releases).
    $github = @(
        @{ id = "dynamic_parent"; name = "Dynamic Parent"; version = "2.1.0"; dir = "2.1.0"; format = "legacy"; zip = "dynamic_parent_v2.1.0.zip"; min = "5.0.0"
            origin = [ordered]@{ url = "https://github.com/romanvolodin/dynamic_parent"; repo = "romanvolodin/dynamic_parent"; git_ref = "v2.1.0"; commit = ""; path = ""; built = $false } },
        @{ id = "matalogue"; name = "Matalogue"; version = "1.5.1+b4af476"; dir = "1.5.1_b4af476"; format = "extension"; zip = "matalogue-1.5.1.zip"; min = "4.2.0"
            origin = [ordered]@{ url = "https://github.com/gregzaal/Matalogue"; repo = "gregzaal/Matalogue"; git_ref = "master"; commit = "b4af476e85bed94d3749a930e9c663cf2bd25ae7"; path = ""; built = $true } }
    )
    foreach ($a in $github) {
        $d = Join-Path $Library "addons\github\$($a.id)\$($a.dir)"
        $null = New-Item -ItemType Directory -Force $d
        Set-Content (Join-Path $d $a.zip) ""
        Write-Json (Join-Path $d "addon.json") ([ordered]@{
            id = $a.id; name = $a.name; version = $a.version; kind = "add-on"; source = "github"; archive = $a.zip
            sha256 = ("0" * 64); blender_version_min = $a.min; blender_version_max = $null; platforms = @(); fetched = $now - 3600
            format = $a.format; origin = $a.origin
        })
    }

    # --- config, UI prefs, a running Blender and the download listing -------------------------
    $null = New-Item -ItemType Directory -Force $cfgDir
    Set-Content -Encoding utf8NoBOM $env:SPINDLE_CONFIG "library_dir = '$Library'`nfavourites = [`"blender-5.3.0-stable.02f0899d78e3`"]`n"
    Write-Json (Join-Path $Library "run\instances\24816.json") ([ordered]@{
        pid = 24816; blender_version = "5.3.0"; binary = (Join-Path $Library "builds\blender-5.3.0-stable.02f0899d78e3\blender.exe")
        profile = "studio"; file = "C:\Projects\robot-arm\robot_arm_v12.blend"; started = $now - 4000
        heartbeat = $now + 3600; mcp = @(@{ addon = "blender_mcp"; host = "127.0.0.1"; port = 9876 })
    })
    $runDir = Join-Path $Library "run"
    if ($Catalog) {
        Copy-Item $Catalog (Join-Path $runDir "available.json")
    } else {
        & $cli builds available --refresh | Out-Null
    }

    # --- preference backups: two by hand, and the automatic one a restore makes ------------------
    $first = & $cli profiles backup default --note "Before trying the new keymap"
    & $cli profiles backup default --full --note "Studio setup" | Out-Null
    $firstId = ([regex]::Match(($first -join ' '), "to (\S+) \(")).Groups[1].Value
    & $cli profiles restore $firstId | Out-Null

    # --- pictures -----------------------------------------------------------------------------
    $null = New-Item -ItemType Directory -Force $out
    $shots = @(
        @("builds", "dark", "builds.png", @()),
        @("builds", "light", "builds-light.png", @()),
        @("running", "dark", "running.png", @()),
        @("downloads", "dark", "downloads.png", @()),
        @("addons", "dark", "addons.png", @()),
        @("builds", "dark", "compatibility.png", @("--show-compat", "blender-4.1.1-stable")),
        @("profiles", "dark", "backups.png", @("--show-backups", "default")),
        # Looks the repository up on GitHub (it has no releases, so the build offer shows).
        @("addons", "dark", "github.png", @("--show-github", "https://github.com/Radivarig/UvSquares"))
    )
    foreach ($s in $shots) {
        $png = Join-Path $out $s[2]
        & $app --screenshot $png --view $s[0] --theme $s[1] @($s[3]) | Out-Null
        if (-not (Test-Path $png)) { throw "no picture for $($s[2])" }
        Write-Host "saved $png"
    }
} finally {
    Remove-Item Env:\SPINDLE_CONFIG -ErrorAction SilentlyContinue
    if (Test-Path $Library) { Remove-Item $Library -Recurse -Force }
}
