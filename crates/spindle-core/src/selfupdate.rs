//! Spindle updating itself from its GitHub releases.
//!
//! Each release carries one archive per platform (`spindle-v<ver>-<os>-<arch>.zip` or
//! `.tar.gz`, holding `spindle`, `spindle_app` and `spindle-mcp`) and a `SHA256SUMS` file.
//! Installing downloads the archive for this platform, checks it against `SHA256SUMS`,
//! unpacks it, and swaps each binary next to the running one: the old file is renamed to
//! `<name>.old` (Windows lets a running program be renamed, not overwritten) and the new
//! one takes its place. `cleanup()` removes the `.old` files on the next start.

use crate::Spindle;
use crate::downloads::{client, expected_sha256, get_text};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

pub const REPO: &str = "deepfold9118/spindle";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// Programs a release archive carries (without the platform's .exe suffix).
pub const BINARIES: [&str; 3] = ["spindle", "spindle_app", "spindle-mcp"];
/// Documents shipped next to the programs, refreshed with them.
const DOCS: [&str; 4] = ["LICENSE", "NOTICE", "THIRD-PARTY-LICENSES.html", "README.md"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    /// "0.2.0" (the tag without its leading "v").
    pub version: String,
    pub tag: String,
    /// Release page.
    pub url: String,
    pub notes: String,
    pub assets: Vec<Asset>,
    /// Unix seconds when this was fetched (for the app's daily check).
    #[serde(default)]
    pub fetched: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// The newest published release.
pub fn latest() -> Result<Release> {
    let client = client()?;
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .context("asking GitHub for the latest Spindle release")?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("no Spindle release published yet");
    }
    if !resp.status().is_success() {
        bail!("GitHub answered {}", resp.status());
    }
    let gh: GhRelease = serde_json::from_str(&resp.text()?).context("reading the release")?;
    if gh.draft || gh.prerelease {
        bail!("the latest release is a draft or pre-release");
    }
    Ok(Release {
        version: gh.tag_name.trim_start_matches('v').to_owned(),
        tag: gh.tag_name,
        url: gh.html_url,
        notes: gh.body.unwrap_or_default(),
        assets: gh.assets.into_iter().map(|a| Asset { name: a.name, url: a.browser_download_url, size: a.size }).collect(),
        fetched: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0),
    })
}

/// True when `release` is newer than the running Spindle.
pub fn is_newer(release: &Release) -> bool {
    crate::version::compare(&release.version, CURRENT).is_gt()
}

/// "windows-x64", "linux-x64", "macos-arm64", ...
pub fn platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("windows-x64"),
        ("linux", "x86_64") => Some("linux-x64"),
        ("macos", "aarch64") => Some("macos-arm64"),
        ("macos", "x86_64") => Some("macos-x64"),
        _ => None,
    }
}

/// The archive of `release` for this platform.
pub fn asset_for_platform(release: &Release) -> Option<&Asset> {
    let p = platform()?;
    release.assets.iter().find(|a| {
        a.name.starts_with("spindle-") && a.name.contains(&format!("-{p}.")) && (a.name.ends_with(".zip") || a.name.ends_with(".tar.gz"))
    })
}

/// The folder holding the running Spindle programs, refusing development builds
/// (`target/debug`, `target/release`), which cargo owns.
pub fn install_dir() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let dir = exe.parent().context("no folder for the running program")?.to_path_buf();
    let is_cargo = dir.parent().and_then(Path::file_name).is_some_and(|n| n == "target")
        || dir.ancestors().nth(2).and_then(Path::file_name).is_some_and(|n| n == "target");
    if is_cargo && std::env::var_os("SPINDLE_SELF_UPDATE_ANY_DIR").is_none() {
        bail!("{} is a development build (cargo's target folder); update it with git and cargo", dir.display());
    }
    Ok(dir)
}

/// Download, verify and install `release` into `dir` (normally `install_dir()`).
/// Returns the files replaced. `on_progress(done, total)` reports the download.
pub fn install(sp: &Spindle, release: &Release, dir: &Path, mut on_progress: impl FnMut(u64, u64)) -> Result<Vec<PathBuf>> {
    let asset = asset_for_platform(release)
        .with_context(|| format!("release {} has no archive for {}", release.tag, platform().unwrap_or("this platform")))?;
    let sums = release
        .assets
        .iter()
        .find(|a| a.name == "SHA256SUMS")
        .with_context(|| format!("release {} has no SHA256SUMS", release.tag))?;
    let client = client()?;
    let want = expected_sha256(&get_text(&client, &sums.url)?, &asset.name)?;

    let work = sp.run_dir().join("self-update");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work)?;
    let result = (|| -> Result<Vec<PathBuf>> {
        let archive = work.join(&asset.name);
        let mut resp = client.get(&asset.url).send().with_context(|| format!("downloading {}", asset.name))?;
        if !resp.status().is_success() {
            bail!("{} answered {}", asset.url, resp.status());
        }
        let total = resp.content_length().unwrap_or(asset.size);
        let mut out = BufWriter::new(File::create(&archive)?);
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        let mut done = 0u64;
        loop {
            let n = resp.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            hasher.update(&buf[..n]);
            done += n as u64;
            on_progress(done, total);
        }
        out.flush()?;
        drop(out);
        let got = hex::encode(hasher.finalize());
        if got != want {
            bail!("SHA-256 mismatch for {}: expected {want}, got {got}", asset.name);
        }
        let unpacked = work.join("unpacked");
        fs::create_dir_all(&unpacked)?;
        extract(&archive, &unpacked)?;
        replace_files(&unpacked, dir)
    })();
    let _ = fs::remove_dir_all(&work);
    result
}

