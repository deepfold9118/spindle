//! Blender builds to download: official releases from download.blender.org (Stable and
//! LTS) and builder.blender.org's daily, experimental-branch and patch (PR) builds.
//! Installing streams the archive into `<library>/downloads`, checks its SHA-256, unpacks
//! it into `<library>/builds/<name>` and records where it came from (`spindle-build.json`).

use crate::Spindle;
use crate::builds::{self, Build, BuildMeta, Channel};
use crate::version;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const RELEASES: &str = "https://download.blender.org/release/";
const BUILDER: &str = "https://builder.blender.org/download/";

/// A build that can be downloaded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Available {
    pub channel: Channel,
    pub version: String,
    /// alpha, beta, candidate, stable or lts.
    pub cycle: String,
    /// Release series ("4.2") for releases; builder branch (`main`, `v52`, `main-PR1234`) otherwise.
    pub branch: String,
    pub hash: String,
    pub url: String,
    pub file_name: String,
    pub size: Option<u64>,
    /// Where the archive's SHA-256 is published.
    pub sha256_url: Option<String>,
    pub published: Option<i64>,
    /// Folder name it installs as under `<library>/builds`.
    pub name: String,
}

/// Everything downloadable, as cached in `<library>/run/available.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Catalog {
    /// Unix seconds when it was fetched.
    pub fetched: i64,
    pub builds: Vec<Available>,
    /// Sources that failed (the rest are still listed).
    pub errors: Vec<String>,
}

impl Catalog {
    pub fn age_secs(&self) -> i64 {
        now() - self.fetched
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub(crate) fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(concat!("Spindle/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(3 * 3600))
        .build()?)
}

pub(crate) fn get_text(client: &reqwest::blocking::Client, url: &str) -> Result<String> {
    let resp = client.get(url).send().with_context(|| format!("requesting {url}"))?;
    if !resp.status().is_success() {
        bail!("{url} answered {}", resp.status());
    }
    Ok(resp.text()?)
}

fn cache_path(sp: &Spindle) -> PathBuf {
    sp.run_dir().join("available.json")
}

/// The last fetched catalog, if any.
pub fn cached(sp: &Spindle) -> Option<Catalog> {
    serde_json::from_slice(&fs::read(cache_path(sp)).ok()?).ok()
}

/// Fetch every source (in parallel) and update the cache.
pub fn fetch(sp: &Spindle) -> Result<Catalog> {
    let client = client()?;
    let cfg = &sp.config.downloads;
    let (releases, daily, experimental, patch) = std::thread::scope(|s| {
        let r = s.spawn(|| fetch_releases(&client, &cfg.lts_series, cfg.release_series));
        let d = s.spawn(|| fetch_builder(&client, "daily"));
        let e = s.spawn(|| fetch_builder(&client, "experimental"));
        let p = s.spawn(|| fetch_builder(&client, "patch"));
        let join = |h: std::thread::ScopedJoinHandle<'_, Result<Vec<Available>>>| {
            h.join().unwrap_or_else(|_| Err(anyhow::anyhow!("fetch thread panicked")))
        };
        (join(r), join(d), join(e), join(p))
    });
    let mut cat = Catalog { fetched: now(), ..Default::default() };
    for (what, res) in [("releases", releases), ("daily builds", daily), ("experimental builds", experimental)] {
        match res {
            Ok(list) => cat.builds.extend(list),
            Err(e) => cat.errors.push(format!("{what}: {e:#}")),
        }
    }
    match patch {
        Ok(mut list) => {
            list.sort_by_key(|a| std::cmp::Reverse(a.published.unwrap_or(0)));
            list.truncate(cfg.experimental_builds);
            cat.builds.extend(list);
        }
        Err(e) => cat.errors.push(format!("patch builds: {e:#}")),
    }
    if cat.builds.is_empty() && !cat.errors.is_empty() {
        bail!("could not list Blender builds: {}", cat.errors.join("; "));
    }
    sort(&mut cat.builds);
    fs::create_dir_all(sp.run_dir())?;
    fs::write(cache_path(sp), serde_json::to_vec_pretty(&cat)?)?;
    Ok(cat)
}

/// Newest first; releases before builder builds of the same version.
fn sort(list: &mut [Available]) {
    list.sort_by(|a, b| {
        version::compare(&b.version, &a.version)
            .then_with(|| b.published.unwrap_or(0).cmp(&a.published.unwrap_or(0)))
            .then_with(|| a.name.cmp(&b.name))
    });
}

// --------------------------------------------------------------------------------------
// download.blender.org

/// Archive suffix of official release builds on this platform.
fn release_suffix() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("-windows-x64.zip"),
        ("windows", "aarch64") => Some("-windows-arm64.zip"),
        ("linux", "x86_64") => Some("-linux-x64.tar.xz"),
        ("macos", "aarch64") => Some("-macos-arm64.dmg"),
        ("macos", "x86_64") => Some("-macos-x64.dmg"),
        _ => None,
    }
}

