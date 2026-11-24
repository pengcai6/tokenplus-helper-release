//! Claude 桌面版 adapter — writes a gateway profile into
//! ~/Library/Application Support/Claude-3p.
//! Ported from helper/adapters/claude_desktop.py.

use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

use crate::fsutil::{assert_plan_current, atomic_write, json_text, parse_json_object, snapshot};
use crate::tpapi::{required_base_gateway, required_key, TokenPlusClient};
use crate::types::{Detected, EnvCtx, Plan, Probe};

use super::base::{claude_models, clip, display_path, mac_app, version_of};

const PROFILE_ID: &str = "7a1c9f60-5b2e-4c3a-9d18-3f6b0e2a41d7";

pub struct ClaudeDesktopAdapter {
    api: Arc<TokenPlusClient>,
}

impl ClaudeDesktopAdapter {
    pub fn new(api: Arc<TokenPlusClient>) -> Self {
        Self { api }
    }

    fn root(&self, env: &EnvCtx) -> PathBuf {
        env.home
            .join("Library")
            .join("Application Support")
            .join("Claude-3p")
    }

    /// (claude_desktop_config.json, configLibrary/_meta.json, profile json)
    fn paths(&self, env: &EnvCtx) -> Vec<PathBuf> {
        let root = self.root(env);
        vec![
            root.join("claude_desktop_config.json"),
            root.join("configLibrary").join("_meta.json"),
            root.join("configLibrary").join(format!("{PROFILE_ID}.json")),
        ]
    }
}

