use crate::builds::{self, Build};
use crate::profile::Profile;
use crate::{Spindle, backup, compat, launch, version};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

/// How Blender installs an archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// An extension (`blender_manifest.toml`, Blender 4.2+), installed into the profile's
    /// `user_default` repository.
    #[default]
    Extension,
    /// A legacy add-on (`bl_info`), installed into the profile's `scripts/addons`.
    Legacy,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Extension => "Extension",
            Format::Legacy => "Legacy add-on",
        }
    }
}

/// Where a GitHub add-on came from.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Origin {
    /// The link it was added from, normalised (adding it again checks for a newer version).
    pub url: String,
    /// "owner/repo".
    pub repo: String,
    /// Release tag, or the branch, tag or commit a build came from.
    pub git_ref: String,
    pub commit: String,
    /// Folder inside the repository ("" for the root).
    pub path: String,
    /// Spindle built the archive from the source (no release had one).
    pub built: bool,
}

/// One stored archive in Spindle's add-on library:
/// `<library>/addons/<source>/<id>/<version>/{<archive>, addon.json}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddonRecord {
    /// Extension id, or a legacy add-on's module name.
    pub id: String,
    pub name: String,
    pub version: String,
    /// Extension type: "add-on" or "theme".
    pub kind: String,
    /// Where it came from: "superhive" or "github".
    pub source: String,
    /// Archive file name inside the version folder.
    pub archive: String,
    pub sha256: String,
    pub blender_version_min: Option<String>,
    pub blender_version_max: Option<String>,
    #[serde(default)]
    pub platforms: Vec<String>,
    /// Unix seconds when it was stored.
    pub fetched: u64,
    #[serde(default)]
    pub format: Format,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

/// Version order, ignoring build metadata ("1.2.0+abc1234" is a build of 1.2.0), then the
/// newer download first among equal versions.
fn order(a: &AddonRecord, b: &AddonRecord) -> Ordering {
    let core = |v: &str| v.split('+').next().unwrap_or(v).to_owned();
    version::compare(&core(&a.version), &core(&b.version)).then(a.fetched.cmp(&b.fetched))
}

pub struct AddonLibrary {
    root: PathBuf,
}

impl AddonLibrary {
    pub fn new(sp: &Spindle) -> Self {
        Self { root: sp.addons_dir() }
    }

    fn version_dir(&self, source: &str, id: &str, version: &str) -> PathBuf {
        self.root.join(sanitize(source)).join(sanitize(id)).join(sanitize(version))
    }

    pub fn archive_path(&self, rec: &AddonRecord) -> PathBuf {
        self.version_dir(&rec.source, &rec.id, &rec.version).join(&rec.archive)
    }

    pub fn contains(&self, source: &str, id: &str, version: &str) -> bool {
        self.version_dir(source, id, version).join("addon.json").is_file()
    }

    /// Store an archive (already verified by the caller) and its metadata.
    pub fn store(&self, rec: &AddonRecord, bytes: &[u8]) -> Result<PathBuf> {
        let dir = self.version_dir(&rec.source, &rec.id, &rec.version);
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let archive = dir.join(&rec.archive);
        fs::write(&archive, bytes)?;
        // Metadata last: its presence marks the entry as complete.
        fs::write(dir.join("addon.json"), serde_json::to_vec_pretty(rec)?)?;
        Ok(archive)
    }

    /// Every stored version, newest first within each id.
    pub fn list(&self) -> Result<Vec<AddonRecord>> {
        let mut out = Vec::new();
        if !self.root.is_dir() {
            return Ok(out);
        }
        for source in read_dirs(&self.root)? {
            for id in read_dirs(&source)? {
                for ver in read_dirs(&id)? {
                    if let Ok(text) = fs::read_to_string(ver.join("addon.json")) {
                        if let Ok(rec) = serde_json::from_str::<AddonRecord>(&text) {
                            out.push(rec);
                        }
                    }
                }
            }
        }
        out.sort_by(|a, b| a.id.to_lowercase().cmp(&b.id.to_lowercase()).then_with(|| order(b, a)));
        Ok(out)
    }

    /// Newest stored version of an add-on, from any source.
    pub fn latest(&self, id: &str) -> Result<Option<AddonRecord>> {
        Ok(self.list()?.into_iter().filter(|r| r.id.eq_ignore_ascii_case(id)).max_by(order))
    }

