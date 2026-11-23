//! GitHub as a download source for Spindle's add-on library.
//!
//! Adding a repository link takes the add-on from its newest release: the first asset (a
//! `.zip`, or a single-file add-on's `.py`) that holds an extension or a legacy add-on,
//! preferring one named for this platform, checked against the SHA-256 GitHub publishes for
//! it. When there is no release, or no release asset is an add-on, `add` returns an offer
//! instead, and `build` (once the user agrees) downloads that exact commit, finds the add-on
//! in it and packages it the way a release would be: an extension with a library build's own
//! `blender --command extension build`, a legacy add-on as a zip of its module. Nothing from
//! the repository is run.
//!
//! Links may name a release (`/releases/tag/<tag>`), or a branch, tag or commit and a folder
//! (`/tree/<ref>/<folder>`), which builds that folder from source. Requests are anonymous
//! (GitHub allows 60 an hour) unless `GITHUB_TOKEN` or `GH_TOKEN` is set, which also reaches
//! private repositories.

use crate::addons::{self, AddonInfo, AddonLibrary, AddonRecord, Origin, sanitize};
use crate::builds::{self, Build};
use crate::downloads::client;
use crate::superhive::{current_platform, percent_decode};
use crate::{Spindle, compat};
use anyhow::{Context, Result, bail};
use reqwest::blocking::{Client, RequestBuilder, Response};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SOURCE: &str = "github";
const API: &str = "https://api.github.com";
/// How far below the repository root (or the linked folder) to look for the add-on.
const SEARCH_DEPTH: usize = 3;

// --------------------------------------------------------------------------------------
// Links

/// A parsed GitHub link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub owner: String,
    pub repo: String,
    pub target: Target,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// The newest release (without one, building the default branch is offered).
    Latest,
    /// The release with this tag.
    Release(String),
    /// Everything after `/tree/`: a ref and maybe a folder, split once resolved (branch names
    /// can hold slashes).
    Tree(String),
}

impl Link {
    /// "owner/repo".
    pub fn repo_name(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    /// The link, normalised.
    pub fn url(&self) -> String {
        let base = format!("https://github.com/{}/{}", self.owner, self.repo);
        match &self.target {
            Target::Latest => base,
            Target::Release(tag) => format!("{base}/releases/tag/{tag}"),
            Target::Tree(rest) => format!("{base}/tree/{rest}"),
        }
    }
}

/// Accepts `https://github.com/owner/repo` (with or without the scheme, `www.` and `.git`),
/// `git@github.com:owner/repo.git`, `owner/repo`, and links to a release
/// (`/releases/tag/<tag>`), a tree (`/tree/<ref>/<folder>`) or a commit. Any other page of a
/// repository (issues, a file, the releases list) means the repository itself.
pub fn parse_link(input: &str) -> Result<Link> {
    let s = input.trim();
    let bad = || anyhow::anyhow!("'{s}' is not a GitHub repository link (like https://github.com/owner/repo)");
    let s = s.split(['?', '#']).next().unwrap_or(s);
    let rest = match s.strip_prefix("git@github.com:") {
        Some(r) => r,
        None => {
            let had_scheme = s.contains("://");
            let host = s.split_once("://").map_or(s, |(_, r)| r);
            let host = host.strip_prefix("www.").unwrap_or(host);
            match host.strip_prefix("github.com/") {
                Some(r) => r,
                // owner/repo
                None if !had_scheme && !host.split('/').next().unwrap_or_default().contains('.') => host,
                None => return Err(bad()),
            }
        }
    };
    let segs: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
    if segs.len() < 2 {
        return Err(bad());
    }
    let owner = segs[0];
    let repo = segs[1].strip_suffix(".git").unwrap_or(segs[1]);
    let valid = |x: &str, extra: &[char]| {
        !x.is_empty() && x.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || extra.contains(&c))
    };
    if !valid(owner, &[]) || !valid(repo, &['.', '_']) || repo.chars().all(|c| c == '.') {
        return Err(bad());
    }
    let tail: Vec<String> = segs[2..].iter().map(|p| percent_decode(p)).collect();
    let tail: Vec<&str> = tail.iter().map(String::as_str).collect();
    let target = match tail.as_slice() {
        ["releases", "tag", tag @ ..] if !tag.is_empty() => Target::Release(tag.join("/")),
        ["tree" | "commit", rest @ ..] if !rest.is_empty() => Target::Tree(rest.join("/")),
        _ => Target::Latest,
    };
    Ok(Link { owner: owner.to_owned(), repo: repo.to_owned(), target })
}

