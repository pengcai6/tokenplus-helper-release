//! Codex adapter — merges TokenPlus provider into ~/.codex/config.toml.
//!
//! Port of helper/adapters/codex.py. The TOML merge is a literal port of
//! the Python line-level merge (byte-identical output); toml_edit is used
//! wherever Python parsed TOML with tomllib.

use anyhow::{bail, Result};
use regex::Regex;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use super::base;
use super::Adapter;
use crate::fsutil;
use crate::tpapi::{clean_model, required_base, required_key, TokenPlusClient};
use crate::types::{Detected, EnvCtx, Plan, Probe};

const CODEX_BUNDLES: [&str; 3] = [
    "com.openai.codex",
    "com.openai.codex.alpha",
    "com.openai.codex.beta",
];

const BEGIN_BASE: &str = "# >>> tokenplus codex setup (base) >>>";
const END_BASE: &str = "# <<< tokenplus codex setup (base) <<<";
const BEGIN_PROVIDER: &str = "# >>> tokenplus codex setup (provider) >>>";
const END_PROVIDER: &str = "# <<< tokenplus codex setup (provider) <<<";

const LEGACY_BEGIN: [&str; 6] = [
    BEGIN_BASE,
    BEGIN_PROVIDER,
    "# >>> yituohub codex setup (base) >>>",
    "# >>> yituohub codex setup (provider) >>>",
    "# >>> zhishuhub codex setup (base) >>>",
    "# >>> zhishuhub codex setup (provider) >>>",
];

const LEGACY_END: [&str; 6] = [
    END_BASE,
    END_PROVIDER,
    "# <<< yituohub codex setup (base) <<<",
    "# <<< yituohub codex setup (provider) <<<",
    "# <<< zhishuhub codex setup (base) <<<",
    "# <<< zhishuhub codex setup (provider) <<<",
];

fn managed_keys_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"^(model|model_provider|model_reasoning_effort|model_catalog_json|profile|approval_policy|sandbox_mode)[ \t]*=",
        )
        .unwrap()
    })
}

fn blank_runs_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\n{3,}").unwrap())
}

fn locale_key_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^localeOverride[ \t]*=").unwrap())
}

fn provider_id_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^[A-Za-z0-9_-]+$").unwrap())
}

fn codex_home(env: &EnvCtx) -> PathBuf {
    env.env
        .get("CODEX_HOME")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| env.home.join(".codex"))
}

fn paths(env: &EnvCtx) -> (PathBuf, PathBuf) {
    let root = codex_home(env);
    (root.join("config.toml"), root.join("models_cache.json"))
}

#[allow(dead_code)] // `path` is carried for parity with the Python {"path","id"} dict
struct MacApp {
    path: PathBuf,
    id: String,
}

/// plutil -extract CFBundleIdentifier raw — via the plist crate.
fn bundle_id(info_plist: &PathBuf) -> Option<String> {
    let value = plist::Value::from_file(info_plist).ok()?;
    value
        .as_dictionary()?
        .get("CFBundleIdentifier")?
        .as_string()
        .map(|s| s.trim().to_string())
}

fn find_codex_mac_app(env: &EnvCtx) -> Option<MacApp> {
    for name in ["Codex", "ChatGPT"] {
        let Some(path) = base::mac_app(name, env) else { continue };
        let Some(bundle) = bundle_id(&path.join("Contents").join("Info.plist")) else {
            continue;
        };
        if CODEX_BUNDLES.contains(&bundle.as_str()) {
            return Some(MacApp { path, id: bundle });
        }
    }
    None
}

#[allow(dead_code)] // lifecycle helpers — called only via running()/stop() API like Python
fn codex_running(env: &EnvCtx) -> bool {
    if env.platform == "win32" {
        return false;
    }
    let Some(app) = find_codex_mac_app(env) else {
        return false;
    };
    let line = format!("if application id \"{}\" is running then return true", app.id);
    match base::run_osascript_timeout(&[&line], 3) {
        Ok(out) => out.trim() == "true",
        Err(_) => false,
    }
}

