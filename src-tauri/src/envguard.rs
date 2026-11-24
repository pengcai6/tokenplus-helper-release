//! Environment guard — port of helper/envguard.py.
//!
//! Finds shell/launchctl/hosts settings that would bypass TokenPlus and
//! disables the fixable ones (lines are commented out, never deleted).

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

const BASE_VARS: [&str; 3] = ["ANTHROPIC_BASE_URL", "OPENAI_BASE_URL", "OPENAI_API_BASE"];
const DIR_VARS: [&str; 2] = ["CODEX_HOME", "CLAUDE_CONFIG_DIR"];
const KEY_VARS: [&str; 3] = ["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY", "OPENAI_API_KEY"];
const TP_HOSTS: [&str; 2] = ["tokenplus.cn", "api.tokenplus.cn"];
const SHELL_FILES: [&str; 6] = [
    ".zshrc", ".zprofile", ".zshenv", ".bash_profile", ".bashrc", ".profile",
];
const DISABLED_MARK: &str = "# [TokenPlus 助手已停用]";

fn watched_vars() -> &'static [&'static str] {
    &["ANTHROPIC_BASE_URL", "OPENAI_BASE_URL", "OPENAI_API_BASE",
      "CODEX_HOME", "CLAUDE_CONFIG_DIR",
      "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY", "OPENAI_API_KEY"]
}

fn assignment() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(&format!(
            r"^\s*(?:export\s+)?({})\s*=(.*)$",
            watched_vars().join("|")
        ))
        .unwrap()
    })
}

fn is_tp_url(value: &str) -> bool {
    let host = url_host(value.trim());
    !host.is_empty()
        && TP_HOSTS
            .iter()
            .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

/// urlparse().hostname — "" when there is no real host.
fn url_host(value: &str) -> String {
    let rest = match value.split_once("://") {
        Some((_, rest)) => rest,
        None => return String::new(),
    };
    let authority = rest
        .split(|c| c == '/' || c == '?' || c == '#')
        .next()
        .unwrap_or_default();
    let authority = authority.rsplit('@').next().unwrap_or_default();
    let host = if authority.starts_with('[') {
        authority
            .split(']')
            .next()
            .unwrap_or_default()
            .trim_start_matches('[')
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    host.to_lowercase()
}

fn mask(name: &str, value: &str) -> String {
    if KEY_VARS.contains(&name) {
        if value.chars().count() > 8 {
            let head: String = value.chars().take(5).collect();
            let tail: String = value
                .chars()
                .rev()
                .take(2)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            return format!("{head}…{tail}");
        }
        return "***".to_string();
    }
    if value.chars().count() > 120 {
        let head: String = value.chars().take(117).collect();
        return format!("{head}…");
    }
    value.to_string()
}

fn unquote(value: &str) -> String {
    static TRAILING_COMMENT: OnceLock<regex::Regex> = OnceLock::new();
    let re = TRAILING_COMMENT.get_or_init(|| regex::Regex::new(r"\s+#.*$").unwrap());
    let trimmed = re.replace(value.trim(), "").to_string();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() >= 2
        && (chars[0] == '\'' || chars[0] == '"')
        && chars[chars.len() - 1] == chars[0]
    {
        return chars[1..chars.len() - 1].iter().collect();
    }
    trimmed
}

fn suspicious(values: &[Value]) -> Vec<Value> {
    let foreign: HashSet<String> = values
        .iter()
        .filter(|item| {
            BASE_VARS.contains(&item["name"].as_str().unwrap_or_default())
                && !item["value"].as_str().unwrap_or_default().is_empty()
                && !is_tp_url(item["value"].as_str().unwrap_or_default())
        })
        .map(|item| {
            item["name"]
                .as_str()
                .unwrap_or_default()
                .split('_')
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    let mut out = Vec::new();
    for item in values {
        let name = item["name"].as_str().unwrap_or_default();
        let value = item["value"].as_str().unwrap_or_default();
        if value.is_empty() {
            continue;
        }
        if BASE_VARS.contains(&name) {
            if !is_tp_url(value) {
                out.push(item.clone());
            }
        } else if DIR_VARS.contains(&name) {
            out.push(item.clone());
        } else if KEY_VARS.contains(&name)
            && foreign.contains(name.split('_').next().unwrap_or_default())
        {
            out.push(item.clone());
        }
    }
    out
}

fn parse_shell_file(text: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        if let Some(m) = assignment().captures(raw) {
            out.push(json!({
                "name": m.get(1).unwrap().as_str(),
                "value": unquote(m.get(2).unwrap().as_str()),
                "line": index,
            }));
        }
    }
    out
}

fn hosts_redirects(text: &str) -> Vec<Value> {
    static COMMENT: OnceLock<regex::Regex> = OnceLock::new();
    let re = COMMENT.get_or_init(|| regex::Regex::new(r"#.*$").unwrap());
    let mut out = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let content = re.replace(raw, "").trim().to_string();
        if content.is_empty() {
            continue;
        }
        let names: Vec<String> = content
            .split_whitespace()
            .skip(1)
            .map(|n| n.to_lowercase())
            .collect();
        if names.iter().any(|n| {
            TP_HOSTS
                .iter()
                .any(|d| n == d || n.ends_with(&format!(".{d}")))
        }) {
            out.push(json!({"line": index, "value": content}));
        }
    }
    out
}

fn run(command: &str, args: &[&str]) -> (bool, String) {
    let child = Command::new(command)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(_) => return (false, String::new()),
    };
    // subprocess.run(timeout=8): kill on timeout, like Python.
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut buf = Vec::new();
                if let Some(mut stdout) = child.stdout.take() {
                    use std::io::Read;
                    let _ = stdout.read_to_end(&mut buf);
                }
                return (status.success(), String::from_utf8_lossy(&buf).into_owned());
            }
            Ok(None) => {
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return (false, String::new());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return (false, String::new()),
        }
    }
}