    /// Install a stored archive into a profile and enable it, with a library build Spindle
    /// picks to run Blender's installer ([`pick_installer`]). Returns that build.
    pub fn install_into(&self, sp: &Spindle, profile: &Profile, rec: &AddonRecord) -> Result<Build> {
        let builds = builds::list(sp)?;
        let build = pick_installer(&builds, backup::prefs_saved_by(&profile.root), rec)?.clone();
        self.install(sp, &build, profile, rec)?;
        Ok(build)
    }

    /// Install a stored archive into a profile with the given build and enable it: an
    /// extension through the build's own extension tooling (into the profile's local
    /// `user_default` repository), a legacy add-on through Blender's add-on installer (into
    /// the profile's `scripts/addons`). Preferences are saved so the add-on stays enabled,
    /// which is why it's refused while a Blender runs with the profile: that Blender saves
    /// its own preferences when it quits.
    pub fn install(&self, sp: &Spindle, build: &Build, profile: &Profile, rec: &AddonRecord) -> Result<String> {
        let running = backup::running_with(sp, &profile.name);
        if !running.is_empty() {
            bail!(
                "Blender is running with profile '{}' (pid {}); close it first, or it saves its preferences when it quits and switches {} off again",
                profile.name,
                running.iter().map(u32::to_string).collect::<Vec<_>>().join(", "),
                rec.name
            );
        }
        // Installing saves the preferences, so an older build would rewrite newer ones:
        // back them up first, as a launch would.
        let _ = backup::before_launch(sp, build, profile);
        let archive = self.archive_path(rec).to_string_lossy().into_owned();
        let args: Vec<String> = match rec.format {
            Format::Extension => {
                if let Some(s) = build.series().filter(|s| *s < compat::EXTENSIONS_SINCE) {
                    bail!(
                        "{} is an extension, which needs Blender 4.2 or newer to install; {} is Blender {}.{}",
                        rec.name,
                        build.name,
                        s.0,
                        s.1
                    );
                }
                ["--command", "extension", "install-file", "--repo", "user_default", "--enable", &archive]
                    .map(str::to_owned)
                    .to_vec()
            }
            // The paths go after `--`, which Blender leaves to the script (no quoting issues).
            Format::Legacy => ["--python-exit-code", "1", "--python-expr", LEGACY_INSTALL, "--", &archive, &rec.id]
                .map(str::to_owned)
                .to_vec(),
        };
        let out = launch::run_background(sp, build, profile, &args).map_err(|e| {
            // The legacy installer explains a failure in one line; Blender's traceback is noise.
            let text = format!("{e:#}");
            match text.lines().find_map(|l| l.trim().strip_prefix("Spindle error: ")) {
                Some(why) => anyhow::anyhow!("{} {}: {why}", rec.name, rec.version),
                None => e,
            }
        })?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// The build that installs `rec` into a profile whose preferences Blender `prefs` saved
/// (None: no preferences yet). Only builds that can install it qualify: Blender 4.2+ for an
/// extension, and inside the add-on's own Blender range. Installing saves the preferences,
/// so of those it takes the oldest series that isn't older than the preferences (newest
/// build in it), which keeps them readable by the builds that read them now; failing that,
/// the newest.
pub fn pick_installer<'a>(builds: &'a [Build], prefs: Option<(u32, u32)>, rec: &AddonRecord) -> Result<&'a Build> {
    let full = |b: &Build| b.version.clone().or_else(|| b.series().map(|(x, y)| format!("{x}.{y}.0")));
    let fits = |b: &&Build| {
        let (Some(v), Some(series)) = (full(b), b.series()) else { return false };
        (rec.format == Format::Legacy || series >= compat::EXTENSIONS_SINCE)
            && rec.blender_version_min.as_deref().is_none_or(|min| version::compare(&v, min) != Ordering::Less)
            && rec.blender_version_max.as_deref().is_none_or(|max| version::compare(&v, max) == Ordering::Less)
    };
    let mut ok: Vec<&Build> = builds.iter().filter(fits).collect();
    if ok.is_empty() {
        let min = rec.blender_version_min.clone().or_else(|| (rec.format == Format::Extension).then(|| "4.2".to_owned()));
        let range = match (min, &rec.blender_version_max) {
            (Some(min), Some(max)) => format!("Blender {min} or newer, older than {max},"),
            (Some(min), None) => format!("Blender {min} or newer"),
            (None, Some(max)) => format!("a Blender older than {max}"),
            (None, None) => "a Blender build".to_owned(),
        };
        bail!("{} needs {range} to install, and the library has none; download one under Download Builds", rec.name);
    }
    ok.sort_by(|a, b| version::compare(&full(b).unwrap_or_default(), &full(a).unwrap_or_default()));
    let series = prefs.and_then(|p| ok.iter().filter_map(|b| b.series()).filter(|s| *s >= p).min());
    Ok(series.and_then(|s| ok.iter().find(|b| b.series() == Some(s)).copied()).unwrap_or(ok[0]))
}