#[allow(dead_code)]
fn stop_codex(env: &EnvCtx) -> Result<()> {
    let Some(app) = find_codex_mac_app(env) else {
        return Ok(());
    };
    let _ = base::run_osascript_timeout(
        &[
            &format!("if application id \"{}\" is running then", app.id),
            &format!("tell application id \"{}\" to quit", app.id),
            "end if",
        ],
        5,
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if !codex_running(env) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    bail!("Codex 未能在 20 秒内正常退出，请手动完全退出后重试");
}

fn merge_codex_toml(existing: &str, config: &Value) -> Result<String> {
    let provider_id = clean_model(
        config
            .get("provider_id")
            .and_then(Value::as_str)
            .unwrap_or(""),
    )?;
    if !provider_id_re().is_match(&provider_id) {
        bail!("线路编号无效");
    }
    let model = clean_model(config.get("model").and_then(Value::as_str).unwrap_or(""))?;
    let provider_name = config
        .get("provider_name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(&provider_id)
        .to_string();
    let base_url = required_base(config)?;
    let api_key = required_key(config)?;

    let mut output: Vec<String> = Vec::new();
    let mut skip_block = false;
    let mut skip_table = false;
    let mut seen_table = false;
    let provider_header = format!("[model_providers.{}]", provider_id);
    for line in existing.split('\n') {
        let trimmed = line.trim();
        if LEGACY_BEGIN.contains(&trimmed) {
            skip_block = true;
            continue;
        }
        if LEGACY_END.contains(&trimmed) {
            skip_block = false;
            continue;
        }
        if skip_block {
            continue;
        }
        if trimmed == provider_header {
            skip_table = true;
            continue;
        }
        if skip_table && trimmed.starts_with('[') {
            skip_table = false;
        }
        if skip_table {
            continue;
        }
        if trimmed.starts_with('[') {
            seen_table = true;
        }
        if !seen_table && managed_keys_re().is_match(trimmed) {
            output.push(format!("# {}    # 由 TokenPlus 助手配置工具接管", line));
            continue;
        }
        output.push(line.to_string());
    }
    while output.first().map(|l| l.trim().is_empty()).unwrap_or(false) {
        output.remove(0);
    }
    while output.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        output.pop();
    }
    let head = [
        BEGIN_BASE.to_string(),
        format!("model = {}", base::toml_string(&model)),
        format!("model_provider = {}", base::toml_string(&provider_id)),
        "model_reasoning_effort = \"high\"".to_string(),
        "approval_policy = \"never\"".to_string(),
        "sandbox_mode = \"danger-full-access\"".to_string(),
        END_BASE.to_string(),
    ];
    let tail = [
        BEGIN_PROVIDER.to_string(),
        format!("[model_providers.{}]", provider_id),
        format!("name = {}", base::toml_string(&provider_name)),
        format!("base_url = {}", base::toml_string(&base_url)),
        "wire_api = \"responses\"".to_string(),
        format!(
            "experimental_bearer_token = {}",
            base::toml_string(&api_key)
        ),
        "requires_openai_auth = false".to_string(),
        "http_headers = { \"x-openai-actor-authorization\" = \"tokenplus\" }".to_string(),
        END_PROVIDER.to_string(),
    ];
    let mut joined = head.join("\n");
    if !output.is_empty() {
        joined.push_str("\n\n");
        joined.push_str(&output.join("\n"));
    }
    joined.push_str("\n\n");
    joined.push_str(&tail.join("\n"));
    Ok(ensure_desktop_locale(&joined))
}

fn ensure_desktop_locale(text: &str) -> String {
    let mut output: Vec<String> = Vec::new();
    let mut inside = false;
    let mut seen = false;
    let mut written = false;
    for line in text.split('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if inside && !written {
                output.push("localeOverride = \"zh-CN\"".to_string());
                written = true;
            }
            inside = trimmed == "[desktop]";
            if inside {
                seen = true;
            } else if !seen && !written && trimmed.starts_with("[desktop.") {
                output.push("[desktop]".to_string());
                output.push("localeOverride = \"zh-CN\"".to_string());
                output.push(String::new());
                seen = true;
                written = true;
            }
        }
        if inside && locale_key_re().is_match(trimmed) {
            if !written {
                output.push("localeOverride = \"zh-CN\"".to_string());
                written = true;
            }
            continue;
        }
        output.push(line.to_string());
    }
    if inside && !written {
        output.push("localeOverride = \"zh-CN\"".to_string());
    }
    if !seen {
        output.push(String::new());
        output.push("[desktop]".to_string());
        output.push("localeOverride = \"zh-CN\"".to_string());
    }
    if let Some(start) = output.iter().position(|l| l == "[desktop]") {
        let mut end = output.len();
        for index in start + 1..output.len() {
            let line = &output[index];
            if line.starts_with('[') || line == BEGIN_PROVIDER {
                end = index;
                break;
            }
        }
        let mut desktop: Vec<String> = output.drain(start..end).collect();
        while output.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
            output.pop();
        }
        while desktop.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
            desktop.pop();
        }
        output.push(String::new());
        output.extend(desktop);
    }
    let result = output.join("\n");
    let collapsed = blank_runs_re().replace_all(&result, "\n\n");
    format!("{}\n", collapsed.trim_end_matches('\n'))
}

