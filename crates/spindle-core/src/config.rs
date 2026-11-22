use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

pub const DEFAULT_PROFILE: &str = "default";

/// Persistent settings, stored as TOML in the platform config directory
/// (override the location with `SPINDLE_CONFIG`).
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Root of the library (builds, add-ons, profiles). Defaults to the local data dir.
    pub library_dir: Option<PathBuf>,
    pub default_profile: Option<String>,
    /// Builds that live outside the library: name -> Blender executable.
    pub builds: BTreeMap<String, PathBuf>,
    pub superhive: SuperhiveConfig,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SuperhiveConfig {
    /// Blender extensions API v1 index URL of the Superhive repository.
    pub repo_url: String,
}

impl Default for SuperhiveConfig {
    fn default() -> Self {
        Self { repo_url: "https://superhivemarket.com/api/v1/extensions/".into() }
    }
}

/// Loaded configuration plus resolved library layout.
pub struct Spindle {
    pub config_path: PathBuf,
    pub config: Config,
    pub library: PathBuf,
}

impl Spindle {
    pub fn load() -> Result<Self> {
        let config_path = match std::env::var_os("SPINDLE_CONFIG") {
            Some(p) => PathBuf::from(p),
            None => project_dirs()?.config_dir().join("config.toml"),
        };
        let config: Config = if config_path.exists() {
            let text = fs::read_to_string(&config_path)
                .with_context(|| format!("reading {}", config_path.display()))?;
            toml::from_str(&text).with_context(|| format!("parsing {}", config_path.display()))?
        } else {
            Config::default()
        };
        let library = match &config.library_dir {
            Some(dir) => dir.clone(),
            None => project_dirs()?.data_local_dir().join("library"),
        };
        Ok(Self { config_path, config, library })
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.config_path, toml::to_string_pretty(&self.config)?)
            .with_context(|| format!("writing {}", self.config_path.display()))
    }

    pub fn builds_dir(&self) -> PathBuf {
        self.library.join("builds")
    }
    pub fn addons_dir(&self) -> PathBuf {
        self.library.join("addons")
    }
    pub fn profiles_dir(&self) -> PathBuf {
        self.library.join("profiles")
    }
    /// Heartbeat files written by the bridge script inside each running Blender.
    pub fn instances_dir(&self) -> PathBuf {
        self.library.join("run").join("instances")
    }

    pub fn profile_name(&self, explicit: Option<&str>) -> String {
        explicit
            .map(str::to_owned)
            .or_else(|| self.config.default_profile.clone())
            .unwrap_or_else(|| DEFAULT_PROFILE.to_owned())
    }
}

fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("", "", "Spindle").context("could not determine the user's home directory")
}
