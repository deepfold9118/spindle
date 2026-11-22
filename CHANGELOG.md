# Changelog

## [0.1.1] - 2026-11-22

- **Installers**: `install.ps1` (Windows) and `install.sh` (Linux, macOS) install Spindle for the
  current user with one command: a SHA-256-checked download of the latest release, a Start menu
  or application-menu shortcut, PATH, and optionally start at sign-in and `.blend` opening.
  Running them again upgrades; `-Uninstall` / `--uninstall` removes the programs, shortcut,
  PATH entry and registrations and leaves your library alone. Each release carries both.
- **Add Spindle to PATH**: a new Customize Program switch, and `spindle desktop --path on|off`.
  Windows adds the folder to the user `Path`; Linux and macOS link the programs into
  `~/.local/bin`.
- Moving Spindle re-points start at sign-in, `.blend` opening and PATH, but a second copy (a
  development build, say) no longer takes them over from one that still exists.

## [0.1.0] - 2026-11-22

First release.

- **Builds**: download official Stable and LTS releases (download.blender.org) and Daily,
  experimental-branch and patch builds (builder.blender.org) on Windows, Linux and macOS,
  each checked against its published SHA-256. Remove builds to the Recycle Bin / Trash.
  Star favourites; the first one is the default build.
- **New-build alerts**: every six hours Spindle looks for newer builds of what's installed and
  announces each one once.
- **Shared profiles**: every build launches with the same preferences, add-ons and extensions
  (`BLENDER_USER_RESOURCES`, plus the older variables for builds before 3.6).
- **Profile compatibility checks**: before an older build opens a profile, Spindle says exactly
  what will break and why, and asks first when the build can't read the preferences at all.
- **.blend files**: open them with Spindle (Windows "Open with", Linux MIME type) and each file
  opens in the build of the Blender that saved it, read from its header (compressed files too).
- **Agents**: a bridge script in each profile reports every running Blender and its MCP add-on
  port; `spindle-mcp` serves that to agents, and can launch builds.
- **Superhive**: purchased extensions download into Spindle's add-on library; the API token
  lives in the OS credential store.
- **Desktop**: tray icon (Windows notification area, Linux StatusNotifierItem, macOS menu bar),
  start at sign-in, single instance on Windows, and self-update from GitHub releases.