fn fetch_releases(client: &reqwest::blocking::Client, lts: &[String], newest: usize) -> Result<Vec<Available>> {
    let Some(suffix) = release_suffix() else { return Ok(Vec::new()) };
    let index = get_text(client, RELEASES)?;
    let mut series: Vec<String> = hrefs(&index)
        .filter_map(|h| h.strip_prefix("Blender")?.strip_suffix('/').map(str::to_owned))
        .filter(|s| builds::parse_series(s).is_some_and(|v| v >= (2, 80)))
        .collect();
    series.sort_by(|a, b| version::compare(b, a));
    series.dedup();
    let wanted: Vec<String> = series
        .iter()
        .enumerate()
        .filter(|(i, s)| *i < newest || lts.contains(s))
        .map(|(_, s)| s.clone())
        .collect();
    let results: Vec<Result<Option<Available>>> = std::thread::scope(|s| {
        let handles: Vec<_> = wanted
            .iter()
            .map(|ser| s.spawn(move || newest_release(client, ser, suffix, lts.contains(ser))))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err(anyhow::anyhow!("panicked")))).collect()
    });
    let mut out = Vec::new();
    for r in results {
        if let Some(a) = r? {
            out.push(a);
        }
    }
    Ok(out)
}

/// The newest patch release of one series, e.g. Blender4.2/ -> 4.2.9.
fn newest_release(client: &reqwest::blocking::Client, series: &str, suffix: &str, lts: bool) -> Result<Option<Available>> {
    let base = format!("{RELEASES}Blender{series}/");
    let page = get_text(client, &base)?;
    Ok(parse_release_listing(&page, &base, series, suffix, lts))
}

fn parse_release_listing(page: &str, base: &str, series: &str, suffix: &str, lts: bool) -> Option<Available> {
    // Apache index lines: <a href="blender-4.2.9-windows-x64.zip">…</a>  12-Jun-2025 10:00  345678901
    let mut best: Option<(String, String, Option<u64>, Option<i64>)> = None;
    for line in page.lines() {
        let Some(href) = hrefs(line).next() else { continue };
        let Some(v) = href.strip_prefix("blender-").and_then(|r| r.strip_suffix(suffix)) else { continue };
        if !v.chars().all(|c| c.is_ascii_digit() || c == '.') || builds::parse_series(v) != builds::parse_series(series) {
            continue;
        }
        if best.as_ref().is_none_or(|(bv, ..)| version::compare(v, bv).is_gt()) {
            let tail = line.rsplit("</a>").next().unwrap_or("");
            let mut words = tail.split_whitespace();
            let date = words.next().and_then(|d| words.next().map(|t| format!("{d} {t}")));
            let size = tail.split_whitespace().last().and_then(|s| s.parse().ok());
            best = Some((v.to_owned(), href.to_owned(), size, date.as_deref().and_then(parse_apache_date)));
        }
    }
    let (v, file, size, published) = best?;
    let cycle = if lts { "lts" } else { "stable" };
    Some(Available {
        channel: if lts { Channel::Lts } else { Channel::Stable },
        name: format!("blender-{v}-{cycle}"),
        version: v.clone(),
        cycle: cycle.into(),
        branch: series.to_owned(),
        hash: String::new(),
        url: format!("{base}{file}"),
        file_name: file,
        size,
        sha256_url: Some(format!("{base}blender-{v}.sha256")),
        published,
    })
}

