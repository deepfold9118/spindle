//! spindle-mcp: an MCP server (stdio, newline-delimited JSON-RPC) that lets agents see
//! which Blenders Spindle launched are running *right now*, and where each one's
//! Blender MCP add-on listens, instead of guessing from what is installed.
//!
//! Hand-rolled to keep the scaffold small; swap for the `rmcp` SDK when the tool set grows.

use anyhow::Result;
use serde_json::{Value, json};
use spindle_core::profile::Profile;
use spindle_core::{Spindle, builds, compat, instances, launch};
use std::io::{BufRead, Write};

const PROTOCOL_VERSION: &str = "2025-06-18";

fn main() -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&msg),
            Err(e) => Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
        };
        if let Some(resp) = response {
            writeln!(stdout, "{resp}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

fn handle(msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    // Notifications (no id) never get a response.
    let id = id?;
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => ok(id, json!({
            "protocolVersion": params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSION),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "spindle", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "Use list_running_blenders to find the Blender instances that are actually running \
                and the host/port of each one's Blender MCP add-on. Connect your Blender MCP client to that port."
        })),
        "ping" => ok(id, json!({})),
        "tools/list" => ok(id, json!({ "tools": tools() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match call(name, &args) {
                Ok(v) => ok(id, json!({
                    "content": [{ "type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default() }],
                    "structuredContent": v,
                    "isError": false
                })),
                Err(e) => ok(id, json!({
                    "content": [{ "type": "text", "text": format!("{e:#}") }],
                    "isError": true
                })),
            }
        }
        _ => error(id, -32601, &format!("method not found: {method}")),
    })
}

fn tools() -> Value {
    json!([
        {
            "name": "list_running_blenders",
            "description": "Blender instances launched by Spindle that are running right now: pid, version, \
                executable, profile, open file, and the host/port of any enabled Blender MCP add-on.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "list_builds",
            "description": "Blender builds available in the Spindle library.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "launch_blender",
            "description": "Start a Blender build with a shared Spindle profile. It appears in \
                list_running_blenders a few seconds after its window opens.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "build": { "type": "string", "description": "Build name or version, e.g. \"5.3\"" },
                    "profile": { "type": "string", "description": "Profile name (default profile if omitted)" },
                    "file": { "type": "string", "description": "Optional .blend file to open" },
                    "force": { "type": "boolean", "description": "Launch even when this build can't read the profile (older builds); off by default" }
                },
                "required": ["build"]
            }
        }
    ])
}

fn call(name: &str, args: &Value) -> Result<Value> {
    let sp = Spindle::load()?;
    match name {
        "list_running_blenders" => Ok(json!({ "instances": instances::list(&sp)? })),
        "list_builds" => Ok(json!({ "builds": builds::list(&sp)? })),
        "launch_blender" => {
            let query = args.get("build").and_then(Value::as_str).unwrap_or_default();
            let build = builds::find(&sp, query)?;
            let profile = Profile::open(&sp, &sp.profile_name(args.get("profile").and_then(Value::as_str)))?;
            let extra: Vec<String> = args.get("file").and_then(Value::as_str).map(|f| vec![f.to_owned()]).unwrap_or_default();
            let report = compat::check(&build, &profile.name, &profile.root);
            let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
            if report.worst() == Some(compat::Severity::Blocker) && !force {
                anyhow::bail!(
                    "not launched: {} can't use profile '{}' safely: {}. Pass force=true to launch anyway, \
                     or use a profile made for older builds. Report: {}",
                    build.name,
                    profile.name,
                    report.issues[0].title,
                    serde_json::to_string(&report.issues)?
                );
            }
            let pid = launch::launch(&sp, &build, &profile, &extra)?;
            let warnings: Vec<&compat::Issue> =
                report.issues.iter().filter(|i| i.severity >= compat::Severity::Warning).collect();
            Ok(json!({ "pid": pid, "build": build.name, "profile": profile.name, "compatibility_warnings": warnings }))
        }
        _ => anyhow::bail!("unknown tool: {name}"),
    }
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}
