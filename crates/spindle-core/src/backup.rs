//! Backups of a profile's preferences: zip archives in `<library>/backups/<profile>/`.
//!
//! A preferences backup holds the profile's `config/` folder (userpref.blend, startup.blend,
//! bookmarks, recent files). A full backup holds the whole profile, add-ons and extensions
//! included. Restoring first backs up what it replaces, so a restore can itself be undone.
//! Spindle also backs up the preferences automatically before an older Blender opens them,
//! since that Blender can drop (or, before 4.5, wipe) settings it doesn't know when it saves.

use crate::Spindle;
use crate::blendfile;
use crate::builds::Build;
use crate::compat::BlendRead;
use crate::instances;
use crate::profile::Profile;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const META: &str = "spindle-backup.json";
/// Automatic backups kept per profile; manual ones are kept until removed.
pub const KEEP_AUTOMATIC: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Manual,
    Automatic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub profile: String,
    /// Unix seconds.
    pub created: i64,
    pub kind: Kind,
    /// Add-ons and extensions included (the whole profile), not just `config/`.
    pub full: bool,
    pub note: String,
    /// The Blender that last saved userpref.blend, e.g. "5.3".
    pub saved_by: Option<String>,
    pub spindle: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Backup {
    /// File stem, e.g. `default-20261122-153012`; what `restore` takes.
    pub id: String,
    pub path: PathBuf,
    pub size: u64,
    #[serde(flatten)]
    pub meta: Meta,
}

impl Backup {
    /// "2026-11-22 15:30" in local time.
    pub fn when(&self) -> String {
        local(self.meta.created, "%Y-%m-%d %H:%M")
    }
}

pub fn backups_dir(sp: &Spindle) -> PathBuf {
    sp.library.join("backups")
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn local(secs: i64, fmt: &str) -> String {
    match jiff::Timestamp::from_second(secs) {
        Ok(t) => t.to_zoned_datetime(jiff::tz::TimeZone::system()).strftime(fmt).to_string(),
        Err(_) => secs.to_string(),
    }
}

/// The Blender series that last saved the profile's preferences.
pub fn prefs_saved_by(profile_root: &Path) -> Option<(u32, u32)> {
    match blendfile::read_header(&profile_root.join("config").join("userpref.blend")).ok()? {
        BlendRead::Header(h) => Some(h.series),
        _ => None,
    }
}

/// Back up a profile's preferences (`full`: the whole profile).
pub fn create(sp: &Spindle, profile: &Profile, full: bool, kind: Kind, note: &str) -> Result<Backup> {
    let dir = backups_dir(sp).join(&profile.name);
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let created = now();
    let stamp = local(created, "%Y%m%d-%H%M%S");
    let mut id = format!("{}-{stamp}", profile.name);
    let mut n = 2;
    while dir.join(format!("{id}.zip")).exists() {
        id = format!("{}-{stamp}-{n}", profile.name);
        n += 1;
    }
    let path = dir.join(format!("{id}.zip"));
    let meta = Meta {
        profile: profile.name.clone(),
        created,
        kind,
        full,
        note: note.to_owned(),
        saved_by: prefs_saved_by(&profile.root).map(|s| format!("{}.{}", s.0, s.1)),
        spindle: env!("CARGO_PKG_VERSION").to_owned(),
    };
    let part = path.with_extension("zip.part");
    let written = (|| -> Result<()> {
        let mut zip = zip::ZipWriter::new(BufWriter::new(File::create(&part)?));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .large_file(full);
        zip.start_file(META, opts)?;
        zip.write_all(&serde_json::to_vec_pretty(&meta)?)?;
        if full {
            add_tree(&mut zip, &profile.root, &profile.root, opts)?;
        } else {
            let config = profile.root.join("config");
            if config.is_dir() {
                add_tree(&mut zip, &profile.root, &config, opts)?;
            }
        }
        zip.finish()?.flush()?;
        Ok(())
    })();
    if let Err(e) = written {
        let _ = fs::remove_file(&part);
        return Err(e).with_context(|| format!("writing {}", path.display()));
    }
    fs::rename(&part, &path)?;
    if kind == Kind::Automatic {
        prune(sp, &profile.name);
    }
    let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    Ok(Backup { id, path, size, meta })
}

fn add_tree<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    base: &Path,
    dir: &Path,
    opts: zip::write::SimpleFileOptions,
) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        let rel = path.strip_prefix(base)?.to_string_lossy().replace('\\', "/");
        let ty = e.file_type()?;
        if ty.is_dir() {
            if rel == "__pycache__" || rel.ends_with("/__pycache__") {
                continue;
            }
            zip.add_directory(format!("{rel}/"), opts)?;
            add_tree(zip, base, &path, opts)?;
        } else if ty.is_file() {
            zip.start_file(rel, opts)?;
            std::io::copy(&mut File::open(&path).with_context(|| format!("reading {}", path.display()))?, zip)?;
        }
    }
    Ok(())
}