fn hrefs(html: &str) -> impl Iterator<Item = &str> {
    html.split("href=\"").skip(1).filter_map(|s| s.split('"').next())
}

/// "12-Jun-2025 10:00" -> unix seconds (UTC).
fn parse_apache_date(s: &str) -> Option<i64> {
    let (date, time) = s.split_once(' ')?;
    let mut d = date.split('-');
    let day: i64 = d.next()?.parse().ok()?;
    let month = d.next()?;
    let mon = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
        .iter()
        .position(|m| *m == month)? as i64
        + 1;
    let year: i64 = d.next()?.parse().ok()?;
    let (h, m) = time.split_once(':')?;
    // Days from civil (Howard Hinnant).
    let y = if mon <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mon + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + h.parse::<i64>().ok()? * 3600 + m.parse::<i64>().ok()? * 60)
}

// --------------------------------------------------------------------------------------
// builder.blender.org

#[derive(Deserialize)]
struct BuilderEntry {
    url: String,
    version: String,
    #[serde(default)]
    branch: String,
    #[serde(default)]
    patch: Option<String>,
    #[serde(default)]
    hash: String,
    platform: String,
    #[serde(default)]
    architecture: String,
    file_name: String,
    #[serde(default)]
    file_size: Option<u64>,
    file_extension: String,
    #[serde(default)]
    file_mtime: Option<i64>,
    #[serde(default)]
    release_cycle: String,
}

/// builder.blender.org (platform, architecture, file extension) of this platform's archives.
fn builder_platform() -> Option<(&'static str, &'static str, &'static str)> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some(("windows", "amd64", "zip")),
        ("windows", "aarch64") => Some(("windows", "arm64", "zip")),
        ("linux", "x86_64") => Some(("linux", "x86_64", "xz")),
        ("macos", "aarch64") => Some(("darwin", "arm64", "dmg")),
        ("macos", "x86_64") => Some(("darwin", "x86_64", "dmg")),
        _ => None,
    }
}

fn fetch_builder(client: &reqwest::blocking::Client, kind: &str) -> Result<Vec<Available>> {
    let Some(platform) = builder_platform() else { return Ok(Vec::new()) };
    let body = get_text(client, &format!("{BUILDER}{kind}/?format=json&v=1"))?;
    let entries: Vec<BuilderEntry> =
        serde_json::from_str(&body).with_context(|| format!("builder.blender.org {kind} listing"))?;
    Ok(parse_builder(entries, kind, platform))
}

fn parse_builder(entries: Vec<BuilderEntry>, kind: &str, (os, arch, ext): (&str, &str, &str)) -> Vec<Available> {
    let sums: std::collections::HashSet<String> =
        entries.iter().filter(|e| e.file_extension == "sha256").map(|e| e.file_name.clone()).collect();
    let mut out = Vec::new();
    for e in entries {
        if e.platform != os || e.architecture != arch || e.file_extension != ext {
            continue;
        }
        let channel = if kind == "daily" { Channel::Daily } else { Channel::Experimental };
        let branch = if e.branch.is_empty() { e.patch.clone().unwrap_or_default() } else { e.branch.clone() };
        let short_hash: String = e.hash.chars().take(12).collect();
        let cycle = if e.release_cycle.is_empty() { "alpha".to_owned() } else { e.release_cycle.clone() };
        let sum_name = format!("{}.sha256", e.file_name);
        out.push(Available {
            channel,
            name: sanitize(&format!("blender-{}-{cycle}+{branch}.{short_hash}", e.version)),
            sha256_url: sums.contains(&sum_name).then(|| format!("{}.sha256", e.url)),
            version: e.version,
            cycle,
            branch,
            hash: short_hash,
            url: e.url,
            file_name: e.file_name,
            size: e.file_size,
            published: e.file_mtime,
        });
    }
    out
}

fn sanitize(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.+".contains(c) { c } else { '_' }).collect()
}

// --------------------------------------------------------------------------------------
// Updates

