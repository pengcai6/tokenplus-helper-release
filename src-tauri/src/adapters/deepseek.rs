//! DeepSeek Harness adapter — TokenPlus provider in settings.yaml + credentials.
//!
//! Port of helper/adapters/deepseek.py. The Python version uses
//! ruamel.yaml round-trip mode to keep comments; this port uses the
//! `yaml-edit` lossless CST so untouched text keeps its comments and
//! quoting byte-for-byte.

use anyhow::{bail, Result};
use regex::Regex;
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use super::base;
use super::Adapter;
use crate::fsutil;
use crate::tpapi::{assert_service_url, TokenPlusClient};
use crate::types::{Detected, EnvCtx, Plan, Probe};

const REF_KEY: &str = "TOKENPLUS_API_KEY";
const PROVIDER_ID: &str = "tokenplus";

fn key_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^sk-[A-Za-z0-9_-]+$").unwrap())
}

fn model_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^[\w./:-]+$").unwrap())
}

fn ref_name_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$").unwrap())
}

/// The validated providers list — raises "DeepSeek 配置不完整" /
/// "默认线路无效" exactly like the Python providers().
fn providers(config: &Value) -> Result<Vec<Value>> {
    let values: Vec<Value> = config
        .get("providers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let ids: Vec<Option<&str>> = values
        .iter()
        .map(|p| p.get("provider_id").and_then(Value::as_str))
        .collect();
    let id_set: HashSet<Option<&str>> = ids.iter().cloned().collect();
    let expected_set: HashSet<Option<&str>> = [Some(PROVIDER_ID)].into_iter().collect();
    if values.len() != 1 || id_set != expected_set {
        bail!("DeepSeek 配置不完整");
    }
    for provider in &values {
        let api_key = provider.get("api_key").and_then(Value::as_str).unwrap_or("");
        let models = provider.get("models").and_then(Value::as_array);
        let model_ok = models
            .map(|ms| {
                !ms.is_empty()
                    && provider.get("model").map(|m| ms.contains(m)).unwrap_or(false)
                    && ms.iter()
                        .all(|m| model_re().is_match(&base::py_display(Some(m))))
            })
            .unwrap_or(false);
        if !key_re().is_match(api_key) || !model_ok {
            bail!("DeepSeek 配置不完整");
        }
        let base_url = provider
            .get("base_url")
            .and_then(Value::as_str)
            .unwrap_or("");
        assert_service_url(base_url, &["/v1"])?;
    }
    let first_id = ids[0].unwrap_or(PROVIDER_ID).to_string();
    let selected = config
        .get("default_provider_id")
        .filter(|v| base::truthy(v))
        .map(|v| base::py_display(Some(v)))
        .unwrap_or_else(|| first_id.clone());
    if selected != first_id {
        bail!("默认线路无效");
    }
    Ok(values)
}

fn provider_str<'a>(provider: &'a Value, key: &str) -> &'a str {
    provider.get(key).and_then(Value::as_str).unwrap_or("")
}
// ================= YAML (yaml-edit — lossless, comment-preserving) ==========
//
// Same contract as the Python ruamel round-trip port: untouched text keeps
// its comments/quoting byte-for-byte; writes land inside the CST.

use std::str::FromStr;
use yaml_edit::path::{PathError, YamlPath};
use yaml_edit::{Document, YamlNode, YamlValue};

fn document(text: Option<&str>, path: &PathBuf) -> Result<Document> {
    let raw = text.unwrap_or("");
    let doc = if raw.trim().is_empty() {
        Document::new_mapping()
    } else {
        Document::from_str(raw).map_err(|_| {
            anyhow::anyhow!(
                "现有配置不是有效的 YAML 映射，未修改：{}",
                path.to_string_lossy()
            )
        })?
    };
    if doc.as_mapping().is_none() {
        bail!(
            "现有配置不是有效的 YAML 映射，未修改：{}",
            path.to_string_lossy()
        );
    }
    Ok(doc)
}

fn is_map(node: &YamlNode) -> bool {
    if node.as_mapping().is_some() {
        return true;
    }
    // `!!omap`/`!!pairs` values are TaggedNodes wrapping a pair sequence
    node.as_tagged()
        .map(|t| {
            t.as_mapping().is_some()
                || t.as_ordered_mapping().is_some()
                || t.as_pairs().is_some()
        })
        .unwrap_or(false)
}

