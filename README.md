<p align="center">
  <img src="artwork/spindle_hoz_logo_lockup.svg" alt="Spindle" width="440">
</p>

<p align="center">A build, profile and add-on manager for Blender, written in Rust.</p>

---

- **Download Blender builds**: official Stable and LTS releases from download.blender.org,
  plus Daily, experimental-branch and patch (PR) builds from builder.blender.org. Every
  download is checked against its published SHA-256 before it is unpacked into the library.
- **New-build alerts**: Spindle checks for newer builds of what you have installed (a newer
  patch in the same series, a newer daily of the same branch, a new release series) every six
  hours and announces each one once with a notification.
- **Shared profiles**: every build is launched with `BLENDER_USER_RESOURCES` pointing at a
  Spindle profile, so all builds (old and newly downloaded) use the same preferences,
  add-ons and extensions. Nothing is copied per build, and a `portable/` folder can't hide them.
- **Profile compatibility checks**: before an older build is launched onto a profile, Spindle
  says exactly what will break and why (see below), and asks before going ahead.
- **Running-instance discovery for agents**: a bridge script in each profile's
  `scripts/startup` writes a heartbeat for every running Blender, including the host and port
  of any enabled Blender MCP add-on. `spindle-mcp` serves that to agents over MCP.
- **Superhive as a download source**: purchased extensions are fetched from Superhive's
  Blender extensions (API v1) repository into Spindle's add-on library, verified by SHA-256.
  The API token lives in the OS credential store and is never written into Blender preferences.
- **Stays in the tray**: closing the window keeps Spindle in the notification area to launch
  builds and watch for new ones. Starting Spindle again brings the running one forward.

## Profile compatibility

Every build shares one profile, so a build older than the Blender that last saved it can meet
files it doesn't understand. `spindle check <build>` (and the app, before launching) reports:

| Check | Why it matters |
|---|---|
| `userpref.blend` in the 5.0+ .blend format, build older than 4.5 | **Blocker.** Blender reports "not a blend file" and starts with factory preferences; if it then saves preferences, the shared profile is overwritten with defaults. |
| Preferences saved by a newer Blender | Settings added after the build's version are ignored, and dropped if it saves preferences. |
| `startup.blend` in the 5.0+ format, build older than 4.5 | The factory startup scene opens instead. |
| Enabled extensions on a build older than 4.2 | The extensions platform doesn't exist there; each one is named. |
| `blender_version_min` / `blender_version_max` of enabled extensions | Blender refuses to load them; each one is named with the version it needs. |
| Extension wheels built for another Python | Bundled packages fail to import (the build's Python is read from its `python3XX.dll`). |
| Legacy add-ons whose `bl_info["blender"]` is newer than the build | Blender flags them and they may fail to register. |

Builds older than 3.6 ignore `BLENDER_USER_RESOURCES` (verified with 2.93), so Spindle also sets
`BLENDER_USER_CONFIG`, `BLENDER_USER_SCRIPTS` and `BLENDER_USER_DATAFILES` for them.
`spindle launch` and the MCP `launch_blender` tool refuse a blocker unless forced
(`--force` / `"force": true`); warnings are printed or returned with the launch.

## Layout

```
app/                     desktop UI (eframe/egui), built the same way and in the same ArtCraft
                         style as Fontabulous / Fontabulous Studio: custom title bar, library tree,
                         cards, Task Panel, status bar, Inter + Lucide, light/dark; tray icon
artwork/                 logo lockups and the symbol (the app's icons are drawn from spindle_symbol.svg)
crates/spindle-core      library logic (builds, downloads, profiles, compatibility, launch,
                         instances, add-ons, superhive)
crates/spindle-launcher  `spindle` CLI
crates/spindle-mcp       MCP server (stdio)
bridge/spindle_bridge.py heartbeat script, embedded into spindle-core and installed into profiles
```

Library (default: platform local data dir; set with `spindle set-library <dir>`):

```
builds/<name>/blender.exe
builds/<name>/spindle-build.json                  where a downloaded build came from (channel, branch, commit)
profiles/<name>/{config,scripts,extensions,...}    same layout as %APPDATA%/Blender Foundation/Blender/<ver>
addons/<source>/<id>/<version>/{<archive>.zip, addon.json}
downloads/                                        partial build downloads
run/instances/<pid>.json                          bridge heartbeats (stale after 20 s)
run/available.json                                cached list of downloadable builds
```

## Usage

Desktop app:

```
cargo run -p spindle_app
spindle_app --tray                 # start hidden in the notification area
spindle_app --quit                 # ask a running Spindle to exit (installers, updaters)
spindle_app --screenshot out.png [--view builds|running|addons|profiles|downloads]
            [--theme dark|light] [--show-compat <build>]
```

`--screenshot` draws the real UI against the library, saves the picture and quits (the
Fontabulous build check); `--view`, `--theme` and `--show-compat` apply to that run only.
On Windows 11 a new tray icon starts in the overflow (^) area; turn it on under
Settings > Personalization > Taskbar > Other system tray icons.

CLI:

```
cargo build --release
spindle info
spindle builds list
spindle builds available [--channel lts|stable|daily|experimental] [--refresh] [--json]
spindle builds download 4.5        # newest 4.5 release; or an exact name from `available`
spindle builds updates
spindle builds remove <build>      # library builds go to the Recycle Bin; external ones are unregistered
spindle check <build> [--profile default] [--json]
spindle launch 5.3 [--profile default] [--force] [-- file.blend]
spindle instances [--json]
spindle superhive login            # prompts; or `--stdin`
spindle superhive list | sync
spindle addons list
spindle addons install <id> --build 5.3
```

Agent config (any MCP client):

```json
{ "mcpServers": { "spindle": { "command": "path/to/spindle-mcp" } } }
```

Tools: `list_running_blenders`, `list_builds`, `launch_blender`.

## Testing

```
cargo test                 # unit tests
pwsh scripts/e2e.ps1       # end-to-end against the real library (throwaway profile, deleted after)
```

The e2e script checks the profile redirect on every build, launch -> heartbeat -> `spindle-mcp`
-> UI screenshots of every view, both themes and the compatibility dialog (`checks/ui/`,
overwritten each run) -> close, Superhive list/sync (when a token is stored), installing a
library add-on with one build and seeing it enabled in another, the download listings for every
channel, the compatibility verdict for every build (and `spindle-mcp` refusing an old build),
removing an external build, and the tray (close to tray, show, single instance, `--quit`).
Downloading a whole build isn't repeated each run; `spindle builds download <version>` does it.

## Superhive notes

- Index: `GET https://superhivemarket.com/api/v1/extensions/` with `Authorization: Bearer <token>`
  returns a Blender extensions v1 listing of purchases.
- Archive downloads answer **403** to the Bearer header and only accept the token as a `token`
  query parameter. Spindle tries the header first and falls back to the query form; the URL with
  the token is never logged (reqwest errors are stripped of URLs).
- `GET /api/v1/orders` also works (orders with their order items) if richer metadata is needed.

## Not done yet

- Build downloads and the tray are Windows-only for now (the rest is cross-platform).
- Starting Spindle with Windows (`--tray` exists for it; there's no setting yet).
- Favourite builds, `.blend` file association, and Spindle updating itself.
- Licence, screenshots and GitHub releases, ahead of publishing.
