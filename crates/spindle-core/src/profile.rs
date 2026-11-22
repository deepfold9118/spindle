use crate::Spindle;
use crate::builds::Build;
use anyhow::{Context, Result, bail};
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

/// The bridge script, installed into every profile's `scripts/startup` so it runs
/// in every Blender that Spindle launches, regardless of which add-ons are enabled.
/// Oldest series verified to honour `BLENDER_USER_RESOURCES` (3.6 does, 2.93 doesn't).
pub const USER_RESOURCES_SINCE: (u32, u32) = (3, 6);

const BRIDGE_SCRIPT: &str = include_str!("../../../bridge/spindle_bridge.py");

/// A shared Blender user-resources folder (same layout as
/// `%APPDATA%/Blender Foundation/Blender/<version>`: config/, scripts/, extensions/, ...).
/// Every build launched with a profile reads and writes its preferences and add-ons here.
#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub root: PathBuf,
}

impl Profile {
    /// Open (creating if needed) a profile and make sure the bridge is installed.
    pub fn open(sp: &Spindle, name: &str) -> Result<Self> {
        if name.is_empty() || name.contains(['/', '\\', '.']) {
            bail!("invalid profile name '{name}'");
        }
        let root = sp.profiles_dir().join(name);
        for sub in ["config", "scripts/startup", "extensions"] {
            fs::create_dir_all(root.join(sub)).with_context(|| format!("creating profile {}", root.display()))?;
        }
        let profile = Self { name: name.to_owned(), root };
        profile.install_bridge()?;
        Ok(profile)
    }

    pub fn list(sp: &Spindle) -> Result<Vec<String>> {
        let dir = sp.profiles_dir();
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut names: Vec<String> = fs::read_dir(dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        Ok(names)
    }

    pub fn bridge_path(&self) -> PathBuf {
        self.root.join("scripts").join("startup").join("spindle_bridge.py")
    }

    fn install_bridge(&self) -> Result<()> {
        let path = self.bridge_path();
        if fs::read_to_string(&path).ok().as_deref() != Some(BRIDGE_SCRIPT) {
            fs::write(&path, BRIDGE_SCRIPT).with_context(|| format!("writing {}", path.display()))?;
        }
        Ok(())
    }

    /// Environment that points Blender at this profile and tells the bridge where to report.
    /// `BLENDER_USER_RESOURCES` redirects all user files (and beats a `portable/` folder),
    /// so every build sees the same preferences, add-ons and extensions. Builds older than
    /// 3.6 ignore it (2.93 was verified to), so they also get the per-folder variables.
    pub fn env(&self, sp: &Spindle, build: &Build) -> Vec<(&'static str, OsString)> {
        let mut env = vec![
            ("BLENDER_USER_RESOURCES", self.root.clone().into_os_string()),
            ("SPINDLE_PROFILE", self.name.clone().into()),
            ("SPINDLE_INSTANCES_DIR", sp.instances_dir().into_os_string()),
        ];
        if build.series().is_none_or(|s| s < USER_RESOURCES_SINCE) {
            env.push(("BLENDER_USER_CONFIG", self.root.join("config").into_os_string()));
            env.push(("BLENDER_USER_SCRIPTS", self.root.join("scripts").into_os_string()));
            env.push(("BLENDER_USER_DATAFILES", self.root.join("datafiles").into_os_string()));
        }
        env
    }
}