fn extract(archive: &Path, dest: &Path) -> Result<()> {
    let name = archive.to_string_lossy().to_lowercase();
    if name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(File::open(archive)?).context("opening the update archive")?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i)?;
            let Some(rel) = entry.enclosed_name() else { continue };
            let path = dest.join(rel);
            if entry.is_dir() {
                fs::create_dir_all(&path)?;
                continue;
            }
            if let Some(p) = path.parent() {
                fs::create_dir_all(p)?;
            }
            std::io::copy(&mut entry, &mut File::create(&path)?)?;
            #[cfg(unix)]
            if let Some(mode) = entry.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(mode))?;
            }
        }
        Ok(())
    } else {
        let status = std::process::Command::new("tar").arg("-xzf").arg(archive).arg("-C").arg(dest).status()?;
        if !status.success() {
            bail!("tar couldn't unpack the update ({status})");
        }
        Ok(())
    }
}

/// Find `name` anywhere under `root` (archives may or may not have a top folder).
fn find_file(root: &Path, name: &str) -> Option<PathBuf> {
    for e in fs::read_dir(root).ok()?.filter_map(|e| e.ok()) {
        let p = e.path();
        if p.is_dir() {
            if let Some(f) = find_file(&p, name) {
                return Some(f);
            }
        } else if p.file_name().is_some_and(|n| n == name) {
            return Some(p);
        }
    }
    None
}

fn replace_files(unpacked: &Path, dir: &Path) -> Result<Vec<PathBuf>> {
    let exe = std::env::consts::EXE_SUFFIX;
    let programs: Vec<String> = BINARIES.iter().map(|b| format!("{b}{exe}")).collect();
    if find_file(unpacked, &programs[0]).is_none() {
        bail!("the update archive has no {}", programs[0]);
    }
    let mut replaced = Vec::new();
    for name in programs.iter().map(String::as_str).chain(DOCS) {
        let Some(new) = find_file(unpacked, name) else { continue };
        let target = dir.join(name);
        if target.exists() {
            let old = dir.join(format!("{name}.old"));
            let _ = fs::remove_file(&old);
            fs::rename(&target, &old).with_context(|| format!("moving {} aside", target.display()))?;
        }
        if let Err(e) = fs::copy(&new, &target) {
            // Put the old file back so the install keeps working.
            let _ = fs::rename(dir.join(format!("{name}.old")), &target);
            return Err(e).with_context(|| format!("installing {}", target.display()));
        }
        replaced.push(target);
    }
    Ok(replaced)
}

/// Remove `<name>.old` files left by an update (they can't be deleted while running).
pub fn cleanup() {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(dir) = exe.parent() else { return };
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.filter_map(|e| e.ok()) {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(base) = name.strip_suffix(".old") {
            let known = BINARIES.iter().any(|b| base == *b || base == format!("{b}.exe")) || DOCS.contains(&base);
            if known {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}

/// The last release fetched by the app's daily check (`run/spindle-release.json`).
pub fn cached(sp: &Spindle) -> Option<Release> {
    fs::read(sp.run_dir().join("spindle-release.json")).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

pub fn store(sp: &Spindle, release: &Release) {
    let _ = fs::create_dir_all(sp.run_dir());
    if let Ok(json) = serde_json::to_vec_pretty(release) {
        let _ = fs::write(sp.run_dir().join("spindle-release.json"), json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(version: &str, names: &[&str]) -> Release {
        Release {
            version: version.into(),
            tag: format!("v{version}"),
            url: String::new(),
            notes: String::new(),
            assets: names.iter().map(|n| Asset { name: (*n).into(), url: format!("https://x/{n}"), size: 1 }).collect(),
            fetched: 0,
        }
    }

    #[test]
    fn newer_and_platform_assets() {
        assert!(is_newer(&rel("99.0.0", &[])));
        assert!(!is_newer(&rel(CURRENT, &[])));
        assert!(!is_newer(&rel("0.0.1", &[])));
        let r = rel(
            "0.2.0",
            &[
                "spindle-v0.2.0-windows-x64.zip",
                "spindle-v0.2.0-linux-x64.tar.gz",
                "spindle-v0.2.0-macos-arm64.tar.gz",
                "spindle-v0.2.0-macos-x64.tar.gz",
                "SHA256SUMS",
            ],
        );
        let a = asset_for_platform(&r).unwrap();
        assert!(a.name.contains(platform().unwrap()));
    }

    #[test]
    fn replaces_files_and_keeps_old() {
        let base = std::env::temp_dir().join(format!("spindle-selfupdate-test-{}", std::process::id()));
        let (src, dst) = (base.join("unpacked/spindle-v9-x"), base.join("install"));
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();
        let exe = std::env::consts::EXE_SUFFIX;
        for b in BINARIES {
            fs::write(src.join(format!("{b}{exe}")), "new").unwrap();
            fs::write(dst.join(format!("{b}{exe}")), "old").unwrap();
        }
        fs::write(src.join("NOTICE"), "notice").unwrap();
        let replaced = replace_files(&base.join("unpacked"), &dst).unwrap();
        assert_eq!(replaced.len(), 4);
        assert_eq!(fs::read_to_string(dst.join(format!("spindle{exe}"))).unwrap(), "new");
        assert_eq!(fs::read_to_string(dst.join(format!("spindle{exe}.old"))).unwrap(), "old");
        fs::remove_dir_all(&base).unwrap();
    }
}