// --------------------------------------------------------------------------------------
// GitHub API

#[derive(Deserialize)]
struct GhRepo {
    full_name: String,
    default_branch: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GhAsset {
    name: String,
    /// API URL; with `Accept: application/octet-stream` it serves the file (private
    /// repositories too, with a token).
    url: String,
    #[serde(default)]
    size: u64,
    /// "sha256:<hex>", computed by GitHub when the asset was uploaded.
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Deserialize)]
struct GhCommit {
    sha: String,
    commit: GhCommitInfo,
}

#[derive(Deserialize)]
struct GhCommitInfo {
    committer: Option<GhSignature>,
}

#[derive(Deserialize)]
struct GhSignature {
    date: Option<String>,
}

fn token() -> Option<String> {
    ["GITHUB_TOKEN", "GH_TOKEN"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .map(|t| t.trim().to_owned())
        .find(|t| !t.is_empty())
}

fn request(client: &Client, url: &str, accept: &str) -> RequestBuilder {
    let req = client.get(url).header("Accept", accept).header("X-GitHub-Api-Version", "2022-11-28");
    match token() {
        Some(t) => req.bearer_auth(t),
        None => req,
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// GET an API path. None when GitHub has no such thing (404, or 422 for an unknown ref).
fn api_get(client: &Client, path: &str, accept: &str) -> Result<Option<Response>> {
    let resp = request(client, &format!("{API}/{path}"), accept)
        .send()
        .with_context(|| format!("asking GitHub for {path}"))?;
    let status = resp.status();
    if status.is_success() {
        return Ok(Some(resp));
    }
    if status == 404 || status == 422 {
        return Ok(None);
    }
    let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
    if (status == 403 || status == 429) && header("x-ratelimit-remaining").as_deref() == Some("0") {
        let wait = header("x-ratelimit-reset")
            .and_then(|r| r.parse::<u64>().ok())
            .map(|reset| format!(" for the next {} min", reset.saturating_sub(now()) / 60 + 1))
            .unwrap_or_default();
        if token().is_some() {
            bail!("GitHub's request limit for your token is used up{wait}");
        }
        bail!("GitHub's limit of 60 requests an hour without a token is used up{wait}; set GITHUB_TOKEN to a GitHub token to lift it");
    }
    if status == 401 {
        bail!("GitHub refused the token in GITHUB_TOKEN / GH_TOKEN ({status})");
    }
    bail!("GitHub answered {status} for {path}")
}

fn api_json<T: DeserializeOwned>(client: &Client, path: &str) -> Result<Option<T>> {
    match api_get(client, path, "application/vnd.github+json")? {
        None => Ok(None),
        Some(resp) => {
            Ok(Some(serde_json::from_reader(resp).with_context(|| format!("reading GitHub's answer for {path}"))?))
        }
    }
}

fn commit(client: &Client, repo: &str, git_ref: &str) -> Result<Option<GhCommit>> {
    api_json(client, &format!("repos/{repo}/commits/{}", encode_path(git_ref)))
}

/// Percent-encode a ref or tag for a URL path (slashes stay).
fn encode_path(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Stream a download into `dest`; returns its SHA-256.
fn download(client: &Client, url: &str, accept: &str, dest: &Path) -> Result<String> {
    let mut resp = request(client, url, accept).send().with_context(|| format!("downloading {url}"))?;
    if !resp.status().is_success() {
        bail!("{url} answered {}", resp.status());
    }
    let mut out = BufWriter::new(File::create(dest)?);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = resp.read(&mut buf).context("reading the download")?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
    }
    out.flush()?;
    Ok(hex::encode(hasher.finalize()))
}

// --------------------------------------------------------------------------------------
// Adding

/// What adding a link came to.
#[derive(Debug, Clone)]
pub enum Added {
    /// Downloaded or built into the library.
    Stored(AddonRecord),
    /// That version is already in the library.
    Present(AddonRecord),
    /// Nothing to download: building from the source needs the user's go-ahead (`build`).
    Offer(SourceOffer),
}

/// Building an add-on from one commit of a repository.
#[derive(Debug, Clone)]
pub struct SourceOffer {
    /// The link as added (kept as the add-on's origin).
    pub url: String,
    /// "owner/repo".
    pub repo: String,
    /// Branch, tag or commit.
    pub git_ref: String,
    pub commit: String,
    /// Commit date ("2026-11-20"), when GitHub gives one.
    pub date: String,
    /// Folder inside the repository ("" for the root).
    pub path: String,
    /// Why there's nothing to download, e.g. "owner/repo has no releases".
    pub reason: String,
}

impl SourceOffer {
    /// "main (commit 1a2b3c4, 2026-11-20)", with the folder when there is one.
    pub fn describe(&self) -> String {
        let date = if self.date.is_empty() { String::new() } else { format!(", {}", self.date) };
        let at = format!("{} (commit {}{date})", self.git_ref, short(&self.commit));
        if self.path.is_empty() { at } else { format!("the {} folder of {at}", self.path) }
    }
}

fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// The GitHub add-on in the library matching `pred` on its origin.
fn find(lib: &AddonLibrary, pred: impl Fn(&Origin) -> bool) -> Option<AddonRecord> {
    lib.list().ok()?.into_iter().find(|r| r.source == SOURCE && r.origin.as_ref().is_some_and(&pred))
}

/// Add the add-on a link points at: download it from the release it names (the newest
/// release by default), or offer to build it from source when there is nothing to download.
pub fn add(sp: &Spindle, lib: &AddonLibrary, link: &Link) -> Result<Added> {
    let client = client()?;
    let name = link.repo_name();
    let repo: GhRepo = api_json(&client, &format!("repos/{name}"))?
        .with_context(|| format!("there is no GitHub repository {name} (if it is private, set GITHUB_TOKEN to reach it)"))?;
    let repo_name = repo.full_name;
    let url = link.url();
    let release = match &link.target {
        Target::Tree(rest) => {
            let (git_ref, path, c) = resolve_tree(&client, &repo_name, rest)?;
            let what = if path.is_empty() { git_ref.clone() } else { format!("the {path} folder of {git_ref}") };
            return Ok(offer(lib, &repo_name, &url, &git_ref, &path, c, format!("The link points at {what}")));
        }
        Target::Release(tag) => Some(
            api_json::<GhRelease>(&client, &format!("repos/{repo_name}/releases/tags/{}", encode_path(tag)))?
                .with_context(|| format!("{repo_name} has no release tagged {tag}"))?,
        ),
        Target::Latest => {
            let list: Vec<GhRelease> =
                api_json(&client, &format!("repos/{repo_name}/releases?per_page=20"))?.unwrap_or_default();
            // The newest release, or the newest pre-release when there are only those.
            let (stable, pre): (Vec<_>, Vec<_>) = list.into_iter().filter(|r| !r.draft).partition(|r| !r.prerelease);
            stable.into_iter().next().or_else(|| pre.into_iter().next())
        }
    };
    let offer_at = |git_ref: &str, reason: String| -> Result<Added> {
        let c = commit(&client, &repo_name, git_ref)?
            .with_context(|| format!("{git_ref} is not a branch, tag or commit of {repo_name}"))?;
        Ok(offer(lib, &repo_name, &url, git_ref, "", c, reason))
    };
    let Some(release) = release else {
        return offer_at(&repo.default_branch, format!("{repo_name} has no releases"));
    };
    let tag = release.tag_name.clone();
    if let Some(have) = find(lib, |o| o.repo.eq_ignore_ascii_case(&repo_name) && o.git_ref == tag && !o.built) {
        return Ok(Added::Present(have));
    }
    let candidates = rank_assets(&release.assets, current_platform());
    if candidates.is_empty() {
        return offer_at(&tag, format!("Release {tag} of {repo_name} has no add-on archive, only its source code"));
    }
    let work = WorkDir::new(sp)?;
    let mut problems = Vec::new();
    for asset in candidates.into_iter().take(3) {
        match fetch_asset(&client, asset, &work.0) {
            Ok((path, sha, info)) => {
                let version = info.version.clone();
                let origin = Origin { url, repo: repo_name, git_ref: tag, built: false, ..Default::default() };
                return store(lib, info, &path, sha, version, origin);
            }
            Err(e) => problems.push(format!("{}: {e:#}", asset.name)),
        }
    }
    offer_at(&tag, format!("No archive in release {tag} of {repo_name} is a Blender add-on ({})", problems.join("; ")))
}

fn offer(lib: &AddonLibrary, repo: &str, url: &str, git_ref: &str, path: &str, c: GhCommit, reason: String) -> Added {
    // That commit (and folder) is built already.
    if let Some(have) = find(lib, |o| o.built && o.repo.eq_ignore_ascii_case(repo) && o.commit == c.sha && o.path == path) {
        return Added::Present(have);
    }
    Added::Offer(SourceOffer {
        url: url.to_owned(),
        repo: repo.to_owned(),
        git_ref: git_ref.to_owned(),
        date: c.commit.committer.and_then(|s| s.date).and_then(|d| d.get(..10).map(str::to_owned)).unwrap_or_default(),
        commit: c.sha,
        path: path.to_owned(),
        reason,
    })
}

/// Split `<ref>/<folder>` from a tree link: the shortest prefix GitHub knows as a ref.
fn resolve_tree(client: &Client, repo: &str, rest: &str) -> Result<(String, String, GhCommit)> {
    let segs: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    for i in 1..=segs.len().min(6) {
        let git_ref = segs[..i].join("/");
        if let Some(c) = commit(client, repo, &git_ref)? {
            return Ok((git_ref, segs[i..].join("/"), c));
        }
    }
    bail!("{repo} has no branch, tag or commit named {}", segs.first().unwrap_or(&rest))
}

/// Release assets worth trying, best first: archives (then `.py` files) named for this
/// platform, then ones named for no platform; ones named for another platform are left out,
/// and source archives go last.
fn rank_assets<'a>(assets: &'a [GhAsset], platform: &str) -> Vec<&'a GhAsset> {
    let (os, arch) = platform.split_once('-').unwrap_or((platform, ""));
    let mut ranked: Vec<(u8, &GhAsset)> = assets
        .iter()
        .filter_map(|a| {
            let lower = a.name.to_lowercase();
            let py = lower.ends_with(".py");
            if !(py || lower.ends_with(".zip")) {
                return None;
            }
            let words: Vec<&str> = lower.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect();
            let has = |list: &[&str]| words.iter().any(|w| list.contains(w));
            let oses: Vec<&str> = [
                ("windows", &["windows", "win", "win32", "win64"][..]),
                ("macos", &["macos", "mac", "osx", "darwin"][..]),
                ("linux", &["linux"][..]),
            ]
            .iter()
            .filter(|(_, w)| has(w))
            .map(|(o, _)| *o)
            .collect();
            let arches: Vec<&str> = [("x64", has(&["x64", "amd64"]) || lower.contains("x86_64")), ("arm64", has(&["arm64", "aarch64"]))]
                .iter()
                .filter(|(_, named)| *named)
                .map(|(a, _)| *a)
                .collect();
            if (!oses.is_empty() && !oses.contains(&os)) || (!arches.is_empty() && !arches.contains(&arch)) {
                return None;
            }
            let score = u8::from(oses.is_empty()) + u8::from(py) + 2 * u8::from(has(&["source", "src"]));
            Some((score, a))
        })
        .collect();
    ranked.sort_by_key(|(score, _)| *score);
    ranked.into_iter().map(|(_, a)| a).collect()
}

/// Download one release asset, verify it and read what it holds.
fn fetch_asset(client: &Client, asset: &GhAsset, work: &Path) -> Result<(PathBuf, String, AddonInfo)> {
    let file = sanitize(&asset.name);
    let path = work.join(&file);
    let sha = download(client, &asset.url, "application/octet-stream", &path)?;
    let size = fs::metadata(&path)?.len();
    if asset.size != 0 && size != asset.size {
        bail!("the download ended early: {size} of {} bytes", asset.size);
    }
    if let Some(want) = asset.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")) {
        if !want.eq_ignore_ascii_case(&sha) {
            bail!("SHA-256 mismatch: GitHub lists {want}, the download is {sha}");
        }
    }
    let info = match file.strip_suffix(".py").or_else(|| file.strip_suffix(".PY")) {
        Some(module) => addons::parse_bl_info(&fs::read_to_string(&path)?, module)
            .context("it has no bl_info, so it isn't a Blender add-on")?,
        None => addons::inspect_zip(&path)?,
    };
    Ok((path, sha, info))
}

