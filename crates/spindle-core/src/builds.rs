use crate::Spindle;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Release channel of a build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Lts,
    Stable,
    Daily,
    Experimental,
    Custom,
}

impl Channel {
    /// Best guess from a folder name, for builds Spindle didn't download itself.
    pub fn from_name(name: &str) -> Self {
        let n = name.to_lowercase();
        if n.contains("lts") {
            Channel::Lts
        } else if n.contains("+pr") || n.contains("-pr") || n.contains("experimental") || n.contains("patch") {
            Channel::Experimental
        } else if n.contains("alpha") || n.contains("beta") || n.contains("daily") {
            Channel::Daily
        } else if n.contains("stable") || n.contains("candidate") {
            Channel::Stable
        } else {
            Channel::Custom
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Channel::Lts => "LTS",
            Channel::Stable => "Stable",
            Channel::Daily => "Daily",
            Channel::Experimental => "Experimental",
            Channel::Custom => "Custom",
        }
    }
}

/// Written into a build folder by `downloads::install`, so Spindle knows where it came from.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BuildMeta {
    pub channel: Option<Channel>,
    pub version: String,
    /// Builder risk/release cycle: alpha, beta, candidate, stable, lts.
    pub cycle: String,
    /// Source branch (`main`, `v52`, an experimental branch or PR) or the release series.
    pub branch: String,
    pub hash: String,
    pub url: String,
    /// Unix seconds of the upstream build.
    pub published: Option<i64>,
    /// Unix seconds when Spindle installed it.
    pub installed: Option<i64>,
}

pub const META_FILE: &str = "spindle-build.json";

#[derive(Debug, Clone, Serialize)]
pub struct Build {
    pub name: String,
    pub executable: PathBuf,
    /// From Spindle's metadata, else parsed from the folder name, e.g.
    /// "blender-5.3.0-stable.02f0899d78e3" -> "5.3.0", else the build's `X.Y` data folder.
    pub version: Option<String>,
    /// True when registered from outside the library (config `builds` table).
    pub external: bool,
    pub channel: Channel,
    /// Present for builds Spindle downloaded.
    pub meta: Option<BuildMeta>,
    /// Starred by the user (config `favourites`).
    pub favourite: bool,
}

impl Build {
    /// The build folder: the one holding `blender(.exe)`, or `Blender.app` on macOS.
    pub fn root(&self) -> &Path {
        build_root(&self.executable)
    }

    /// (major, minor) of the build, from its version or its `X.Y` data folder.
    pub fn series(&self) -> Option<(u32, u32)> {
        self.version.as_deref().and_then(parse_series).or_else(|| data_folder_series(self.root()))
    }
}

/// Build folder of an executable: its parent, or the folder holding `Blender.app`
/// (`<root>/Blender.app/Contents/MacOS/Blender`).
pub fn build_root(exe: &Path) -> &Path {
    let up = |n: usize| exe.ancestors().nth(n);
    let in_bundle = up(1).and_then(Path::file_name).is_some_and(|n| n == "MacOS")
        && up(3).and_then(Path::extension).is_some_and(|e| e == "app");
    let root = if in_bundle { up(4) } else { up(1) };
    root.unwrap_or(Path::new("."))
}

pub fn executable_name() -> &'static str {
    if cfg!(windows) {
        "blender.exe"
    } else if cfg!(target_os = "macos") {
        "Blender.app/Contents/MacOS/Blender"
    } else {
        "blender"
    }
}