/// A newer build for something already installed, or a new release series.
#[derive(Debug, Clone, Serialize)]
pub struct Update {
    /// The installed build it replaces; None for a release newer than any installed build.
    pub installed: Option<String>,
    pub available: Available,
}

/// Newer builds of what's installed: a newer patch in the same release series and channel,
/// a newer daily of the same branch, and a release series newer than every installed
/// release.
pub fn updates(installed: &[Build], catalog: &Catalog) -> Vec<Update> {
    let mut out = Vec::new();
    let names: std::collections::HashSet<&str> = installed.iter().map(|b| b.name.as_str()).collect();
    for b in installed.iter().filter(|b| !b.external) {
        let Some(v) = b.version.as_deref() else { continue };
        let series = builds::parse_series(v);
        let candidate = catalog
            .builds
            .iter()
            .filter(|a| !names.contains(a.name.as_str()))
            .filter(|a| match b.channel {
                Channel::Stable | Channel::Lts => {
                    matches!(a.channel, Channel::Stable | Channel::Lts)
                        && builds::parse_series(&a.version) == series
                        && version::compare(&a.version, v).is_gt()
                }
                Channel::Daily => {
                    let branch = b.meta.as_ref().map(|m| m.branch.as_str()).unwrap_or("");
                    a.channel == Channel::Daily
                        && a.branch == branch
                        && a.published.unwrap_or(0) > b.meta.as_ref().and_then(|m| m.published).unwrap_or(i64::MAX)
                }
                _ => false,
            })
            .max_by(|x, y| version::compare(&x.version, &y.version).then(x.published.cmp(&y.published)));
        if let Some(a) = candidate {
            if !out.iter().any(|u: &Update| u.available.name == a.name) {
                out.push(Update { installed: Some(b.name.clone()), available: a.clone() });
            }
        }
    }
    // A release series newer than every installed build.
    let newest_installed = installed.iter().filter_map(|b| b.series()).max();
    if let Some(a) = catalog
        .builds
        .iter()
        .filter(|a| a.channel == Channel::Stable || a.channel == Channel::Lts)
        .max_by(|x, y| version::compare(&x.version, &y.version))
    {
        if builds::parse_series(&a.version) > newest_installed && !names.contains(a.name.as_str()) {
            out.push(Update { installed: None, available: a.clone() });
        }
    }
    out
}

// --------------------------------------------------------------------------------------
// Install

/// Progress of one download, shared with the caller (UI thread).
#[derive(Debug, Default)]
pub struct Progress {
    pub done: AtomicU64,
    /// 0 when unknown.
    pub total: AtomicU64,
    phase: AtomicU8,
    pub cancel: AtomicBool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Downloading,
    Verifying,
    Extracting,
}

impl Progress {
    pub fn phase(&self) -> Phase {
        match self.phase.load(Ordering::Relaxed) {
            1 => Phase::Verifying,
            2 => Phase::Extracting,
            _ => Phase::Downloading,
        }
    }
    fn set(&self, phase: Phase, done: u64, total: u64) {
        self.phase.store(phase as u8, Ordering::Relaxed);
        self.done.store(done, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
    }
    /// 0..1, or None when the total is unknown.
    pub fn fraction(&self) -> Option<f32> {
        let t = self.total.load(Ordering::Relaxed);
        (t > 0).then(|| self.done.load(Ordering::Relaxed) as f32 / t as f32)
    }
}

/// Download, verify and unpack a build into the library. Returns the installed build.
pub fn install(sp: &Spindle, a: &Available, progress: &Progress) -> Result<Build> {
    let dest = sp.builds_dir().join(&a.name);
    if dest.exists() {
        bail!("{} is already installed", a.name);
    }
    fs::create_dir_all(sp.downloads_dir())?;
    let part = sp.downloads_dir().join(format!("{}.part", a.file_name));
    let result = download(a, &part, progress).and_then(|()| unpack(sp, a, &part, &dest, progress));
    let _ = fs::remove_file(&part);
    result?;
    builds::list(sp)?
        .into_iter()
        .find(|b| b.name == a.name)
        .with_context(|| format!("{} unpacked but has no Blender executable", a.name))
}

fn download(a: &Available, part: &Path, progress: &Progress) -> Result<()> {
    let client = client()?;
    let expected = match &a.sha256_url {
        Some(url) => Some(expected_sha256(&get_text(&client, url)?, &a.file_name)?),
        None => None,
    };
    let mut resp = client.get(&a.url).send().with_context(|| format!("requesting {}", a.url))?;
    if !resp.status().is_success() {
        bail!("{} answered {}", a.url, resp.status());
    }
    let total = resp.content_length().or(a.size).unwrap_or(0);
    progress.set(Phase::Downloading, 0, total);
    let mut out = BufWriter::with_capacity(1 << 20, File::create(part)?);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        if progress.cancel.load(Ordering::Relaxed) {
            bail!("download cancelled");
        }
        let n = resp.read(&mut buf).context("reading the download")?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        done += n as u64;
        progress.done.store(done, Ordering::Relaxed);
    }
    out.flush()?;
    progress.set(Phase::Verifying, done, done);
    if total != 0 && done != total {
        bail!("download ended early: {done} of {total} bytes");
    }
    if let Some(want) = expected {
        let got = hex::encode(hasher.finalize());
        if got != want {
            bail!("SHA-256 mismatch for {}: expected {want}, got {got}", a.file_name);
        }
    }
    Ok(())
}