fn store(lib: &AddonLibrary, info: AddonInfo, archive: &Path, sha: String, version: String, origin: Origin) -> Result<Added> {
    if lib.contains(SOURCE, &info.id, &version) {
        if let Some(have) = lib.list()?.into_iter().find(|r| r.source == SOURCE && r.id == info.id && r.version == version) {
            return Ok(Added::Present(have));
        }
    }
    let rec = AddonRecord {
        id: info.id,
        name: info.name,
        version,
        kind: info.kind,
        source: SOURCE.into(),
        archive: archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        sha256: sha,
        blender_version_min: info.blender_version_min,
        blender_version_max: info.blender_version_max,
        platforms: info.platforms,
        fetched: now(),
        format: info.format,
        origin: Some(origin),
    };
    lib.store(&rec, &fs::read(archive)?)?;
    Ok(Added::Stored(rec))
}

/// A scratch folder under `<library>/run/github`, removed when dropped.
struct WorkDir(PathBuf);

impl WorkDir {
    fn new(sp: &Spindle) -> Result<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.subsec_nanos() ^ std::process::id();
        let path = sp.run_dir().join("github").join(format!("{stamp:08x}"));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).with_context(|| format!("creating {}", path.display()))?;
        Ok(Self(path))
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
        if let Some(parent) = self.0.parent() {
            let _ = fs::remove_dir(parent); // only when empty
        }
    }
}