fn is_null(node: &YamlNode) -> bool {
    node.as_scalar().map(|s| s.is_null()).unwrap_or(false)
}

/// decoded scalar string (unquoted/unescaped), None for non-scalars
fn scalar_str(node: &YamlNode) -> Option<String> {
    node.as_scalar().map(|s| s.as_string())
}

/// `m.get(k1).get(k2)...` — None on a missing/Null leaf, Err when a
/// non-map intermediate is descended into (Python AttributeError).
fn map_path(root: &Document, path: &[&str]) -> Result<Option<YamlNode>> {
    match root.try_get_path(&path.join(".")) {
        Ok(node) => {
            if is_null(&node) {
                Ok(None)
            } else {
                Ok(Some(node))
            }
        }
        Err(PathError::NotFound { .. }) => Ok(None),
        Err(e) => Err(anyhow::anyhow!("{e}")),
    }
}

/// tolerant get(): any miss or non-map hop yields None (`or {}` in Python).
fn get_opt(root: &Document, path: &[&str]) -> Option<YamlNode> {
    map_path(root, path).ok().flatten()
}

fn map_get<'a>(node: &'a YamlNode, key: &str) -> Option<YamlNode> {
    if let Some(v) = node.get(key) {
        return Some(v);
    }
    // `!!omap`/`!!pairs`: tagged sequence of single-key mappings
    let entries = node
        .as_tagged()
        .and_then(|t| t.as_ordered_mapping().or_else(|| t.as_pairs()))?;
    entries
        .into_iter()
        .find(|e| e.key_matches(key))
        .and_then(|e| e.value_node())
}

/// settings[key] = value — try_set_path creates missing intermediate
/// mappings and replaces an existing leaf in place.
fn yset(doc: &Document, path: &[&str], value: impl yaml_edit::AsYaml) -> Result<()> {
    doc.try_set_path(&path.join("."), value)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

fn yaml_text(doc: &Document) -> String {
    let mut t = doc.to_string();
    if !t.ends_with('\n') {
        t.push('\n');
    }
    t
}


// ================= adapter =================

pub struct DeepSeekAdapter {
    api: Arc<TokenPlusClient>,
}

impl DeepSeekAdapter {
    pub fn new(api: Arc<TokenPlusClient>) -> Self {
        Self { api }
    }

    /// [settings.yaml, .credentials.yaml]
    fn paths(env: &EnvCtx) -> Vec<PathBuf> {
        let root = env
            .env
            .get("DSH_HOME")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| env.home.join(".dsh"));
        vec![
            root.join("settings.yaml"),
            root.join(".credentials.yaml"),
        ]
    }

    /// credentials validation shared by plan/apply — raises the same
    /// ValueErrors as the Python plan().
    fn validate(settings: &Document, credentials: &Document) -> Result<()> {
        // credentials root non-empty + version != 1 → incompatible
        let creds_non_empty = credentials
            .as_mapping()
            .map(|m| !m.is_empty())
            .unwrap_or(false);
        let version_ok = credentials
            .get("version")
            .and_then(|n| n.as_scalar().map(|s| s.value()))
            .map(|raw| {
                let t = raw.trim();
                t == "1" || t == "1.0" || t.eq_ignore_ascii_case("true")
            })
            .unwrap_or(false);
        if creds_non_empty && !version_ok {
            bail!("现有凭据格式不兼容，请先更新 DeepSeek Harness");
        }
        for section in ["refs", "records"] {
            if let Some(node) = get_opt(credentials, &[section]) {
                if !is_map(&node) {
                    bail!("现有凭据结构不兼容，未修改任何配置");
                }
            }
        }
        if let Some(refs) = get_opt(credentials, &["refs"]).and_then(|n| n.as_mapping().cloned()) {
            for key in refs.keys() {
                let name = key.as_scalar().map(|s| s.as_string()).unwrap_or_default();
                let ok = ref_name_re().is_match(&name)
                    && refs
                        .get(&name)
                        .and_then(|v| scalar_str(&v))
                        .map(|s| !s.is_empty())
                        .unwrap_or(false);
                if !ok {
                    bail!("现有凭据引用格式不兼容，未修改任何配置");
                }
            }
        }
        for parts in [&["llm-pi-ai"][..], &["llm-pi-ai", "providers"][..]] {
            if let Some(node) = get_opt(settings, parts) {
                if !is_map(&node) {
                    bail!("现有模型配置结构不兼容，未修改任何配置");
                }
            }
        }
        Ok(())
    }
}

