//! Profile compatibility with older builds. Every build shares one profile, so a build
//! older than the Blender that last saved it can meet preferences, extensions and add-ons
//! it doesn't understand. `check` names each problem and what causes it.

use crate::builds::{self, Build};
use serde::Serialize;
use std::fs;
use std::path::Path;

/// Oldest series that can read the .blend format introduced by Blender 5.0
/// (`BLENDER17-01v0500` headers). Blender 4.5 LTS learned to read it.
pub const READS_NEW_BLEND_FORMAT: (u32, u32) = (4, 5);
/// The extensions platform (blender_manifest.toml, `bl_ext.*` modules) arrived in 4.2.
pub const EXTENSIONS_SINCE: (u32, u32) = (4, 2);
/// Spindle's bridge script relies on `bpy.app.timers` (2.80+).
pub const BRIDGE_SINCE: (u32, u32) = (2, 80);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    /// Launching will lose or ignore the profile's settings.
    Blocker,
}

#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub severity: Severity,
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub build: String,
    pub build_series: Option<String>,
    pub profile: String,
    pub issues: Vec<Issue>,
}

impl Report {
    pub fn worst(&self) -> Option<Severity> {
        self.issues.iter().map(|i| i.severity).max()
    }
    /// Anything the user should see before launching.
    pub fn needs_attention(&self) -> bool {
        self.worst().is_some_and(|s| s >= Severity::Warning)
    }
}

/// Header of a .blend file: which Blender saved it, and in which file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlendHeader {
    pub series: (u32, u32),
    /// The 5.0+ format (`BLENDER17-01v0500`), unreadable before 4.5.
    pub new_format: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendRead {
    Header(BlendHeader),
    /// zstd or gzip compressed: the saving version isn't visible without decompressing.
    Compressed,
    NotBlend,
}

pub fn parse_blend_header(bytes: &[u8]) -> BlendRead {
    if bytes.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) || bytes.starts_with(&[0x1F, 0x8B]) {
        return BlendRead::Compressed;
    }
    if bytes.len() < 12 || &bytes[..7] != b"BLENDER" {
        return BlendRead::NotBlend;
    }
    let digits = |b: &[u8]| -> Option<u32> {
        b.iter().all(u8::is_ascii_digit).then(|| std::str::from_utf8(b).ok()?.parse().ok())?
    };
    // Legacy: "BLENDER" + '_'/'-' (pointer size) + 'v'/'V' (endianness) + "405".
    if matches!(bytes[7], b'_' | b'-') && matches!(bytes[8], b'v' | b'V') {
        if let Some(v) = digits(&bytes[9..12]) {
            return BlendRead::Header(BlendHeader { series: (v / 100, v % 100), new_format: false });
        }
    }
    // 5.0+: "BLENDER" + "17" (header size) + '-' + "01" (format) + 'v' + "0503".
    if bytes.len() >= 17 && bytes[7].is_ascii_digit() && bytes[9] == b'-' && matches!(bytes[12], b'v' | b'V') {
        if let Some(v) = digits(&bytes[13..17]) {
            return BlendRead::Header(BlendHeader { series: (v / 100, v % 100), new_format: true });
        }
    }
    BlendRead::NotBlend
}

fn fmt(s: (u32, u32)) -> String {
    format!("{}.{}", s.0, s.1)
}

/// The Python a build bundles, from `python3XX.dll` (Windows) or `X.Y/python/lib/python3.XX`.
pub fn build_python(build: &Build) -> Option<(u32, u32)> {
    let root = build.root();
    for e in fs::read_dir(root).ok()?.filter_map(|e| e.ok()) {
        let n = e.file_name().to_string_lossy().to_lowercase();
        if let Some(d) = n.strip_prefix("python3").and_then(|r| r.strip_suffix(".dll")) {
            if !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()) {
                return Some((3, d.parse().ok()?));
            }
        }
    }
    let lib = crate::builds::data_folder(root)?.join("python").join("lib");
    fs::read_dir(lib).ok()?.filter_map(|e| e.ok()).find_map(|e| {
        let n = e.file_name().to_string_lossy().into_owned();
        builds::parse_series(n.strip_prefix("python")?)
    })
}

/// Can a wheel file run on this Python? Reads the python/abi tags of
/// `name-version(-build)?-python-abi-platform.whl`.
fn wheel_fits(file: &str, py: (u32, u32)) -> bool {
    let stem = file.rsplit(['/', '\\']).next().unwrap_or(file).trim_end_matches(".whl");
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() < 5 {
        return true;
    }
    let (pytags, abi) = (parts[parts.len() - 3], parts[parts.len() - 2]);
    pytags.split('.').any(|tag| {
        if tag == "py3" || tag == "py2.py3" || tag.starts_with("py3") && !tag[3..].chars().any(|c| c.is_ascii_digit()) {
            return true;
        }
        let Some(num) = tag.strip_prefix("cp3").or_else(|| tag.strip_prefix("py3")) else { return false };
        let Ok(minor) = num.parse::<u32>() else { return false };
        if abi == "abi3" { py.0 == 3 && py.1 >= minor } else { py == (3, minor) }
    })
}

