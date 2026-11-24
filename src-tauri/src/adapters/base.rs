//! Adapter helpers — port of helper/adapters/base.py.

use anyhow::{bail, Result};
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::types::EnvCtx;

/// Regex-valid stand-in key for previews; service masks all sk-… anyway.
pub const PREVIEW_KEY: &str = "sk-0000000000000000";

/// Python truthiness for a JSON value.
pub(crate) fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

pub(crate) fn truthy_opt(v: Option<&Value>) -> bool {
    v.map(truthy).unwrap_or(false)
}

/// Python str() for a scalar-ish JSON value (f-string `"{v}"`).
pub(crate) fn py_display(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "None".to_string(),
        Some(Value::Bool(b)) => if *b { "True" } else { "False" }.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(_)) | Some(Value::Object(_)) => {
            serde_json::to_string(v.unwrap()).unwrap_or_default()
        }
    }
}

/// Python type name for error messages mimicking AttributeError/TypeError.
pub(crate) fn py_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() { "int" } else { "float" }
        }
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

/// Copy of a built config with every embedded key swapped for PREVIEW_KEY.
pub fn masked_config(config: &Value) -> Value {
    let mut clone = config.clone();
    let Some(obj) = clone.as_object_mut() else { return clone };
    if obj.get("api_key").map(truthy).unwrap_or(false) {
        obj.insert("api_key".to_string(), Value::String(PREVIEW_KEY.into()));
    }
    if let Some(Value::Array(providers)) = obj.get_mut("providers") {
        for p in providers.iter_mut() {
            let Some(pmap) = p.as_object_mut() else { continue };
            if pmap.get("api_key").map(truthy).unwrap_or(false) {
                pmap.insert("api_key".into(), PREVIEW_KEY.into());
            }
        }
    }
    clone
}

/// Cap a multi-line preview, noting how many lines were dropped.
pub fn clip(text: &str, limit: usize) -> String {
    let lines: Vec<&str> = text.trim_end_matches('\n').split('\n').collect();
    if lines.len() <= limit {
        return lines.join("\n");
    }
    format!(
        "{}\n…（预览省略余下 {} 行）",
        lines[..limit].join("\n"),
        lines.len() - limit
    )
}

fn bad_model_chars() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r#"[\r\n"\\]"#).unwrap())
}

/// config["models"] dict or the four tier keys; every value must be a
/// non-empty filename-safe string.
pub fn claude_models(config: &Value) -> Result<HashMap<String, String>> {
    let mut value: HashMap<String, String> = HashMap::new();
    if let Some(models) = config.get("models").and_then(Value::as_object) {
        for (k, v) in models {
            let s = match v {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => py_display(Some(other)),
            };
            value.insert(k.clone(), s);
        }
    } else {
        for k in ["default", "opus", "sonnet", "haiku"] {
            value.insert(
                k.to_string(),
                config
                    .get(k)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            );
        }
    }
    if value
        .values()
        .any(|m| m.is_empty() || bad_model_chars().is_match(m))
    {
        bail!("服务器返回的模型信息不完整");
    }
    Ok(value)
}

/// json.dumps(value) — same escaping serde_json uses for strings.
pub fn toml_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

/// nested(value, path) — dict-of-dicts lookup, None when any hop misses.
#[allow(dead_code)] // faithful port of base.py helper kept for contract parity
pub fn nested<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.as_object()?.get(*key)?;
    }
    Some(current)
}

/// "~/..." when path is under home.
pub fn display_path(home: &str, path: &str) -> String {
    let prefix = format!("{}/", home);
    match path.strip_prefix(&prefix) {
        Some(rest) => format!("~/{}", rest),
        None => path.to_string(),
    }
}