// --------------------------------------------------------------------------------------
// Building from source

/// Download the offered commit, find the add-on in it, build an installable archive and
/// store it. The version is the add-on's own plus the commit ("1.3.2+1a2b3c4").
pub fn build(sp: &Spindle, lib: &AddonLibrary, offer: &SourceOffer) -> Result<Added> {
    if let Some(have) =
        find(lib, |o| o.built && o.repo.eq_ignore_ascii_case(&offer.repo) && o.commit == offer.commit && o.path == offer.path)
    {
        return Ok(Added::Present(have));
    }
    let client = client()?;
    let work = WorkDir::new(sp)?;
    let zipball = work.0.join("source.zip");
    download(&client, &format!("{API}/repos/{}/zipball/{}", offer.repo, offer.commit), "application/vnd.github+json", &zipball)
        .with_context(|| format!("downloading the source of {}", offer.repo))?;
    let src = work.0.join("src");
    unzip(&zipball, &src)?;
    // GitHub's archives hold one folder, "<owner>-<repo>-<commit>".
    let top = match fs::read_dir(&src)?.filter_map(|e| e.ok().map(|e| e.path())).collect::<Vec<_>>().as_slice() {
        [only] if only.is_dir() => only.clone(),
        _ => src.clone(),
    };
    let mut dir = top.clone();
    for part in offer.path.split('/').filter(|p| !p.is_empty()) {
        if part == ".." || part == "." || part.contains(['\\', ':']) {
            bail!("'{}' is not a folder of the repository", offer.path);
        }
        dir.push(part);
    }
    if !dir.is_dir() {
        bail!("{} has no folder {} at {}", offer.repo, offer.path, offer.git_ref);
    }
    let repo_short = offer.repo.rsplit('/').next().unwrap_or(&offer.repo);
    let module_of = |d: &Path| -> String {
        let name = if d == top { repo_short.to_owned() } else { d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() };
        python_module(&name)
    };
    let hint = format!("https://github.com/{}/tree/{}", offer.repo, offer.git_ref);
    let out = work.0.join("out");
    fs::create_dir_all(&out)?;
    let (archive, info) = match locate(&dir, &top, &module_of, &hint)? {
        Found::Extension { dir, legacy } => match extension_builder(sp) {
            Some(b) => build_extension(&b, &dir, &work.0, &out)?,
            // An add-on that is both: without a 4.2+ build, package it as the legacy add-on.
            None => match legacy {
                Some(info) => (package(&dir, &info, &out)?, info),
                None => bail!(
                    "{} is an extension: building it needs Blender 4.2 or newer, and the library has no such build (download one under Download Builds)",
                    offer.repo
                ),
            },
        },
        Found::Legacy { path, info } => (package(&path, &info, &out)?, info),
    };
    let sha = hex::encode(Sha256::digest(fs::read(&archive)?));
    let version = format!("{}+{}", info.version, short(&offer.commit));
    let origin = Origin {
        url: offer.url.clone(),
        repo: offer.repo.clone(),
        git_ref: offer.git_ref.clone(),
        commit: offer.commit.clone(),
        path: offer.path.clone(),
        built: true,
    };
    store(lib, info, &archive, sha, version, origin)
}

