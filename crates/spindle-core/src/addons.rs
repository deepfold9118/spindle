use crate::builds::Build;
use crate::profile::Profile;
use crate::{Spindle, launch, version};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// One stored archive in Spindle's add-on library:
/// `<library>/addons/<source>/<id>/<version>/{<archive>, addon.json}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddonRecord {
    pub id: String,
    pub name: String,
    pub version: String,
    /// Extension type: "add-on" or "theme".
    pub kind: String,
    /// Where it came from, e.g. "superhive".
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
        out.sort_by(|a, b| {
            a.id.to_lowercase()
                .cmp(&b.id.to_lowercase())
                .then_with(|| version::compare(&b.version, &a.version))
        });
        Ok(out)
    }

    /// Newest stored version of an add-on, from any source.
    pub fn latest(&self, id: &str) -> Result<Option<AddonRecord>> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|r| r.id.eq_ignore_ascii_case(id))
            .max_by(|a, b| version::compare(&a.version, &b.version)))
    }

    /// Install a stored archive into a profile's local `user_default` repository,
    /// using the given build's own extension tooling, and enable it.
    pub fn install(&self, sp: &Spindle, build: &Build, profile: &Profile, rec: &AddonRecord) -> Result<String> {
        let archive = self.archive_path(rec);
        let args = [
            "--command",
            "extension",
            "install-file",
            "--repo",
            "user_default",
            "--enable",
            &archive.to_string_lossy(),
        ]
        .map(str::to_owned);
        let out = launch::run_background(sp, build, profile, &args)?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

fn read_dirs(dir: &std::path::Path) -> Result<Vec<PathBuf>> {
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
