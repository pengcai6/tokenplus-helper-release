//! Orchestration service — port of helper/service.py.
//!
//! Owns the TokenPlus session/key, builds per-tool configs (the part the
//! yituohub server used to return), and runs the guarded connect flow:
//! detect -> plan -> backup -> recheck -> apply -> verify -> record.

use anyhow::Result;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::adapters::{self, Adapter};
use crate::backups::BackupService;
use crate::catalog::{tool_meta, TOOL_ORDER};
use crate::envguard::{fix_environment, scan_environment};
use crate::fsutil::now_ms;
use crate::store::{KeyStore, StateStore};
use crate::tpapi::{api_error, truthy, TokenPlusClient, GATEWAY_BASE};
use crate::types::{EnvCtx, FileSnap, Probe};

/// Claude Code builds requests as ANTHROPIC_BASE_URL + "/v1/messages",
/// so its base is the API origin, unlike the /v1 gateway root the others use.
pub const CLAUDE_BASE: &str = "https://api.tokenplus.cn";
const DEFAULT_MODEL_FALLBACK: &str = "claude-sonnet-4-5";

fn key_mask() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"sk-[A-Za-z0-9_-]{4,}").unwrap())
}

/// Never let a real (or placeholder) key reach the preview payload.
fn mask(text: impl ToString) -> String {
    key_mask()
        .replace_all(&text.to_string(), "sk-***")
        .into_owned()
}

fn plan_file(home: &str, file: &FileSnap) -> Value {
    json!({
        "path": adapters::display_path(home, &file.path.to_string_lossy()),
        "exists": file.original.is_some(),
        "size": file.original.as_ref().map(|s| s.len()).unwrap_or(0),
    })
}

fn pick(models: &[String], keywords: &[&str]) -> String {
    for keyword in keywords {
        for model in models {
            if model.to_lowercase().contains(keyword) {
                return model.clone();
            }
        }
    }
    String::new()
}

/// Build the adapter config that the yituohub server used to return.
pub fn build_config(tool: &str, key: &str, models: &[String], model: &str) -> Value {
    let models: Vec<String> = models.iter().filter(|m| !m.is_empty()).cloned().collect();
    let chosen = if model.is_empty() {
        models.first().cloned().unwrap_or_else(|| DEFAULT_MODEL_FALLBACK.to_string())
    } else {
        model.to_string()
    };
    if tool == "claude-code" || tool == "claude-desktop" {
        let sonnet = pick(&models, &["sonnet"]);
        let opus = pick(&models, &["opus"]);
        let haiku = pick(&models, &["haiku"]);
        let tiers = json!({
            "default": if model.is_empty() {
                if sonnet.is_empty() { chosen.clone() } else { sonnet.clone() }
            } else { model.to_string() },
            "opus": if opus.is_empty() { chosen.clone() } else { opus },
            "sonnet": if sonnet.is_empty() { chosen.clone() } else { sonnet },
            "haiku": if haiku.is_empty() { chosen.clone() } else { haiku },
        });
        let default_model = tiers["default"].clone();
        let mut config = json!({
            "api_key": key,
            "models": tiers,
            "default": default_model,
            "pool": models,
        });
        let key_name = if tool == "claude-desktop" {
            "gateway_base_url"
        } else {
            "base_url"
        };
        config[key_name] = json!(CLAUDE_BASE);
        if tool == "claude-desktop" {
            config["api"] = json!("anthropic-messages");
        }
        return config;
    }
    if tool == "deepseek" || tool == "workbuddy" {
        static DEEPSEEK_RE: OnceLock<regex::Regex> = OnceLock::new();
        let deepseek_re =
            DEEPSEEK_RE.get_or_init(|| regex::Regex::new(r"(?i)^deepseek-").unwrap());
        let pool: Vec<String> = {
            let filtered: Vec<String> = models
                .iter()
                .filter(|m| tool != "deepseek" || deepseek_re.is_match(m))
                .cloned()
                .collect();
            if !filtered.is_empty() {
                filtered
            } else if !models.is_empty() {
                models.clone()
            } else {
                vec![chosen.clone()]
            }
        };
        let provider = json!({
            "provider_id": "tokenplus",
            "provider_name": "TokenPlus",
            "api": "openai-completions",
            "api_key": key,
            "base_url": GATEWAY_BASE,
            "model": if model.is_empty() { pool[0].as_str() } else { model },
            "models": pool,
        });
        return json!({"default_provider_id": "tokenplus", "providers": [provider]});
    }
    json!({
        "provider_id": "tokenplus",
        "provider_name": "TokenPlus",
        "api_key": key,
        "base_url": GATEWAY_BASE,
        "model": chosen.clone(),
        "models": if models.is_empty() { vec![chosen] } else { models },
    })
}