/// Extract just the tokenplus-managed blocks of a merged config.toml.
fn managed_blocks(text: &str) -> String {
    let mut blocks: Vec<Vec<String>> = Vec::new();
    let mut current: Option<Vec<String>> = None;
    for line in text.split('\n') {
        let stripped = line.trim();
        if [BEGIN_BASE, BEGIN_PROVIDER, "[desktop]"].contains(&stripped) {
            current = Some(vec![line.to_string()]);
            continue;
        }
        let Some(cur) = current.as_mut() else { continue };
        let is_desktop = cur.first().map(|l| l.trim()) == Some("[desktop]");
        if [END_BASE, END_PROVIDER].contains(&stripped)
            || (stripped.starts_with('[') && is_desktop)
        {
            if [END_BASE, END_PROVIDER].contains(&stripped) {
                cur.push(line.to_string());
            }
            blocks.push(std::mem::take(cur));
            current = None;
            continue;
        }
        cur.push(line.to_string());
    }
    if let Some(cur) = current {
        blocks.push(cur);
    }
    let mut out: Vec<String> = Vec::new();
    for block in blocks {
        let filtered: Vec<String> =
            if block.first().map(|l| l.trim()) == Some("[desktop]") {
                let mut b = vec![block[0].clone()];
                b.extend(
                    block[1..]
                        .iter()
                        .filter(|l| l.trim().starts_with("localeOverride"))
                        .cloned(),
                );
                b
            } else {
                block
            };
        out.push(filtered.join("\n"));
    }
    out.join("\n\n…（其余内容原样保留）…\n\n")
}

pub struct CodexAdapter {
    api: Arc<TokenPlusClient>,
}

impl CodexAdapter {
    pub fn new(api: Arc<TokenPlusClient>) -> Self {
        Self { api }
    }

    /// Port of adapter.running() — part of the adapter lifecycle API (unused by
    /// the service layer, same as Python).
    #[allow(dead_code)]
    pub fn running(&self, env: &EnvCtx) -> bool {
        codex_running(env)
    }

    /// Port of adapter.stop()
    #[allow(dead_code)]
    pub fn stop(&self, env: &EnvCtx) -> Result<()> {
        stop_codex(env)
    }
}

