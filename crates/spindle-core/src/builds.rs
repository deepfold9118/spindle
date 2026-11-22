use crate::Spindle;
use anyhow::{Result, bail};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct Build {
    pub name: String,
    pub executable: PathBuf,
    /// Parsed from the folder name, e.g. "blender-5.3.0-stable.02f0899d78e3" -> "5.3.0".
    pub version: Option<String>,
    /// True when registered from outside the library (config `builds` table).
    pub external: bool,
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
            if exe.is_file() {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                builds.push(Build { version: version_from_name(&name), name, executable: exe, external: false });
            }
        }
    }
    for (name, exe) in &sp.config.builds {
        builds.push(Build {
            name: name.clone(),
            executable: exe.clone(),
            version: version_from_name(name).or_else(|| exe.parent().and_then(|p| version_from_name(&p.to_string_lossy()))),
            external: true,
        });
    }
    builds.sort_by(|a, b| {
        crate::version::compare(b.version.as_deref().unwrap_or(""), a.version.as_deref().unwrap_or(""))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(builds)
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

fn version_from_name(name: &str) -> Option<String> {
    let last = Path::new(name).file_name()?.to_string_lossy().into_owned();
    last.split(['-', ' '])
        .find(|part| part.chars().next().is_some_and(|c| c.is_ascii_digit()) && part.contains('.'))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::version_from_name;

    #[test]
    fn parses_launcher_folder_names() {
        assert_eq!(version_from_name("blender-5.3.0-stable.02f0899d78e3").as_deref(), Some("5.3.0"));
        assert_eq!(version_from_name("Blender 5.2").as_deref(), Some("5.2"));
        assert_eq!(version_from_name("custom"), None);
    }
}