pub struct HelperService {
    pub env: EnvCtx,
    pub state: StateStore,
    pub keys: KeyStore,
    pub backups: BackupService,
    pub api: Arc<TokenPlusClient>,
    pub adapters: HashMap<String, Box<dyn Adapter>>,
    pub root: PathBuf,
}

fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "win32"
    } else {
        "linux"
    }
}

impl HelperService {
    pub fn new(root: &Path) -> Self {
        let env = EnvCtx {
            home: dirs::home_dir().unwrap_or_default(),
            env: std::env::vars().collect(),
            platform: platform(),
        };
        let api = Arc::new(TokenPlusClient::new());
        Self {
            env,
            state: StateStore::new(root),
            keys: KeyStore::new(),
            backups: BackupService::new(root),
            adapters: adapters::all(api.clone()),
            api,
            root: root.to_path_buf(),
        }
    }

    // ---------- session / key ----------

    pub fn get_key(&self) -> Option<String> {
        self.keys.get("api_key")
    }

    pub fn has_key(&self) -> bool {
        self.get_key().is_some()
    }

    pub fn key_hint(&self) -> String {
        let key = self.get_key().unwrap_or_default();
        if key.chars().count() > 12 {
            let head: String = key.chars().take(6).collect();
            let tail: String = key
                .chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("{head}…{tail}")
        } else if !key.is_empty() {
            "已保存".to_string()
        } else {
            String::new()
        }
    }

    /// Cookies harvested from the embedded TokenPlus browser.
    pub fn set_cookies(&self, cookies: &Map<String, Value>) -> Result<Value> {
        self.api.set_cookies(cookies);
        let session = self.api.session()?;
        if !self.api.logged_in(&session) {
            return Err(api_error("还没有登录，请先在网页里完成登录"));
        }
        self.keys.set_json(
            "portal_session",
            &json!({"savedAt": now_ms(), "cookies": cookies}),
        )?;
        Ok(session)
    }

    pub fn portal_logged_in(&self) -> bool {
        self.api
            .session()
            .map(|s| self.api.logged_in(&s))
            .unwrap_or(false)
    }