/// A .sha256 file is either the bare hash (builder) or "hash  filename" lines (releases).
pub(crate) fn expected_sha256(text: &str, file_name: &str) -> Result<String> {
    let is_hash = |s: &str| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit());
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(h), name) = (parts.next(), parts.next()) else { continue };
        if is_hash(h) && name.is_none_or(|n| n.trim_start_matches('*') == file_name) {
            return Ok(h.to_lowercase());
        }
    }
    bail!("no SHA-256 for {file_name} in its checksum file")
}

fn unpack(sp: &Spindle, a: &Available, archive: &Path, dest: &Path, progress: &Progress) -> Result<()> {
    let tmp = sp.builds_dir().join(format!(".incoming-{}", a.name));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp)?;
    let result = (|| -> Result<()> {
        // The format comes from the published file name: the download itself is saved as
        // "<file name>.part".
        let name = a.file_name.to_lowercase();
        if name.ends_with(".zip") {
            unzip(archive, &tmp, progress)?;
        } else if name.ends_with(".tar.xz") {
            untar(archive, &tmp, progress)?;
        } else if name.ends_with(".dmg") {
            undmg(archive, &tmp, progress)?;
        } else {
            bail!("don't know how to unpack {name}");
        }
        // Release archives hold one top folder ("blender-4.2.9-windows-x64/"): use its contents.
        let entries: Vec<PathBuf> = fs::read_dir(&tmp)?.filter_map(|e| e.ok().map(|e| e.path())).collect();
        let root = match entries.as_slice() {
            [only] if only.is_dir() => only.clone(),
            _ => tmp.clone(),
        };
        if !root.join(builds::executable_name()).is_file() {
            bail!("the archive has no {}", builds::executable_name());
        }
        let meta = BuildMeta {
            channel: Some(a.channel),
            version: a.version.clone(),
            cycle: a.cycle.clone(),
            branch: a.branch.clone(),
            hash: a.hash.clone(),
            url: a.url.clone(),
            published: a.published,
            installed: Some(now()),
        };
        fs::write(root.join(builds::META_FILE), serde_json::to_vec_pretty(&meta)?)?;
        fs::rename(&root, dest).with_context(|| format!("moving the build into {}", dest.display()))?;
        Ok(())
    })();
    let _ = fs::remove_dir_all(&tmp);
    result
}

fn unzip(archive: &Path, tmp: &Path, progress: &Progress) -> Result<()> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?).context("opening the build archive")?;
    let count = zip.len() as u64;
    progress.set(Phase::Extracting, 0, count);
    for i in 0..zip.len() {
        if progress.cancel.load(Ordering::Relaxed) {
            bail!("cancelled while unpacking");
        }
        let mut entry = zip.by_index(i)?;
        let Some(rel) = entry.enclosed_name() else { continue };
        let path = tmp.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&path)?;
        } else {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut f = BufWriter::new(File::create(&path)?);
            std::io::copy(&mut entry, &mut f)?;
            f.flush()?;
            #[cfg(unix)]
            if let Some(mode) = entry.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(mode))?;
            }
        }
        progress.done.store(i as u64 + 1, Ordering::Relaxed);
    }
    Ok(())
}