/// All builds: folders in `<library>/builds` containing a Blender executable,
/// plus externally registered ones.
pub fn list(sp: &Spindle) -> Result<Vec<Build>> {
    let mut builds = Vec::new();
    let dir = sp.builds_dir();
    if dir.is_dir() {
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            let exe = path.join(executable_name());
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if exe.is_file() && !name.starts_with('.') {
                builds.push(from_folder(name, exe, false));
            }
        }
    }
    for (name, exe) in &sp.config.builds {
        let mut b = from_folder(name.clone(), exe.clone(), true);
        if b.version.is_none() {
            b.version = version_from_name(&build_root(exe).to_string_lossy());
        }
        builds.push(b);
    }
    let fav = |name: &str| sp.config.favourites.iter().position(|f| f == name);
    for b in &mut builds {
        b.favourite = fav(&b.name).is_some();
    }
    // Favourites first (in the order they were starred), then newest version first.
    builds.sort_by(|a, b| {
        fav(&a.name).unwrap_or(usize::MAX).cmp(&fav(&b.name).unwrap_or(usize::MAX)).then_with(|| {
            crate::version::compare(b.version.as_deref().unwrap_or(""), a.version.as_deref().unwrap_or(""))
                .then_with(|| a.name.cmp(&b.name))
        })
    });
    Ok(builds)
}

/// The build to launch when none is named: the first favourite, else the newest build.
pub fn default_build(sp: &Spindle) -> Result<Build> {
    list(sp)?.into_iter().next().context("no builds yet (download one with `spindle builds download <version>`)")
}

/// Star or unstar a build. Starring appends it to the favourites; to make a build the
/// default, unstar the others or star it first.
pub fn set_favourite(sp: &mut Spindle, name: &str, on: bool) -> Result<()> {
    let favs = &mut sp.config.favourites;
    let had = favs.iter().any(|f| f == name);
    if on && !had {
        favs.push(name.to_owned());
    } else if !on && had {
        favs.retain(|f| f != name);
    } else {
        return Ok(());
    }
    sp.save()
}

