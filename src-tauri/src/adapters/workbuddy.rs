//! WorkBuddy adapter — merges tagged model entries into models.json.
//!
//! Port of helper/adapters/workbuddy.py.

use anyhow::{bail, Result};
use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use super::base;
use super::Adapter;
use crate::fsutil;
use crate::tpapi::{assert_service_url, TokenPlusClient};
use crate::types::{Detected, EnvCtx, Plan, Probe};

const MANAGED_TAG: &str = "tokenplus-helper";
const PROVIDER_ID: &str = "tokenplus";

fn key_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^sk-[A-Za-z0-9_-]+$").unwrap())
}

fn workbuddy_dir(env: &EnvCtx) -> PathBuf {
    let override_dir = env
        .env
        .get("WORKBUDDY_CONFIG_DIR")
        .filter(|s| !s.is_empty())
        .or_else(|| env.env.get("CODEBUDDY_CONFIG_DIR").filter(|s| !s.is_empty()))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if !override_dir.is_empty() {
        return PathBuf::from(override_dir);
    }
    let domestic = env.home.join(".workbuddy");
    let international = env.home.join(".workbuddy-ai");
    if !domestic.exists() && international.exists() {
        international
    } else {
        domestic
    }
}

fn providers(config: &Value) -> Result<Vec<Value>> {
    let values: Vec<Value> = config
        .get("providers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let id_set: HashSet<Option<&str>> = values
        .iter()
        .map(|p| p.get("provider_id").and_then(Value::as_str))
        .collect();
    let expected: HashSet<Option<&str>> = [Some(PROVIDER_ID)].into_iter().collect();
    if values.len() != 1 || id_set != expected {
        bail!("WorkBuddy 配置不完整");
    }
    for provider in &values {
        let api = provider.get("api").and_then(Value::as_str).unwrap_or("");
        let api_key = provider.get("api_key").and_then(Value::as_str).unwrap_or("");
        let name_ok = base::truthy_opt(provider.get("provider_name"));
        let models = provider.get("models").and_then(Value::as_array);
        let model_ok = models
            .map(|ms| {
                !ms.is_empty()
                    && provider.get("model").map(|m| ms.contains(m)).unwrap_or(false)
                    && ms.iter().all(|m| {
                        m.as_str()
                            .map(|s| !s.is_empty() && s == s.trim())
                            .unwrap_or(false)
                    })
            })
            .unwrap_or(false);
        if !(api == "openai-responses" || api == "openai-completions")
            || !key_re().is_match(api_key)
            || !name_ok
            || !model_ok
        {
            bail!("WorkBuddy 配置不完整");
        }
        let base_url = provider
            .get("base_url")
            .and_then(Value::as_str)
            .unwrap_or("");
        assert_service_url(base_url, &["/v1"])?;
    }
    Ok(values)
}

/// parse_models() — {root, models} from the raw file text.
fn parse_models(text: Option<&str>, path: &PathBuf) -> Result<(Value, Vec<Value>)> {
    let Some(text) = text else {
        return Ok((json!([]), vec![]));
    };
    if text.is_empty() {
        return Ok((json!([]), vec![]));
    }
    let stripped = text.strip_prefix('\u{feff}').unwrap_or(text);
    let current: Value = serde_json::from_str(stripped).map_err(|_| {
        anyhow::anyhow!(
            "现有 models.json 不是有效 JSON，未修改：{}",
            path.to_string_lossy()
        )
    })?;
    if !current.is_object() && !current.is_array() {
        bail!("现有 models.json 格式不兼容，未修改配置");
    }
    if let Value::Object(map) = &current {
        if let Some(models) = map.get("models") {
            if !models.is_array() {
                bail!("现有 models.json 格式不兼容，未修改配置");
            }
        }
        if let Some(available) = map.get("availableModels") {
            let ok = available
                .as_array()
                .map(|a| a.iter().all(|m| m.is_string()))
                .unwrap_or(false);
            if !ok {
                bail!("现有 models.json 格式不兼容，未修改配置");
            }
        }
    }
    let models = match &current {
        Value::Array(a) => a.clone(),
        Value::Object(map) => map
            .get("models")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => vec![],
    };
    Ok((current, models))
}

/// model_entries() — the tagged entries we merge into models.json.
fn model_entries(config: &Value) -> Result<Vec<Value>> {
    let mut added = Vec::new();
    for provider in providers(config)? {
        let base_url = provider
            .get("base_url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim_end_matches('/');
        let api = provider.get("api").and_then(Value::as_str).unwrap_or("");
        let models = provider
            .get("models")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for m in &models {
            let model_id = base::py_display(Some(m));
            let url = if api == "openai-responses" {
                base_url.to_string()
            } else {
                format!("{}/chat/completions", base_url)
            };
            added.push(json!({
                "id": model_id,
                "name": model_id,
                "vendor": provider.get("provider_name"),
                "api": api,
                "url": url,
                "apiKey": provider.get("api_key"),
                "maxInputTokens": 32768,
                "maxOutputTokens": 4096,
                "supportsToolCall": true,
                "tags": ["custom", MANAGED_TAG, provider.get("provider_id")],
            }));
        }
    }
    let ids: HashSet<&str> = added
        .iter()
        .filter_map(|m| m.get("id").and_then(Value::as_str))
        .collect();
    if ids.len() != added.len() {
        bail!("不同分组返回了同名模型，未修改配置");
    }
    Ok(added)
}

/// `m.get("tags")` list containing MANAGED_TAG; non-dict entries raise
/// like Python's m.get on a non-dict (AttributeError).
fn is_managed(m: &Value) -> Result<bool> {
    if !m.is_object() {
        bail!("'{}' object has no attribute 'get'", base::py_type(m));
    }
    Ok(m
        .get("tags")
        .and_then(Value::as_array)
        .map(|tags| tags.iter().any(|t| t.as_str() == Some(MANAGED_TAG)))
        .unwrap_or(false))
}

/// Entry order for previews — mirrors Python's dict insertion order.
fn entry_to_j(entry: &Value) -> base::J {
    const ORDER: [&str; 10] = [
        "id",
        "name",
        "vendor",
        "api",
        "url",
        "apiKey",
        "maxInputTokens",
        "maxOutputTokens",
        "supportsToolCall",
        "tags",
    ];
    let mut pairs = Vec::new();
    for key in ORDER {
        if let Some(v) = entry.get(key) {
            pairs.push((key.to_string(), value_to_j(v)));
        }
    }
    base::J::Obj(pairs)
}

fn value_to_j(v: &Value) -> base::J {
    match v {
        Value::Null => base::J::Null,
        Value::Bool(b) => base::J::Bool(*b),
        Value::Number(n) => base::J::Int(n.as_i64().unwrap_or(0)),
        Value::String(s) => base::J::Str(s.clone()),
        Value::Array(a) => base::J::Arr(a.iter().map(value_to_j).collect()),
        Value::Object(o) => base::J::Obj(
            o.iter().map(|(k, v)| (k.clone(), value_to_j(v))).collect(),
        ),
    }
}

pub struct WorkBuddyAdapter {
    api: Arc<TokenPlusClient>,
}

impl WorkBuddyAdapter {
    pub fn new(api: Arc<TokenPlusClient>) -> Self {
        Self { api }
    }

    fn file(env: &EnvCtx) -> PathBuf {
        workbuddy_dir(env).join("models.json")
    }
}

impl Adapter for WorkBuddyAdapter {
    fn id(&self) -> &'static str {
        "workbuddy"
    }

    fn restart(&self) -> &'static str {
        "重启 WorkBuddy 后生效"
    }

    fn detect(&self, env: &EnvCtx) -> Detected {
        let app = if env.platform == "darwin" {
            base::mac_app("WorkBuddy", env).or_else(|| base::mac_app("WorkBuddy AI", env))
        } else {
            None
        };
        let installed = app.is_some() || workbuddy_dir(env).exists();
        Detected {
            installed,
            supported: true,
            version: if app.is_some() {
                "已安装".to_string()
            } else {
                String::new()
            },
            files: vec![base::display_path(
                &env.home.to_string_lossy(),
                &Self::file(env).to_string_lossy(),
            )],
            message: if installed {
                String::new()
            } else {
                "请先安装 WorkBuddy".to_string()
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
            let file = Self::file(env);
            let (_, models) =
                parse_models(fsutil::read_text_if_exists(&file)?.as_deref(), &file)?;
            let mut managed_models = Vec::new();
            for m in &models {
                if is_managed(m)? {
                    managed_models.push(m.clone());
                }
            }
            let wanted = model_entries(expected)?;
            let ok = wanted.iter().all(|want| {
                managed_models.iter().any(|m| {
                    m.get("id") == want.get("id")
                        && m.get("url") == want.get("url")
                        && m.get("apiKey") == want.get("apiKey")
                        && m.get("api") == want.get("api")
                })
            });
            Ok(if ok { "ok" } else if managed { "drift" } else { "todo" })
        };
        check().unwrap_or("error")
    }

    fn plan(&self, env: &EnvCtx) -> Result<Plan> {
        let file = fsutil::snapshot(&Self::file(env))?;
        parse_models(file.original.as_deref(), &file.path)?;
        Ok(Plan {
            files: vec![file],
            summary: "只增删带 tokenplus-helper 标记的模型，保留自定义模型与筛选列表".to_string(),
        })
    }

    fn describe(&self, config: &Value, plan: &Plan) -> (Vec<String>, Option<String>) {
        let masked = base::masked_config(config);
        let entries = match model_entries(&masked) {
            Ok(e) => e,
            Err(_) => return (vec![plan.summary.clone()], None),
        };
        let mut shown: Vec<base::J> =
            entries.iter().take(3).map(entry_to_j).collect();
        if entries.len() > 3 {
            shown.push(base::J::Obj(vec![(
                "id".to_string(),
                base::J::Str(format!("… 共 {} 个", entries.len())),
            )]));
        }
        let mut id_list = entries[..entries.len().min(4)]
            .iter()
            .filter_map(|e| e.get("id").and_then(Value::as_str).map(String::from))
            .collect::<Vec<_>>()
            .join("、");
        if entries.len() > 4 {
            id_list.push('…');
        }
        let changes = vec![
            plan.summary.clone(),
            format!("写入 {} 个带 tokenplus-helper 标记的模型条目", entries.len()),
            id_list,
            "apiKey = sk-***".to_string(),
        ];
        let preview = base::clip(&base::json_text_j(&base::J::Arr(shown)), 60);
        (changes, Some(preview))
    }

    fn apply(&self, _env: &EnvCtx, config: &Value, plan: &Plan) -> Result<()> {
        fsutil::assert_plan_current(plan)?;
        let file = &plan.files[0];
        let (root, models) = parse_models(file.original.as_deref(), &file.path)?;
        let added = model_entries(config)?;
        let ids: HashSet<String> = added
            .iter()
            .filter_map(|m| m.get("id").and_then(Value::as_str).map(String::from))
            .collect();
        let mut retained = Vec::new();
        for m in &models {
            if !is_managed(m)? {
                retained.push(m.clone());
            }
        }
        if retained.iter().any(|m| {
            m.get("id")
                .and_then(Value::as_str)
                .map(|id| ids.contains(id))
                .unwrap_or(false)
        }) {
            bail!("已有同名自定义模型，未覆盖原配置；请先重命名或移除同名模型");
        }
        let mut merged = retained;
        merged.extend(added);
        let output = if root.is_array() {
            Value::Array(merged)
        } else {
            let mut out = root.as_object().cloned().unwrap_or_default();
            out.insert("models".to_string(), Value::Array(merged));
            if base::truthy_opt(out.get("availableModels")) {
                let mut removed: HashSet<String> = HashSet::new();
                for m in &models {
                    if is_managed(m)? {
                        removed.insert(
                            m.get("id")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                        );
                    }
                }
                let mut seen: Vec<Value> = Vec::new();
                let candidates: Vec<String> = out
                    .get("availableModels")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .filter(|id| !removed.contains(id))
                    .chain(ids.iter().cloned())
                    .collect();
                for id in candidates {
                    if !seen.iter().any(|v| v.as_str() == Some(id.as_str())) {
                        seen.push(Value::String(id));
                    }
                }
                out.insert("availableModels".to_string(), Value::Array(seen));
            }
            Value::Object(out)
        };
        fsutil::atomic_write(&file.path, &fsutil::json_text(&output))
    }

    fn verify(&self, _env: &EnvCtx, config: &Value) -> Result<Probe> {
        self.api.probe(self.id(), config)
    }

    fn current_model(&self, config: &Value) -> String {
        if !base::truthy(config) {
            return String::new();
        }
        let selected = config.get("default_provider_id");
        let providers_arr = config.get("providers").and_then(Value::as_array);
        let provider = providers_arr
            .and_then(|ps| ps.iter().find(|p| p.get("provider_id") == selected))
            .or_else(|| providers_arr.and_then(|ps| ps.first()));
        provider
            .and_then(|p| p.get("model"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    }

    fn available_models(&self, config: &Value) -> Vec<String> {
        if !base::truthy(config) {
            return vec![];
        }
        providers(config)
            .map(|values| {
                values
                    .iter()
                    .flat_map(|p| {
                        p.get("models")
                            .and_then(Value::as_array)
                            .cloned()
                            .unwrap_or_default()
                    })
                    .map(|m| base::py_display(Some(&m)))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn set_model(&self, config: &Value, model: &str) -> Result<Value> {
        let values = providers(config)?;
        let provider = values.iter().find(|p| {
            p.get("models")
                .and_then(Value::as_array)
                .map(|ms| ms.iter().any(|m| m.as_str() == Some(model)))
                .unwrap_or(false)
        });
        let Some(provider) = provider else {
            bail!("这个模型不在账号已开通的列表中");
        };
        let pid = provider.get("provider_id").cloned().unwrap_or(Value::Null);
        let mut new_providers = values.clone();
        for p in new_providers.iter_mut() {
            if p.get("provider_id") == Some(&pid) {
                if let Some(obj) = p.as_object_mut() {
                    obj.insert("model".to_string(), Value::String(model.to_string()));
                }
            }
        }
        let mut out = config.clone();
        if let Some(obj) = out.as_object_mut() {
            obj.insert("default_provider_id".to_string(), pid);
            obj.insert("providers".to_string(), Value::Array(new_providers));
        }
        Ok(out)
    }
}