#[cfg(unix)]
fn is_exec(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_exec(path: &Path) -> bool {
    path.exists()
}

/// Find a CLI binary: ~/.local/bin, ~/.{name}/bin, then PATH.
pub fn executable(name: &str, env: &EnvCtx) -> Option<PathBuf> {
    let candidates = [
        env.home.join(".local").join("bin").join(name),
        env.home.join(format!(".{}", name)).join("bin").join(name),
    ];
    for candidate in &candidates {
        if candidate.exists() {
            return Some(candidate.clone());
        }
    }
    let sep = if env.platform == "win32" { ';' } else { ':' };
    if let Some(path_var) = env.env.get("PATH") {
        for directory in path_var.split(sep) {
            if directory.is_empty() {
                continue;
            }
            let candidate = Path::new(directory).join(name);
            if candidate.exists() && is_exec(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// Run a command with stdout/stderr captured and a wall-clock timeout
/// (subprocess.run(timeout=…) equivalent — kills on expiry).
pub(crate) fn run_command(cmd: &mut Command, timeout_secs: u64) -> Result<Output> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        if child.try_wait()?.is_some() {
            return Ok(child.wait_with_output()?);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("timed out");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `binary --version` first line, capped at 80 chars; "已安装" on failure.
pub fn version_of(binary: Option<&PathBuf>) -> String {
    let Some(binary) = binary else {
        return String::new();
    };
    let mut cmd = Command::new(binary);
    cmd.arg("--version");
    let out = match run_command(&mut cmd, 3) {
        Ok(o) => o,
        Err(_) => return "已安装".to_string(),
    };
    let pick = if !out.stdout.is_empty() {
        &out.stdout
    } else {
        &out.stderr
    };
    let text = match String::from_utf8(pick.clone()) {
        Ok(t) => t,
        Err(_) => return "已安装".to_string(),
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "已安装".to_string();
    }
    trimmed
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(80)
        .collect()
}

/// /Applications or ~/Applications .app bundle (env["applications"] is
/// always None in default_env, so only the default roots apply).
pub fn mac_app(name: &str, env: &EnvCtx) -> Option<PathBuf> {
    let roots = [
        PathBuf::from("/Applications"),
        env.home.join("Applications"),
    ];
    for root in roots {
        let path = root.join(format!("{}.app", name));
        if path.exists() {
            return Some(path);
        }
    }
    None
}

/// Parse TOML, preserving the document like Python's tomllib loads
/// (parse errors bubble up as anyhow errors).
pub fn toml_parse(text: &str) -> Result<toml_edit::DocumentMut> {
    Ok(text.parse::<toml_edit::DocumentMut>()?)
}

fn toml_item_get<'a>(item: &'a toml_edit::Item, key: &str) -> Option<&'a toml_edit::Item> {
    item.as_table_like().and_then(|t| t.get(key))
}

/// nested() equivalent for a parsed TOML document.
pub(crate) fn toml_nested<'a>(
    doc: &'a toml_edit::DocumentMut,
    path: &[&str],
) -> Option<&'a toml_edit::Item> {
    if path.is_empty() {
        return None;
    }
    let mut item = doc.as_table().get(path[0])?;
    for key in &path[1..] {
        item = toml_item_get(item, key)?;
    }
    Some(item)
}

/// toml_nested → &str convenience.
pub(crate) fn toml_nested_str<'a>(
    doc: &'a toml_edit::DocumentMut,
    path: &[&str],
) -> Option<&'a str> {
    toml_nested(doc, path).and_then(|i| i.as_str())
}

/// /usr/bin/osascript with the default 5s timeout.
#[allow(dead_code)] // mirrors base.py's public helper surface
pub fn run_osascript(lines: &[&str]) -> Result<String> {
    run_osascript_timeout(lines, 5)
}

/// /usr/bin/osascript -e … — raises on non-zero exit like Python.
pub(crate) fn run_osascript_timeout(lines: &[&str], timeout_secs: u64) -> Result<String> {
    let mut cmd = Command::new("/usr/bin/osascript");
    for line in lines {
        cmd.arg("-e").arg(line);
    }
    let out = run_command(&mut cmd, timeout_secs)?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        bail!("{}", if stderr.is_empty() { "osascript failed" } else { &stderr });
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

// ---------- ordered JSON rendering (json.dumps byte-faithful previews) ----------

/// Ordered JSON node so previews keep the insertion order Python's dicts
/// had (serde_json::Map is a BTreeMap and would sort keys).
pub(crate) enum J {
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

impl J {
    pub fn s(text: &str) -> J {
        J::Str(text.to_string())
    }
    pub fn i(n: i64) -> J {
        J::Int(n)
    }
}

pub(crate) fn json_quote(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

fn dump_indent(v: &J, level: usize, out: &mut String) {
    let ind = "  ".repeat(level);
    let ind1 = "  ".repeat(level + 1);
    match v {
        J::Null => out.push_str("null"),
        J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        J::Int(n) => out.push_str(&n.to_string()),
        J::Str(s) => out.push_str(&json_quote(s)),
        J::Arr(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                out.push_str(&ind1);
                dump_indent(item, level + 1, out);
                if i + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&ind);
            out.push(']');
        }
        J::Obj(pairs) => {
            if pairs.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{\n");
            for (i, (k, val)) in pairs.iter().enumerate() {
                out.push_str(&ind1);
                out.push_str(&json_quote(k));
                out.push_str(": ");
                dump_indent(val, level + 1, out);
                if i + 1 < pairs.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&ind);
            out.push('}');
        }
    }
}

/// json.dumps(v, indent=2, ensure_ascii=False) + "\n" — like fsutil.json_text
/// but preserving insertion order for hand-built preview objects.
pub(crate) fn json_text_j(v: &J) -> String {
    let mut s = String::new();
    dump_indent(v, 0, &mut s);
    s.push('\n');
    s
}

/// json.dumps(v, ensure_ascii=False) — compact with ", "/": " separators.
pub(crate) fn json_compact_j(v: &J) -> String {
    fn go(v: &J, out: &mut String) {
        match v {
            J::Null => out.push_str("null"),
            J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            J::Int(n) => out.push_str(&n.to_string()),
            J::Str(s) => out.push_str(&json_quote(s)),
            J::Arr(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    go(item, out);
                }
                out.push(']');
            }
            J::Obj(pairs) => {
                out.push('{');
                for (i, (k, val)) in pairs.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&json_quote(k));
                    out.push_str(": ");
                    go(val, out);
                }
                out.push('}');
            }
        }
    }
    let mut s = String::new();
    go(v, &mut s);
    s
}
