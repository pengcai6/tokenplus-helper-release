//! Claude Code adapter — merges 8 env keys into ~/.claude/settings.json.
//!
//! Port of helper/adapters/claude_code.py.

use anyhow::{anyhow, Result};
use serde_json::{Map, Value};
use std::path::PathBuf;
use std::sync::Arc;

use super::base;
use super::Adapter;
use crate::fsutil;
use crate::tpapi::{clean_model, required_base, required_key, TokenPlusClient};
use crate::types::{Detected, EnvCtx, Plan, Probe};

pub struct ClaudeCodeAdapter {
    api: Arc<TokenPlusClient>,
}

impl ClaudeCodeAdapter {
    pub fn new(api: Arc<TokenPlusClient>) -> Self {
        Self { api }
    }

    fn file(env: &EnvCtx) -> PathBuf {
        env.home.join(".claude").join("settings.json")
    }

    fn wanted(config: &Value) -> Result<Vec<(&'static str, String)>> {
        let models = base::claude_models(config)?;
        let model = |k: &str| -> Result<String> {
            models
                .get(k)
                .cloned()
                .ok_or_else(|| anyhow!("'{}'", k))
        };
        Ok(vec![
            ("ANTHROPIC_BASE_URL", required_base(config)?),
            ("ANTHROPIC_AUTH_TOKEN", required_key(config)?),
            ("ANTHROPIC_MODEL", model("default")?),
            ("ANTHROPIC_DEFAULT_OPUS_MODEL", model("opus")?),
            ("ANTHROPIC_DEFAULT_SONNET_MODEL", model("sonnet")?),
            ("ANTHROPIC_DEFAULT_HAIKU_MODEL", model("haiku")?),
            ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1".into()),
            ("DISABLE_AUTOUPDATER", "1".into()),
        ])
    }
}