    /// Create an API key through the portal session, then save it.
    pub fn auto_create_key(&self, name: &str) -> Result<Value> {
        let session = self.api.session()?;
        if !self.api.logged_in(&session) {
            return Err(api_error("还没有登录，请先在网页里完成登录"));
        }
        let mut account_id = session["current_account"]["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| session["user"]["account_id"].as_str())
            .unwrap_or_default()
            .to_string();
        if account_id.is_empty() {
            let overview = self.api.account_overview()?;
            account_id = overview["account"]["id"]
                .as_str()
                .unwrap_or_default()
                .to_string();
        }
        let projects = if account_id.is_empty() {
            Vec::new()
        } else {
            self.api.list_projects(&account_id)?
        };
        let project = projects
            .first()
            .ok_or_else(|| api_error("账号下没有可用项目，请先在控制台创建项目"))?;
        let created = self.api.create_api_key(
            &account_id,
            project["id"].as_str().unwrap_or_default(),
            name,
        )?;
        let mut key = created["key"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| created["api_key"].as_str())
            .unwrap_or_default()
            .to_string();
        if let Some(inner) = created["api_key"].as_object() {
            key = inner["key"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(&key)
                .to_string();
        }
        static KEY_RE: OnceLock<regex::Regex> = OnceLock::new();
        let re = KEY_RE.get_or_init(|| regex::Regex::new(r"^sk-[A-Za-z0-9_-]+$").unwrap());
        if !re.is_match(&key) {
            return Err(api_error("已创建密钥，但没有读到明文；请在网页里复制后手动粘贴"));
        }
        self.import_key(&key)
    }

    pub fn import_key(&self, key: &str) -> Result<Value> {
        let key = key.trim();
        static KEY_RE: OnceLock<regex::Regex> = OnceLock::new();
        let re = KEY_RE.get_or_init(|| regex::Regex::new(r"^sk-[A-Za-z0-9_-]+$").unwrap());
        if !re.is_match(key) {
            return Err(api_error("密钥格式不对，应以 sk- 开头"));
        }
        let models = self.api.list_models(key)?;
        self.keys.set("api_key", key)?;
        self.keys.set_json(
            "models",
            &json!({"list": models, "savedAt": now_ms()}),
        )?;
        Ok(json!({"ok": true, "key": self.key_hint(), "models": models}))
    }

    pub fn models(&self, refresh: bool) -> Vec<String> {
        let cached = self.keys.get_json("models").unwrap_or_else(|| json!({}));
        let cached_list: Vec<String> = cached["list"]
            .as_array()
            .map(|a| a.iter().filter_map(|m| m.as_str().map(String::from)).collect())
            .unwrap_or_default();
        if !refresh && !cached_list.is_empty() {
            return cached_list;
        }
        let key = match self.get_key() {
            Some(key) => key,
            None => return cached_list,
        };
        match self.api.list_models(&key) {
            Ok(models) => {
                let _ = self.keys.set_json(
                    "models",
                    &json!({"list": models, "savedAt": now_ms()}),
                );
                models
            }
            Err(_) => cached_list,
        }
    }

    /// Force a re-fetch of the upstream model list, then rebuild the model
    /// pool inside every managed tool's expected config and re-apply it so
    /// the written files offer the new upstream models too.
    pub fn refresh_models(&self) -> Result<Value> {
        let key = self
            .get_key()
            .ok_or_else(|| api_error("还没有接入密钥，请先完成 TokenPlus 登录"))?;
        let models = self.api.list_models(&key)?;
        let _ = self.keys.set_json(
            "models",
            &json!({"list": models, "savedAt": now_ms()}),
        );
        let mut updated = Vec::new();
        let mut errors = Map::new();
        for tool_id in TOOL_ORDER {
            let mut record = match self.managed(tool_id) {
                Some(record) => record,
                None => continue,
            };
            if !record["managed"].as_bool().unwrap_or(false) {
                continue;
            }
            let expected = record.get("expected").cloned().unwrap_or(Value::Null);
            if !truthy(&expected) {
                continue;
            }
            let adapter = match self.adapters.get(tool_id) {
                Some(adapter) => adapter,
                None => continue,
            };
            let result = (|| -> Result<()> {
                let config = build_config(
                    tool_id,
                    &key,
                    &models,
                    &adapter.current_model(&expected),
                );
                let plan = adapter.plan(&self.env)?;
                self.backups.create(tool_id, &plan.files)?;
                adapter.apply(&self.env, &config, &plan)?;
                record["expected"] = config.clone();
                record["model"] = json!(adapter.current_model(&config));
                self.state.set_managed(tool_id, &record)
            })();
            match result {
                Ok(()) => updated.push(tool_id.to_string()),
                Err(e) => {
                    errors.insert(tool_id.to_string(), json!(e.to_string()));
                }
            }
        }
        Ok(json!({
            "ok": true,
            "models": models,
            "updated": updated,
            "errors": errors,
        }))
    }

    pub fn logout(&self) {
        for name in ["api_key", "models", "portal_session"] {
            self.keys.delete(name);
        }
    }

    // ---------- tool flow ----------

    fn managed(&self, tool: &str) -> Option<Value> {
        self.state.get_managed(tool)
    }

    pub fn tool_catalog(&self) -> Vec<Value> {
        let mut out = Vec::new();
        for tool_id in TOOL_ORDER {
            let adapter = match self.adapters.get(tool_id) {
                Some(adapter) => adapter,
                None => continue,
            };
            let meta = match tool_meta(tool_id) {
                Some(meta) => meta,
                None => continue,
            };
            let record = self.managed(tool_id).unwrap_or_else(|| json!({}));
            let expected = record.get("expected").cloned().unwrap_or(Value::Null);
            let managed_flag = record["managed"].as_bool().unwrap_or(false);
            let detected = adapter.detect(&self.env);
            let status = adapter.status(&self.env, expected_option(&record), managed_flag);
            let config = if truthy(&expected) { expected } else { json!({}) };
            out.push(json!({
                "id": tool_id,
                "name": meta.name,
                "kind": meta.kind,
                "keys": meta.keys,
                "restart": meta.restart,
                "colors": meta.colors,
                "installed": detected.installed,
                "version": detected.version,
                "files": detected.files,
                "message": detected.message,
                "status": status,
                "managed": managed_flag,
                "model": if truthy(&config) { adapter.current_model(&config) } else { String::new() },
                "models": if truthy(&config) { adapter.available_models(&config) } else { Vec::<String>::new() },
            }));
        }
        out
    }

    pub fn tool_detail(&self, tool: &str) -> Result<Value> {
        let mut item = self
            .tool_catalog()
            .into_iter()
            .find(|t| t["id"].as_str() == Some(tool))
            .ok_or_else(|| anyhow::anyhow!("未知工具：{tool}"))?;
        item["backups"] = json!(self.backups.list(Some(tool)));
        item["hasKey"] = json!(self.has_key());
        Ok(item)
    }

    /// tools.connect_plan — what a single connect would touch.
    pub fn connect_plan(&self, tool: &str) -> Result<Value> {
        if !self.has_key() {
            return Err(api_error("还没有接入密钥，请先完成 TokenPlus 登录"));
        }
        let adapter = self
            .adapters
            .get(tool)
            .ok_or_else(|| anyhow::anyhow!("未知工具：{tool}"))?;
        let detected = adapter.detect(&self.env);
        if !detected.installed {
            return Err(api_error(if detected.message.is_empty() {
                "这个工具还没有安装".to_string()
            } else {
                detected.message
            }));
        }
        let plan = adapter.plan(&self.env)?;
        Ok(json!({
            "summary": plan.summary,
            "files": plan
                .files
                .iter()
                .map(|f| json!({
                    "path": f.path.to_string_lossy(),
                    "exists": f.original.is_some(),
                }))
                .collect::<Vec<_>>(),
        }))
    }

    /// What-will-change preview for installed tools needing attention.
    pub fn tools_plan(&self) -> Value {
        let mut out = Map::new();
        let home = self.env.home.to_string_lossy().to_string();
        let key = self
            .get_key()
            .unwrap_or_else(|| adapters::PREVIEW_KEY.to_string());
        for tool_id in TOOL_ORDER {
            let adapter = match self.adapters.get(tool_id) {
                Some(adapter) => adapter,
                None => continue,
            };
            let detected = adapter.detect(&self.env);
            if !detected.installed {
                continue;
            }
            let record = self.managed(tool_id).unwrap_or_else(|| json!({}));
            let status = adapter.status(
                &self.env,
                expected_option(&record),
                record["managed"].as_bool().unwrap_or(false),
            );
            if !["todo", "drift", "error"].contains(&status) {
                continue;
            }
            let mut entry = json!({
                "status": status, "summary": "", "files": [],
                "changes": [], "preview": "",
            });
            let result: Result<()> = (|| {
                let plan = adapter.plan(&self.env)?;
                let config = self.config_for(tool_id, "", Some(&key));
                entry["summary"] = json!(plan.summary);
                entry["files"] = json!(plan
                    .files
                    .iter()
                    .map(|f| plan_file(&home, f))
                    .collect::<Vec<_>>());
                let (changes, preview) = adapter.describe(&config, &plan);
                entry["changes"] = json!(changes
                    .iter()
                    .map(|line| mask(line))
                    .collect::<Vec<_>>());
                entry["preview"] = json!(mask(preview.unwrap_or_default()));
                if let Some(models) = (|| -> Option<Vec<Value>> {
                    let models: Vec<Value> = adapter
                        .available_models(&config)
                        .into_iter()
                        .map(|m| json!({"id": m}))
                        .collect();
                    if models.is_empty() {
                        None
                    } else {
                        Some(models)
                    }
                })() {
                    entry["models"] = json!(models);
                }
                Ok(())
            })();
            if let Err(error) = result {
                entry["error"] = json!(mask(error.to_string()));
                // Python appends to entry["files"] (may already hold plan files)
                for path in &detected.files {
                    let full = if let Some(rest) = path.strip_prefix("~/") {
                        format!("{home}/{rest}")
                    } else {
                        path.clone()
                    };
                    let p = Path::new(&full);
                    let exists = p.exists();
                    let size = if exists {
                        p.metadata().map(|m| m.len()).unwrap_or(0)
                    } else {
                        0
                    };
                    let file_entry = json!({"path": path, "exists": exists, "size": size});
                    if let Some(list) = entry["files"].as_array_mut() {
                        list.push(file_entry);
                    } else {
                        entry["files"] = json!([file_entry]);
                    }
                }
            }
            out.insert(tool_id.to_string(), entry);
        }
        Value::Object(out)
    }

    /// service._config_for — config a connect would write for `tool`.
    pub fn config_for(&self, tool: &str, model: &str, key: Option<&str>) -> Value {
        let key = key
            .filter(|k| !k.is_empty())
            .map(String::from)
            .or_else(|| self.get_key())
            .unwrap_or_default();
        let record = self.managed(tool).unwrap_or_else(|| json!({}));
        let expected = record.get("expected").cloned().unwrap_or(Value::Null);
        let current_model = if !model.is_empty() {
            model.to_string()
        } else if truthy(&expected) {
            self.adapters
                .get(tool)
                .map(|a| a.current_model(&expected))
                .unwrap_or_default()
        } else {
            String::new()
        };
        build_config(tool, &key, &self.models(false), &current_model)
    }

    pub fn connect(&self, tool: &str, model: &str, probe: bool) -> Result<Value> {
        if !self.has_key() {
            return Err(api_error("还没有接入密钥，请先完成 TokenPlus 登录"));
        }
        let adapter = self
            .adapters
            .get(tool)
            .ok_or_else(|| anyhow::anyhow!("未知工具：{tool}"))?;
        let detected = adapter.detect(&self.env);
        if !detected.installed {
            return Err(api_error(if detected.message.is_empty() {
                "这个工具还没有安装".to_string()
            } else {
                detected.message
            }));
        }
        let config = self.config_for(tool, model, None);
        let plan = adapter.plan(&self.env)?;
        self.backups.create(tool, &plan.files)?;
        if let Err(error) = adapter.apply(&self.env, &config, &plan) {
            // restore originals like the original orchestrator
            for file in &plan.files {
                if let Some(original) = &file.original {
                    let _ = crate::fsutil::atomic_write(&file.path, original);
                } else {
                    crate::fsutil::remove_file(&file.path);
                }
            }
            return Err(error);
        }
        let result = if probe {
            adapter.verify(&self.env, &config)?
        } else {
            Probe {
                ok: true,
                latency: 0.0,
                message: "已跳过连接测试".to_string(),
            }
        };
        self.state.set_managed(
            tool,
            &json!({
                "managed": true, "expected": config,
                "model": adapter.current_model(&config),
                "connectedAt": now_ms(),
            }),
        )?;
        Ok(json!({
            "ok": true,
            "status": adapter.status(&self.env, Some(&config), true),
            "restart": adapter.restart(),
            "probe": result,
            "model": adapter.current_model(&config),
        }))
    }

    pub fn test(&self, tool: &str) -> Result<Value> {
        let record = self.managed(tool).unwrap_or_else(|| json!({}));
        let expected = record.get("expected").cloned().unwrap_or(Value::Null);
        if !truthy(&expected) {
            return Err(api_error("还没有接入这个工具"));
        }
        let adapter = self
            .adapters
            .get(tool)
            .ok_or_else(|| anyhow::anyhow!("未知工具：{tool}"))?;
        Ok(json!(adapter.verify(&self.env, &expected)?))
    }

    pub fn set_model(&self, tool: &str, model: &str) -> Result<Value> {
        let adapter = self
            .adapters
            .get(tool)
            .ok_or_else(|| anyhow::anyhow!("未知工具：{tool}"))?;
        let mut record = self.managed(tool).unwrap_or_else(|| json!({}));
        let expected = record.get("expected").cloned().unwrap_or(Value::Null);
        if !truthy(&expected) {
            return Err(api_error("还没有接入这个工具"));
        }
        let config = adapter.set_model(&expected, model)?;
        let plan = adapter.plan(&self.env)?;
        self.backups.create(tool, &plan.files)?;
        adapter.apply(&self.env, &config, &plan)?;
        record["expected"] = config.clone();
        record["model"] = json!(adapter.current_model(&config));
        self.state.set_managed(tool, &record)?;
        Ok(json!({
            "ok": true,
            "model": record["model"],
            "restart": adapter.restart(),
        }))
    }

    pub fn restore(&self, tool: &str, backup_id: &str) -> Result<()> {
        self.backups.restore(tool, backup_id)
    }

    /// Mark a tool as 'back to official' without touching files.
    pub fn unmanage(&self, tool: &str) -> Result<Value> {
        let mut record = self.managed(tool).unwrap_or_else(|| json!({}));
        record["managed"] = json!(false);
        self.state.set_managed(tool, &record)?;
        Ok(json!({"ok": true}))
    }

    // ---------- settings / checks ----------

    pub fn settings(&self) -> Value {
        self.state.settings()
    }

    pub fn set_settings(&self, settings: &Value) -> Result<Value> {
        let mut merged = self.state.settings();
        if let Some(obj) = settings.as_object() {
            for (key, value) in obj {
                merged[key] = json!(truthy(value));
            }
        }
        self.state.set_settings(&merged)?;
        Ok(merged)
    }

    pub fn env_scan(&self) -> Vec<Value> {
        scan_environment(&self.env.home.to_string_lossy(), self.env.platform)
    }

    pub fn env_fix(&self, issues: Option<Vec<Value>>) -> Result<Vec<Value>> {
        let issues = match issues {
            Some(issues) => issues,
            None => self.env_scan(),
        };
        Ok(fix_environment(
            &self.env.home.to_string_lossy(),
            &issues,
            |path, text| {
                let _ = self.backups.create(
                    "env",
                    &[FileSnap {
                        path: PathBuf::from(path),
                        original: Some(text.to_string()),
                    }],
                );
            },
        ))
    }
}

/// record.get("expected") -> Option<&Value> (Python `or None` semantics:
/// a falsy expected is treated as missing).
fn expected_option(record: &Value) -> Option<&Value> {
    record.get("expected").filter(|v| truthy(v))
}
