//! Application state: library data from spindle-core, UI preferences, background jobs
//! (Superhive sync, GitHub add-ons, add-on installs, build downloads), the update checker and
//! tray, and the
//! actions the menus, task panel and cards call.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use spindle_core::addons::{AddonLibrary, AddonRecord};
pub use spindle_core::builds::Channel;
use spindle_core::builds::{self, Build};
use spindle_core::selfupdate;
use spindle_core::compat::{self, Report};
use spindle_core::downloads::{self, Available, Catalog, Progress, Update};
use spindle_core::instances::{self, Instance};
use spindle_core::profile::Profile;
use spindle_core::backup::{self, Backup};
use spindle_core::github::{self, SourceOffer};
use spindle_core::{Spindle, launch, superhive};

use crate::theme::{self, Palette, ThemeMode};

/// What the centre area shows (chosen in the left tree).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum View {
    Builds,
    Running,
    Addons,
    Profiles,
    Downloads,
}

impl View {
    pub fn title(self) -> &'static str {
        match self {
            View::Downloads => "Download Builds",
            View::Builds => "Builds",
            View::Running => "Running Blenders",
            View::Addons => "Add-on Library",
            View::Profiles => "Profiles",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RightTab {
    Tasks,
    Properties,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub theme: ThemeMode,
    pub view: View,
    pub show_left: bool,
    pub show_right: bool,
    pub show_stable: bool,
    pub show_lts: bool,
    pub show_daily: bool,
    pub show_experimental: bool,
    /// Closing the window keeps Spindle in the notification area.
    pub close_to_tray: bool,
    /// The "still running in the tray" balloon was shown once.
    pub tray_hint_shown: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            theme: ThemeMode::Dark,
            view: View::Builds,
            show_left: true,
            show_right: true,
            show_stable: true,
            show_lts: true,
            show_daily: true,
            show_experimental: true,
            close_to_tray: true,
            tray_hint_shown: false,
        }
    }
}

/// Selection in the centre area.
#[derive(Clone, Debug, PartialEq)]
pub enum Selection {
    None,
    Build(String),
    Instance(u32),
    Addon { source: String, id: String, version: String },
    Profile(String),
    /// A build in the download catalog, by install name.
    Available(String),
}

pub struct Job {
    pub label: String,
    rx: Receiver<Result<String, String>>,
    /// Set for build downloads.
    pub progress: Option<Arc<Progress>>,
}

/// Profile compatibility warning shown before launching (or on request).
#[derive(Clone)]
pub struct CompatDialog {
    pub build: String,
    pub report: Report,
}

/// The profile builds launch with, shared with the tray thread.
static LAUNCH_PROFILE: Mutex<Option<String>> = Mutex::new(None);

pub fn launch_profile(sp: &Spindle) -> String {
    LAUNCH_PROFILE.lock().ok().and_then(|p| p.clone()).unwrap_or_else(|| sp.profile_name(None))
}

/// Launch a build without the window (from the tray). False when the app should handle it
/// instead: the profile check has something to say, or the launch failed.
#[cfg_attr(windows, allow(dead_code))]
pub fn launch_quietly(name: &str) -> bool {
    (|| -> anyhow::Result<bool> {
        let sp = Spindle::load()?;
        let build = builds::find(&sp, name)?;
        let profile = spindle_core::profile::Profile::open(&sp, &launch_profile(&sp))?;
        if compat::check(&build, &profile.name, &profile.root).needs_attention() {
            return Ok(false);
        }
        spindle_core::launch::launch(&sp, &build, &profile, &[])?;
        Ok(true)
    })()
    .unwrap_or(false)
}

/// Result of one update check, from the checker thread.
pub struct CheckResult {
    pub manual: bool,
    pub outcome: Result<usize, String>,
}

/// The Add from GitHub dialog: a link, then (when there is no release to download) the
/// offer to build from the source. It stays open while the lookup or build runs and shows
/// what went wrong in full, since the status bar has room for one line.
#[derive(Clone, Default)]
pub struct GithubDialog {
    pub link: String,
    /// A lookup or build is running.
    pub busy: bool,
    pub error: Option<String>,
    pub offer: Option<SourceOffer>,
    /// Focus the link field on the next frame.
    pub focus: bool,
}

/// What a GitHub job hands back to the dialog.
enum GithubResult {
    Offer(SourceOffer),
    Failed(String),
    Done,
}

#[derive(Default)]
pub struct Dialogs {
    pub superhive_login: bool,
    pub token_input: String,
    /// Install dialog: (add-on id, chosen build name, chosen profile).
    pub install: Option<(String, String, String)>,
    pub new_profile: Option<String>,
    pub about: bool,
    pub compat: Option<CompatDialog>,
    /// Remove-build confirmation, by build name.
    pub remove: Option<String>,
    /// "Spindle updates" dialog.
    pub self_update: bool,
    /// Backups dialog, by profile name.
    pub backups: Option<String>,
    /// Restore confirmation inside the backups dialog, by backup id.
    pub restore: Option<String>,
    pub github: Option<GithubDialog>,
}

pub struct App {
    pub pal: Palette,
    /// Logo masks rasterised at exact device-pixel sizes, tinted where drawn.
    marks: std::collections::HashMap<u32, egui::TextureHandle>,
    pub prefs: Prefs,
    pub prefs_dirty: bool,
    pub frame_no: u64,
    pub screenshot: Option<PathBuf>,
    pub shot_requested: bool,

    pub sp: Spindle,
    pub builds: Vec<Build>,
    pub instances: Vec<Instance>,
    pub addons: Vec<AddonRecord>,
    pub profiles: Vec<String>,
    /// Preference backups of every profile, newest first.
    pub backups: Vec<Backup>,
    pub token_stored: bool,
    pub load_error: Option<String>,