/// Linux builds are .tar.xz; every Linux has `tar` with xz support, which also keeps
/// permissions and symlinks right.
fn untar(archive: &Path, tmp: &Path, progress: &Progress) -> Result<()> {
    progress.set(Phase::Extracting, 0, 0);
    run_tool(std::process::Command::new("tar").arg("-xJf").arg(archive).arg("-C").arg(tmp), progress, "tar")
}

/// macOS builds are disk images holding Blender.app: mount, copy the app with `ditto`
/// (keeps the code signature, symlinks and extended attributes), unmount.
fn undmg(archive: &Path, tmp: &Path, progress: &Progress) -> Result<()> {
    progress.set(Phase::Extracting, 0, 0);
    let mount = tmp.with_extension("mount");
    let _ = fs::remove_dir_all(&mount);
    fs::create_dir_all(&mount)?;
    run_tool(
        std::process::Command::new("hdiutil")
            .args(["attach", "-nobrowse", "-readonly", "-noautoopen", "-mountpoint"])
            .arg(&mount)
            .arg(archive),
        progress,
        "hdiutil attach",
    )?;
    let copied = (|| {
        let app = fs::read_dir(&mount)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .context("the disk image has no .app")?;
        run_tool(std::process::Command::new("ditto").arg(&app).arg(tmp.join("Blender.app")), progress, "ditto")
    })();
    let _ = std::process::Command::new("hdiutil").args(["detach", "-quiet", "-force"]).arg(&mount).status();
    let _ = fs::remove_dir(&mount);
    copied
}

/// Run an unpacking tool, stopping it if the download is cancelled.
fn run_tool(cmd: &mut std::process::Command, progress: &Progress, what: &str) -> Result<()> {
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {what}"))?;
    loop {
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(());
            }
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                let _ = std::io::Read::read_to_string(&mut e, &mut err);
            }
            bail!("{what} failed ({status}): {}", err.trim());
        }
        if progress.cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("cancelled while unpacking");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Group a catalog by channel (for listings).