/// Is a module name enabled in the preferences? `bAddon.module` is stored as a
/// NUL-terminated string, so look for the name followed by NUL.
fn enabled_in(prefs: Option<&[u8]>, module: &str) -> bool {
    let Some(bytes) = prefs else { return true };
    let mut needle = module.as_bytes().to_vec();
    needle.push(0);
    bytes.windows(needle.len()).any(|w| w == needle.as_slice())
}

/// Check a profile folder against a build.
pub fn check(build: &Build, profile_name: &str, profile_root: &Path) -> Report {
    let mut issues = Vec::new();
    let series = build.series();
    let mut report = Report {
        build: build.name.clone(),
        build_series: series.map(fmt),
        profile: profile_name.to_owned(),
        issues: Vec::new(),
    };
    let Some(series) = series else {
        report.issues.push(Issue {
            severity: Severity::Info,
            title: "Unknown Blender version".into(),
            detail: "Spindle couldn't tell which Blender this is, so it can't check the profile against it.".into(),
        });
        return report;
    };
    let v = fmt(series);

    if series < BRIDGE_SINCE {
        issues.push(Issue {
            severity: Severity::Warning,
            title: "Agents won't see this Blender".into(),
            detail: format!(
                "Spindle's bridge script needs Blender {} or newer; Blender {v} won't report itself to agents.",
                fmt(BRIDGE_SINCE)
            ),
        });
    }

    // Preferences and startup file.
    let prefs_path = profile_root.join("config").join("userpref.blend");
    let prefs = fs::read(&prefs_path).ok();
    let mut prefs_plain: Option<&[u8]> = None;
    let manifests = extension_manifests(profile_root);
    let legacy = legacy_addons(profile_root);
    // When the preferences can't be read nothing is enabled, so per-add-on issues don't apply.
    let mut prefs_unreadable = false;
    if let Some(bytes) = &prefs {
        match parse_blend_header(bytes) {
            BlendRead::Header(h) => {
                prefs_plain = Some(bytes);
                if h.new_format && series < READS_NEW_BLEND_FORMAT {
                    prefs_unreadable = true;
                    let enabled = manifests.iter().filter(|m| enabled_in(prefs_plain, &m.module())).count()
                        + legacy.iter().filter(|(m, _)| enabled_in(prefs_plain, m)).count();
                    issues.push(Issue {
                        severity: Severity::Blocker,
                        title: "Preferences can't be read".into(),
                        detail: format!(
                            "userpref.blend was saved by Blender {} in the .blend format introduced in 5.0. \
                             Blender {v} can't open that format (only {} and newer can) and reports it as \"not a \
                             blend file\", so it starts with factory preferences: none of the {enabled} add-ons and \
                             extensions enabled in this profile, and no keymap or theme changes. If it then saves \
                             preferences (auto-save on exit), the shared profile is overwritten with those defaults \
                             for every build. Spindle backs the preferences up before launching it; restore them \
                             from the profile's backups.",
                            fmt(h.series),
                            fmt(READS_NEW_BLEND_FORMAT)
                        ),
                    });
                } else if h.series > series {
                    issues.push(Issue {
                        severity: Severity::Warning,
                        title: format!("Preferences were saved by newer Blender {}", fmt(h.series)),
                        detail: format!(
                            "Blender {v} reads the settings it knows and ignores ones added after {v}. If it saves \
                             preferences (auto-save on exit), the newer settings are dropped from the shared profile \
                             for every build. Spindle backs the preferences up before launching it; restore them \
                             from the profile's backups."
                        ),
                    });
                }
            }
            BlendRead::Compressed => issues.push(Issue {
                severity: Severity::Info,
                title: "Preferences are compressed".into(),
                detail: "userpref.blend is compressed, so Spindle can't tell which Blender saved it.".into(),
            }),
            BlendRead::NotBlend => issues.push(Issue {
                severity: Severity::Warning,
                title: "Preferences file is damaged".into(),
                detail: "userpref.blend isn't a valid .blend file; Blender will start with factory preferences.".into(),
            }),
        }
    }
    let startup = profile_root.join("config").join("startup.blend");
    if let Ok(bytes) = fs::read(&startup) {
        if let BlendRead::Header(h) = parse_blend_header(&bytes[..bytes.len().min(32)]) {
            if h.new_format && series < READS_NEW_BLEND_FORMAT {
                issues.push(Issue {
                    severity: Severity::Warning,
                    title: "Startup file can't be read".into(),
                    detail: format!(
                        "startup.blend was saved by Blender {} in the 5.0 .blend format; Blender {v} opens its \
                         factory startup scene instead.",
                        fmt(h.series)
                    ),
                });
            }
        }
    }

    // Extensions.
    if prefs_unreadable {
        // Covered by the blocker above.
    } else if series < EXTENSIONS_SINCE {
        let enabled: Vec<&Manifest> = manifests.iter().filter(|m| enabled_in(prefs_plain, &m.module())).collect();
        if !enabled.is_empty() {
            issues.push(Issue {
                severity: Severity::Warning,
                title: format!("{} enabled extension(s) won't load", enabled.len()),
                detail: format!(
                    "Extensions need Blender {} or newer; Blender {v} only loads legacy add-ons from scripts/addons. \
                     Not available: {}.",
                    fmt(EXTENSIONS_SINCE),
                    names(&enabled)
                ),
            });
        }
    } else {
        let python = build_python(build);
        // Compare minimums against the full version when known ("4.2.3"), else assume the newest patch.
        let full = build
            .version
            .clone()
            .filter(|bv| bv.split('.').count() >= 3)
            .unwrap_or_else(|| format!("{v}.999"));
        for m in manifests.iter().filter(|m| enabled_in(prefs_plain, &m.module())) {
            if let Some(min) = m.min.as_deref().filter(|min| crate::version::compare(min, &full).is_gt()) {
                issues.push(Issue {
                    severity: Severity::Warning,
                    title: format!("{} needs Blender {min}", m.name),
                    detail: format!(
                        "The extension '{}' {} (repository {}) declares blender_version_min = {min}; Blender {v} \
                         refuses to load it.",
                        m.name, m.version, m.repo
                    ),
                });
            } else if let Some(max) = m.max.as_deref().filter(|max| crate::version::compare(&format!("{v}.0"), max).is_ge()) {
                issues.push(Issue {
                    severity: Severity::Warning,
                    title: format!("{} is limited to Blender below {max}", m.name),
                    detail: format!(
                        "The extension '{}' {} declares blender_version_max = {max}; Blender {v} won't load it.",
                        m.name, m.version
                    ),
                });
            }
            if let Some(py) = python {
                if !m.wheels.is_empty() && !m.wheels.iter().any(|w| wheel_fits(w, py)) {
                    let tags: Vec<String> = m
                        .wheels
                        .iter()
                        .filter_map(|w| w.trim_end_matches(".whl").split('-').rev().nth(2).map(str::to_owned))
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect();
                    issues.push(Issue {
                        severity: Severity::Warning,
                        title: format!("{} bundles packages for another Python", m.name),
                        detail: format!(
                            "The extension '{}' ships Python wheels tagged {}; Blender {v} runs Python {}.{}, so \
                             its bundled packages fail to import.",
                            m.name,
                            tags.join(", "),
                            py.0,
                            py.1
                        ),
                    });
                }
            }
        }
    }

    // Legacy add-ons (scripts/addons).
    for (module, min) in legacy.iter().filter(|_| !prefs_unreadable).cloned() {
        if min > series && enabled_in(prefs_plain, &module) {
            issues.push(Issue {
                severity: Severity::Warning,
                title: format!("{module} was made for Blender {}", fmt(min)),
                detail: format!(
                    "The add-on '{module}' declares bl_info \"blender\": ({}, {}, 0). Blender {v} marks it as made for \
                     a newer version and it may fail to register if it uses newer API.",
                    min.0, min.1
                ),
            });
        }
    }

    issues.sort_by(|a, b| b.severity.cmp(&a.severity));
    report.issues = issues;
    report
}