fn from_folder(name: String, exe: PathBuf, external: bool) -> Build {
    let root = build_root(&exe).to_path_buf();
    let meta: Option<BuildMeta> =
        fs::read(root.join(META_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let version = meta
        .as_ref()
        .map(|m| m.version.clone())
        .filter(|v| !v.is_empty())
        .or_else(|| version_from_name(&name))
        .or_else(|| data_folder_series(&root).map(|(a, b)| format!("{a}.{b}")));
    let channel = meta.as_ref().and_then(|m| m.channel).unwrap_or_else(|| Channel::from_name(&name));
    Build { name, executable: exe, version, external, channel, meta, favourite: false }
}

/// Find a build by exact name, or by a query that matches exactly one build
/// (version prefix like "5.3", or a substring of the name).
pub fn find(sp: &Spindle, query: &str) -> Result<Build> {
    let builds = list(sp)?;
    if let Some(b) = builds.iter().find(|b| b.name == query) {
        return Ok(b.clone());
    }
    let matches: Vec<&Build> = builds
        .iter()
        .filter(|b| {
            b.version.as_deref().is_some_and(|v| v == query || v.starts_with(&format!("{query}.")))
                || b.name.contains(query)
        })
        .collect();
    match matches.as_slice() {
        [one] => Ok((*one).clone()),
        [] => bail!("no build matches '{query}' (try `spindle builds list`)"),
        many => bail!(
            "'{query}' matches several builds: {}",
            many.iter().map(|b| b.name.as_str()).collect::<Vec<_>>().join(", ")
        ),
    }
}

pub fn register_external(sp: &mut Spindle, name: &str, executable: &Path) -> Result<()> {
    if !executable.is_file() {
        bail!("{} is not a file", executable.display());
    }
    sp.config.builds.insert(name.to_owned(), executable.to_path_buf());
    sp.save()
}

/// What `remove` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removed {
    /// The build folder went to the Recycle Bin / Trash.
    Trashed,
    /// An external build was forgotten; its files were left alone.
    Unregistered,
}

/// Remove a build. Library builds go to the Recycle Bin (recoverable); external builds are
/// only unregistered. Refuses while a Blender from that build is running.
pub fn remove(sp: &mut Spindle, build: &Build) -> Result<Removed> {
    let exe = build.executable.to_string_lossy().to_lowercase();
    if let Some(i) = crate::instances::list(sp)?.iter().find(|i| i.binary.to_lowercase() == exe) {
        bail!("Blender {} from this build is running (pid {}); close it first", i.blender_version, i.pid);
    }
    if build.external {
        sp.config.builds.remove(&build.name);
        sp.config.favourites.retain(|f| f != &build.name);
        sp.save()?;
        return Ok(Removed::Unregistered);
    }
    let root = build.root().to_path_buf();
    let builds_dir = sp.builds_dir();
    if root.parent() != Some(builds_dir.as_path()) {
        bail!("{} is not inside the library's builds folder", root.display());
    }
    trash::delete(&root).with_context(|| format!("moving {} to the Recycle Bin", root.display()))?;
    if sp.config.favourites.iter().any(|f| f == &build.name) {
        sp.config.favourites.retain(|f| f != &build.name);
        sp.save()?;
    }
    Ok(Removed::Trashed)
}

pub fn version_from_name(name: &str) -> Option<String> {
    let last = Path::new(name).file_name()?.to_string_lossy().into_owned();
    last.split(['-', ' '])
        .find(|part| part.chars().next().is_some_and(|c| c.is_ascii_digit()) && part.contains('.'))
        .map(|part| part.split('+').next().unwrap_or(part).to_owned())
}

/// "4.2.9" -> (4, 2)
pub fn parse_series(v: &str) -> Option<(u32, u32)> {
    let mut it = v.split('.');
    let major = it.next()?.trim().parse().ok()?;
    let minor: String = it.next()?.chars().take_while(|c| c.is_ascii_digit()).collect();
    Some((major, minor.parse().ok()?))
}

/// Every Blender build has a data folder named after its series ("5.3/"), next to the
/// executable or, on macOS, in `Blender.app/Contents/Resources`.
fn data_folder_series(root: &Path) -> Option<(u32, u32)> {
    data_folder(root).and_then(|d| parse_series(&d.file_name()?.to_string_lossy()))
}

/// The build's `X.Y` data folder (scripts, datafiles, python).
pub fn data_folder(root: &Path) -> Option<PathBuf> {
    [root.to_path_buf(), root.join("Blender.app/Contents/Resources")].into_iter().find_map(|dir| {
        fs::read_dir(dir)
            .ok()?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| parse_series(&e.file_name().to_string_lossy()).map(|s| (s, e.path())))
            .max_by_key(|(s, _)| *s)
            .map(|(_, p)| p)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_launcher_folder_names() {
        assert_eq!(version_from_name("blender-5.3.0-stable.02f0899d78e3").as_deref(), Some("5.3.0"));
        assert_eq!(version_from_name("blender-5.4.0-alpha+main.abc").as_deref(), Some("5.4.0"));
        assert_eq!(version_from_name("Blender 5.2").as_deref(), Some("5.2"));
        assert_eq!(version_from_name("custom"), None);
        assert_eq!(parse_series("4.2.9"), Some((4, 2)));
        assert_eq!(parse_series("2.93"), Some((2, 93)));
    }

    #[test]
    fn build_roots() {
        assert_eq!(build_root(Path::new("/l/builds/b-5.3/blender")), Path::new("/l/builds/b-5.3"));
        assert_eq!(build_root(Path::new("/l/builds/b-5.3/Blender.app/Contents/MacOS/Blender")), Path::new("/l/builds/b-5.3"));
        assert_eq!(build_root(Path::new(r"C:\b\blender-5.3\blender.exe")), Path::new(r"C:\b\blender-5.3"));
    }

    #[test]
    fn channel_from_names() {
        assert_eq!(Channel::from_name("blender-5.2.0-lts.fbe6228777e7"), Channel::Lts);
        assert_eq!(Channel::from_name("blender-5.3.0-stable.02f0899d78e3"), Channel::Stable);
        assert_eq!(Channel::from_name("blender-5.4.0-alpha+main.abc"), Channel::Daily);
        assert_eq!(Channel::from_name("blender-5.2.3-candidate+main-PR164298.0ef9"), Channel::Experimental);
        assert_eq!(Channel::from_name("my build"), Channel::Custom);
    }
}
