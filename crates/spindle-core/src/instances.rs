use crate::Spindle;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

/// The bridge rewrites its heartbeat every few seconds; older files belong to
/// Blenders that crashed or were killed and are cleaned up.
pub const STALE_AFTER_SECS: f64 = 20.0;

/// A Blender that Spindle launched and that is running right now, as reported by
/// the bridge script inside it (see `bridge/spindle_bridge.py`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    pub pid: u32,
    pub blender_version: String,
    pub binary: String,
    pub profile: Option<String>,
    /// Open .blend file, empty when unsaved.
    pub file: String,
    pub started: f64,
    pub heartbeat: f64,
    /// MCP add-ons enabled in this Blender and where their socket listens.
    #[serde(default)]
    pub mcp: Vec<McpEndpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpEndpoint {
    pub addon: String,
    pub host: Option<String>,
    pub port: Option<u16>,
}

/// Live instances; stale heartbeat files are removed.
pub fn list(sp: &Spindle) -> Result<Vec<Instance>> {
    let dir = sp.instances_dir();
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64();
    let mut live = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Instance>(&t).ok()) {
            Some(inst) if now - inst.heartbeat <= STALE_AFTER_SECS => live.push(inst),
            _ => {
                let _ = fs::remove_file(&path);
            }
        }
    }
    live.sort_by(|a, b| a.started.total_cmp(&b.started));
    Ok(live)
}