pub fn by_channel(cat: &Catalog) -> BTreeMap<&'static str, Vec<&Available>> {
    let mut map: BTreeMap<&'static str, Vec<&Available>> = BTreeMap::new();
    for a in &cat.builds {
        map.entry(a.channel.label()).or_default().push(a);
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A download is unpacked from "<file name>.part" into the library, top folder dropped.
    #[test]
    fn unpacks_a_downloaded_zip() {
        let lib = std::env::temp_dir().join(format!("spindle-unpack-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&lib);
        let sp = Spindle { config_path: lib.join("config.toml"), config: crate::config::Config::default(), library: lib.clone() };
        let file_name = "blender-9.9.9-test-x64.zip";
        let part = lib.join("downloads").join(format!("{file_name}.part"));
        fs::create_dir_all(part.parent().unwrap()).unwrap();
        let mut zip = zip::ZipWriter::new(File::create(&part).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        zip.add_directory("blender-9.9.9-test-x64/", opts).unwrap();
        zip.start_file(format!("blender-9.9.9-test-x64/{}", builds::executable_name()), opts).unwrap();
        zip.write_all(b"not really blender").unwrap();
        zip.start_file("blender-9.9.9-test-x64/9.9/scripts/x.py", opts).unwrap();
        zip.write_all(b"pass").unwrap();
        zip.finish().unwrap();

        let a = Available {
            channel: Channel::Stable,
            version: "9.9.9".into(),
            cycle: "stable".into(),
            branch: "9.9".into(),
            hash: String::new(),
            url: String::new(),
            file_name: file_name.into(),
            size: None,
            sha256_url: None,
            published: None,
            name: "blender-9.9.9-stable".into(),
        };
        let dest = sp.builds_dir().join(&a.name);
        unpack(&sp, &a, &part, &dest, &Progress::default()).unwrap();
        assert!(dest.join(builds::executable_name()).is_file());
        assert!(dest.join("9.9/scripts/x.py").is_file());
        assert!(dest.join(builds::META_FILE).is_file());
        assert!(!sp.builds_dir().join(format!(".incoming-{}", a.name)).exists());
        let _ = fs::remove_dir_all(&lib);
    }

    #[test]
    fn release_listing_picks_newest_patch() {
        let page = r#"<a href="../">../</a>
<a href="blender-4.2.1-windows-x64.zip">blender-4.2.1-windows-x64.zip</a>   20-Aug-2024 10:00   340000000
<a href="blender-4.2.10-windows-x64.zip">blender-4.2.10-windows-x64.zip</a>  15-Jun-2025 09:30   345678901
<a href="blender-4.2.9-windows-x64.zip">blender-4.2.9-windows-x64.zip</a>   01-Apr-2025 08:00   345000000
<a href="blender-4.2.10-windows-x64.msi">blender-4.2.10-windows-x64.msi</a>  15-Jun-2025 09:30   300000000
<a href="blender-4.2.10.sha256">blender-4.2.10.sha256</a>   15-Jun-2025 09:30   1000"#;
        let base = "https://download.blender.org/release/Blender4.2/";
        let a = parse_release_listing(page, base, "4.2", "-windows-x64.zip", true).unwrap();
        assert_eq!(a.version, "4.2.10");
        assert_eq!(a.channel, Channel::Lts);
        assert_eq!(a.name, "blender-4.2.10-lts");
        assert_eq!(a.size, Some(345678901));
        assert_eq!(a.sha256_url.as_deref(), Some("https://download.blender.org/release/Blender4.2/blender-4.2.10.sha256"));
        assert_eq!(a.published, Some(1749979800)); // 2025-06-15 09:30 UTC
    }

    #[test]
    fn builder_listing_pairs_checksums_and_filters_platform() {
        let json = r#"[
 {"url":"https://cdn/x/blender-5.4.0-alpha+main.abcdef123456-windows.amd64-release.zip","app":"Blender","version":"5.4.0","branch":"main","patch":"","hash":"abcdef123456","platform":"windows","architecture":"amd64","file_name":"blender-5.4.0-alpha+main.abcdef123456-windows.amd64-release.zip","file_size":10,"file_extension":"zip","file_mtime":5,"release_cycle":"alpha"},
 {"url":"https://cdn/x/blender-5.4.0-alpha+main.abcdef123456-windows.amd64-release.zip.sha256","app":"Blender","version":"5.4.0","branch":"main","hash":"abcdef123456","platform":"windows","architecture":"amd64","file_name":"blender-5.4.0-alpha+main.abcdef123456-windows.amd64-release.zip.sha256","file_size":64,"file_extension":"sha256","release_cycle":"alpha"},
 {"url":"https://cdn/x/a.zip","app":"Blender","version":"5.4.0","branch":"main","hash":"1","platform":"linux","architecture":"x86_64","file_name":"a.zip","file_extension":"zip","release_cycle":"alpha"}
]"#;
        let linux = parse_builder(serde_json::from_str(json).unwrap(), "daily", ("linux", "x86_64", "zip"));
        assert_eq!(linux.len(), 1);
        let list = parse_builder(serde_json::from_str(json).unwrap(), "daily", ("windows", "amd64", "zip"));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "blender-5.4.0-alpha+main.abcdef123456");
        assert!(list[0].sha256_url.as_deref().unwrap().ends_with(".zip.sha256"));
        assert_eq!(builds::Channel::from_name(&list[0].name), Channel::Daily);
    }

    #[test]
    fn checksum_formats() {
        let h = "a".repeat(64);
        assert_eq!(expected_sha256(&h, "x.zip").unwrap(), h);
        let lines = format!("{}  blender-4.2.10-linux-x64.tar.xz\n{h}  blender-4.2.10-windows-x64.zip\n", "b".repeat(64));
        assert_eq!(expected_sha256(&lines, "blender-4.2.10-windows-x64.zip").unwrap(), h);
        assert!(expected_sha256(&lines, "missing.zip").is_err());
    }
}
