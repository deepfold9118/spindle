//! Superhive as a download source for Spindle's add-on library.
//!
//! Superhive exposes a Blender extensions API v1 repository (the same one Blender's
//! "Get Extensions" talks to). Spindle reads that index with the user's API token and
//! downloads purchased archives into its own library. It does not register the
//! repository in Blender or copy the token anywhere else.

use crate::addons::{AddonLibrary, AddonRecord, Format, sanitize};
use crate::{Spindle, version};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const SOURCE: &str = "superhive";
const KEYRING_SERVICE: &str = "spindle";
const KEYRING_USER: &str = "superhive-api-token";

fn entry() -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?)
}

/// Store the API token in the OS credential store (Windows Credential Manager,
/// macOS Keychain, Secret Service on Linux).
pub fn set_token(token: &str) -> Result<()> {
    let token = token.trim();
    if token.is_empty() {
        bail!("empty token");
    }
    entry()?.set_password(token).context("saving token to the credential store")
}

pub fn has_token() -> bool {
    entry().and_then(|e| Ok(e.get_password()?)).is_ok()
}

pub fn clear_token() -> Result<()> {
    entry()?.delete_credential().context("removing token from the credential store")
}

fn token() -> Result<String> {
    entry()?
        .get_password()
        .context("no Superhive token stored; run `spindle superhive login` first")
}

#[derive(Debug, Deserialize)]
struct Index {
    version: String,
    #[serde(default)]
    blocklist: Vec<serde_json::Value>,
    #[serde(default)]
    data: Vec<Entry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub archive_url: String,
    pub archive_hash: String,
    #[serde(default)]
    pub archive_size: u64,
    pub blender_version_min: Option<String>,
    pub blender_version_max: Option<String>,
    #[serde(default)]
    pub platforms: Vec<String>,
}

#[derive(Debug, Default)]
pub struct SyncReport {
    pub downloaded: Vec<AddonRecord>,
    pub up_to_date: Vec<String>,
    pub failed: Vec<(String, String)>,
}

/// Blender's platform tag for this machine, as used in `platforms`.
pub fn current_platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "aarch64") => "windows-arm64",
        ("windows", _) => "windows-x64",
        ("macos", "aarch64") => "macos-arm64",
        ("macos", _) => "macos-x64",
        (_, "aarch64") => "linux-arm64",
        _ => "linux-x64",
    }
}

fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(concat!("Spindle/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(300))
        .build()?)
}

fn get(client: &reqwest::blocking::Client, url: &str, token: &str) -> Result<reqwest::blocking::Response> {
    let resp = client
        .get(url)
        .bearer_auth(token)
        .send()
        .map_err(reqwest::Error::without_url)
        .with_context(|| format!("requesting {}", redact(url)))?;
    check(resp, url)
}

/// Archive downloads: Superhive's index accepts the Bearer header, but its download
/// endpoint answers 403 to it and only accepts the token as a `token` query parameter.
/// Try the header first (what Blender sends), then fall back to the query form.
fn get_archive(client: &reqwest::blocking::Client, url: &str, token: &str) -> Result<reqwest::blocking::Response> {
    let resp = client
        .get(url)
        .bearer_auth(token)
        .send()
        .map_err(reqwest::Error::without_url)
        .with_context(|| format!("requesting {}", redact(url)))?;
    if resp.status() != 401 && resp.status() != 403 {
        return check(resp, url);
    }
    let sep = if url.contains('?') { '&' } else { '?' };
    let with_token = format!("{url}{sep}token={}", query_encode(token));
    let resp = client
        .get(&with_token)
        .send()
        .map_err(reqwest::Error::without_url)
        .with_context(|| format!("requesting {}", redact(url)))?;
    check(resp, url)
}

fn check(resp: reqwest::blocking::Response, url: &str) -> Result<reqwest::blocking::Response> {
    match resp.status() {
        s if s.is_success() => Ok(resp),
        s if s == 401 || s == 403 => bail!("Superhive refused {} ({s}); check the token with `spindle superhive login`", redact(url)),
        s => bail!("{} returned {s}", redact(url)),
    }
}

/// Percent-encode everything outside RFC 3986 unreserved characters.
fn query_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Fetch the purchased-extensions index and pick, per id, the newest entry for this platform.
pub fn fetch_index(sp: &Spindle) -> Result<Vec<Entry>> {
    let token = token()?;
    let body = get(&client()?, &sp.config.superhive.repo_url, &token)?
        .bytes()
        .map_err(reqwest::Error::without_url)?;
    let index: Index =
        serde_json::from_slice(&body).context("Superhive index is not a Blender extensions listing")?;
    if index.version != "v1" {
        bail!("unsupported extensions index version '{}'", index.version);
    }
    let blocked: Vec<String> = index
        .blocklist
        .iter()
        .filter_map(|b| b.get("id").and_then(|v| v.as_str()).or(b.as_str()).map(str::to_owned))
        .collect();
    Ok(select_for_platform(index.data, &blocked, current_platform()))
}

