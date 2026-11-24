//! Shared types for the TokenPlus helper backend.
//! Mirrors the dict shapes produced by the Python implementation so the
//! renderer (renderer/app.js) sees identical JSON over the bridge.

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

/// Execution context for adapters — same role as the Python `env` dict.
#[derive(Debug, Clone)]
pub struct EnvCtx {
    pub home: PathBuf,
    pub env: HashMap<String, String>,
    pub platform: &'static str, // "darwin" | "win32" | "linux"
}

/// Snapshot of a file the plan intends to touch (fsutil.snapshot).
#[derive(Debug, Clone)]
pub struct FileSnap {
    pub path: PathBuf,
    pub original: Option<String>,
}

/// What an adapter plans to change (plan()).
#[derive(Debug, Clone)]
pub struct Plan {
    pub summary: String,
    pub files: Vec<FileSnap>,
}

/// Result of adapter.detect() — serialized to the renderer as-is.
#[derive(Debug, Clone, Serialize)]
pub struct Detected {
    pub installed: bool,
    pub supported: bool,
    pub version: String,
    pub files: Vec<String>,
    pub message: String,
}

/// Connection probe result (verify()).
#[derive(Debug, Clone, Serialize)]
pub struct Probe {
    pub ok: bool,
    pub latency: f64,
    pub message: String,
}

/// Per-tool catalog metadata (helper/catalog.py TOOL_META).
#[derive(Debug, Clone)]
pub struct ToolMeta {
    pub name: &'static str,
    pub kind: &'static str,
    pub restart: &'static str,
    pub keys: &'static str,
    pub colors: [&'static str; 3],
    pub guide_tool: &'static str,
}

/// Managed record stored in state.json (set_managed).
pub type ManagedMap = HashMap<String, Value>;