impl Adapter for DeepSeekAdapter {
    fn id(&self) -> &'static str {
        "deepseek"
    }

    fn restart(&self) -> &'static str {
        "重启 DeepSeek Harness 后生效"
    }

    fn detect(&self, env: &EnvCtx) -> Detected {
        let app = if env.platform == "darwin" {
            base::mac_app("DeepSeek Harness", env)
        } else {
            None
        };
        let installed = app.is_some()
            || env.home.join(".dsh").exists()
            || env.env.get("DSH_HOME").map(|s| !s.is_empty()).unwrap_or(false);
        Detected {
            installed,
            supported: true,
            version: if app.is_some() {
                "已安装".to_string()
            } else {
                String::new()
            },
            files: Self::paths(env)
                .iter()
                .map(|p| base::display_path(&env.home.to_string_lossy(), &p.to_string_lossy()))
                .collect(),
            message: if installed {
                String::new()
            } else {
                "请先安装 DeepSeek Harness".to_string()
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
            let paths = Self::paths(env);
            let Some(settings_text) = fsutil::read_text_if_exists(&paths[0])? else {
                return Ok(if managed { "drift" } else { "todo" });
            };
            let Some(credentials_text) = fsutil::read_text_if_exists(&paths[1])? else {
                return Ok(if managed { "drift" } else { "todo" });
            };
            let settings = document(Some(&settings_text), &paths[0])?;
            let credentials = document(Some(&credentials_text), &paths[1])?;
            let expected_providers = providers(expected)?;
            let default_pid = expected
                .get("default_provider_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .or_else(|| {
                    expected
                        .get("providers")
                        .and_then(Value::as_array)
                        .and_then(|a| a.first())
                        .and_then(|p| p.get("provider_id"))
                        .and_then(Value::as_str)
                        .map(String::from)
                })
                .unwrap_or_default();
            let route = map_path(&settings, &["llm-pi-ai", "providers"])?;
            let refs = map_path(&credentials, &["refs"])?;
            let records = map_path(&credentials, &["records"])?;
            for provider in &expected_providers {
                let pid = provider_str(provider, "provider_id");
                let api_key = provider_str(provider, "api_key");
                let base_url = provider_str(provider, "base_url").trim_end_matches('/');
                let entry = route.as_ref().and_then(|r| map_get(r, pid));
                // `entry or {}` — a truthy non-map leaf raises in Python
                if let Some(e) = &entry {
                    if !is_map(e) && !is_null(e) {
                        bail!("non-map provider entry");
                    }
                }
                let entry_base = entry
                    .filter(is_map)
                    .and_then(|e| map_get(&e, "baseURL"))
                    .and_then(|n| scalar_str(&n));
                if entry_base.as_deref() != Some(base_url) {
                    return Ok(if managed { "drift" } else { "todo" });
                }
                let ref_val = refs
                    .as_ref()
                    .and_then(|r| map_get(r, REF_KEY))
                    .and_then(|n| scalar_str(&n));
                if ref_val.as_deref() != Some(api_key) {
                    return Ok(if managed { "drift" } else { "todo" });
                }
                let record = records
                    .as_ref()
                    .and_then(|r| map_get(r, &format!("llm-pi-ai/{}", pid)));
                if let Some(rec) = &record {
                    if !is_map(rec) && !is_null(rec) {
                        bail!("non-map credential record");
                    }
                }
                let record_key = record
                    .filter(is_map)
                    .and_then(|r| map_get(&r, "key"))
                    .and_then(|n| scalar_str(&n));
                if record_key.as_deref() != Some(api_key) {
                    return Ok(if managed { "drift" } else { "todo" });
                }
            }
            let default_model = map_path(&settings, &["agent-default-model"])?;
            let default_provider = expected
                .get("providers")
                .and_then(Value::as_array)
                .and_then(|ps| {
                    ps.iter().find(|p| {
                        p.get("provider_id").and_then(Value::as_str) == Some(default_pid.as_str())
                    })
                })
                .cloned()
                .unwrap_or(Value::Null);
            let ok = default_model
                .as_ref()
                .and_then(|m| map_get(m, "provider"))
                .and_then(|n| scalar_str(&n))
                .as_deref()
                == Some(default_pid.as_str())
                && default_model
                    .as_ref()
                    .and_then(|m| map_get(m, "model"))
                    .and_then(|n| scalar_str(&n))
                    .as_deref()
                    == Some(provider_str(&default_provider, "model"));
            Ok(if ok { "ok" } else if managed { "drift" } else { "todo" })
        };
        check().unwrap_or("error")
    }

    fn plan(&self, env: &EnvCtx) -> Result<Plan> {
        let mut files = Vec::new();
        for path in Self::paths(env) {
            files.push(fsutil::snapshot(&path)?);
        }
        let settings = document(files[0].original.as_deref(), &files[0].path)?;
        let credentials = document(files[1].original.as_deref(), &files[1].path)?;
        Self::validate(&settings, &credentials)?;
        Ok(Plan {
            files,
            summary: "保留 YAML 注释，更新 TokenPlus 线路与专用密钥".to_string(),
        })
    }

    fn describe(&self, config: &Value, plan: &Plan) -> (Vec<String>, Option<String>) {
        let masked = base::masked_config(config);
        let values = match providers(&masked) {
            Ok(v) => v,
            Err(_) => return (vec![plan.summary.clone()], None),
        };
        let provider = &values[0];
        let pid = provider_str(provider, "provider_id").to_string();
        let base_url = provider_str(provider, "base_url").trim_end_matches('/');
        let model = provider_str(provider, "model");
        let models = provider
            .get("models")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut shown: Vec<base::J> = models
            .iter()
            .take(6)
            .map(|m| {
                let id = base::py_display(Some(m));
                base::J::Obj(vec![
                    ("id".to_string(), base::J::Str(id.clone())),
                    ("name".to_string(), base::J::Str(id)),
                ])
            })
            .collect();
        if models.len() > 6 {
            shown.push(base::J::Obj(vec![
                ("id".to_string(), base::J::s("…")),
                ("name".to_string(), base::J::Str(format!("共 {} 个", models.len()))),
            ]));
        }
        let fragment = base::J::Obj(vec![
            (
                "llm-pi-ai".to_string(),
                base::J::Obj(vec![(
                    "providers".to_string(),
                    base::J::Obj(vec![(
                        pid.clone(),
                        base::J::Obj(vec![
                            (
                                "displayName".to_string(),
                                base::J::Str(provider_str(provider, "provider_name").to_string()),
                            ),
                            ("apiKeyEnv".to_string(), base::J::s(REF_KEY)),
                            ("api".to_string(), base::J::s("openai-completions")),
                            ("baseURL".to_string(), base::J::Str(base_url.to_string())),
                            (
                                "compat".to_string(),
                                base::J::Obj(vec![(
                                    "thinkingFormat".to_string(),
                                    base::J::s("deepseek"),
                                )]),
                            ),
                            ("models".to_string(), base::J::Arr(shown)),
                        ]),
                    )]),
                )]),
            ),
            (
                "agent-default-model".to_string(),
                base::J::Obj(vec![
                    ("provider".to_string(), base::J::Str(pid.clone())),
                    ("model".to_string(), base::J::Str(model.to_string())),
                ]),
            ),
        ]);
        let credentials = base::J::Obj(vec![
            (
                "refs".to_string(),
                base::J::Obj(vec![(REF_KEY.to_string(), base::J::s("sk-***"))]),
            ),
            (
                "records".to_string(),
                base::J::Obj(vec![(
                    format!("llm-pi-ai/{}", pid),
                    base::J::Obj(vec![
                        ("kind".to_string(), base::J::s("api-key")),
                        ("key".to_string(), base::J::s("sk-***")),
                    ]),
                )]),
            ),
            ("version".to_string(), base::J::i(1)),
        ]);
        let changes = vec![
            plan.summary.clone(),
            format!("新增模型供应商 {}（settings.yaml）", pid),
            format!("baseURL = {}", base_url),
            "写入专用密钥 TOKENPLUS_API_KEY = sk-***（.credentials.yaml）".to_string(),
            format!("默认模型 = {}，共 {} 个模型", model, models.len()),
        ];
        let preview = base::clip(
            &format!(
                "{}\n# .credentials.yaml\n{}",
                to_yv(&fragment).to_yaml_string(0),
                to_yv(&credentials).to_yaml_string(0),
            ),
            60,
        );
        (changes, Some(preview))
    }

    fn apply(&self, env: &EnvCtx, config: &Value, plan: &Plan) -> Result<()> {
        fsutil::assert_plan_current(plan)?;
        let _ = self.plan(env)?; // re-validate like the original
        let values = providers(config)?;
        let settings = document(plan.files[0].original.as_deref(), &plan.files[0].path)?;
        let credentials =
            document(plan.files[1].original.as_deref(), &plan.files[1].path)?;
        for provider in &values {
            let pid = provider_str(provider, "provider_id").to_string();
            let entry = YamlValue::OrderedMapping(vec![
                (
                    "displayName".to_string(),
                    YamlValue::from(provider_str(provider, "provider_name")),
                ),
                ("apiKeyEnv".to_string(), YamlValue::from(REF_KEY)),
                ("api".to_string(), YamlValue::from("openai-completions")),
                (
                    "baseURL".to_string(),
                    YamlValue::from(provider_str(provider, "base_url").trim_end_matches('/')),
                ),
                (
                    "compat".to_string(),
                    YamlValue::OrderedMapping(vec![(
                        "thinkingFormat".to_string(),
                        YamlValue::from("deepseek"),
                    )]),
                ),
                (
                    "models".to_string(),
                    YamlValue::Sequence(
                        provider
                            .get("models")
                            .and_then(Value::as_array)
                            .cloned()
                            .unwrap_or_default()
                            .iter()
                            .map(|m| {
                                let id = base::py_display(Some(m));
                                YamlValue::OrderedMapping(vec![
                                    ("id".to_string(), YamlValue::from(id.clone())),
                                    ("name".to_string(), YamlValue::from(id)),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ]);
            yset(&settings, &["llm-pi-ai", "providers", &pid], entry)?;
            yset(
                &credentials,
                &["refs", REF_KEY],
                YamlValue::from(provider_str(provider, "api_key")),
            )?;
            yset(
                &credentials,
                &["records", &format!("llm-pi-ai/{}", pid)],
                YamlValue::OrderedMapping(vec![
                    ("kind".to_string(), YamlValue::from("api-key")),
                    (
                        "key".to_string(),
                        YamlValue::from(provider_str(provider, "api_key")),
                    ),
                ]),
            )?;
        }
        let selected = values
            .iter()
            .find(|p| {
                p.get("provider_id").and_then(Value::as_str)
                    == Some(
                        config
                            .get("default_provider_id")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| provider_str(&values[0], "provider_id")),
                    )
            })
            .cloned()
            .unwrap_or_else(|| values[0].clone());
        yset(
            &settings,
            &["agent-default-model"],
            YamlValue::OrderedMapping(vec![
                (
                    "provider".to_string(),
                    YamlValue::from(provider_str(&selected, "provider_id")),
                ),
                (
                    "model".to_string(),
                    YamlValue::from(provider_str(&selected, "model")),
                ),
            ]),
        )?;
        yset(&credentials, &["version"], YamlValue::from(1i64))?;
        fsutil::atomic_write(&plan.files[1].path, &yaml_text(&credentials))?;
        fsutil::atomic_write(&plan.files[0].path, &yaml_text(&settings))?;
        Ok(())
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

/// Convert an ordered preview J into a YamlValue (same order).
fn to_yv(j: &base::J) -> YamlValue {
    match j {
        base::J::Null => YamlValue::Scalar(yaml_edit::ScalarValue::null()),
        base::J::Bool(b) => YamlValue::from(*b),
        base::J::Int(n) => YamlValue::from(*n),
        base::J::Str(s) => YamlValue::from(s.clone()),
        base::J::Arr(items) => YamlValue::Sequence(items.iter().map(to_yv).collect()),
        base::J::Obj(pairs) => YamlValue::OrderedMapping(
            pairs.iter().map(|(k, v)| (k.clone(), to_yv(v))).collect(),
        ),
    }
}
