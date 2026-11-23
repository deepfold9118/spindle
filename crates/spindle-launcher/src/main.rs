use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use spindle_core::addons::AddonLibrary;
use spindle_core::profile::Profile;
use spindle_core::builds::Channel;
use spindle_core::downloads::{self, Available, Progress};
use spindle_core::{Spindle, backup, blendfile, builds, compat, desktop, github, instances, launch, superhive};
use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;

/// Spindle: a build, profile and add-on manager for Blender.
#[derive(Parser)]
#[command(name = "spindle", version)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show where Spindle keeps its config and library.
    Info,
    /// Set the library folder (builds, add-ons, profiles).
    SetLibrary { dir: PathBuf },
    #[command(subcommand)]
    Builds(BuildsCmd),
    /// List profiles, and back up or restore their preferences.
    Profiles {
        #[command(subcommand)]
        cmd: Option<ProfilesCmd>,
    },
    /// Launch a build with a shared profile.
    Launch {
        /// Build name, or a version like "5.3". Defaults to the first favourite, else the newest build.
        build: Option<String>,
        #[arg(long, short)]
        profile: Option<String>,
        /// Launch even when the profile can't be read by this build.
        #[arg(long)]
        force: bool,
        /// Extra Blender arguments (after `--`).
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Open a .blend file with the build that suits it (the series that saved it, else the
    /// default build, else the newest).
    Open {
        file: PathBuf,
        /// Use this build instead of choosing one.
        #[arg(long, short)]
        build: Option<String>,
        #[arg(long, short)]
        profile: Option<String>,
        /// Open even when the build can't use the profile, or the file is newer than every build.
        #[arg(long)]
        force: bool,
        /// Only say which build would open it.
        #[arg(long)]
        dry_run: bool,
    },
    /// Show or change desktop integration: starting Spindle at sign-in, opening .blend files.
    Desktop {
        /// Start the Spindle app in the tray when you sign in: on or off.
        #[arg(long, value_parser = parse_on_off)]
        autostart: Option<bool>,
        /// Offer Spindle for opening .blend files: on or off.
        #[arg(long, value_parser = parse_on_off)]
        open_blend: Option<bool>,
        /// Put Spindle's programs on PATH (Windows: user Path; elsewhere: ~/.local/bin): on or off.
        #[arg(long, value_parser = parse_on_off)]
        path: Option<bool>,
        /// The Spindle app executable to register (default: spindle_app next to this program).
        #[arg(long)]
        app: Option<PathBuf>,
    },
    /// Check whether a profile works with a build (older builds can't read everything).
    Check {
        build: String,
        #[arg(long, short)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// List Blenders launched by Spindle that are running right now.
    Instances {
        #[arg(long)]
        json: bool,
    },
    /// Update Spindle (these programs) to the newest GitHub release.
    SelfUpdate {
        /// Only say whether there's a newer release.
        #[arg(long)]
        check: bool,
        /// Reinstall the newest release even if it isn't newer.
        #[arg(long)]
        force: bool,
    },
    #[command(subcommand)]
    Addons(AddonsCmd),
    #[command(subcommand)]
    Superhive(SuperhiveCmd),
}

#[derive(Subcommand)]
enum BuildsCmd {
    /// List builds in the library and registered external builds.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Register a Blender executable that lives outside the library.
    Add { name: String, executable: PathBuf },
    /// List Blender builds available to download.
    Available {
        /// lts, stable, daily or experimental.
        #[arg(long, short)]
        channel: Option<String>,
        /// Fetch fresh listings instead of the cache (which refreshes after an hour anyway).
        #[arg(long)]
        refresh: bool,
        #[arg(long)]
        json: bool,
    },
    /// Download and install a build: an exact name from `available`, or a version like "4.5".
    Download {
        query: String,
        #[arg(long, short)]
        channel: Option<String>,
    },
    /// Show newer builds of what's installed.
    Updates,
    /// Remove a build: library builds go to the Recycle Bin, external ones are unregistered.
    Remove { build: String },
    /// Star a build: favourites are listed first, and the first one launches by default.
    Favourite {
        build: String,
        /// Unstar it instead.
        #[arg(long)]
        remove: bool,
    },
}

#[derive(Subcommand)]
enum ProfilesCmd {
    /// List profiles.
    List,
    /// Back up a profile's preferences (its config folder).
    Backup {
        /// Profile name. Defaults to the default profile.
        profile: Option<String>,
        /// Back up the whole profile, add-ons and extensions included.
        #[arg(long)]
        full: bool,
        /// A note to recognise the backup by.
        #[arg(long, default_value = "")]
        note: String,
    },
    /// List backups, newest first.
    Backups {
        profile: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Restore a backup (by id, or a profile name for its newest backup). What it replaces is
    /// backed up first.
    Restore {
        backup: String,
        /// Restore into another profile instead (created if needed).
        #[arg(long)]
        into: Option<String>,
        /// Restore even while Blender runs with the profile.
        #[arg(long)]
        force: bool,
    },
    /// Move a backup to the Recycle Bin / Trash.
    RemoveBackup { backup: String },
}

#[derive(Subcommand)]
enum AddonsCmd {
    /// List archives in the add-on library.
    List,
    /// Install the newest stored version of an add-on into a profile.
    Install {
        id: String,
        /// Build whose extension tooling performs the install.
        #[arg(long, short)]
        build: String,
        #[arg(long, short)]
        profile: Option<String>,
    },
    /// Add an add-on from GitHub: from the repository's newest release, or built from its
    /// source when no release has one (asks first, unless --build).
    Github {
        /// A repository link (https://github.com/owner/repo, or owner/repo), a release link
        /// (.../releases/tag/<tag>), or a branch and folder (.../tree/<ref>/<folder>).
        link: String,
        /// Build from the source without asking when there is no release to download.
        #[arg(long)]
        build: bool,
    },
}

#[derive(Subcommand)]
enum SuperhiveCmd {
    /// Store your Superhive API token in the OS credential store.
    Login {
        /// Read the token from stdin instead of prompting.
        #[arg(long)]
        stdin: bool,
    },
    /// Remove the stored token.
    Logout,
    /// Show purchased extensions available for this platform.
    List,
    /// Download purchased extensions that aren't in the add-on library yet.
    Sync,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut sp = Spindle::load()?;
    match cli.command {
        Cmd::Info => {
            println!("config:    {}", sp.config_path.display());
            println!("library:   {}", sp.library.display());
            println!("builds:    {}", sp.builds_dir().display());
            println!("addons:    {}", sp.addons_dir().display());
            println!("profiles:  {}", sp.profiles_dir().display());
            println!("instances: {}", sp.instances_dir().display());
            println!("superhive: {} (token {})", sp.config.superhive.repo_url,
                if superhive::has_token() { "stored" } else { "not set" });
        }
        Cmd::SetLibrary { dir } => {
            sp.config.library_dir = Some(dir.clone());
            sp.save()?;
            println!("library set to {}", dir.display());
        }
        Cmd::Builds(BuildsCmd::List { json: true }) => {
            println!("{}", serde_json_pretty(&builds::list(&sp)?)?);
        }
        Cmd::Builds(BuildsCmd::List { json: false }) => {
            for b in builds::list(&sp)? {
                println!("{} {:<10} {:<13} {}{}", if b.favourite { "*" } else { " " },
                    b.version.as_deref().unwrap_or("?"), b.channel.label(), b.name,
                    if b.external { format!("  ({})", b.executable.display()) } else { String::new() });
            }
        }
        Cmd::Builds(BuildsCmd::Add { name, executable }) => {
            builds::register_external(&mut sp, &name, &executable)?;
            println!("registered {name}");
        }
        Cmd::Builds(BuildsCmd::Available { channel, refresh, json }) => {
            let channel = channel.as_deref().map(parse_channel).transpose()?;
            let cat = catalog(&sp, refresh)?;
            let list: Vec<&Available> = cat.builds.iter().filter(|a| channel.is_none_or(|c| a.channel == c)).collect();
            if json {
                println!("{}", serde_json_pretty(&list)?);
            } else {
                let installed: Vec<String> = builds::list(&sp)?.into_iter().map(|b| b.name).collect();
                for a in &list {
                    println!("{:<13} {:<9} {:<10} {:>7}  {}{}", a.channel.label(), a.version, a.cycle,
                        a.size.map(|s| format!("{} MB", s / 1_000_000)).unwrap_or_default(), a.name,
                        if installed.contains(&a.name) { "  (installed)" } else { "" });
                }
            }
            for e in &cat.errors {
                eprintln!("warning: {e}");
            }
        }
        Cmd::Builds(BuildsCmd::Download { query, channel }) => {
            let channel = channel.as_deref().map(parse_channel).transpose()?;
            let a = pick(&catalog(&sp, false)?, &query, channel)?;
            eprintln!("downloading {} ({})", a.name, a.url);
            let build = install_with_progress(&sp, &a)?;
            println!("installed {} at {}", build.name, build.root().display());
        }
        Cmd::Builds(BuildsCmd::Updates) => {
            let ups = downloads::updates(&builds::list(&sp)?, &catalog(&sp, false)?);
            if ups.is_empty() {
                println!("everything is up to date");
            }
            for u in ups {
                let from = u.installed.unwrap_or_else(|| format!("new {} release", u.available.channel.label()));
                println!("{from:<40} -> {}", u.available.name);
            }
        }
        Cmd::Builds(BuildsCmd::Favourite { build, remove }) => {
            let b = builds::find(&sp, &build)?;
            builds::set_favourite(&mut sp, &b.name, !remove)?;
            println!("{} {}", if remove { "unstarred" } else { "starred" }, b.name);
        }
        Cmd::Builds(BuildsCmd::Remove { build }) => {
            let b = builds::find(&sp, &build)?;
            match builds::remove(&mut sp, &b)? {
                builds::Removed::Trashed => println!("moved {} to the Recycle Bin", b.name),
                builds::Removed::Unregistered => println!("unregistered {} (its files were left alone)", b.name),
            }
        }
        Cmd::Check { build, profile, json } => {
            let b = builds::find(&sp, &build)?;
            let p = Profile::open(&sp, &sp.profile_name(profile.as_deref()))?;
            let report = compat::check(&b, &p.name, &p.root);
            if json {
                println!("{}", serde_json_pretty(&report)?);
            } else {
                print_report(&report);
            }
        }
        Cmd::Profiles { cmd } => match cmd.unwrap_or(ProfilesCmd::List) {
            ProfilesCmd::List => {
                for p in Profile::list(&sp)? {
                    println!("{p}");
                }
            }
            ProfilesCmd::Backup { profile, full, note } => {
                let name = sp.profile_name(profile.as_deref());
                if !Profile::list(&sp)?.contains(&name) {
                    bail!("no profile '{name}'");
                }
                let profile = Profile::open(&sp, &name)?;
                let b = backup::create(&sp, &profile, full, backup::Kind::Manual, &note)?;
                println!("backed up {} of '{}' to {} ({})", if full { "the whole profile" } else { "the preferences" }, name, b.id, size(b.size));
            }
            ProfilesCmd::Backups { profile, json } => {
                let list = backup::list(&sp, profile.as_deref())?;
                if json {
                    println!("{}", serde_json_pretty(&list)?);
                } else if list.is_empty() {
                    println!("no backups yet (spindle profiles backup <profile>)");
                } else {
                    for b in list {
                        let what = if b.meta.full { "full" } else { "prefs" };
                        let kind = if b.meta.kind == backup::Kind::Automatic { "auto" } else { "" };
                        let by = b.meta.saved_by.as_deref().map(|v| format!("Blender {v}")).unwrap_or_default();
                        println!("{:<34} {}  {what:<5} {kind:<4} {:>9}  {by:<12} {}", b.id, b.when(), size(b.size), b.meta.note);
                    }
                }
            }
            ProfilesCmd::Restore { backup: query, into, force } => {
                let b = backup::find(&sp, &query)?;
                let undo = backup::restore(&sp, &b, into.as_deref(), force)?;
                let target = into.as_deref().unwrap_or(&b.meta.profile);
                println!("restored {} into profile '{target}'", b.id);
                println!("what it replaced is in backup {} (spindle profiles restore {} to undo)", undo.id, undo.id);
            }
            ProfilesCmd::RemoveBackup { backup: query } => {
                let b = backup::find(&sp, &query)?;
                if b.id != query {
                    bail!("give the backup's id to remove it (the newest of '{query}' is {})", b.id);
                }
                backup::remove(&b)?;
                println!("moved {} to the Recycle Bin", b.id);
            }
        },
        Cmd::Launch { build, profile, force, args } => {
            let build = match build {
                Some(q) => builds::find(&sp, &q)?,
                None => builds::default_build(&sp)?,
            };
            let profile = Profile::open(&sp, &sp.profile_name(profile.as_deref()))?;
            let report = compat::check(&build, &profile.name, &profile.root);
            if report.needs_attention() {
                print_report(&report);
                if report.worst() == Some(compat::Severity::Blocker) && !force {
                    bail!("not launching: this build can't use the profile safely (pass --force to launch anyway)");
                }
            }
            let pid = launch::launch(&sp, &build, &profile, &args)?;
            println!("launched {} with profile '{}' (pid {pid})", build.name, profile.name);
        }
        Cmd::Open { file, build, profile, force, dry_run } => {
            if !file.is_file() {
                bail!("{} not found", file.display());
            }
            let file = std::path::absolute(&file)?;
            let profile = Profile::open(&sp, &sp.profile_name(profile.as_deref()))?;
            let (build, warning) = match build {
                Some(q) => (builds::find(&sp, &q)?, None),
                None => {
                    let c = blendfile::choose_build(&sp, &file, |b| profile_usable(b, &profile))?;
                    if let Some(s) = c.saved_by {
                        println!("saved by Blender {}.{}", s.0, s.1);
                    }
                    (c.build, c.warning)
                }
            };
            if let Some(w) = &warning {
                println!("warning: {w}");
            }
            let report = compat::check(&build, &profile.name, &profile.root);
            if report.needs_attention() {
                print_report(&report);
            }
            if dry_run {
                println!("would open in {} with profile '{}'", build.name, profile.name);
                return Ok(());
            }
            let blocked = report.worst() == Some(compat::Severity::Blocker) || warning.is_some();
            if blocked && !force {
                bail!("not opening: see above (pass --force to open anyway)");
            }
            let pid = launch::launch(&sp, &build, &profile, &[file.to_string_lossy().into_owned()])?;
            println!("opened {} in {} with profile '{}' (pid {pid})", file.display(), build.name, profile.name);
        }
        Cmd::Desktop { autostart, open_blend, path, app } => {
            let app = match app {
                Some(a) if a.is_file() => std::path::absolute(&a)?,
                Some(a) => bail!("{} not found", a.display()),
                None => sibling_app()?,
            };
            if let Some(on) = autostart {
                desktop::set_autostart(on, &app)?;
            }
            if let Some(on) = open_blend {
                desktop::set_blend_association(on, &app)?;
            }
            if let Some(on) = path {
                desktop::set_on_path(on, app.parent().context("no folder for the app")?)?;
            }
            let show = |p: Option<PathBuf>| p.map(|p| format!("on ({})", p.display())).unwrap_or("off".into());
            println!("start at sign-in:  {}", show(desktop::autostart()));
            println!("open .blend files: {}", show(desktop::blend_association()));
            println!("on PATH:           {}", show(desktop::path_dir()));
            #[cfg(not(windows))]
            if let Some((bin, false)) = desktop::user_bin().filter(|_| desktop::path_dir().is_some()) {
                println!("note: {} isn't on your PATH yet; add it in your shell profile", bin.display());
            }
        }
        Cmd::SelfUpdate { check, force } => {
            use spindle_core::selfupdate;
            let r = selfupdate::latest()?;
            selfupdate::store(&sp, &r);
            let newer = selfupdate::is_newer(&r);
            println!("this version: {}", selfupdate::CURRENT);
            println!("newest:       {} ({})", r.version, r.url);
            if check || (!newer && !force) {
                if !newer {
                    println!("up to date");
                }
                return Ok(());
            }
            let dir = selfupdate::install_dir()?;
            let mut last = 0;
            let files = selfupdate::install(&sp, &r, &dir, |done, total| {
                let pct = if total > 0 { done * 100 / total } else { 0 };
                if pct >= last + 10 {
                    last = pct;
                    eprint!("\rdownloading {pct}%");
                }
            })?;
            eprintln!();
            for f in &files {
                println!("updated {}", f.display());
            }
            println!("Spindle {} installed; restart the app (and MCP clients) to use it", r.version);
        }
        Cmd::Instances { json } => {
            let list = instances::list(&sp)?;
            if json {
                println!("{}", serde_json_pretty(&list)?);
            } else if list.is_empty() {
                println!("no running Blender launched by Spindle");
            } else {
                for i in list {
                    let mcp: Vec<String> = i.mcp.iter()
                        .map(|m| format!("{}@{}:{}", m.addon, m.host.as_deref().unwrap_or("?"),
                            m.port.map(|p| p.to_string()).unwrap_or("?".into())))
                        .collect();
                    println!("pid {:<7} Blender {:<8} profile {:<10} file {}  mcp [{}]",
                        i.pid, i.blender_version, i.profile.as_deref().unwrap_or("-"),
                        if i.file.is_empty() { "(unsaved)" } else { &i.file }, mcp.join(", "));
                }
            }
        }
        Cmd::Addons(AddonsCmd::List) => {
            let lib = AddonLibrary::new(&sp);
            for r in lib.list()? {
                let from = r.origin.as_ref().map(|o| format!("  {}", o.url)).unwrap_or_default();
                println!("{:<28} {:<12} {:<8} {:<10} {}{from}", r.id, r.version, r.kind, r.source, r.archive);
            }
        }
        Cmd::Addons(AddonsCmd::Install { id, build, profile }) => {
            let lib = AddonLibrary::new(&sp);
            let rec = lib.latest(&id)?.with_context(|| format!("'{id}' is not in the add-on library"))?;
            let build = builds::find(&sp, &build)?;
            let profile = Profile::open(&sp, &sp.profile_name(profile.as_deref()))?;
            lib.install(&sp, &build, &profile, &rec)?;
            println!("installed {} {} into profile '{}'", rec.id, rec.version, profile.name);
        }
        Cmd::Addons(AddonsCmd::Github { link, build }) => {
            let lib = AddonLibrary::new(&sp);
            let link = github::parse_link(&link)?;
            let mut added = github::add(&sp, &lib, &link)?;
            if let github::Added::Offer(offer) = added.clone() {
                println!("{}.", offer.reason);
                println!("Spindle can download {} of {} and build the add-on from it.", offer.describe(), offer.repo);
                let go = build || (std::io::stdin().is_terminal() && confirm("Download the source and build it? [y/N] ")?);
                if !go {
                    bail!("nothing added; run again with --build to build it from the source");
                }
                added = github::build(&sp, &lib, &offer)?;
            }
            match added {
                github::Added::Stored(r) => {
                    let from = match &r.origin {
                        Some(o) if o.built => format!("built from {} ({})", o.git_ref, &o.commit[..o.commit.len().min(7)]),
                        Some(o) => format!("from release {}", o.git_ref),
                        None => String::new(),
                    };
                    println!("added {} {} to the add-on library, {from}", r.id, r.version);
                    println!("install it with: spindle addons install {} --build <build>", r.id);
                }
                github::Added::Present(r) => println!("{} {} is already in the add-on library", r.id, r.version),
                github::Added::Offer(_) => bail!("nothing to download"),
            }
        }
        Cmd::Superhive(SuperhiveCmd::Login { stdin }) => {
            let token = if stdin {
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s)?;
                s
            } else {
                rpassword::prompt_password("Superhive API token: ")?
            };
            superhive::set_token(&token)?;
            println!("token stored in the OS credential store");
        }
        Cmd::Superhive(SuperhiveCmd::Logout) => {
            superhive::clear_token()?;
            println!("token removed");
        }
        Cmd::Superhive(SuperhiveCmd::List) => {
            let lib = AddonLibrary::new(&sp);
            for e in superhive::fetch_index(&sp)? {
                let have = lib.contains(superhive::SOURCE, &e.id, &e.version);
                println!("{:<28} {:<12} {:<8} {}", e.id, e.version, e.kind, if have { "in library" } else { "new" });
            }
        }
        Cmd::Superhive(SuperhiveCmd::Sync) => {
            let report = superhive::sync(&sp, &AddonLibrary::new(&sp))?;
            for r in &report.downloaded {
                println!("downloaded  {} {}", r.id, r.version);
            }
            for s in &report.up_to_date {
                println!("up to date  {s}");
            }
            for (what, err) in &report.failed {
                println!("FAILED      {what}: {err}");
            }
            if !report.failed.is_empty() {
                bail!("{} download(s) failed", report.failed.len());
            }
        }
    }
    Ok(())
}

fn parse_channel(s: &str) -> Result<Channel> {
    Ok(match s.to_lowercase().as_str() {
        "lts" => Channel::Lts,
        "stable" | "release" => Channel::Stable,
        "daily" => Channel::Daily,
        "experimental" | "patch" | "branch" => Channel::Experimental,
        other => bail!("unknown channel '{other}' (lts, stable, daily or experimental)"),
    })
}

/// The cached catalog when it's under an hour old, else a fresh fetch.
fn catalog(sp: &Spindle, refresh: bool) -> Result<downloads::Catalog> {
    match downloads::cached(sp) {
        Some(c) if !refresh && c.age_secs() < 3600 => Ok(c),
        _ => downloads::fetch(sp),
    }
}

/// Exact name, else the newest build whose version matches (releases preferred).
/// The build can use the profile (no blocker such as unreadable preferences).
fn profile_usable(b: &builds::Build, profile: &Profile) -> bool {
    compat::check(b, &profile.name, &profile.root).worst() != Some(compat::Severity::Blocker)
}

fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes"))
}

fn parse_on_off(s: &str) -> Result<bool, String> {
    match s.to_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Ok(true),
        "off" | "false" | "no" | "0" => Ok(false),
        _ => Err("expected on or off".into()),
    }
}