fn select_for_platform(entries: Vec<Entry>, blocked: &[String], platform: &str) -> Vec<Entry> {
    let os_word = platform.split('-').next().unwrap_or(platform);
    let mut best: BTreeMap<String, Entry> = BTreeMap::new();
    for e in entries {
        if blocked.contains(&e.id) || !(e.platforms.is_empty() || e.platforms.iter().any(|p| p == platform)) {
            continue;
        }
        let better = match best.get(&e.id) {
            None => true,
            Some(cur) => match version::compare(&e.version, &cur.version) {
                std::cmp::Ordering::Greater => true,
                // Same version listed per platform: prefer the archive named for ours.
                std::cmp::Ordering::Equal => {
                    archive_name(&e.archive_url).to_lowercase().contains(os_word)
                        && !archive_name(&cur.archive_url).to_lowercase().contains(os_word)
                }
                std::cmp::Ordering::Less => false,
            },
        };
        if better {
            best.insert(e.id.clone(), e);
        }
    }
    best.into_values().collect()
}

/// Download every purchased extension version that isn't in the library yet.
pub fn sync(sp: &Spindle, library: &AddonLibrary) -> Result<SyncReport> {
    let entries = fetch_index(sp)?;
    let token = token()?;
    let client = client()?;
    let mut report = SyncReport::default();
    for e in entries {
        if library.contains(SOURCE, &e.id, &e.version) {
            report.up_to_date.push(format!("{} {}", e.id, e.version));
            continue;
        }
        match download(&client, &token, &e).and_then(|(bytes, sha)| {
            let rec = AddonRecord {
                id: e.id.clone(),
                name: e.name.clone(),
                version: e.version.clone(),
                kind: e.kind.clone(),
                source: SOURCE.into(),
                archive: sanitize(&archive_name(&e.archive_url)),
                sha256: sha,
                blender_version_min: e.blender_version_min.clone(),
                blender_version_max: e.blender_version_max.clone(),
                platforms: e.platforms.clone(),
                fetched: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                format: Format::Extension,
                origin: None,
            };
            library.store(&rec, &bytes)?;
            Ok(rec)
        }) {
            Ok(rec) => report.downloaded.push(rec),
            Err(err) => report.failed.push((format!("{} {}", e.id, e.version), format!("{err:#}"))),
        }
    }
    Ok(report)
}

/// Download one archive and verify its size and sha256 against the index.
fn download(client: &reqwest::blocking::Client, token: &str, e: &Entry) -> Result<(Vec<u8>, String)> {
    let bytes = get_archive(client, &e.archive_url, token)?
        .bytes()
        .map_err(reqwest::Error::without_url)?
        .to_vec();
    if e.archive_size != 0 && bytes.len() as u64 != e.archive_size {
        bail!("size mismatch: got {} bytes, index says {}", bytes.len(), e.archive_size);
    }
    let sha = hex::encode(Sha256::digest(&bytes));
    let expected = e.archive_hash.strip_prefix("sha256:").unwrap_or(&e.archive_hash);
    if !expected.eq_ignore_ascii_case(&sha) {
        bail!("sha256 mismatch: got {sha}, index says {expected}");
    }
    Ok((bytes, sha))
}

fn archive_name(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.rsplit('/').next().unwrap_or(path);
    percent_decode(last)
}

pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(b) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Drop query strings from URLs before they appear in errors.
fn redact(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, version: &str, url: &str, platforms: &[&str]) -> Entry {
        Entry {
            id: id.into(),
            name: id.into(),
            version: version.into(),
            kind: "add-on".into(),
            archive_url: url.into(),
            archive_hash: String::new(),
            archive_size: 0,
            blender_version_min: None,
            blender_version_max: None,
            platforms: platforms.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn picks_newest_and_platform_archive() {
        let all = ["windows-x64", "macos-arm64", "linux-x64"];
        let picked = select_for_platform(
            vec![
                entry("a", "1.0.0", "https://x/a-1.0.0-macos.zip", &all),
                entry("a", "1.0.0", "https://x/a-1.0.0-windows.zip", &all),
                entry("b", "26.1.7", "https://x/b%2026.1.7.zip", &[]),
                entry("b", "26.1.10", "https://x/b%2026.1.10.zip", &[]),
                entry("c", "1.0", "https://x/c.zip", &["macos-arm64"]),
            ],
            &[],
            "windows-x64",
        );
        let got: Vec<(String, String)> =
            picked.iter().map(|e| (e.id.clone(), archive_name(&e.archive_url))).collect();
        assert_eq!(
            got,
            vec![("a".into(), "a-1.0.0-windows.zip".into()), ("b".into(), "b 26.1.10.zip".into())]
        );
    }
}