fn read_meta(path: &Path) -> Result<Meta> {
    let mut zip = zip::ZipArchive::new(File::open(path)?)?;
    let entry = zip.by_name(META).context("not a Spindle backup")?;
    Ok(serde_json::from_reader(entry)?)
}

/// Backups, newest first; of one profile, or of all of them.
pub fn list(sp: &Spindle, profile: Option<&str>) -> Result<Vec<Backup>> {
    let root = backups_dir(sp);
    let dirs: Vec<PathBuf> = match profile {
        Some(p) => vec![root.join(p)],
        None => match fs::read_dir(&root) {
            Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect(),
            Err(_) => Vec::new(),
        },
    };
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.filter_map(|e| e.ok()) {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "zip") {
                continue;
            }
            let Ok(meta) = read_meta(&path) else { continue };
            let id = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            out.push(Backup { id, path, size, meta });
        }
    }
    out.sort_by(|a, b| b.meta.created.cmp(&a.meta.created).then_with(|| b.id.cmp(&a.id)));
    Ok(out)
}

/// A backup by id (or the newest one of a profile, given the profile's name).
pub fn find(sp: &Spindle, query: &str) -> Result<Backup> {
    let all = list(sp, None)?;
    if let Some(b) = all.iter().find(|b| b.id == query) {
        return Ok(b.clone());
    }
    if let Some(b) = all.iter().find(|b| b.meta.profile == query) {
        return Ok(b.clone());
    }
    bail!("no backup '{query}' (see `spindle profiles backups`)")
}

/// Keep the newest `KEEP_AUTOMATIC` automatic backups of a profile.
fn prune(sp: &Spindle, profile: &str) {
    let Ok(all) = list(sp, Some(profile)) else { return };
    for b in all.iter().filter(|b| b.meta.kind == Kind::Automatic).skip(KEEP_AUTOMATIC) {
        let _ = fs::remove_file(&b.path);
    }
}

/// Move a backup to the Recycle Bin / Trash.
pub fn remove(backup: &Backup) -> Result<()> {
    trash::delete(&backup.path).with_context(|| format!("moving {} to the Recycle Bin", backup.path.display()))
}

/// Blenders running with `profile` right now.
pub fn running_with(sp: &Spindle, profile: &str) -> Vec<u32> {
    instances::list(sp)
        .unwrap_or_default()
        .into_iter()
        .filter(|i| i.profile.as_deref() == Some(profile))
        .map(|i| i.pid)
        .collect()
}