fn names(list: &[&Manifest]) -> String {
    let mut n: Vec<String> = list.iter().map(|m| m.name.clone()).collect();
    if n.len() > 8 {
        let more = n.len() - 8;
        n.truncate(8);
        n.push(format!("and {more} more"));
    }
    n.join(", ")
}

#[derive(Debug)]
struct Manifest {
    repo: String,
    id: String,
    name: String,
    version: String,
    min: Option<String>,
    max: Option<String>,
    wheels: Vec<String>,
}

impl Manifest {
    fn module(&self) -> String {
        format!("bl_ext.{}.{}", self.repo, self.id)
    }
}

/// `<profile>/extensions/<repo>/<id>/blender_manifest.toml`
fn extension_manifests(root: &Path) -> Vec<Manifest> {
    let mut out = Vec::new();
    let Ok(repos) = fs::read_dir(root.join("extensions")) else { return out };
    for repo in repos.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()) {
        let repo_name = repo.file_name().to_string_lossy().into_owned();
        if repo_name.starts_with('.') {
            continue;
        }
        let Ok(exts) = fs::read_dir(repo.path()) else { continue };
        for ext in exts.filter_map(|e| e.ok()) {
            let Ok(text) = fs::read_to_string(ext.path().join("blender_manifest.toml")) else { continue };
            let Ok(t) = text.parse::<toml::Table>() else { continue };
            let s = |k: &str| t.get(k).and_then(|v| v.as_str()).map(str::to_owned);
            let dir = ext.file_name().to_string_lossy().into_owned();
            out.push(Manifest {
                repo: repo_name.clone(),
                // The module name is the folder name (normally equal to the manifest id).
                id: dir.clone(),
                name: s("name").unwrap_or(dir),
                version: s("version").unwrap_or_default(),
                min: s("blender_version_min"),
                max: s("blender_version_max"),
                wheels: t
                    .get("wheels")
                    .and_then(|w| w.as_array())
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
                    .unwrap_or_default(),
            });
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// (module, bl_info "blender" series) of add-ons in `<profile>/scripts/addons`.
fn legacy_addons(root: &Path) -> Vec<(String, (u32, u32))> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(root.join("scripts").join("addons")) else { return out };
    for e in entries.filter_map(|e| e.ok()) {
        let path = e.path();
        let (module, file) = if path.is_dir() {
            (e.file_name().to_string_lossy().into_owned(), path.join("__init__.py"))
        } else if path.extension().is_some_and(|x| x == "py") {
            (path.file_stem().unwrap().to_string_lossy().into_owned(), path.clone())
        } else {
            continue;
        };
        if let Some(series) = fs::read_to_string(&file).ok().as_deref().and_then(bl_info_blender) {
            out.push((module, series));
        }
    }
    out.sort();
    out
}

/// `"blender": (4, 2, 0)` inside bl_info -> (4, 2).
fn bl_info_blender(src: &str) -> Option<(u32, u32)> {
    let start = src.find("bl_info")?;
    let rest = &src[start..];
    let key = rest.find("\"blender\"").or_else(|| rest.find("'blender'"))?;
    let after = &rest[key + 9..];
    let open = after.find('(')?;
    let close = after[open..].find(')')? + open;
    let nums: Vec<u32> = after[open + 1..close].split(',').filter_map(|n| n.trim().parse().ok()).collect();
    Some((*nums.first()?, *nums.get(1)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_headers() {
        assert_eq!(
            parse_blend_header(b"BLENDER17-01v0503REND"),
            BlendRead::Header(BlendHeader { series: (5, 3), new_format: true })
        );
        assert_eq!(
            parse_blend_header(b"BLENDER-v405REND"),
            BlendRead::Header(BlendHeader { series: (4, 5), new_format: false })
        );
        assert_eq!(
            parse_blend_header(b"BLENDER_v293REND"),
            BlendRead::Header(BlendHeader { series: (2, 93), new_format: false })
        );
        assert_eq!(parse_blend_header(&[0x28, 0xB5, 0x2F, 0xFD, 0, 0]), BlendRead::Compressed);
        assert_eq!(parse_blend_header(b"hello world, not a blend"), BlendRead::NotBlend);
    }

    #[test]
    fn wheels() {
        assert!(wheel_fits("./wheels/numpy-2.1.0-cp311-cp311-win_amd64.whl", (3, 11)));
        assert!(!wheel_fits("./wheels/numpy-2.1.0-cp313-cp313-win_amd64.whl", (3, 11)));
        assert!(wheel_fits("./wheels/pkg-1.0-cp38-abi3-win_amd64.whl", (3, 11)));
        assert!(!wheel_fits("./wheels/pkg-1.0-cp312-abi3-win_amd64.whl", (3, 11)));
        assert!(wheel_fits("./wheels/six-1.16.0-py2.py3-none-any.whl", (3, 11)));
        assert!(wheel_fits("./wheels/tool-1.0-py3-none-any.whl", (3, 7)));
    }

    #[test]
    fn bl_info_versions() {
        let src = "bl_info = {\n    \"name\": \"X\",\n    \"blender\": (4, 2, 0),\n}\n";
        assert_eq!(bl_info_blender(src), Some((4, 2)));
        assert_eq!(bl_info_blender("bl_info = {'blender': (2, 80, 0)}"), Some((2, 80)));
        assert_eq!(bl_info_blender("print('no info')"), None);
    }

    #[test]
    fn enabled_detection() {
        let prefs = b"xx\0bl_ext.user_default.powersave\0yy";
        assert!(enabled_in(Some(prefs), "bl_ext.user_default.powersave"));
        assert!(!enabled_in(Some(prefs), "bl_ext.user_default.power"));
        assert!(enabled_in(None, "anything"));
    }
}