/// The Spindle app shipped next to this CLI.
fn sibling_app() -> Result<PathBuf> {
    let me = std::env::current_exe()?;
    let app = me.with_file_name(format!("spindle_app{}", std::env::consts::EXE_SUFFIX));
    if !app.is_file() {
        bail!("{} not found; pass --app <path to spindle_app>", app.display());
    }
    Ok(app)
}

fn pick(cat: &downloads::Catalog, query: &str, channel: Option<Channel>) -> Result<Available> {
    let pool: Vec<&Available> = cat.builds.iter().filter(|a| channel.is_none_or(|c| a.channel == c)).collect();
    if let Some(a) = pool.iter().find(|a| a.name == query) {
        return Ok((*a).clone());
    }
    let matching: Vec<&Available> = pool
        .into_iter()
        .filter(|a| a.version == query || a.version.starts_with(&format!("{query}.")))
        .collect();
    matching
        .iter()
        .find(|a| matches!(a.channel, Channel::Stable | Channel::Lts))
        .or(matching.first())
        .map(|a| (*a).clone())
        .with_context(|| format!("nothing available matches '{query}' (see `spindle builds available`)"))
}

fn install_with_progress(sp: &Spindle, a: &Available) -> Result<builds::Build> {
    let progress = Progress::default();
    std::thread::scope(|s| {
        let job = s.spawn(|| downloads::install(sp, a, &progress));
        let mut last = String::new();
        while !job.is_finished() {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let line = match (progress.phase(), progress.fraction()) {
                (downloads::Phase::Downloading, Some(f)) => format!("downloading {:>3.0}%", f * 100.0),
                (downloads::Phase::Downloading, None) => "downloading".into(),
                (downloads::Phase::Verifying, _) => "verifying SHA-256".into(),
                (downloads::Phase::Extracting, f) => format!("unpacking {:>3.0}%", f.unwrap_or(0.0) * 100.0),
            };
            // Print every 10% step, not every tick.
            let step = line.trim_end_matches('%').rsplit(' ').next().and_then(|n| n.trim().parse::<u32>().ok());
            let key = match step {
                Some(n) => format!("{} {}", line.split(' ').next().unwrap_or(""), n / 10),
                None => line.clone(),
            };
            if key != last {
                eprintln!("{line}");
                last = key;
            }
        }
        job.join().map_err(|_| anyhow::anyhow!("install thread panicked"))?
    })
}

fn print_report(r: &compat::Report) {
    println!("profile '{}' with {} (Blender {}):", r.profile, r.build, r.build_series.as_deref().unwrap_or("?"));
    if r.issues.is_empty() {
        println!("  compatible");
    }
    for i in &r.issues {
        let tag = match i.severity {
            compat::Severity::Blocker => "BLOCKER",
            compat::Severity::Warning => "WARNING",
            compat::Severity::Info => "info",
        };
        println!("  [{tag}] {}\n      {}", i.title, i.detail);
    }
}

fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b => format!("{} KB", b.div_ceil(1024)),
    }
}

fn serde_json_pretty<T: serde::Serialize>(v: &T) -> Result<String> {
    Ok(serde_json::to_string_pretty(v)?)
}
