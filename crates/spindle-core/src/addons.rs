use crate::builds::Build;
use crate::profile::Profile;
use crate::{Spindle, compat, launch, version};
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

    /// Install a stored archive into a profile with the given build and enable it: an
    /// extension through the build's own extension tooling (into the profile's local
    /// `user_default` repository), a legacy add-on through Blender's add-on installer (into
    /// the profile's `scripts/addons`). Preferences are saved so the add-on stays enabled.
    pub fn install(&self, sp: &Spindle, build: &Build, profile: &Profile, rec: &AddonRecord) -> Result<String> {
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
}