/// Restore a backup into a profile (its own, unless `into` names another). What it replaces
/// is backed up first; that backup is returned. A preferences backup replaces `config/`; a
/// full one replaces the whole profile. Refused while a Blender runs with the profile (it
/// would write its preferences back on exit) unless `force`.
pub fn restore(sp: &Spindle, backup: &Backup, into: Option<&str>, force: bool) -> Result<Backup> {
    let target = into.unwrap_or(&backup.meta.profile);
    let running = running_with(sp, target);
    if !running.is_empty() && !force {
        bail!(
            "Blender is running with profile '{target}' (pid {}); close it first, or it saves its preferences over the restored ones",
            running.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")
        );
    }
    let profile = Profile::open(sp, target)?;
    let undo = create(sp, &profile, backup.meta.full, Kind::Automatic, &format!("Before restoring the backup from {}", backup.when()))?;

    // Unpack next to the profile first, so a damaged archive changes nothing.
    let staging = sp.profiles_dir().join(format!(".restore-{}", now()));
    let _ = fs::remove_dir_all(&staging);
    let unpacked = (|| -> Result<()> {
        let mut zip = zip::ZipArchive::new(File::open(&backup.path)?)?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i)?;
            let Some(rel) = entry.enclosed_name() else { continue };
            if rel.as_os_str() == META {
                continue;
            }
            let path = staging.join(rel);
            if entry.is_dir() {
                fs::create_dir_all(&path)?;
            } else {
                if let Some(p) = path.parent() {
                    fs::create_dir_all(p)?;
                }
                let mut f = BufWriter::new(File::create(&path)?);
                std::io::copy(&mut entry, &mut f)?;
                f.flush()?;
            }
        }
        fs::create_dir_all(staging.join("config"))?;
        Ok(())
    })();
    if let Err(e) = unpacked {
        let _ = fs::remove_dir_all(&staging);
        return Err(e).with_context(|| format!("unpacking {}", backup.path.display()));
    }

    // Swap in what was unpacked.
    let replace: Vec<PathBuf> = if backup.meta.full {
        fs::read_dir(&profile.root)?.filter_map(|e| e.ok()).map(|e| e.path()).collect()
    } else {
        vec![profile.root.join("config")]
    };
    let swapped = (|| -> Result<()> {
        for p in &replace {
            if p.is_dir() {
                fs::remove_dir_all(p).with_context(|| format!("removing {}", p.display()))?;
            } else if p.exists() {
                fs::remove_file(p).with_context(|| format!("removing {}", p.display()))?;
            }
        }
        for e in fs::read_dir(&staging)?.filter_map(|e| e.ok()) {
            let dest = profile.root.join(e.file_name());
            fs::rename(e.path(), &dest).with_context(|| format!("restoring {}", dest.display()))?;
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&staging);
    swapped.with_context(|| format!("restoring into profile '{target}' (what it replaced is in backup {})", undo.id))?;
    // Re-create the standard folders and the bridge script.
    Profile::open(sp, target)?;
    Ok(undo)
}

/// Before `build` opens a profile whose preferences a newer Blender saved, back them up
/// (unless they haven't changed since the last backup). Errors are returned, not fatal:
/// callers launch anyway.
pub fn before_launch(sp: &Spindle, build: &Build, profile: &Profile) -> Result<Option<Backup>> {
    let (Some(saved_by), Some(series)) = (prefs_saved_by(&profile.root), build.series()) else { return Ok(None) };
    if saved_by <= series {
        return Ok(None);
    }
    let prefs = profile.root.join("config").join("userpref.blend");
    let changed = fs::metadata(&prefs)?.modified()?.duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    if list(sp, Some(&profile.name))?.first().is_some_and(|b| b.meta.created >= changed) {
        return Ok(None);
    }
    let note = format!(
        "Before Blender {}.{} opened preferences saved by {}.{}",
        series.0, series.1, saved_by.0, saved_by.1
    );
    create(sp, profile, false, Kind::Automatic, &note).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn scratch(tag: &str) -> Spindle {
        let lib = std::env::temp_dir().join(format!("spindle-backup-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&lib);
        fs::create_dir_all(&lib).unwrap();
        Spindle { config_path: lib.join("config.toml"), config: Config::default(), library: lib }
    }

    #[test]
    fn backup_and_restore_round_trip() {
        let sp = scratch("roundtrip");
        let p = Profile::open(&sp, "default").unwrap();
        let prefs = p.root.join("config").join("userpref.blend");
        fs::write(&prefs, b"BLENDER-v405 original").unwrap();
        fs::create_dir_all(p.root.join("extensions/user_default/foo")).unwrap();
        fs::write(p.root.join("extensions/user_default/foo/x.py"), b"addon").unwrap();

        let b = create(&sp, &p, false, Kind::Manual, "first").unwrap();
        assert_eq!(b.meta.saved_by.as_deref(), Some("4.5"));
        assert!(!b.meta.full);

        fs::write(&prefs, b"BLENDER17-01v0503 changed").unwrap();
        fs::write(p.root.join("config").join("extra.txt"), b"new").unwrap();
        let undo = restore(&sp, &b, None, false).unwrap();
        assert_eq!(fs::read(&prefs).unwrap(), b"BLENDER-v405 original");
        assert!(!p.root.join("config/extra.txt").exists(), "restore replaces config/ entirely");
        assert!(p.root.join("extensions/user_default/foo/x.py").exists(), "a preferences restore leaves add-ons alone");
        assert!(p.bridge_path().exists());
        assert_eq!(undo.meta.kind, Kind::Automatic);
        assert_eq!(undo.meta.saved_by.as_deref(), Some("5.3"));

        // The undo backup puts the changed preferences back.
        restore(&sp, &undo, None, false).unwrap();
        assert_eq!(fs::read(&prefs).unwrap(), b"BLENDER17-01v0503 changed");
        assert_eq!(list(&sp, Some("default")).unwrap().len(), 3);
        assert_eq!(find(&sp, "default").unwrap().meta.kind, Kind::Automatic);
        let _ = fs::remove_dir_all(&sp.library);
    }

    #[test]
    fn full_backup_restores_addons_into_another_profile() {
        let sp = scratch("full");
        let p = Profile::open(&sp, "studio").unwrap();
        fs::write(p.root.join("config/userpref.blend"), b"BLENDER-v405").unwrap();
        fs::create_dir_all(p.root.join("scripts/addons/bar")).unwrap();
        fs::write(p.root.join("scripts/addons/bar/__init__.py"), b"bl_info = {}").unwrap();
        let b = create(&sp, &p, true, Kind::Manual, "").unwrap();
        restore(&sp, &b, Some("copy"), false).unwrap();
        let copy = sp.profiles_dir().join("copy");
        assert!(copy.join("scripts/addons/bar/__init__.py").exists());
        assert!(copy.join("config/userpref.blend").exists());
        assert!(copy.join("scripts/startup/spindle_bridge.py").exists());
        let _ = fs::remove_dir_all(&sp.library);
    }

    #[test]
    fn backs_up_before_an_older_build_opens_newer_preferences() {
        let sp = scratch("launch");
        let p = Profile::open(&sp, "default").unwrap();
        fs::write(p.root.join("config/userpref.blend"), b"BLENDER17-01v0503").unwrap();
        let build = |v: &str| Build {
            name: format!("blender-{v}"),
            executable: sp.library.join("none/blender.exe"),
            version: Some(v.to_owned()),
            external: true,
            channel: crate::builds::Channel::Custom,
            meta: None,
            favourite: false,
        };
        assert!(before_launch(&sp, &build("5.3.0"), &p).unwrap().is_none(), "same series: nothing to protect");
        let b = before_launch(&sp, &build("4.1.1"), &p).unwrap().expect("older build: backed up");
        assert_eq!(b.meta.kind, Kind::Automatic);
        assert!(b.meta.note.contains("4.1") && b.meta.note.contains("5.3"), "{}", b.meta.note);
        assert!(before_launch(&sp, &build("4.1.1"), &p).unwrap().is_none(), "unchanged since the last backup");
        let _ = fs::remove_dir_all(&sp.library);
    }

    #[test]
    fn automatic_backups_are_pruned() {
        let sp = scratch("prune");
        let p = Profile::open(&sp, "default").unwrap();
        create(&sp, &p, false, Kind::Manual, "keep me").unwrap();
        for _ in 0..KEEP_AUTOMATIC + 3 {
            create(&sp, &p, false, Kind::Automatic, "").unwrap();
        }
        let all = list(&sp, Some("default")).unwrap();
        assert_eq!(all.iter().filter(|b| b.meta.kind == Kind::Automatic).count(), KEEP_AUTOMATIC);
        assert_eq!(all.iter().filter(|b| b.meta.kind == Kind::Manual).count(), 1);
        let _ = fs::remove_dir_all(&sp.library);
    }
}