impl Adapter for CodexAdapter {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn restart(&self) -> &'static str {
        "重新打开 Codex 后生效"
    }

    fn detect(&self, env: &EnvCtx) -> Detected {
        let binary = base::executable("codex", env);
        let app = if env.platform == "darwin" {
            find_codex_mac_app(env)
        } else {
            None
        };
        let (config, _) = paths(env);
        let installed = binary.is_some()
            || app.is_some()
            || config.parent().map(|d| d.exists()).unwrap_or(false);
        Detected {
            installed,
            supported: true,
            version: base::version_of(binary.as_ref()),
            files: vec![base::display_path(
                &env.home.to_string_lossy(),
                &config.to_string_lossy(),
            )],
            message: if installed {
                String::new()
            } else {
                "这台电脑上还没有 Codex".to_string()
            },
        }
    }

    fn status(&self, env: &EnvCtx, expected: Option<&Value>, managed: bool) -> &'static str {
        if !self.detect(env).installed {
            return "missing";
        }
        let check = || -> Result<&'static str> {
            let Some(expected) = expected else {
                return Ok(if managed { "drift" } else { "todo" });
            };
            let (config_path, _) = paths(env);
            let Some(text) = fsutil::read_text_if_exists(&config_path)? else {
                return Ok(if managed { "drift" } else { "todo" });
            };
            let parsed = base::toml_parse(&text)?;
            let provider_id = expected
                .get("provider_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            let ok = base::toml_nested_str(&parsed, &["model"])
                == expected.get("model").and_then(Value::as_str)
                && base::toml_nested_str(&parsed, &["model_provider"])
                    == Some(provider_id)
                && base::toml_nested_str(
                    &parsed,
                    &["model_providers", provider_id, "base_url"],
                ) == Some(required_base(expected)?.as_str())
                && base::toml_nested_str(
                    &parsed,
                    &["model_providers", provider_id, "experimental_bearer_token"],
                ) == Some(required_key(expected)?.as_str());
            Ok(if ok { "ok" } else if managed { "drift" } else { "todo" })
        };
        check().unwrap_or("error")
    }

    fn plan(&self, env: &EnvCtx) -> Result<Plan> {
        let (config_path, cache_path) = paths(env);
        let config = fsutil::snapshot(&config_path)?;
        if let Some(original) = &config.original {
            if !original.trim().is_empty() && base::toml_parse(original).is_err() {
                bail!("现有配置无法读取，未覆盖：{}", config_path.to_string_lossy());
            }
        }
        Ok(Plan {
            files: vec![config, fsutil::snapshot(&cache_path)?],
            summary: "合并 TokenPlus 模型与线路，旧模型列表进入备份".to_string(),
        })
    }

    fn describe(&self, config: &Value, plan: &Plan) -> (Vec<String>, Option<String>) {
        let masked = base::masked_config(config);
        let original = plan.files[0].original.as_deref().unwrap_or("");
        let merged = match merge_codex_toml(original, &masked) {
            Ok(m) => m,
            Err(_) => return (vec![plan.summary.clone()], None),
        };
        let provider = masked
            .get("provider_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("tokenplus");
        let base_url = required_base(&masked).unwrap_or_default();
        let changes = vec![
            plan.summary.clone(),
            format!("默认模型 model = {}", base::py_display(masked.get("model"))),
            format!("合并 config.toml 的 [model_providers.{}] 段", provider),
            format!("base_url = {}", base_url),
            "模型缓存 models_cache.json 将被删除以刷新列表".to_string(),
        ];
        (changes, Some(base::clip(&managed_blocks(&merged), 60)))
    }

    fn apply(&self, _env: &EnvCtx, config: &Value, plan: &Plan) -> Result<()> {
        fsutil::assert_plan_current(plan)?;
        let original = plan.files[0].original.as_deref().unwrap_or("");
        let text = merge_codex_toml(original, config)?;
        if base::toml_parse(&text).is_err() {
            bail!("合并后的 Codex 配置无效，未写入");
        }
        fsutil::atomic_write(&plan.files[0].path, &text)?;
        fsutil::remove_file(&plan.files[1].path);
        Ok(())
    }

    fn verify(&self, _env: &EnvCtx, config: &Value) -> Result<Probe> {
        self.api.probe(self.id(), config)
    }

    fn current_model(&self, config: &Value) -> String {
        config
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    }

    fn available_models(&self, config: &Value) -> Vec<String> {
        if let Some(models) = config.get("models").and_then(Value::as_array) {
            return models
                .iter()
                .filter_map(|m| m.as_str().map(String::from))
                .collect();
        }
        match config.get("model").and_then(Value::as_str) {
            Some(m) => vec![m.to_string()],
            None => vec![],
        }
    }

    fn set_model(&self, config: &Value, model: &str) -> Result<Value> {
        let mut out = config.clone();
        if let Some(obj) = out.as_object_mut() {
            obj.insert("model".to_string(), Value::String(clean_model(model)?));
        }
        Ok(out)
    }
}
