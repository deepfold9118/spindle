//! Application state: library data from spindle-core, UI preferences, background jobs
//! (Superhive sync, add-on installs) and the actions the menus, task panel and cards call.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use spindle_core::addons::{AddonLibrary, AddonRecord};
use spindle_core::builds::{self, Build};
use spindle_core::instances::{self, Instance};
use spindle_core::profile::Profile;
use spindle_core::{Spindle, launch, superhive};

use crate::theme::{self, Palette, ThemeMode};

/// What the centre area shows (chosen in the left tree).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum View {
    Builds,
    Running,
    Addons,
    Profiles,
}

impl View {
    pub fn title(self) -> &'static str {
        match self {
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

/// Release channel, parsed from a build's folder name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Lts,
    Stable,
    Daily,
    Experimental,
    Other,
}

impl Channel {
    pub fn of(name: &str) -> Self {
        let n = name.to_lowercase();
        if n.contains("lts") {
            Channel::Lts
        } else if n.contains("stable") || n.contains("candidate") {
            Channel::Stable
        } else if n.contains("alpha") || n.contains("beta") || n.contains("daily") {
            Channel::Daily
        } else if n.contains("experimental") || n.contains("patch") || n.contains("branch") {
            Channel::Experimental
        } else {
            Channel::Other
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Channel::Lts => "LTS",
            Channel::Stable => "Stable",
            Channel::Daily => "Daily",
            Channel::Experimental => "Experimental",
            Channel::Other => "Custom",
        }
    }
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
}

pub struct Job {
    pub label: String,
    rx: Receiver<Result<String, String>>,
}

#[derive(Default)]
pub struct Dialogs {
    pub superhive_login: bool,
    pub token_input: String,
    /// Install dialog: (add-on id, chosen build name, chosen profile).
    pub install: Option<(String, String, String)>,
    pub new_profile: Option<String>,
    pub about: bool,
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
        };
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
        if !self.profiles.contains(&self.active_profile) {
            self.active_profile = self.sp.profile_name(None);
        }
        self.token_stored = superhive::has_token();
        self.instances = instances::list(&self.sp).unwrap_or_default();
    }

    /// Per-frame housekeeping: poll running instances every 2 s and collect finished jobs.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if now - self.last_poll >= 2.0 {
            self.last_poll = now;
            self.instances = instances::list(&self.sp).unwrap_or_default();
        }
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
    }

    pub fn toast(&mut self, ctx: &egui::Context, msg: impl Into<String>) {
        let now = ctx.input(|i| i.time);
        self.status = Some((msg.into(), now + 6.0));
    }

    fn spawn(&mut self, label: &str, work: impl FnOnce() -> Result<String, String> + Send + 'static) {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(work());
        });
        self.jobs.push(Job { label: label.to_owned(), rx });
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
                let shown = match Channel::of(&b.name) {
                    Channel::Lts => self.prefs.show_lts,
                    Channel::Stable => self.prefs.show_stable,
                    Channel::Daily => self.prefs.show_daily,
                    Channel::Experimental => self.prefs.show_experimental,
                    Channel::Other => true,
                };
                shown && (q.is_empty() || b.name.to_lowercase().contains(&q))
            })
            .collect()
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

    pub fn launch_build(&mut self, ctx: &egui::Context, name: &str) {
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

    pub fn launch_newest(&mut self, ctx: &egui::Context) {
        if let Some(name) = self.builds.first().map(|b| b.name.clone()) {
            self.launch_build(ctx, &name);
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

fn prefs_path(sp: &Spindle) -> Option<PathBuf> {
    sp.config_path.parent().map(|d| d.join("ui.json"))
}

fn load_prefs(sp: &Spindle) -> Prefs {
    prefs_path(sp)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}