impl super::Adapter for ClaudeDesktopAdapter {
    fn id(&self) -> &'static str {
        "claude-desktop"
    }

    fn restart(&self) -> &'static str {
        "完全退出 Claude 再重新打开后生效"
    }

    fn detect(&self, env: &EnvCtx) -> Detected {
        if env.platform == "win32" {
            return Detected {
                installed: false,
                supported: false,
                version: String::new(),
                files: vec![],
                message: "Windows 首版暂不支持 Claude 桌面版，不会猜测配置路径".to_string(),
            };
        }
        let app = mac_app("Claude", env);
        let installed = app.is_some() || self.root(env).exists();
        let home = env.home.to_string_lossy();
        Detected {
            installed,
            supported: true,
            version: if app.is_some() {
                "已安装".to_string()
            } else {
                version_of(None)
            },
            files: self
                .paths(env)
                .iter()
                .map(|p| display_path(&home, &p.to_string_lossy()))
                .collect(),
            message: if installed {
                String::new()
            } else {
                "请先安装 Claude 桌面版".to_string()
            },
        }
    }

    fn status(&self, env: &EnvCtx, expected: Option<&Value>, managed: bool) -> &'static str {
        let detection = self.detect(env);
        if !detection.supported || !detection.installed {
            return "missing";
        }
        let fallback = || if managed { "drift" } else { "todo" };
        (|| -> Option<&'static str> {
            let expected = match expected {
                Some(e) => e,
                None => return Some(fallback()),
            };
            let paths = self.paths(env);
            let docs: Vec<serde_json::Map<String, Value>> = paths
                .iter()
                .map(|p| {
                    let snap = snapshot(p).ok()?;
                    parse_json_object(snap.original.as_deref(), p).ok()
                })
                .collect::<Option<Vec<_>>>()?;
            let (mode_file, meta_file, profile_file) = (&docs[0], &docs[1], &docs[2]);
            let ok = profile_file.get("inferenceProvider") == Some(&json!("gateway"))
                && profile_file.get("inferenceCredentialKind") == Some(&json!("static"))
                && profile_file.get("inferenceGatewayBaseUrl").and_then(Value::as_str)
                    == required_base_gateway(expected).ok().as_deref()
                && profile_file.get("inferenceGatewayApiKey").and_then(Value::as_str)
                    == required_key(expected).ok().as_deref()
                && meta_file.get("appliedId") == Some(&json!(PROFILE_ID))
                && mode_file.get("deploymentMode") == Some(&json!("3p"))
                && !profile_file.contains_key("inferenceModels");
            Some(if ok { "ok" } else { fallback() })
        })()
        .unwrap_or("error")
    }

    fn plan(&self, env: &EnvCtx) -> Result<Plan> {
        if env.platform == "win32" {
            bail!("Windows 首版暂不支持 Claude 桌面版");
        }
        let files = self
            .paths(env)
            .iter()
            .map(|p| snapshot(p))
            .collect::<Result<Vec<_>>>()?;
        for file in &files {
            let value = parse_json_object(file.original.as_deref(), &file.path)?;
            if file.path.ends_with("_meta.json") && value.contains_key("entries") {
                if !value["entries"].is_array() {
                    bail!("桌面配置索引 entries 必须是数组，未覆盖");
                }
            }
        }
        Ok(Plan {
            files,
            summary: "新增 TokenPlus 配置档并设为当前，保留其他配置档".to_string(),
        })
    }

    fn describe(&self, config: &Value, plan: &Plan) -> (Vec<String>, Option<String>) {
        let result = (|| -> Result<(Vec<String>, Option<String>)> {
            let base = required_base_gateway(config)?;
            let profile = json!({
                "inferenceProvider": "gateway",
                "inferenceCredentialKind": "static",
                "inferenceGatewayBaseUrl": base,
                "inferenceGatewayApiKey": "sk-***",
            });
            Ok((
                vec![
                    plan.summary.clone(),
                    format!("写入 inferenceGatewayBaseUrl = {base}"),
                    "写入 inferenceGatewayApiKey = sk-***".to_string(),
                    "deploymentMode = \"3p\"，TokenPlus 配置档设为当前".to_string(),
                ],
                Some(clip(&json_text(&profile), 60)),
            ))
        })();
        result.unwrap_or_else(|e| (vec![e.to_string()], None))
    }

    fn apply(&self, _env: &EnvCtx, config: &Value, plan: &Plan) -> Result<()> {
        assert_plan_current(plan)?;
        let (mode_file, meta_file, profile_file) = (&plan.files[0], &plan.files[1], &plan.files[2]);
        let mut mode = parse_json_object(mode_file.original.as_deref(), &mode_file.path)?;
        let mut meta = parse_json_object(meta_file.original.as_deref(), &meta_file.path)?;
        let mut profile =
            parse_json_object(profile_file.original.as_deref(), &profile_file.path)?;
        profile.insert(
            "inferenceProvider".to_string(),
            json!("gateway"),
        );
        profile.insert(
            "inferenceCredentialKind".to_string(),
            json!("static"),
        );
        profile.insert(
            "inferenceGatewayBaseUrl".to_string(),
            json!(required_base_gateway(config)?),
        );
        profile.insert(
            "inferenceGatewayApiKey".to_string(),
            json!(required_key(config)?),
        );
        profile.remove("inferenceModels");
        let entries = meta
            .get("entries")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut kept: Vec<Value> = entries
            .into_iter()
            .filter(|entry| {
                !(entry.is_object() && entry["id"].as_str() == Some(PROFILE_ID))
            })
            .collect();
        kept.push(json!({"id": PROFILE_ID, "name": "TokenPlus"}));
        meta.insert("entries".to_string(), Value::Array(kept));
        meta.insert("appliedId".to_string(), json!(PROFILE_ID));
        mode.insert("deploymentMode".to_string(), json!("3p"));
        claude_models(config)?;
        atomic_write(&profile_file.path, &json_text(&Value::Object(profile)))?;
        atomic_write(&mode_file.path, &json_text(&Value::Object(mode)))?;
        atomic_write(&meta_file.path, &json_text(&Value::Object(meta)))
    }

    fn verify(&self, _env: &EnvCtx, config: &Value) -> Result<Probe> {
        self.api.probe(self.id(), config)
    }

    fn current_model(&self, _config: &Value) -> String {
        "自动".to_string()
    }

    fn available_models(&self, _config: &Value) -> Vec<String> {
        vec!["自动".to_string()]
    }

    fn set_model(&self, config: &Value, _model: &str) -> Result<Value> {
        Ok(config.clone())
    }
}