fn unzip(archive: &Path, dest: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?).context("GitHub's source archive is not a zip")?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let Some(rel) = entry.enclosed_name() else { continue };
        let path = dest.join(&rel);
        if entry.is_dir() {
            fs::create_dir_all(&path)?;
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = File::create(&path).with_context(|| format!("unpacking {}", rel.display()))?;
        std::io::copy(&mut entry, &mut out)?;
    }
    Ok(())
}

/// A valid Python module name for a legacy add-on folder ("Principled-Baker" ->
/// "Principled_Baker"): Blender's installer keeps the name it is zipped under.
fn python_module(name: &str) -> String {
    let mut m: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if m.is_empty() {
        m = "addon".into();
    }
    if m.starts_with(|c: char| c.is_ascii_digit()) {
        m.insert(0, '_');
    }
    m
}

#[derive(Debug)]
enum Found {
    /// A folder with `blender_manifest.toml` (and the legacy add-on it also is, if any).
    Extension { dir: PathBuf, legacy: Option<AddonInfo> },
    /// A legacy add-on: a package folder, or a single `.py` file.
    Legacy { path: PathBuf, info: AddonInfo },
}

/// Find the add-on in a source tree: the shallowest folder with `blender_manifest.toml`;
/// else the shallowest package whose `__init__.py` assigns `bl_info`; else the shallowest
/// single-file add-on. Several at the same depth is ambiguous, and the error names them so
/// the user can link one folder instead.
fn locate(root: &Path, top: &Path, module_of: &dyn Fn(&Path) -> String, hint: &str) -> Result<Found> {
    let levels = levels(root);
    let rel = |p: &Path| p.strip_prefix(top).unwrap_or(p).to_string_lossy().replace('\\', "/");
    let ambiguous = |paths: Vec<String>| -> anyhow::Error {
        anyhow::anyhow!(
            "the repository holds {} add-ons at the same depth ({}); add a link to one of their folders instead, like {hint}/{}",
            paths.len(),
            paths.join(", "),
            paths[0]
        )
    };
    let legacy_package = |d: &Path| -> Option<AddonInfo> {
        addons::parse_bl_info(&fs::read_to_string(d.join("__init__.py")).ok()?, &module_of(d))
    };
    for level in &levels {
        let hits: Vec<&PathBuf> = level.iter().filter(|d| d.join("blender_manifest.toml").is_file()).collect();
        match hits.as_slice() {
            [] => {}
            [d] => return Ok(Found::Extension { dir: (*d).clone(), legacy: legacy_package(d) }),
            many => return Err(ambiguous(many.iter().map(|d| rel(d)).collect())),
        }
    }
    for level in &levels {
        let hits: Vec<(&PathBuf, AddonInfo)> = level.iter().filter_map(|d| Some((d, legacy_package(d)?))).collect();
        match hits.as_slice() {
            [] => {}
            [(d, info)] => return Ok(Found::Legacy { path: (*d).clone(), info: info.clone() }),
            many => return Err(ambiguous(many.iter().map(|(d, _)| rel(d)).collect())),
        }
    }
    for level in &levels {
        let mut hits: Vec<(PathBuf, AddonInfo)> = Vec::new();
        for d in level.iter().filter(|d| !d.join("__init__.py").exists()) {
            for f in files(d).into_iter().filter(|f| f.extension().is_some_and(|x| x == "py")) {
                let module = python_module(&f.file_stem().unwrap_or_default().to_string_lossy());
                if let Some(info) = fs::read_to_string(&f).ok().and_then(|s| addons::parse_bl_info(&s, &module)) {
                    hits.push((f, info));
                }
            }
        }
        match hits.len() {
            0 => {}
            1 => {
                let (path, info) = hits.remove(0);
                return Ok(Found::Legacy { path, info });
            }
            _ => return Err(ambiguous(hits.iter().map(|(f, _)| rel(f)).collect())),
        }
    }
    bail!("no Blender add-on in it: no blender_manifest.toml, and no bl_info in a package or .py file")
}