impl Adapter for ClaudeCodeAdapter {
    fn id(&self) -> &'static str {
        "claude-code"
    }

    fn restart(&self) -> &'static str {
        "新开一个终端窗口即可生效"
    }

    fn detect(&self, env: &EnvCtx) -> Detected {
        let binary = base::executable("claude", env);
        let file = Self::file(env);
        let installed = binary.is_some()
            || file.parent().map(|d| d.exists()).unwrap_or(false);
        Detected {
            installed,
            supported: true,
            version: base::version_of(binary.as_ref()),
            files: vec![base::display_path(
                &env.home.to_string_lossy(),
                &file.to_string_lossy(),
            )],
            message: if installed {
                String::new()
            } else {
                "这台电脑上还没有 Claude Code".to_string()
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
            let path = Self::file(env);
            let snap = fsutil::snapshot(&path)?;
            let settings = fsutil::parse_json_object(snap.original.as_deref(), &path)?;
            let variables = settings.get("env");
            let wanted = Self::wanted(expected)?;
            let ok = match variables {
                Some(Value::Object(vars)) => wanted.iter().all(|(k, v)| {
                    vars.get(*k).and_then(Value::as_str) == Some(v.as_str())
                }),
                _ => false,
            };
            Ok(if ok { "ok" } else if managed { "drift" } else { "todo" })
        };
        check().unwrap_or("error")
    }

    fn plan(&self, env: &EnvCtx) -> Result<Plan> {
        let file = fsutil::snapshot(&Self::file(env))?;
        fsutil::parse_json_object(file.original.as_deref(), &file.path)?;
        Ok(Plan {
            files: vec![file],
            summary: "合并 8 个 TokenPlus 环境设置，其他设置保留".to_string(),
        })
    }

    fn describe(&self, config: &Value, plan: &Plan) -> (Vec<String>, Option<String>) {
        let masked = base::masked_config(config);
        let wanted = match Self::wanted(&masked) {
            Ok(w) => w,
            Err(_) => return (vec![plan.summary.clone()], None),
        };
        let get = |k: &str| -> &str {
            wanted
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.as_str())
                .unwrap_or("")
        };
        let changes = vec![
            plan.summary.clone(),
            format!("写入 ANTHROPIC_BASE_URL = {}", get("ANTHROPIC_BASE_URL")),
            "写入 ANTHROPIC_AUTH_TOKEN = sk-***".to_string(),
            format!("写入 ANTHROPIC_MODEL = {}", get("ANTHROPIC_MODEL")),
            format!(
                "Opus / Sonnet / Haiku 依次映射为 {} / {} / {}",
                get("ANTHROPIC_DEFAULT_OPUS_MODEL"),
                get("ANTHROPIC_DEFAULT_SONNET_MODEL"),
                get("ANTHROPIC_DEFAULT_HAIKU_MODEL")
            ),
            "关闭非必要流量与自动更新".to_string(),
        ];
        let env_obj = base::J::Obj(
            wanted
                .iter()
                .map(|(k, v)| (k.to_string(), base::J::Str(v.clone())))
                .collect(),
        );
        let preview = base::clip(
            &base::json_text_j(&base::J::Obj(vec![("env".to_string(), env_obj)])),
            60,
        );
        (changes, Some(preview))
    }

    fn apply(&self, _env: &EnvCtx, config: &Value, plan: &Plan) -> Result<()> {
        fsutil::assert_plan_current(plan)?;
        let file = &plan.files[0];
        let mut settings = fsutil::parse_json_object(file.original.as_deref(), &file.path)?;
        let mut variables: Map<String, Value> = match settings.get("env") {
            Some(Value::Object(m)) => m.clone(),
            _ => Map::new(),
        };
        for (k, v) in Self::wanted(config)? {
            variables.insert(k.to_string(), Value::String(v));
        }
        settings.insert("env".to_string(), Value::Object(variables));
        fsutil::atomic_write(&file.path, &fsutil::json_text(&Value::Object(settings)))
    }

    fn verify(&self, _env: &EnvCtx, config: &Value) -> Result<Probe> {
        self.api.probe(self.id(), config)
    }

    fn set_model(&self, config: &Value, model: &str) -> Result<Value> {
        let mut models = base::claude_models(config)?;
        models.insert("default".to_string(), clean_model(model)?);
        let default = models.get("default").cloned().unwrap_or_default();
        let mut out = config.clone();
        if let Some(obj) = out.as_object_mut() {
            let map: Map<String, Value> = models
                .into_iter()
                .map(|(k, v)| (k, Value::String(v)))
                .collect();
            obj.insert("models".to_string(), Value::Object(map));
            obj.insert("default".to_string(), Value::String(default));
        }
        Ok(out)
    }

    fn available_models(&self, config: &Value) -> Vec<String> {
        if !base::truthy(config) {
            return vec![];
        }
        let models = match base::claude_models(config) {
            Ok(m) => m,
            Err(_) => return vec![],
        };
        let mut seen: Vec<String> = vec![];
        let mut push = |m: &String| {
            if !m.is_empty() && !seen.contains(m) {
                seen.push(m.clone());
            }
        };
        // Python iterates the dict's insertion order; the built config is
        // always {default, opus, sonnet, haiku} — then any extra keys.
        for k in ["default", "opus", "sonnet", "haiku"] {
            if let Some(m) = models.get(k) {
                push(m);
            }
        }
        let mut rest: Vec<&String> = models.keys().collect();
        rest.sort();
        for k in rest {
            if !["default", "opus", "sonnet", "haiku"].contains(&k.as_str()) {
                push(models.get(k).unwrap());
            }
        }
        // full upstream pool, so any account model can be picked
        for m in config
            .get("pool")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            push(&m.to_string());
        }
        seen
    }

    fn current_model(&self, config: &Value) -> String {
        if !base::truthy(config) {
            return String::new();
        }
        base::claude_models(config)
            .ok()
            .and_then(|m| m.get("default").cloned())
            .unwrap_or_default()
    }
}
