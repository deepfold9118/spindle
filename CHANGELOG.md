# Changelog

## [0.1.4] - 2026-11-22

- **Add-ons from GitHub**: Add Add-on from GitHub… (Library menu, Task Panel, add-on library)
  and `spindle addons github <link>` add an add-on from a repository link. Spindle downloads it
  from the newest release (an asset for this platform, checked against GitHub's SHA-256). With
  no release to download, it offers to build one from the source: an extension with a library
  build's own `blender --command extension build`, a legacy add-on as a zip under a valid
  module name. Nothing from the repository is run. Links to a release, branch, tag, commit or
  folder work too, and Check GitHub for a Newer Version adds an add-on's link again.
- **Legacy add-ons** (`bl_info`) in the add-on library install with Blender's add-on installer
  into the profile's `scripts/addons`; extensions still go into `user_default`, and Spindle
  says so instead of trying when the chosen build is older than 4.2.
- Background Blender runs (add-on installs, extension builds) start without a console window
  on Windows.

## [0.1.3] - 2026-11-22

- **Preference backups**: back up a profile's preferences (`config/`: userpref.blend, the
  startup file, bookmarks, recent files) or the whole profile with its add-ons and extensions,
  and restore any backup later, from the Task Panel (Back Up Preferences, Back Up Whole Profile,
  Restore Preferences…) or `spindle profiles backup | backups | restore | remove-backup`.
  Restoring backs up what it replaces first, and waits until no Blender is running with the
  profile. A backup can also be restored into another profile.
- **Automatic backups**: before a build older than the Blender that saved a profile's
  preferences opens them, Spindle backs them up (the newest ten automatic backups per profile
  are kept). The compatibility warnings say so.
- Upgrading (the installer, or Help > Spindle Updates) asks Windows to refresh its icon cache,
  so the taskbar shows the new release's icon instead of a cached older one.

## [0.1.2] - 2026-11-22

- **New logo**: a simpler Spindle mark that stays readable at tray and taskbar sizes. The
  program icon, tray icon, title bar and README use it, and the app's accent colour follows
  the logo's new orange (#f28526).

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