/// Folders by depth below `root` (root first), skipping hidden folders and caches.
fn levels(root: &Path) -> Vec<Vec<PathBuf>> {
    let mut out = vec![vec![root.to_path_buf()]];
    for _ in 0..SEARCH_DEPTH {
        let next: Vec<PathBuf> = out
            .last()
            .into_iter()
            .flatten()
            .flat_map(|d| {
                let mut subs: Vec<PathBuf> = fs::read_dir(d)
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir() && !skipped(&e.file_name().to_string_lossy()))
                    .map(|e| e.path())
                    .collect();
                subs.sort();
                subs
            })
            .collect();
        if next.is_empty() {
            break;
        }
        out.push(next);
    }
    out
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut f: Vec<PathBuf> =
        fs::read_dir(dir).into_iter().flatten().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file()).collect();
    f.sort();
    f
}

/// Repository clutter left out of searches and packages (what Blender's own extension
/// builder leaves out by default, plus caches).
fn skipped(name: &str) -> bool {
    let lower = name.to_lowercase();
    name.starts_with('.')
        || matches!(name, "__pycache__" | "node_modules" | "venv")
        || [".pyc", ".pyo", ".blend1", ".zip"].iter().any(|x| lower.ends_with(x))
}

/// Zip a legacy add-on the way Blender's installer expects it: `<module>/...` for a package,
/// `<module>.py` for a single file.
fn package(src: &Path, info: &AddonInfo, out: &Path) -> Result<PathBuf> {
    let path = out.join(sanitize(&format!("{}-{}.zip", info.id, info.version)));
    let mut zip = zip::ZipWriter::new(BufWriter::new(File::create(&path)?));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    if src.is_file() {
        zip.start_file(format!("{}.py", info.id), opts)?;
        std::io::copy(&mut File::open(src)?, &mut zip)?;
    } else {
        zip.add_directory(format!("{}/", info.id), opts)?;
        add_dir(&mut zip, src, &info.id, opts)?;
    }
    zip.finish()?.flush()?;
    Ok(path)
}