/// An add-on found installed in a profile.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstalledAddon {
    /// Extension id (its folder in the repository) or legacy module name.
    pub id: String,
    pub version: Option<String>,
    pub format: Format,
    /// Extension repository folder ("user_default", "blender_org", ...); None for a legacy add-on.
    pub repo: Option<String>,
    /// Enabled in the profile's preferences; None when there are none Spindle can read.
    pub enabled: Option<bool>,
}

/// Add-ons installed in a profile folder: extensions in its local repositories
/// (`extensions/<repo>/<id>`) and legacy add-ons in `scripts/addons`.
pub fn installed_in(profile_root: &Path) -> Vec<InstalledAddon> {
    let prefs = fs::read(profile_root.join("config").join("userpref.blend")).ok();
    // Compressed preferences hide the module names.
    let prefs = prefs.as_deref().filter(|b| b.starts_with(b"BLENDER"));
    let enabled = |module: &str| prefs.map(|b| compat::enabled_in(Some(b), module));
    let visible = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.starts_with(['.', '_']));
    let mut out = Vec::new();
    for repo in read_dirs(&profile_root.join("extensions")).unwrap_or_default() {
        let Some(repo_name) = visible(&repo) else { continue };
        for dir in read_dirs(&repo).unwrap_or_default() {
            let Some(id) = visible(&dir) else { continue };
            let Ok(text) = fs::read_to_string(dir.join("blender_manifest.toml")) else { continue };
            out.push(InstalledAddon {
                version: parse_manifest(&text).ok().map(|i| i.version),
                format: Format::Extension,
                enabled: enabled(&format!("bl_ext.{repo_name}.{id}")),
                repo: Some(repo_name.clone()),
                id,
            });
        }
    }
    for entry in fs::read_dir(profile_root.join("scripts").join("addons")).into_iter().flatten().flatten() {
        let path = entry.path();
        let Some(name) = visible(&path) else { continue };
        let (module, source) = if path.is_dir() {
            (name, path.join("__init__.py"))
        } else if let Some(m) = name.strip_suffix(".py") {
            (m.to_owned(), path.clone())
        } else {
            continue;
        };
        let Some(info) = fs::read_to_string(&source).ok().and_then(|s| parse_bl_info(&s, &module)) else { continue };
        out.push(InstalledAddon { enabled: enabled(&module), id: module, version: Some(info.version), format: Format::Legacy, repo: None });
    }
    out
}

/// Run inside Blender with `-- <archive> <module>`. A failing step prints one
/// "Spindle error:" line (the operator's own message) and exits with 1.
const LEGACY_INSTALL: &str = "\
import bpy, sys
archive, module = sys.argv[sys.argv.index('--') + 1:][:2]
def step(what, op, **kw):
    try:
        done, why = op(**kw) == {'FINISHED'}, ''
    except RuntimeError as e:
        done, why = False, str(e).strip()
    if not done:
        why = why[7:] if why.startswith('Error: ') else why
        print('Spindle error: ' + what + (': ' + why.strip() if why else ''), file=sys.stderr)
        sys.exit(1)
step('Blender could not install it', bpy.ops.preferences.addon_install, filepath=archive, overwrite=True)
step('it installed but failed to enable', bpy.ops.preferences.addon_enable, module=module)
bpy.ops.wm.save_userpref()
print('Spindle: installed and enabled', module)
";

fn read_dirs(dir: &Path) -> Result<Vec<PathBuf>> {
    Ok(fs::read_dir(dir)?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect())
}

/// Keep path components safe on every platform.
pub(crate) fn sanitize(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') { c } else { '_' })
        .collect();
    let trimmed = cleaned.trim_matches(['.', ' ']);
    if trimmed.is_empty() { "_".into() } else { trimmed.to_owned() }
}

// --------------------------------------------------------------------------------------
// What an archive holds

/// An add-on as described by its `blender_manifest.toml` or `bl_info`.
#[derive(Debug, Clone, PartialEq)]
pub struct AddonInfo {
    pub format: Format,
    /// Extension id, or the legacy add-on's module name.
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: String,
    pub blender_version_min: Option<String>,
    pub blender_version_max: Option<String>,
    pub platforms: Vec<String>,
}

