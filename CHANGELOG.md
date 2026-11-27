# Changelog

## [0.1.9] - 2026-11-26

- **A file icon for .blend files**: on Windows, `.blend` files that open with Spindle show the
  Spindle blend-file icon (a page with the Blender logo and the Spindle mark) in place of the
  Spindle program icon. An existing registration switches to it when 0.1.9 starts.

## [0.1.8] - 2026-11-23

- **Add add-ons from a file**: Add Add-on from File… (Library menu, Task Panel, empty add-on
  library), dropping `.zip` files on the window, and `spindle addons add <zip>...` add add-on
  archives to the library under From File. This covers store purchases the store's API doesn't
  list: Superhive's API lists only extensions, so legacy add-ons such as MESHmachine never appear
  in Sync Superhive Purchases. Nothing in the archive is run; adding a version the library
  already has does nothing.
- **Open with Spindle on the right-click menu**: Open .blend Files with Spindle (and the installer's
  `-OpenBlendFiles`) now also adds Open with Spindle to the right-click menu of `.blend` files on
  Windows, whichever app opens them by default (Windows 11 lists it under Show more options).
  An existing registration gains it when 0.1.8 starts.
- Installing an add-on no longer runs it in the background Blender: it's switched on in the saved
  preferences and starts the next time Blender opens with the profile. Some add-ons crashed the
  background Blender while registering (MESHmachine 0.20.1 in Blender 5.3), which failed the
  install.
- The add-on library and `spindle addons list` no longer show an add-on as "off" when a profile
  has a switched-off copy of it in another repository next to the copy it uses.

## [0.1.7] - 2026-11-23

- **Add-ons install into profiles, not builds**: the install dialog asks only for the profile.
  Spindle runs Blender's installer with a library build that can install the add-on (4.2+ for
  an extension, inside the add-on's own Blender range), preferring the series that last saved
  the profile's preferences so they keep their format. `spindle addons install <id>` no longer
  needs `--build` (it's still there to override the choice).
- **Where each add-on is installed**: the add-on library shows which profiles have each add-on
  ("Installed in default", with the version when it differs and "off" when it's switched off,
  or "Not installed in a profile"), and so do its Properties and `spindle addons list`.
- Installing into a profile is refused while a Blender is running with it: that Blender saves
  its preferences when it quits and would switch the add-on off again. The dialog says which
  Blender to close.

## [0.1.6] - 2026-11-22

- **Build downloads unpack again**: every download from Download Builds (and
  `spindle builds download`) stopped after verifying with "don't know how to unpack
  ….zip.part", because Spindle picked the unpacker from the name of the partly downloaded file
  instead of the published one. Official releases, Daily and experimental builds now unpack into
  the library as intended.

## [0.1.5] - 2026-11-22

- **Finding downloads**: with no builds in the library, the Builds page now leads with
  Download Builds… (the list of official releases, Daily and experimental builds from
  blender.org) and offers Add Build… for a build you already have below it. Before, it only
  offered Add Build….
- Download Builds says when the channel switches under Show hide listings ("Not shown: 8 Stable
  (switched off under Show)") and has a Show All button, so a switched-off channel no longer
  looks like missing releases.

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