fn add_dir<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    dir: &Path,
    prefix: &str,
    opts: zip::write::SimpleFileOptions,
) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        if skipped(&name) {
            continue;
        }
        let rel = format!("{prefix}/{name}");
        let ty = e.file_type()?;
        if ty.is_dir() {
            zip.add_directory(format!("{rel}/"), opts)?;
            add_dir(zip, &e.path(), &rel, opts)?;
        } else if ty.is_file() {
            zip.start_file(rel, opts)?;
            std::io::copy(&mut File::open(e.path())?, zip)?;
        }
    }
    Ok(())
}

/// The build that builds extensions: the default build if it is 4.2 or newer, else the
/// newest such build.
fn extension_builder(sp: &Spindle) -> Option<Build> {
    builds::list(sp).ok()?.into_iter().find(|b| b.series().is_some_and(|s| s >= compat::EXTENSIONS_SINCE))
}

/// `blender --command extension build`, with factory settings and a scratch user folder so
/// no real profile is read or written. Returns the archive and what it holds.
fn build_extension(build: &Build, dir: &Path, work: &Path, out: &Path) -> Result<(PathBuf, AddonInfo)> {
    let user = work.join("blender-user");
    fs::create_dir_all(&user)?;
    let mut cmd = std::process::Command::new(&build.executable);
    cmd.args(["-b", "--factory-startup", "--command", "extension", "build", "--source-dir"])
        .arg(dir)
        .arg("--output-dir")
        .arg(out)
        .env("BLENDER_USER_RESOURCES", &user)
        .stdin(std::process::Stdio::null());
    for var in ["BLENDER_USER_CONFIG", "BLENDER_USER_SCRIPTS", "BLENDER_USER_DATAFILES", "BLENDER_USER_EXTENSIONS"] {
        cmd.env_remove(var);
    }
    if let Some(d) = build.executable.parent() {
        cmd.current_dir(d);
    }
    crate::launch::hide_console(&mut cmd);
    let o = cmd.output().with_context(|| format!("running {}", build.executable.display()))?;
    let label = format!("Blender {}", build.version.as_deref().unwrap_or(&build.name));
    if !o.status.success() {
        let text = format!("{}\n{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
        let lines: Vec<&str> = text.lines().map(str::trim_end).filter(|l| !l.is_empty()).collect();
        bail!("{label} could not build the extension:\n{}", lines[lines.len().saturating_sub(12)..].join("\n"));
    }
    let archive = files(out)
        .into_iter()
        .find(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")))
        .with_context(|| format!("{label}'s extension build made no archive"))?;
    let info = addons::inspect_zip(&archive)?;
    Ok((archive, info))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(owner: &str, repo: &str, target: Target) -> Link {
        Link { owner: owner.into(), repo: repo.into(), target }
    }

    #[test]
    fn links() {
        let latest = link("gregzaal", "Matalogue", Target::Latest);
        for s in [
            "https://github.com/gregzaal/Matalogue",
            "https://github.com/gregzaal/Matalogue/",
            "http://www.github.com/gregzaal/Matalogue.git",
            "github.com/gregzaal/Matalogue",
            "git@github.com:gregzaal/Matalogue.git",
            "gregzaal/Matalogue",
            "  https://github.com/gregzaal/Matalogue/issues?q=open#top ",
            "https://github.com/gregzaal/Matalogue/releases",
            "https://github.com/gregzaal/Matalogue/blob/master/README.md",
        ] {
            assert_eq!(parse_link(s).unwrap(), latest, "{s}");
        }
        assert_eq!(
            parse_link("https://github.com/o/r/releases/tag/v1.2%2Bb").unwrap(),
            link("o", "r", Target::Release("v1.2+b".into()))
        );
        assert_eq!(
            parse_link("https://github.com/o/r.blend/tree/feature/x/addons/foo").unwrap(),
            link("o", "r.blend", Target::Tree("feature/x/addons/foo".into()))
        );
        assert_eq!(parse_link("https://github.com/o/r/commit/abc123").unwrap(), link("o", "r", Target::Tree("abc123".into())));
        for bad in ["", "matalogue", "https://gitlab.com/o/r", "gitlab.com/o/r", "https://github.com/o", "https://github.com/o/..", "o!/r"] {
            assert!(parse_link(bad).is_err(), "{bad}");
        }
        assert_eq!(link("o", "r", Target::Tree("main/a".into())).url(), "https://github.com/o/r/tree/main/a");
    }

    #[test]
    fn asset_ranking() {
        let assets: Vec<GhAsset> = [
            "addon-1.0-source.zip",
            "addon-1.0-linux-x64.zip",
            "addon-1.0.zip",
            "addon-1.0-windows-arm64.zip",
            "addon-1.0-windows-x64.zip",
            "addon.py",
            "checksums.txt",
            "addon-1.0-macos_arm64.zip",
        ]
        .iter()
        .map(|n| GhAsset { name: (*n).into(), url: String::new(), size: 0, digest: None })
        .collect();
        let names = |platform| rank_assets(&assets, platform).iter().map(|a| a.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names("windows-x64"), ["addon-1.0-windows-x64.zip", "addon-1.0.zip", "addon.py", "addon-1.0-source.zip"]);
        assert_eq!(names("macos-arm64"), ["addon-1.0-macos_arm64.zip", "addon-1.0.zip", "addon.py", "addon-1.0-source.zip"]);
    }

    #[test]
    fn module_names() {
        assert_eq!(python_module("Principled-Baker"), "Principled_Baker");
        assert_eq!(python_module("3d-tools"), "_3d_tools");
        assert_eq!(python_module("uv_squares"), "uv_squares");
    }

    /// A repository with a legacy package at its root is found as the repository's module,
    /// packaged under that name without repository clutter, and reads back as the same add-on.
    #[test]
    fn locates_and_packages_legacy_add_ons() {
        let base = std::env::temp_dir().join(format!("spindle-github-test-{}", std::process::id()));
        let top = base.join("owner-My-Addon-abc1234");
        fs::create_dir_all(top.join("ops")).unwrap();
        fs::create_dir_all(top.join(".github")).unwrap();
        fs::create_dir_all(top.join("__pycache__")).unwrap();
        fs::write(top.join("__init__.py"), "bl_info = {\"name\": \"My Addon\", \"version\": (2, 0, 1), \"blender\": (3, 6, 0)}\n").unwrap();
        fs::write(top.join("ops").join("__init__.py"), "").unwrap();
        fs::write(top.join(".github").join("ci.yml"), "").unwrap();
        fs::write(top.join("__pycache__").join("x.pyc"), "").unwrap();
        fs::write(top.join("old-release.zip"), "").unwrap();
        let module_of = |d: &Path| python_module(&if d == top { "My-Addon".to_owned() } else { d.file_name().unwrap().to_string_lossy().into_owned() });
        let Found::Legacy { path, info } = locate(&top, &top, &module_of, "hint").unwrap() else { panic!("not legacy") };
        assert_eq!((info.id.as_str(), info.version.as_str(), path.as_path()), ("My_Addon", "2.0.1", top.as_path()));
        let out = base.join("out");
        fs::create_dir_all(&out).unwrap();
        let zip_path = package(&path, &info, &out).unwrap();
        let mut zip = zip::ZipArchive::new(File::open(&zip_path).unwrap()).unwrap();
        let mut names: Vec<String> = (0..zip.len()).map(|i| zip.by_index(i).unwrap().name().unwrap().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["My_Addon/", "My_Addon/__init__.py", "My_Addon/ops/", "My_Addon/ops/__init__.py"]);
        drop(zip);
        assert_eq!(addons::inspect_zip(&zip_path).unwrap(), info);

        // Two extensions side by side: ambiguous, and the error offers a folder link.
        let two = base.join("two");
        for id in ["a", "b"] {
            fs::create_dir_all(two.join(id)).unwrap();
            fs::write(two.join(id).join("blender_manifest.toml"), format!("id = \"{id}\"\nversion = \"1.0.0\"\n")).unwrap();
        }
        let err = locate(&two, &two, &module_of, "https://github.com/o/r/tree/main").unwrap_err().to_string();
        assert!(err.contains("2 add-ons") && err.contains("https://github.com/o/r/tree/main/a"), "{err}");
        fs::remove_dir_all(&base).unwrap();
    }
}