    pub selection: Selection,
    pub right_tab: RightTab,
    pub quick_find: String,
    pub active_profile: String,

    pub jobs: Vec<Job>,
    pub status: Option<(String, f64)>,
    pub dlg: Dialogs,
    last_poll: f64,

    /// Downloadable builds (cached in the library's run folder) and updates for what's installed.
    pub catalog: Option<Catalog>,
    pub updates: Vec<Update>,
    catalog_stamp: Option<std::time::SystemTime>,
    pub checking: bool,
    checker: Option<Sender<bool>>,
    check_rx: Option<Receiver<CheckResult>>,
    /// Compatibility of each build with the launch profile.
    pub compat: HashMap<String, Report>,
    compat_profile: String,

    pub tray: Option<crate::tray::Tray>,
    tray_rx: Option<Receiver<crate::tray::TrayEvent>>,
    pub main_hwnd: Option<isize>,
    pub quitting: bool,
    /// Desktop integration state (registry / autostart files), read on refresh.
    pub autostart: bool,
    pub opens_blend: bool,
    /// This copy's folder is on PATH.
    pub on_path: bool,
    /// A Spindle release newer than this one (from the daily check), and the restart flag
    /// an installed update sets.
    pub spindle_release: Option<spindle_core::selfupdate::Release>,
    restart_pending: Arc<std::sync::atomic::AtomicBool>,
    github_result: Arc<Mutex<Option<GithubResult>>>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, screenshot: Option<PathBuf>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        let sp = Spindle::load().unwrap_or_else(|_| Spindle {
            config_path: PathBuf::from("config.toml"),
            config: Default::default(),
            library: PathBuf::from("library"),
        });
        let prefs = load_prefs(&sp);
        theme::apply(&cc.egui_ctx, prefs.theme);
        let active_profile = sp.profile_name(None);
        let mut app = Self {
            pal: theme::palette(prefs.theme),
            marks: std::collections::HashMap::new(),
            prefs,
            prefs_dirty: false,
            frame_no: 0,
            screenshot,
            shot_requested: false,
            sp,
            builds: Vec::new(),
            instances: Vec::new(),
            addons: Vec::new(),
            profiles: Vec::new(),
            backups: Vec::new(),
            token_stored: false,
            load_error: None,
            selection: Selection::None,
            right_tab: RightTab::Tasks,
            quick_find: String::new(),
            active_profile,
            jobs: Vec::new(),
            status: None,
            dlg: Dialogs::default(),
            last_poll: f64::NEG_INFINITY,
            catalog: None,
            updates: Vec::new(),
            catalog_stamp: None,
            checking: false,
            checker: None,
            check_rx: None,
            compat: HashMap::new(),
            compat_profile: String::new(),
            tray: None,
            tray_rx: None,
            main_hwnd: window_handle(cc),
            quitting: false,
            autostart: false,
            opens_blend: false,
            on_path: false,
            spindle_release: None,
            restart_pending: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            github_result: Arc::new(Mutex::new(None)),
        };
        if app.screenshot.is_none() {
            app.start_background(&cc.egui_ctx);
            // Keep sign-in and .blend registrations pointing at this executable.
            if let Ok(exe) = std::env::current_exe() {
                spindle_core::desktop::refresh(&exe);
            }
            spindle_core::selfupdate::cleanup();
        }
        app.refresh();
        if let Some(b) = app.builds.first() {
            app.selection = Selection::Build(b.name.clone());
        }
        app
    }

    // ----------------------------------------------------------------------------------
    // Data

    /// Reload builds, add-ons, profiles and the token state from disk.
    pub fn refresh(&mut self) {
        match Spindle::load() {
            Ok(sp) => self.sp = sp,
            Err(e) => self.load_error = Some(format!("{e:#}")),
        }
        self.builds = builds::list(&self.sp).unwrap_or_default();
        self.addons = AddonLibrary::new(&self.sp).list().unwrap_or_default();
        self.profiles = Profile::list(&self.sp).unwrap_or_default();
        self.backups = backup::list(&self.sp, None).unwrap_or_default();
        if !self.profiles.contains(&self.active_profile) {
            self.active_profile = self.sp.profile_name(None);
        }
        self.token_stored = superhive::has_token();
        self.autostart = spindle_core::desktop::autostart().is_some();
        self.opens_blend = spindle_core::desktop::blend_association().is_some();
        self.on_path = own_dir().is_some_and(|d| spindle_core::desktop::is_on_path(&d));
        self.instances = instances::list(&self.sp).unwrap_or_default();
        self.reload_catalog(true);
        self.recheck_compat();
        if let Some(t) = &self.tray {
            t.set_builds(
                self.builds
                    .iter()
                    .map(|b| {
                        let star = if b.favourite { "\u{2605} " } else { "" };
                        (b.name.clone(), format!("{star}Blender {} ({})", b.version.as_deref().unwrap_or(&b.name), b.channel.label()))
                    })
                    .collect(),
            );
        }
    }

    /// Re-read the download catalog when the cache file changed (or `force`).
    fn reload_catalog(&mut self, force: bool) {
        let path = self.sp.run_dir().join("available.json");
        let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if force || stamp != self.catalog_stamp {
            self.catalog_stamp = stamp;
            self.catalog = downloads::cached(&self.sp);
        }
        self.updates = self.catalog.as_ref().map(|c| downloads::updates(&self.builds, c)).unwrap_or_default();
    }

    /// Check every build against the launch profile.
    fn recheck_compat(&mut self) {
        self.compat_profile = self.active_profile.clone();
        if let Ok(mut p) = LAUNCH_PROFILE.lock() {
            *p = Some(self.active_profile.clone());
        }
        let root = self.profile_dir(&self.active_profile);
        self.compat = self.builds.iter().map(|b| (b.name.clone(), compat::check(b, &self.active_profile, &root))).collect();
    }

    /// Tray icon and the update checker (not in screenshot runs).
    fn start_background(&mut self, ctx: &egui::Context) {
        // Windows hosts the tray on its own thread and needs the window handle to show and
        // hide the window; elsewhere the icon lives on this (main) thread.
        let main = if cfg!(windows) { self.main_hwnd } else { Some(0) };
        if let Some(hwnd) = main {
            let (tx, rx) = channel();
            self.tray = crate::tray::Tray::start(hwnd, ctx.clone(), tx);
            self.tray_rx = Some(rx);
        }
        let (trigger, trigger_rx) = channel::<bool>();
        let (result_tx, result_rx) = channel();
        let tray = self.tray.clone();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("spindle-updates".into())
            .spawn(move || update_checker(ctx, trigger_rx, result_tx, tray))
            .ok();
        self.checker = Some(trigger);
        self.check_rx = Some(result_rx);
    }

    /// Ask the checker thread for a fresh catalog now.
    pub fn check_updates(&mut self, ctx: &egui::Context, manual: bool) {
        if self.checking {
            return;
        }
        if let Some(t) = &self.checker {
            if t.send(manual).is_ok() {
                self.checking = true;
                if manual {
                    self.toast(ctx, "Checking blender.org for builds…");
                }
            }
        }
    }

    /// Per-frame housekeeping: poll running instances every 2 s and collect finished jobs.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if now - self.last_poll >= 2.0 {
            self.last_poll = now;
            self.instances = instances::list(&self.sp).unwrap_or_default();
            self.reload_catalog(false);
            self.spindle_release = selfupdate::cached(&self.sp).filter(selfupdate::is_newer);
            if self.restart_pending.load(std::sync::atomic::Ordering::Relaxed) {
                self.restart_after_update(ctx);
            }
            if let Some(t) = &self.tray {
                let mut tip = format!("Spindle · {} running", self.instances.len());
                if !self.updates.is_empty() {
                    tip += &format!(" · {} new build(s)", self.updates.len());
                }
                t.set_tooltip(&tip);
            }
        }
        if self.compat_profile != self.active_profile {
            self.recheck_compat();
        }
        if let Some(rx) = &self.check_rx {
            let results: Vec<CheckResult> = rx.try_iter().collect();
            for r in results {
                self.checking = false;
                self.reload_catalog(true);
                match r.outcome {
                    Ok(n) if r.manual => {
                        let ups = self.updates.len();
                        let msg = if ups == 0 {
                            format!("{n} builds available · everything installed is up to date")
                        } else {
                            format!("{n} builds available · {ups} newer than what's installed")
                        };
                        self.toast(ctx, msg);
                    }
                    Ok(_) => {}
                    Err(e) => self.toast(ctx, format!("Error: checking for builds failed: {e}")),
                }
            }
        }
        #[cfg(not(windows))]
        if let Some(t) = &self.tray {
            t.pump();
        }
        self.handle_tray(ctx);
        ctx.request_repaint_after(Duration::from_millis(if self.jobs.is_empty() { 2000 } else { 150 }));

        let mut finished = Vec::new();
        self.jobs.retain(|job| match job.rx.try_recv() {
            Ok(result) => {
                finished.push(result);
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                finished.push(Err(format!("{} stopped unexpectedly", job.label)));
                false
            }
        });
        if !finished.is_empty() {
            self.refresh();
            for r in finished {
                match r {
                    Ok(msg) => self.toast(ctx, msg),
                    Err(msg) => self.toast(ctx, format!("Error: {msg}")),
                }
            }
        }
        let github = self.github_result.lock().ok().and_then(|mut r| r.take());
        match github {
            // An offer opens the dialog again if it was closed while looking.
            Some(GithubResult::Offer(offer)) => {
                let d = self.dlg.github.get_or_insert_with(|| GithubDialog { link: offer.url.clone(), ..Default::default() });
                d.busy = false;
                d.error = None;
                d.offer = Some(offer);
            }
            Some(GithubResult::Failed(e)) => {
                if let Some(d) = self.dlg.github.as_mut().filter(|d| d.busy) {
                    d.busy = false;
                    d.error = Some(e);
                }
            }
            Some(GithubResult::Done) => {
                if self.dlg.github.as_ref().is_some_and(|d| d.busy) {
                    self.dlg.github = None;
                }
            }
            None => {}
        }
    }

    pub fn toast(&mut self, ctx: &egui::Context, msg: impl Into<String>) {
        let now = ctx.input(|i| i.time);
        self.status = Some((msg.into(), now + 6.0));
    }

    fn spawn(&mut self, label: &str, work: impl FnOnce() -> Result<String, String> + Send + 'static) {
        self.spawn_with(label, None, work);
    }

    fn spawn_with(&mut self, label: &str, progress: Option<Arc<Progress>>, work: impl FnOnce() -> Result<String, String> + Send + 'static) {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(work());
        });
        self.jobs.push(Job { label: label.to_owned(), rx, progress });
    }

    pub fn backups_of(&self, profile: &str) -> Vec<&Backup> {
        self.backups.iter().filter(|b| b.meta.profile == profile).collect()
    }

    /// Back up a profile's preferences (`full`: the whole profile) in the background.
    pub fn back_up_profile(&mut self, name: &str, full: bool) {
        let label = format!("Backing up {name}");
        if self.busy(&label) {
            return;
        }
        let name = name.to_owned();
        self.spawn(&label, move || {
            let sp = Spindle::load().map_err(|e| format!("{e:#}"))?;
            let profile = Profile::open(&sp, &name).map_err(|e| format!("{e:#}"))?;
            let b = backup::create(&sp, &profile, full, backup::Kind::Manual, "").map_err(|e| format!("{e:#}"))?;
            let what = if full { "the whole profile" } else { "the preferences" };
            Ok(format!("Backed up {what} of '{name}' ({})", crate::ui_center::fmt_size(Some(b.size))))
        });
    }

    /// Restore a backup into its profile in the background.
    pub fn restore_backup(&mut self, id: &str) {
        let label = format!("Restoring {id}");
        if self.busy(&label) {
            return;
        }
        let id = id.to_owned();
        self.spawn(&label, move || {
            let sp = Spindle::load().map_err(|e| format!("{e:#}"))?;
            let b = backup::find(&sp, &id).map_err(|e| format!("{e:#}"))?;
            backup::restore(&sp, &b, None, false).map_err(|e| format!("{e:#}"))?;
            Ok(format!("Restored '{}' from {}; what it replaced was backed up first", b.meta.profile, b.when()))
        });
    }

    pub fn remove_backup(&mut self, ctx: &egui::Context, id: &str) {
        let Some(b) = self.backups.iter().find(|b| b.id == id).cloned() else { return };
        match backup::remove(&b) {
            Ok(()) => self.toast(ctx, format!("Backup from {} moved to the Recycle Bin", b.when())),
            Err(e) => self.toast(ctx, format!("Error: {e:#}")),
        }
        self.backups = backup::list(&self.sp, None).unwrap_or_default();
    }

    pub fn download_progress(&self, name: &str) -> Option<Arc<Progress>> {
        let label = format!("Downloading {name}");
        self.jobs.iter().find(|j| j.label == label).and_then(|j| j.progress.clone())
    }

    pub fn busy(&self, label: &str) -> bool {
        self.jobs.iter().any(|j| j.label == label)
    }

    /// The logo mask for a square `points` tall, rasterised at the exact device-pixel
    /// size (sharper than letting the GPU shrink a large texture).
    pub fn mark_texture(&mut self, ctx: &egui::Context, points: f32) -> egui::TextureHandle {
        let px = (points * ctx.pixels_per_point()).round().max(8.0) as u32;
        self.marks
            .entry(px)
            .or_insert_with(|| {
                let n = px as usize;
                ctx.load_texture(
                    format!("spindle-mark-{px}"),
                    egui::ColorImage::from_rgba_unmultiplied([n, n], &crate::logo::rasterize_mask(px)),
                    egui::TextureOptions::LINEAR,
                )
            })
            .clone()
    }

    // ----------------------------------------------------------------------------------
    // Lookups

    pub fn visible_builds(&self) -> Vec<&Build> {
        let q = self.quick_find.to_lowercase();
        self.builds
            .iter()
            .filter(|b| {
                self.channel_shown(b.channel) && (q.is_empty() || b.name.to_lowercase().contains(&q))
            })
            .collect()
    }

    pub fn channel_shown(&self, c: Channel) -> bool {
        match c {
            Channel::Lts => self.prefs.show_lts,
            Channel::Stable => self.prefs.show_stable,
            Channel::Daily => self.prefs.show_daily,
            Channel::Experimental => self.prefs.show_experimental,
            Channel::Custom => true,
        }
    }

    /// Downloadable builds that pass the channel chips and the search box.
    pub fn visible_available(&self) -> Vec<&Available> {
        let q = self.quick_find.to_lowercase();
        self.catalog
            .iter()
            .flat_map(|c| c.builds.iter())
            .filter(|a| self.channel_shown(a.channel))
            .filter(|a| q.is_empty() || a.name.to_lowercase().contains(&q) || a.branch.to_lowercase().contains(&q))
            .collect()
    }

    /// The installed build an available one corresponds to (same name, or same commit).
    pub fn installed_as(&self, a: &Available) -> Option<&Build> {
        self.builds.iter().find(|b| {
            b.name == a.name
                // An official release matches a stable/LTS build of the same version.
                || (a.hash.is_empty()
                    && matches!(a.channel, Channel::Stable | Channel::Lts)
                    && matches!(b.channel, Channel::Stable | Channel::Lts)
                    && b.version.as_deref() == Some(a.version.as_str()))
                || (!a.hash.is_empty()
                    && (b.meta.as_ref().is_some_and(|m| m.hash == a.hash) || b.name.ends_with(&format!(".{}", a.hash))))
        })
    }

    pub fn selected_available(&self) -> Option<&Available> {
        match &self.selection {
            Selection::Available(n) => self.catalog.as_ref()?.builds.iter().find(|a| &a.name == n),
            _ => None,
        }
    }

    pub fn matches_find(&self, text: &str) -> bool {
        self.quick_find.is_empty() || text.to_lowercase().contains(&self.quick_find.to_lowercase())
    }

    /// Running instances started from this build.
    pub fn running_of(&self, build: &Build) -> Vec<&Instance> {
        let exe = build.executable.to_string_lossy().to_lowercase();
        self.instances.iter().filter(|i| i.binary.to_lowercase() == exe).collect()
    }

    pub fn selected_build(&self) -> Option<&Build> {
        match &self.selection {
            Selection::Build(n) => self.builds.iter().find(|b| &b.name == n),
            _ => None,
        }
    }

    pub fn selected_instance(&self) -> Option<&Instance> {
        match &self.selection {
            Selection::Instance(pid) => self.instances.iter().find(|i| i.pid == *pid),
            _ => None,
        }
    }

    pub fn selected_addon(&self) -> Option<&AddonRecord> {
        match &self.selection {
            Selection::Addon { source, id, version } => {
                self.addons.iter().find(|a| &a.source == source && &a.id == id && &a.version == version)
            }
            _ => None,
        }
    }

    /// Newest version of each add-on id (the library keeps every version).
    pub fn latest_addons(&self) -> Vec<&AddonRecord> {
        let mut out: Vec<&AddonRecord> = Vec::new();
        for a in &self.addons {
            if !out.iter().any(|o| o.id.eq_ignore_ascii_case(&a.id)) {
                out.push(a); // list() is newest-first within each id
            }
        }
        out
    }

    pub fn profile_dir(&self, name: &str) -> PathBuf {
        self.sp.profiles_dir().join(name)
    }

    // ----------------------------------------------------------------------------------
    // Actions

    /// Select the first item of the current view (used for per-run view overrides).
    pub fn select_first_in_view(&mut self) {
        self.selection = match self.prefs.view {
            View::Builds => self.builds.first().map(|b| Selection::Build(b.name.clone())),
            View::Running => self.instances.first().map(|i| Selection::Instance(i.pid)),
            View::Addons => self
                .latest_addons()
                .first()
                .map(|a| Selection::Addon { source: a.source.clone(), id: a.id.clone(), version: a.version.clone() }),
            View::Profiles => self.profiles.first().map(|p| Selection::Profile(p.clone())),
            View::Downloads => self.visible_available().first().map(|a| Selection::Available(a.name.clone())),
        }
        .unwrap_or(Selection::None);
    }

    pub fn set_view(&mut self, v: View) {
        if self.prefs.view != v {
            self.prefs.view = v;
            self.prefs_dirty = true;
        }
    }

    pub fn set_theme(&mut self, ctx: &egui::Context, mode: ThemeMode) {
        self.prefs.theme = mode;
        self.pal = theme::palette(mode);
        theme::apply(ctx, mode);
        self.prefs_dirty = true;
    }

    /// Launch, unless the profile has compatibility problems with this build: then show them
    /// first (the dialog's Launch Anyway calls `launch_build_now`).
    pub fn launch_build(&mut self, ctx: &egui::Context, name: &str) {
        if self.compat_profile != self.active_profile {
            self.recheck_compat();
        }
        if let Some(report) = self.compat.get(name).filter(|r| r.needs_attention()) {
            self.dlg.compat = Some(CompatDialog { build: name.to_owned(), report: report.clone() });
            return;
        }
        self.launch_build_now(ctx, name);
    }

    pub fn launch_build_now(&mut self, ctx: &egui::Context, name: &str) {
        let result = builds::find(&self.sp, name).and_then(|b| {
            let profile = Profile::open(&self.sp, &self.active_profile)?;
            launch::launch(&self.sp, &b, &profile, &[]).map(|pid| (b, pid))
        });
        match result {
            Ok((b, pid)) => {
                let v = b.version.unwrap_or(b.name);
                self.toast(ctx, format!("Launched Blender {v} with profile '{}' (pid {pid})", self.active_profile));
                self.last_poll = f64::NEG_INFINITY;
            }
            Err(e) => self.toast(ctx, format!("Error: {e:#}")),
        }
    }

    /// Launch the default build: the first favourite, else the newest (builds are listed that way).
    pub fn launch_default(&mut self, ctx: &egui::Context) {
        if let Some(name) = self.builds.first().map(|b| b.name.clone()) {
            self.launch_build(ctx, &name);
        }
    }

    /// Ask GitHub for the newest Spindle release (Help > Check for Spindle Updates).
    pub fn check_spindle_release(&mut self) {
        self.spawn("Checking for Spindle updates", move || {
            let sp = Spindle::load().map_err(|e| format!("{e:#}"))?;
            let r = selfupdate::latest().map_err(|e| format!("{e:#}"))?;
            selfupdate::store(&sp, &r);
            Ok(if selfupdate::is_newer(&r) {
                format!("Spindle {} is available (this is {})", r.version, selfupdate::CURRENT)
            } else {
                format!("Spindle {} is the newest version", selfupdate::CURRENT)
            })
        });
    }

    /// Download and install the newer release, then restart into it.
    pub fn install_spindle_update(&mut self, ctx: &egui::Context) {
        let Some(release) = self.spindle_release.clone() else { return };
        let dir = match selfupdate::install_dir() {
            Ok(d) => d,
            Err(e) => return self.toast(ctx, format!("Error: {e:#}")),
        };
        let restart = self.restart_pending.clone();
        self.toast(ctx, format!("Downloading Spindle {}…", release.version));
        self.spawn("Updating Spindle", move || {
            let sp = Spindle::load().map_err(|e| format!("{e:#}"))?;
            selfupdate::install(&sp, &release, &dir, |_, _| {}).map_err(|e| format!("{e:#}"))?;
            restart.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(format!("Spindle {} installed; restarting", release.version))
        });
    }

    /// Start the freshly installed Spindle (it waits for this one to exit) and quit.
    fn restart_after_update(&mut self, ctx: &egui::Context) {
        self.restart_pending.store(false, std::sync::atomic::Ordering::Relaxed);
        let started = std::env::current_exe().and_then(|exe| {
            std::process::Command::new(exe).arg("--after-update").arg(std::process::id().to_string()).spawn()
        });
        match started {
            Ok(_) => self.quit(ctx),
            Err(e) => self.toast(ctx, format!("Updated; restart Spindle to use it ({e})")),
        }
    }

    /// Turn starting Spindle (in the tray) at sign-in on or off.
    pub fn toggle_autostart(&mut self, ctx: &egui::Context) {
        let on = !self.autostart;
        let result = std::env::current_exe()
            .map_err(anyhow::Error::from)
            .and_then(|exe| spindle_core::desktop::set_autostart(on, &exe));
        match result {
            Ok(()) => self.toast(ctx, if on { "Spindle will start in the tray when you sign in" } else { "Spindle won't start when you sign in" }),
            Err(e) => self.toast(ctx, format!("Couldn't change starting at sign-in: {e:#}")),
        }
        self.autostart = spindle_core::desktop::autostart().is_some();
    }

    /// Put this copy's programs on PATH, or take them off.
    pub fn toggle_path(&mut self, ctx: &egui::Context) {
        let on = !self.on_path;
        let Some(dir) = own_dir() else { return };
        match spindle_core::desktop::set_on_path(on, &dir) {
            Ok(()) if on => {
                let note = match spindle_core::desktop::user_bin() {
                    Some((bin, false)) if !cfg!(windows) => format!("; add {} to PATH in your shell profile", bin.display()),
                    _ => String::new(),
                };
                self.toast(ctx, format!("spindle and spindle-mcp work in new terminals{note}"))
            }
            Ok(()) => self.toast(ctx, "Spindle is off PATH"),
            Err(e) => self.toast(ctx, format!("Error: {e:#}")),
        }
        self.on_path = spindle_core::desktop::is_on_path(&dir);
    }

    /// Offer (or stop offering) Spindle for opening .blend files.
    pub fn toggle_blend_association(&mut self, ctx: &egui::Context) {
        let on = !self.opens_blend;
        let result = std::env::current_exe()
            .map_err(anyhow::Error::from)
            .and_then(|exe| spindle_core::desktop::set_blend_association(on, &exe));
        match result {
            Ok(()) if on && cfg!(windows) => {
                self.toast(ctx, "Spindle is offered under Open with for .blend files; use Choose Default App to make it the default")
            }
            Ok(()) if on => self.toast(ctx, "Spindle now opens .blend files"),
            Ok(()) => self.toast(ctx, "Spindle no longer opens .blend files"),
            Err(e) => self.toast(ctx, format!("{e:#}")),
        }
        self.opens_blend = spindle_core::desktop::blend_association().is_some();
    }

    /// Star or unstar a build; favourites come first in every list and in the tray menu.
    pub fn toggle_favourite(&mut self, name: &str) {
        let on = !self.builds.iter().any(|b| b.name == name && b.favourite);
        if let Err(e) = builds::set_favourite(&mut self.sp, name, on) {
            self.load_error = Some(format!("{e:#}"));
        }
        self.refresh();
    }

    /// Label of the default build for menus ("Blender 5.3.0").
    pub fn default_label(&self) -> Option<String> {
        self.builds.first().map(|b| format!("Blender {}", b.version.clone().unwrap_or(b.name.clone())))
    }

    /// Download and install a build in the background.
    pub fn start_download(&mut self, ctx: &egui::Context, a: Available) {
        let label = format!("Downloading {}", a.name);
        if self.busy(&label) {
            return;
        }
        self.toast(ctx, format!("Downloading Blender {} ({})…", a.version, a.channel.label()));
        let progress = Arc::new(Progress::default());
        let p = progress.clone();
        self.spawn_with(&label, Some(progress), move || {
            let sp = Spindle::load().map_err(|e| format!("{e:#}"))?;
            let b = downloads::install(&sp, &a, &p).map_err(|e| format!("{e:#}"))?;
            Ok(format!("Installed Blender {} ({})", b.version.unwrap_or(b.name), b.channel.label()))
        });
    }

    pub fn cancel_download(&self, name: &str) {
        if let Some(p) = self.download_progress(name) {
            p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Remove a build (confirmed in the dialog): library builds go to the Recycle Bin.
    pub fn remove_build(&mut self, ctx: &egui::Context, name: &str) {
        let label = format!("Removing {name}");
        if self.busy(&label) {
            return;
        }
        if self.selection == Selection::Build(name.to_owned()) {
            self.selection = Selection::None;
        }
        self.toast(ctx, format!("Removing {name}…"));
        let name = name.to_owned();
        self.spawn(&label, move || {
            let mut sp = Spindle::load().map_err(|e| format!("{e:#}"))?;
            let b = builds::find(&sp, &name).map_err(|e| format!("{e:#}"))?;
            match builds::remove(&mut sp, &b).map_err(|e| format!("{e:#}"))? {
                builds::Removed::Trashed => Ok(format!("Moved {name} to the Recycle Bin")),
                builds::Removed::Unregistered => Ok(format!("Removed {name} from Spindle (its files were left alone)")),
            }
        });
    }

    /// Hide to the notification area (when the tray is running).
    pub fn hide_to_tray(&mut self, _ctx: &egui::Context) -> bool {
        #[cfg(windows)]
        let hidden = match (&self.tray, self.main_hwnd) {
            (Some(_), Some(hwnd)) => {
                crate::tray::hide_main(hwnd);
                true
            }
            _ => false,
        };
        #[cfg(not(windows))]
        let hidden = self.tray.is_some() && {
            crate::tray::hide_main(_ctx);
            true
        };
        if let (true, Some(t)) = (hidden, &self.tray) {
            if !self.prefs.tray_hint_shown {
                t.notify(
                    "Spindle is still running",
                    "It stays in the notification area to launch Blender and watch for new builds. Right-click the icon to quit.",
                    false,
                );
                self.prefs.tray_hint_shown = true;
                self.prefs_dirty = true;
            }
            return true;
        }
        false
    }

    /// Really exit (File > Exit, tray > Quit).
    pub fn quit(&mut self, ctx: &egui::Context) {
        self.quitting = true;
        for j in &self.jobs {
            if let Some(p) = &j.progress {
                p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn handle_tray(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.tray_rx else { return };
        let events: Vec<crate::tray::TrayEvent> = rx.try_iter().collect();
        for ev in events {
            match ev {
                crate::tray::TrayEvent::Shown => self.refresh(),
                crate::tray::TrayEvent::ShowDownloads => self.set_view(View::Downloads),
                crate::tray::TrayEvent::Launch(name) => {
                    self.refresh();
                    self.set_view(View::Builds);
                    self.selection = Selection::Build(name.clone());
                    self.launch_build(ctx, &name);
                }
                crate::tray::TrayEvent::CheckUpdates => self.check_updates(ctx, true),
                crate::tray::TrayEvent::Quit => self.quit(ctx),
            }
        }
    }

    pub fn sync_superhive(&mut self, ctx: &egui::Context) {
        if !self.token_stored {
            self.dlg.superhive_login = true;
            return;
        }
        if self.busy("Syncing Superhive") {
            return;
        }
        self.toast(ctx, "Checking Superhive for purchases…");
        self.spawn("Syncing Superhive", || {
            let sp = Spindle::load().map_err(|e| format!("{e:#}"))?;
            let report = superhive::sync(&sp, &AddonLibrary::new(&sp)).map_err(|e| format!("{e:#}"))?;
            if !report.failed.is_empty() {
                return Err(format!(
                    "{} download(s) failed: {}",
                    report.failed.len(),
                    report.failed.iter().map(|(w, e)| format!("{w} ({e})")).collect::<Vec<_>>().join("; ")
                ));
            }
            Ok(format!(
                "Superhive: {} downloaded, {} already in the library",
                report.downloaded.len(),
                report.up_to_date.len()
            ))
        });
    }

    /// Open the Add from GitHub dialog (with a link: look it up straight away).
    pub fn open_github(&mut self, ctx: &egui::Context, link: Option<String>) {
        let start = link.is_some();
        self.dlg.github = Some(GithubDialog { link: link.unwrap_or_default(), focus: !start, ..Default::default() });
        if start {
            self.start_github_lookup(ctx);
        }
    }

    /// Look up the dialog's link in the background: the add-on is downloaded from a release,
    /// or the dialog turns into the offer to build it from the source.
    pub fn start_github_lookup(&mut self, ctx: &egui::Context) {
        let Some(text) = self.dlg.github.as_ref().map(|d| d.link.clone()) else { return };
        let link = match github::parse_link(&text) {
            Ok(l) => l,
            Err(e) => {
                if let Some(d) = &mut self.dlg.github {
                    d.error = Some(format!("{e:#}"));
                }
                return;
            }
        };
        let label = format!("GitHub {}", link.repo_name());
        if self.busy(&label) {
            return;
        }
        if let Some(d) = &mut self.dlg.github {
            (d.busy, d.error, d.offer) = (true, None, None);
        }
        self.toast(ctx, format!("Looking for an add-on in {}…", link.repo_name()));
        let slot = self.github_result.clone();
        self.spawn(&label, move || {
            let outcome = Spindle::load().and_then(|sp| github::add(&sp, &AddonLibrary::new(&sp), &link));
            finish_github(&slot, outcome)
        });
    }

    /// Build the offered commit in the background.
    pub fn start_github_build(&mut self, ctx: &egui::Context) {
        let Some(offer) = self.dlg.github.as_ref().and_then(|d| d.offer.clone()) else { return };
        let label = format!("GitHub {}", offer.repo);
        if self.busy(&label) {
            return;
        }
        if let Some(d) = &mut self.dlg.github {
            (d.busy, d.error) = (true, None);
        }
        self.toast(ctx, format!("Downloading and building {}…", offer.repo));
        let slot = self.github_result.clone();
        self.spawn(&label, move || {
            let outcome = Spindle::load().and_then(|sp| github::build(&sp, &AddonLibrary::new(&sp), &offer));
            finish_github(&slot, outcome)
        });
    }

    pub fn save_token(&mut self, ctx: &egui::Context) {
        let token = std::mem::take(&mut self.dlg.token_input);
        match superhive::set_token(&token) {
            Ok(()) => {
                self.token_stored = true;
                self.dlg.superhive_login = false;
                self.toast(ctx, "Superhive token saved to the system credential store");
            }
            Err(e) => self.toast(ctx, format!("Error: {e:#}")),
        }
    }

    pub fn install_addon(&mut self, ctx: &egui::Context, id: String, build: String, profile: String) {
        let label = format!("Installing {id}");
        if self.busy(&label) {
            return;
        }
        self.toast(ctx, format!("Installing {id} into profile '{profile}'…"));
        self.spawn(&label, move || {
            let sp = Spindle::load().map_err(|e| format!("{e:#}"))?;
            let lib = AddonLibrary::new(&sp);
            let rec = lib
                .latest(&id)
                .map_err(|e| format!("{e:#}"))?
                .ok_or_else(|| format!("'{id}' is not in the add-on library"))?;
            let b = builds::find(&sp, &build).map_err(|e| format!("{e:#}"))?;
            let p = Profile::open(&sp, &profile).map_err(|e| format!("{e:#}"))?;
            lib.install(&sp, &b, &p, &rec).map_err(|e| format!("{e:#}"))?;
            Ok(format!("Installed {} {} into profile '{profile}'", rec.id, rec.version))
        });
    }

    pub fn create_profile(&mut self, ctx: &egui::Context, name: &str) {
        match Profile::open(&self.sp, name.trim()) {
            Ok(p) => {
                self.toast(ctx, format!("Created profile '{}'", p.name));
                self.refresh();
            }
            Err(e) => self.toast(ctx, format!("Error: {e:#}")),
        }
    }

    pub fn set_default_profile(&mut self, ctx: &egui::Context, name: &str) {
        self.sp.config.default_profile = Some(name.to_owned());
        match self.sp.save() {
            Ok(()) => {
                self.active_profile = name.to_owned();
                self.toast(ctx, format!("'{name}' is now the default profile"));
            }
            Err(e) => self.toast(ctx, format!("Error: {e:#}")),
        }
    }

    pub fn add_external_build(&mut self, ctx: &egui::Context) {
        let Some(exe) = rfd::FileDialog::new()
            .set_title("Choose a Blender executable")
            .add_filter("Blender", &[if cfg!(windows) { "exe" } else { "*" }])
            .pick_file()
        else {
            return;
        };
        let name = exe
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "custom".into());
        match builds::register_external(&mut self.sp, &name, &exe) {
            Ok(()) => {
                self.toast(ctx, format!("Added build '{name}'"));
                self.refresh();
            }
            Err(e) => self.toast(ctx, format!("Error: {e:#}")),
        }
    }

    /// Ready-to-paste MCP client config pointing at spindle-mcp next to this program.
    pub fn mcp_config_snippet(&self) -> String {
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join(if cfg!(windows) { "spindle-mcp.exe" } else { "spindle-mcp" })))
            .unwrap_or_else(|| PathBuf::from("spindle-mcp"));
        serde_json::json!({ "mcpServers": { "spindle": { "command": exe.to_string_lossy() } } }).to_string()
    }

    pub fn save_prefs(&self) {
        if let Some(path) = prefs_path(&self.sp) {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(text) = serde_json::to_string_pretty(&self.prefs) {
                let _ = std::fs::write(path, text);
            }
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if let Some(t) = &self.tray {
            t.shutdown();
        }
    }
}

/// The app window's HWND (Windows), for the tray to show and hide it.
fn window_handle(cc: &eframe::CreationContext<'_>) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match cc.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

/// Fetches the download catalog when asked (`trigger`) and every few hours, and announces
/// builds newer than what's installed (once each) with a tray balloon.
fn update_checker(
    ctx: egui::Context,
    trigger: Receiver<bool>,
    results: Sender<CheckResult>,
    tray: Option<crate::tray::Tray>,
) {
    const EVERY: Duration = Duration::from_secs(6 * 3600);
    let age = Spindle::load().ok().and_then(|sp| downloads::cached(&sp)).map(|c| c.age_secs().max(0) as u64);
    // First check soon after start unless the cache is recent.
    let mut wait = match age {
        Some(a) if a < EVERY.as_secs() => EVERY - Duration::from_secs(a),
        _ => Duration::from_secs(5),
    };
    loop {
        let manual = match trigger.recv_timeout(wait) {
            Ok(m) => m,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => false,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        };
        wait = EVERY;
        let outcome = (|| -> Result<usize, String> {
            let sp = Spindle::load().map_err(|e| format!("{e:#}"))?;
            let cat = downloads::fetch(&sp).map_err(|e| format!("{e:#}"))?;
            let installed = builds::list(&sp).unwrap_or_default();
            let ups = downloads::updates(&installed, &cat);
            // Announce each update once (remembered in the run folder).
            let seen_path = sp.run_dir().join("announced.json");
            let mut seen: Vec<String> =
                std::fs::read(&seen_path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
            let fresh: Vec<&Update> = ups.iter().filter(|u| !seen.contains(&u.available.name)).collect();
            // Spindle itself: once a day (or on a manual check), announced once per version.
            let day_old = selfupdate::cached(&sp).is_none_or(|r| {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
                now - r.fetched > 24 * 3600
            });
            if manual || day_old {
                if let Ok(r) = selfupdate::latest() {
                    selfupdate::store(&sp, &r);
                    let key = format!("spindle-{}", r.version);
                    if selfupdate::is_newer(&r) && !seen.contains(&key) {
                        if let Some(t) = &tray {
                            t.notify(&format!("Spindle {} is available", r.version), "Open Spindle and choose Help > Spindle Updates to install it.", false);
                        }
                        seen.push(key);
                        let _ = std::fs::write(&seen_path, serde_json::to_vec(&seen).unwrap_or_default());
                    }
                }
            }
            if let Some(t) = &tray {
                match fresh.as_slice() {
                    [] if manual => t.notify(
                        "Blender builds checked",
                        &format!("{} builds available; nothing newer than what's installed.", cat.builds.len()),
                        false,
                    ),
                    [] => {}
                    [u] => t.notify(
                        &format!("Blender {} is available", u.available.version),
                        &match &u.installed {
                            Some(i) => format!("A newer {} build than {i}. Click to download it.", u.available.channel.label()),
                            None => format!("A new {} release. Click to download it.", u.available.channel.label()),
                        },
                        true,
                    ),
                    many => t.notify(
                        &format!("{} new Blender builds", many.len()),
                        &many.iter().map(|u| format!("Blender {}", u.available.version)).collect::<Vec<_>>().join(", "),
                        true,
                    ),
                }
            }
            seen.extend(fresh.iter().map(|u| u.available.name.clone()));
            let _ = std::fs::write(&seen_path, serde_json::to_vec(&seen).unwrap_or_default());
            Ok(cat.builds.len())
        })();
        let _ = results.send(CheckResult { manual, outcome });
        ctx.request_repaint();
    }
}

/// Hand a GitHub job's outcome to the dialog; returns the status-bar message.
fn finish_github(slot: &Mutex<Option<GithubResult>>, outcome: anyhow::Result<github::Added>) -> Result<String, String> {
    let (result, toast) = match outcome {
        Ok(github::Added::Offer(o)) => {
            let msg = format!("{}; Spindle can build it from the source", o.reason);
            (GithubResult::Offer(o), Ok(msg))
        }
        Ok(github::Added::Stored(r)) => {
            let msg = match &r.origin {
                Some(o) if o.built => format!("Built {} {} from {} and added it to the add-on library", r.name, r.version, o.git_ref),
                _ => format!("Added {} {} to the add-on library", r.name, r.version),
            };
            (GithubResult::Done, Ok(msg))
        }
        Ok(github::Added::Present(r)) => {
            (GithubResult::Done, Ok(format!("{} {} is already in the add-on library", r.name, r.version)))
        }
        Err(e) => {
            let full = format!("{e:#}");
            let first = full.lines().next().unwrap_or_default().to_owned();
            (GithubResult::Failed(full), Err(first))
        }
    };
    if let Ok(mut s) = slot.lock() {
        *s = Some(result);
    }
    toast
}

/// The folder holding this Spindle's programs.
fn own_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(|p| p.to_path_buf())
}

fn prefs_path(sp: &Spindle) -> Option<PathBuf> {
    sp.config_path.parent().map(|d| d.join("ui.json"))
}

fn load_prefs(sp: &Spindle) -> Prefs {
    prefs_path(sp)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}