fn read(path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn write(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let temp = path.with_file_name(format!(
        "{}.yituo-tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, path)
}

/// Scan shell rc files / environment for overrides that would route tools
/// away from TokenPlus. Returns the issue list (id/text/fixable/…).
pub fn scan_environment(home: &str, platform: &str) -> Vec<Value> {
    let mut issues = Vec::new();
    let mut add = |source: &str, where_: &str, name: &str, value: &str, fixable: bool, line: Option<i64>| {
        issues.push(json!({
            "id": format!("{source}:{where_}:{name}:{}", line.map(|l| l.to_string()).unwrap_or_default()),
            "source": source,
            "name": name,
            "where": where_,
            "value": mask(name, value),
            "fixable": fixable,
            "line": line,
        }));
    };

    if platform == "darwin" {
        let home_path = std::path::Path::new(home);
        for file in SHELL_FILES {
            let path = home_path.join(file);
            let text = match read(&path) {
                Some(text) if !text.is_empty() => text,
                _ => continue,
            };
            for item in suspicious(&parse_shell_file(&text)) {
                add(
                    "shell",
                    &format!("~/{file}"),
                    item["name"].as_str().unwrap_or_default(),
                    item["value"].as_str().unwrap_or_default(),
                    true,
                    item["line"].as_i64(),
                );
            }
        }
        let mut launchd = Vec::new();
        for name in watched_vars() {
            let (ok, stdout) = run("/bin/launchctl", &["getenv", name]);
            let value = if ok { stdout.trim().to_string() } else { String::new() };
            if !value.is_empty() {
                launchd.push(json!({"name": name, "value": value}));
            }
        }
        for item in suspicious(&launchd) {
            add(
                "launchctl",
                "launchctl",
                item["name"].as_str().unwrap_or_default(),
                item["value"].as_str().unwrap_or_default(),
                true,
                None,
            );
        }
    }
    if let Some(hosts) = read(std::path::Path::new("/etc/hosts")) {
        for redirect in hosts_redirects(&hosts) {
            add(
                "hosts",
                "/etc/hosts",
                "hosts",
                redirect["value"].as_str().unwrap_or_default(),
                false,
                redirect["line"].as_i64(),
            );
        }
    }
    issues
}

/// Fix issues; `backup` snapshots a file's current text before rewriting.
pub fn fix_environment<F>(home: &str, issues: &[Value], mut backup: F) -> Vec<Value>
where
    F: FnMut(&str, &str),
{
    let mut fixed: Vec<Value> = Vec::new();
    let mut by_file: HashMap<String, Vec<Value>> = HashMap::new();
    for issue in issues {
        if issue["fixable"].as_bool().unwrap_or(false)
            && issue["source"].as_str() == Some("shell")
        {
            by_file
                .entry(issue["where"].as_str().unwrap_or_default().to_string())
                .or_default()
                .push(issue.clone());
        }
    }
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    for (where_, items) in by_file {
        let relative = where_.strip_prefix("~/").unwrap_or(&where_);
        let path = std::path::Path::new(home).join(relative);
        let text = match read(&path) {
            Some(text) => text,
            None => continue,
        };
        let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
        let still: Vec<Value> = items
            .iter()
            .filter(|issue| {
                let line = issue["line"].as_i64().unwrap_or(-1);
                if line < 0 || line as usize >= lines.len() {
                    return false;
                }
                match assignment().captures(&lines[line as usize]) {
                    Some(m) => {
                        m.get(1).unwrap().as_str()
                            == issue["name"].as_str().unwrap_or_default()
                    }
                    None => false,
                }
            })
            .cloned()
            .collect();
        if still.is_empty() {
            continue;
        }
        backup(&path.to_string_lossy(), &text);
        let mark = DISABLED_MARK.replace(']', &format!(" {date}]"));
        for item in &still {
            if let Some(line) = item["line"].as_i64() {
                lines[line as usize] = format!("{mark} {}", lines[line as usize]);
            }
        }
        if write(&path, &lines.join("\n")).is_ok() {
            fixed.extend(still);
        }
    }
    for issue in issues {
        if !issue["fixable"].as_bool().unwrap_or(false)
            || issue["source"].as_str() != Some("launchctl")
        {
            continue;
        }
        let (ok, _) = run(
            "/bin/launchctl",
            &["unsetenv", issue["name"].as_str().unwrap_or_default()],
        );
        if ok {
            fixed.push(issue.clone());
        }
    }
    fixed
}

/// describe_issue — renderer-facing explanation (used by selfcheck).
pub fn describe_issue(issue: &Value) -> String {
    let source = issue["source"].as_str().unwrap_or_default();
    let where_ = issue["where"].as_str().unwrap_or_default();
    let name = issue["name"].as_str().unwrap_or_default();
    let value = issue["value"].as_str().unwrap_or_default();
    if source == "hosts" {
        return format!(
            "hosts 文件把 TokenPlus 域名指向了别处（{value}），需要管理员权限修改：{where_}"
        );
    }
    let place = if source == "shell" {
        where_.to_string()
    } else if source == "launchctl" {
        "系统登录环境".to_string()
    } else {
        where_.to_string()
    };
    format!("{place} 里的 {name}={value} 会让工具绕过 TokenPlus")
}