/// Read an extension manifest.
pub fn parse_manifest(text: &str) -> Result<AddonInfo> {
    let t: toml::Table = text.parse().context("blender_manifest.toml is not valid TOML")?;
    let s = |k: &str| t.get(k).and_then(|v| v.as_str()).map(str::to_owned);
    let id = s("id").context("blender_manifest.toml has no id")?;
    Ok(AddonInfo {
        format: Format::Extension,
        name: s("name").unwrap_or_else(|| id.clone()),
        version: s("version").context("blender_manifest.toml has no version")?,
        kind: s("type").unwrap_or_else(|| "add-on".into()),
        blender_version_min: s("blender_version_min"),
        blender_version_max: s("blender_version_max"),
        platforms: t
            .get("platforms")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
            .unwrap_or_default(),
        id,
    })
}

/// Read a legacy add-on's `bl_info = {...}` from its `__init__.py` (or single-file module).
/// None when the source doesn't assign `bl_info`.
pub fn parse_bl_info(src: &str, module: &str) -> Option<AddonInfo> {
    let body = bl_info_body(src)?;
    let field = |key: &str| -> Option<&str> {
        let at = body.find(&format!("\"{key}\"")).or_else(|| body.find(&format!("'{key}'")))?;
        body[at + key.len() + 2..].trim_start().strip_prefix(':').map(str::trim_start)
    };
    let version = field("version").and_then(py_tuple).unwrap_or_else(|| "0.0.0".into());
    Some(AddonInfo {
        format: Format::Legacy,
        id: module.to_owned(),
        name: field("name").and_then(py_string).unwrap_or_else(|| module.to_owned()),
        version,
        kind: "add-on".into(),
        blender_version_min: field("blender").and_then(py_tuple),
        blender_version_max: None,
        platforms: Vec::new(),
    })
}

/// The text of the dict assigned to `bl_info` (between its braces).
fn bl_info_body(src: &str) -> Option<&str> {
    let mut from = 0;
    while let Some(at) = src[from..].find("bl_info") {
        let start = from + at;
        from = start + 7;
        // `bl_info = {` or `bl_info: dict = {`, at the start of a line.
        let line_start = src[..start].rfind('\n').map_or(0, |i| i + 1);
        if !src[line_start..start].trim().is_empty() {
            continue;
        }
        let rest = &src[from..];
        let Some(eq) = rest.find('=') else { continue };
        if rest[..eq].contains('\n') || !(rest[..eq].trim().is_empty() || rest[..eq].trim_start().starts_with(':')) {
            continue;
        }
        let after = rest[eq + 1..].trim_start();
        if !after.starts_with('{') {
            continue;
        }
        // Find the matching brace, skipping strings.
        let (mut depth, mut quote) = (0usize, None::<char>);
        let mut prev = '\0';
        for (i, c) in after.char_indices() {
            match quote {
                Some(q) if c == q && prev != '\\' => quote = None,
                Some(_) => {}
                None if c == '"' || c == '\'' => quote = Some(c),
                None if c == '{' => depth += 1,
                None if c == '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&after[1..i]);
                    }
                }
                None => {}
            }
            prev = c;
        }
        return None;
    }
    None
}

fn py_string(s: &str) -> Option<String> {
    let q = s.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let end = s[1..].find(q)?;
    Some(s[1..1 + end].to_owned())
}

/// `(1, 2, 3)` -> "1.2.3".
fn py_tuple(s: &str) -> Option<String> {
    let inner = s.strip_prefix('(').or_else(|| s.strip_prefix('['))?;
    let end = inner.find([')', ']'])?;
    let nums: Vec<&str> = inner[..end].split(',').map(str::trim).filter(|n| !n.is_empty()).collect();
    (!nums.is_empty() && nums.iter().all(|n| n.chars().all(|c| c.is_ascii_digit()))).then(|| nums.join("."))
}

