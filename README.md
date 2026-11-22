# Spindle

A build, profile and add-on manager for Blender, written in Rust.

- **Shared profiles**: every build is launched with `BLENDER_USER_RESOURCES` pointing at a
  Spindle profile, so all builds (old and newly downloaded) use the same preferences,
  add-ons and extensions. Nothing is copied per build, and a `portable/` folder can't hide them.
- **Running-instance discovery for agents**: a bridge script in each profile's
  `scripts/startup` writes a heartbeat for every running Blender, including the host and port
  of any enabled Blender MCP add-on. `spindle-mcp` serves that to agents over MCP.
- **Superhive as a download source**: purchased extensions are fetched from Superhive's
  Blender extensions (API v1) repository into Spindle's add-on library, verified by sha256.
  The API token lives in the OS credential store and is never written into Blender preferences.

## Layout

```
app/                     desktop UI (eframe/egui), built the same way and in the same ArtCraft
                         style as Fontabulous / Fontabulous Studio: custom title bar, library tree,
                         cards, Task Panel, status bar, Inter + Lucide, light/dark
crates/spindle-core      library logic (builds, profiles, launch, instances, add-ons, superhive)
crates/spindle-launcher  `spindle` CLI
crates/spindle-mcp       MCP server (stdio)
bridge/spindle_bridge.py heartbeat script, embedded into spindle-core and installed into profiles
```

Library (default: platform local data dir; set with `spindle set-library <dir>`):

```
builds/<name>/blender.exe
profiles/<name>/{config,scripts,extensions,...}    same layout as %APPDATA%/Blender Foundation/Blender/<ver>
addons/<source>/<id>/<version>/{<archive>.zip, addon.json}
run/instances/<pid>.json                          bridge heartbeats (stale after 20 s)
```

## Usage

Desktop app:

```
cargo run -p spindle_app
spindle_app --screenshot out.png [--view builds|running|addons|profiles] [--theme dark|light]
```

`--screenshot` draws the real UI against the library, saves the picture and quits (the
Fontabulous build check); `--view` / `--theme` apply to that run only.

CLI:

```
cargo build --release
spindle info
spindle builds list
spindle launch 5.3 [--profile default] [-- file.blend]
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
-> UI screenshots of every view and both themes (`checks/ui/`, overwritten each run) -> close, Superhive list/sync (when a token is stored), and installing a library add-on with one
build and seeing it enabled in another.

## Superhive notes

- Index: `GET https://superhivemarket.com/api/v1/extensions/` with `Authorization: Bearer <token>`
  returns a Blender extensions v1 listing of purchases.
- Archive downloads answer **403** to the Bearer header and only accept the token as a `token`
  query parameter. Spindle tries the header first and falls back to the query form; the URL with
  the token is never logged (reqwest errors are stripped of URLs).
- `GET /api/v1/orders` also works (orders with their order items) if richer metadata is needed.

## Not done yet

- Downloading builds from builder.blender.org / release mirrors (builds are currently copied in
  or registered with `spindle builds add`).
- Per-major-version profiles (a newer `userpref.blend` may not load cleanly in much older builds).