/// What a downloaded archive holds: an extension (`blender_manifest.toml` at the top or one
/// folder down, as Blender accepts), a legacy add-on package (`<module>/__init__.py` with
/// `bl_info`), or a single-file legacy add-on (`<module>.py`).
pub fn inspect_zip(path: &Path) -> Result<AddonInfo> {
    let mut zip = zip::ZipArchive::new(File::open(path)?).context("not a zip archive")?;
    let mut names: Vec<(usize, String)> = Vec::new();
    for i in 0..zip.len() {
        let entry = zip.by_index_raw(i)?;
        if let Some(p) = entry.enclosed_name().filter(|_| entry.is_file()) {
            names.push((i, p.to_string_lossy().replace('\\', "/")));
        }
    }
    let depth = |n: &str| n.matches('/').count();
    let mut read = |i: usize| -> Result<String> {
        let mut s = String::new();
        zip.by_index(i)?.take(4 << 20).read_to_string(&mut s)?;
        Ok(s)
    };
    let manifest = names
        .iter()
        .filter(|(_, n)| depth(n) <= 1 && n.rsplit('/').next() == Some("blender_manifest.toml"))
        .min_by_key(|(_, n)| depth(n));
    if let Some((i, _)) = manifest {
        return parse_manifest(&read(*i)?);
    }
    for (i, n) in names.iter().filter(|(_, n)| depth(n) == 1 && n.ends_with("/__init__.py")) {
        let module = n.split('/').next().unwrap_or_default();
        if let Some(info) = parse_bl_info(&read(*i)?, module) {
            return Ok(info);
        }
    }
    for (i, n) in names.iter().filter(|(_, n)| depth(n) == 0 && n.ends_with(".py") && n != "__init__.py") {
        if let Some(info) = parse_bl_info(&read(*i)?, n.trim_end_matches(".py")) {
            return Ok(info);
        }
    }
    bail!("it has no blender_manifest.toml or bl_info, so it isn't a Blender add-on")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bl_info_fields() {
        let src = "# comment mentioning bl_info\nimport bpy\n\nbl_info = {\n    \"name\": \"Uv Squares\",\n    \"description\": \"a {brace} in text\",\n    \"version\": (1, 14, 1),\n    'blender': (2, 80, 0),\n}\n";
        let info = parse_bl_info(src, "UvSquares").unwrap();
        assert_eq!(info.format, Format::Legacy);
        assert_eq!(info.id, "UvSquares");
        assert_eq!(info.name, "Uv Squares");
        assert_eq!(info.version, "1.14.1");
        assert_eq!(info.blender_version_min.as_deref(), Some("2.80.0"));
        assert!(parse_bl_info("from . import bl_info\nprint(bl_info)\n", "x").is_none());
        let annotated = parse_bl_info("bl_info: dict = {'name': 'A'}\n", "a").unwrap();
        assert_eq!((annotated.name.as_str(), annotated.version.as_str()), ("A", "0.0.0"));
    }

    #[test]
    fn manifest_fields() {
        let info = parse_manifest(
            "schema_version = \"1.0.0\"\nid = \"matalogue\"\nname = \"Matalogue\"\nversion = \"1.3.2\"\ntype = \"add-on\"\nblender_version_min = \"4.2.0\"\nplatforms = [\"windows-x64\"]\n",
        )
        .unwrap();
        assert_eq!(info.format, Format::Extension);
        assert_eq!((info.id.as_str(), info.version.as_str()), ("matalogue", "1.3.2"));
        assert_eq!(info.platforms, vec!["windows-x64".to_string()]);
        assert!(parse_manifest("name = \"no id\"").is_err());
    }

    #[test]
    fn build_metadata_orders_by_download_time() {
        let rec = |version: &str, fetched: u64| AddonRecord {
            id: "a".into(),
            name: "a".into(),
            version: version.into(),
            kind: "add-on".into(),
            source: "github".into(),
            archive: "a.zip".into(),
            sha256: String::new(),
            blender_version_min: None,
            blender_version_max: None,
            platforms: Vec::new(),
            fetched,
            format: Format::Legacy,
            origin: None,
        };
        assert_eq!(order(&rec("1.2.0+fff0000", 1), &rec("1.2.0+0000fff", 2)), Ordering::Less);
        assert_eq!(order(&rec("1.10.0+aaa", 1), &rec("1.9.0+bbb", 2)), Ordering::Greater);
    }

    fn extension(min: Option<&str>, max: Option<&str>) -> AddonRecord {
        AddonRecord {
            id: "etr_curves_toolkit".into(),
            name: "Curve Toolkit".into(),
            version: "0.27.0".into(),
            kind: "add-on".into(),
            source: "github".into(),
            archive: "a.zip".into(),
            sha256: String::new(),
            blender_version_min: min.map(str::to_owned),
            blender_version_max: max.map(str::to_owned),
            platforms: Vec::new(),
            fetched: 0,
            format: Format::Extension,
            origin: None,
        }
    }

    #[test]
    fn installer_keeps_the_preferences_format() {
        let build = |v: &str| Build {
            name: format!("blender-{v}"),
            executable: PathBuf::from(format!("none/{v}/blender.exe")),
            version: Some(v.to_owned()),
            external: true,
            channel: crate::builds::Channel::Custom,
            meta: None,
            favourite: false,
        };
        let builds = vec![build("4.1.1"), build("5.2.4"), build("5.3.0"), build("5.3.1"), build("5.4.0")];
        let pick = |prefs, rec: &AddonRecord| pick_installer(&builds, prefs, rec).map(|b| b.name.clone());
        let any = extension(None, None);
        // Same series as the preferences, newest build in it.
        assert_eq!(pick(Some((5, 3)), &any).unwrap(), "blender-5.3.1");
        // No build of that series: the next one up, not the newest.
        assert_eq!(pick(Some((5, 1)), &any).unwrap(), "blender-5.2.4");
        // Everything is older than the preferences, or there are none yet: the newest.
        assert_eq!(pick(Some((6, 0)), &any).unwrap(), "blender-5.4.0");
        assert_eq!(pick(None, &any).unwrap(), "blender-5.4.0");
        // The add-on's own range wins over the preferences.
        assert_eq!(pick(Some((5, 2)), &extension(Some("5.3.0"), None)).unwrap(), "blender-5.3.1");
        assert_eq!(pick(Some((5, 4)), &extension(None, Some("5.4.0"))).unwrap(), "blender-5.3.1");
        // Extensions need 4.2+; nothing fits: say what's needed.
        let only_old = vec![build("4.1.1")];
        let err = pick_installer(&only_old, None, &any).unwrap_err().to_string();
        assert!(err.contains("Blender 4.2 or newer"), "{err}");
        let err = pick_installer(&builds, None, &extension(Some("6.0.0"), None)).unwrap_err().to_string();
        assert!(err.contains("Curve Toolkit needs Blender 6.0.0 or newer"), "{err}");
    }

    #[test]
    fn finds_what_a_profile_has_installed() {
        let root = std::env::temp_dir().join(format!("spindle-installed-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let ext = root.join("extensions/user_default/etr_curves_toolkit");
        fs::create_dir_all(&ext).unwrap();
        fs::write(ext.join("blender_manifest.toml"), "id = \"etr_curves_toolkit\"\nversion = \"0.27.0\"\n").unwrap();
        fs::create_dir_all(root.join("extensions/blender_org/node_arrange")).unwrap();
        fs::write(root.join("extensions/blender_org/node_arrange/blender_manifest.toml"), "id = \"node_arrange\"\nversion = \"1.4.0\"\n").unwrap();
        fs::create_dir_all(root.join("extensions/.cache/x")).unwrap();
        fs::create_dir_all(root.join("scripts/addons/uv_squares")).unwrap();
        fs::write(root.join("scripts/addons/uv_squares/__init__.py"), "bl_info = {\"name\": \"UV Squares\", \"version\": (1, 14, 1)}\n").unwrap();
        fs::write(root.join("scripts/addons/single.py"), "bl_info = {'name': 'Single', 'version': (2, 0)}\n").unwrap();
        fs::write(root.join("scripts/addons/not_an_addon.py"), "print('hi')\n").unwrap();
        fs::create_dir_all(root.join("config")).unwrap();

        // No preferences yet: enabled is unknown.
        assert!(installed_in(&root).iter().all(|a| a.enabled.is_none()));

        fs::write(root.join("config/userpref.blend"), b"BLENDER17-01v0503\0bl_ext.user_default.etr_curves_toolkit\0uv_squares\0").unwrap();
        let mut found = installed_in(&root);
        found.sort_by(|a, b| a.id.cmp(&b.id));
        let summary: Vec<(&str, Option<&str>, Option<&str>, Option<bool>)> =
            found.iter().map(|a| (a.id.as_str(), a.version.as_deref(), a.repo.as_deref(), a.enabled)).collect();
        assert_eq!(
            summary,
            vec![
                ("etr_curves_toolkit", Some("0.27.0"), Some("user_default"), Some(true)),
                ("node_arrange", Some("1.4.0"), Some("blender_org"), Some(false)),
                ("single", Some("2.0"), None, Some(false)),
                ("uv_squares", Some("1.14.1"), None, Some(true)),
            ]
        );
        let _ = fs::remove_dir_all(&root);
    }
}
