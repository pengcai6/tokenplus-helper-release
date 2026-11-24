//! Grok Build adapter — merges TokenPlus model sections into config.toml.
//!
//! Port of helper/adapters/grok.py. Writes both the CLI config
//! (~/.grok/config.toml) and, on macOS, the desktop app config
//! (Application Support/com.grokapp.grok-app/agent-home/config.toml).

use anyhow::{bail, Result};
use regex::Regex;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, OnceLock};

use super::base;
use super::Adapter;
use crate::fsutil;
use crate::tpapi::{required_base, required_key, TokenPlusClient};
use crate::types::{Detected, EnvCtx, Plan, Probe};

const PROVIDER_ID: &str = "tokenplus";

fn model_id_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^[A-Za-z0-9._:/-]+$").unwrap())
}

fn blank_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^[ \t]*$").unwrap())
}

fn table_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^[ \t]*\[").unwrap())
}

fn skip_table_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r#"^[ \t]*\[[ \t]*model[ \t]*\.[ \t]*"?tokenplus"#).unwrap()
    })
}

fn models_table_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^[ \t]*\[[ \t]*models[ \t]*\][ \t]*(#.*)?$").unwrap())
}

fn default_key_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^[ \t]*default[ \t]*=").unwrap())
}

/// The account's usable model list — raises like the Python `models()`.
fn grok_models(config: &Value) -> Result<Vec<String>> {
    let values: Vec<Value> = match config.get("models") {
        Some(Value::Array(a)) => a.clone(),
        _ => match config.get("model") {
            Some(m) if base::truthy(m) => vec![m.clone()],
            _ => vec![],
        },
    };
    let model = config.get("model");
    let bad = values.is_empty()
        || values
            .iter()
            .any(|m| !model_id_re().is_match(&base::py_display(Some(m))))
        || !base::truthy_opt(model)
        || !values.iter().any(|m| Some(m) == model);
    if bad {
        bail!("模型信息不完整");
    }
    Ok(values
        .iter()
        .map(|m| base::py_display(Some(m)))
        .collect())
}

fn merge_grok_toml(existing: &str, default_id: &str, sections: &str) -> String {
    let mut output: Vec<String> = Vec::new();
    let mut skip = false;
    let mut in_models = false;
    let mut found = false;
    let mut blanks = 0usize;

    macro_rules! emit {
        ($line:expr) => {{
            while blanks > 0 {
                output.push(String::new());
                blanks -= 1;
            }
            output.push($line.to_string());
        }};
    }

    for line in existing.split('\n') {
        if blank_re().is_match(line) {
            if !skip {
                blanks += 1;
            }
            continue;
        }
        if table_re().is_match(line) {
            skip = skip_table_re().is_match(line);
            in_models = models_table_re().is_match(line);
            if skip {
                continue;
            }
            emit!(line);
            if in_models {
                output.push(format!("default = {}", base::toml_string(default_id)));
                found = true;
            }
            continue;
        }
        if skip {
            continue;
        }
        if in_models && default_key_re().is_match(line) {
            continue;
        }
        emit!(line);
    }
    if !found {
        output.push(String::new());
        output.push("[models]".to_string());
        output.push(format!("default = {}", base::toml_string(default_id)));
    }
    output.push(String::new());
    output.push(sections.trim_end().to_string());
    output.push(String::new());
    output.join("\n")
}

/// The [model."tokenplus/<id>"] blocks written to the CLI config.
fn cli_sections(config: &Value) -> Result<String> {
    let base_url = required_base(config)?;
    let key = required_key(config)?;
    let mut blocks = Vec::new();
    for model in grok_models(config)? {
        blocks.push(format!(
            "[model.\"{}/{}\"]\nmodel = {}\nname = {}\nbase_url = {}\napi_key = {}\napi_backend = \"responses\"\nsupports_reasoning_effort = true\n",
            PROVIDER_ID,
            model,
            base::toml_string(&model),
            base::toml_string(&format!("TokenPlus · {}", model)),
            base::toml_string(&base_url),
            base::toml_string(&key),
        ));
    }
    Ok(blocks.join("\n"))
}

/// The single [model.tokenplus] block written to the desktop app config.
fn app_section(config: &Value) -> Result<String> {
    let base_url = required_base(config)?;
    let key = required_key(config)?;
    let app_models: Vec<base::J> = grok_models(config)?
        .into_iter()
        .map(|m| {
            base::J::Obj(vec![
                ("id".to_string(), base::J::Str(m.clone())),
                ("name".to_string(), base::J::Str(m)),
            ])
        })
        .collect();
    Ok(format!(
        "[model.{}]\nmodel = {}\nname = \"TokenPlus · Grok\"\nbase_url = {}\napi_key = {}\napi_backend = \"responses\"\nsupports_reasoning_effort = true\napp_provider_mode = \"generic\"\napp_models = '{}'\n",
        PROVIDER_ID,
        base::toml_string(
            config
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("")
        ),
        base::toml_string(&base_url),
        base::toml_string(&key),
        base::json_compact_j(&base::J::Arr(app_models)),
    ))
}

/// cli path plus optional desktop-app path (None on win32).
struct GrokPaths {
    cli: PathBuf,
    app: Option<PathBuf>,
}

impl GrokPaths {
    fn all(&self) -> Vec<&PathBuf> {
        let mut v = vec![&self.cli];
        if let Some(app) = &self.app {
            v.push(app);
        }
        v
    }
}

pub struct GrokAdapter {
    api: Arc<TokenPlusClient>,
}

impl GrokAdapter {
    pub fn new(api: Arc<TokenPlusClient>) -> Self {
        Self { api }
    }

    fn paths(env: &EnvCtx) -> GrokPaths {
        let cli_root = env
            .env
            .get("GROK_HOME")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| env.home.join(".grok"));
        let cli = cli_root.join("config.toml");
        if env.platform == "win32" {
            return GrokPaths { cli, app: None };
        }
        let root = env
            .env
            .get("GROK_APP_HOME")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                env.home
                    .join("Library")
                    .join("Application Support")
                    .join("com.grokapp.grok-app")
            });
        GrokPaths {
            cli,
            app: Some(root.join("agent-home").join("config.toml")),
        }
    }

    /// Port of adapter.install() — official x.ai script, like grok-setup.sh.
    /// Returns a no-op callback like the Python lambda. Unused by the service
    /// layer, same as Python.
    #[allow(dead_code)]
    pub fn install(&self) -> Result<Box<dyn FnOnce() + Send>> {
        let mut cmd = Command::new("bash");
        cmd.arg("-c")
            .arg("curl -fsSL https://x.ai/cli/install.sh | bash");
        let out = base::run_command(&mut cmd, 600)?;
        if !out.status.success() {
            bail!("Grok Build 安装失败，可手动运行：curl -fsSL https://x.ai/cli/install.sh | bash");
        }
        Ok(Box::new(|| {}))
    }
}

impl Adapter for GrokAdapter {
    fn id(&self) -> &'static str {
        "grok"
    }

    fn restart(&self) -> &'static str {
        "新开一个终端窗口或重启 Grok 后生效"
    }

    fn detect(&self, env: &EnvCtx) -> Detected {
        let binary = base::executable("grok", env);
        let app = if env.platform == "darwin" {
            base::mac_app("Grok", env)
        } else {
            None
        };
        let paths = Self::paths(env);
        let app_config_exists = paths
            .app
            .as_ref()
            .and_then(|p| p.parent())
            .map(|d| d.exists())
            .unwrap_or(false);
        let installed = binary.is_some()
            || app.is_some()
            || paths.cli.parent().map(|d| d.exists()).unwrap_or(false)
            || app_config_exists;
        Detected {
            installed,
            supported: true,
            version: base::version_of(binary.as_ref()),
            files: paths
                .all()
                .iter()
                .map(|p| {
                    base::display_path(&env.home.to_string_lossy(), &p.to_string_lossy())
                })
                .collect(),
            message: if installed {
                String::new()
            } else {
                "请先安装 Grok Build：点「去下载安装」打开安装页".to_string()
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
            let base_url = required_base(expected)?;
            let key = required_key(expected)?;
            let default_model = expected
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("");
            let mut ok = true;
            for (index, file) in paths.all().iter().enumerate() {
                let file_ok = (|| -> Result<bool> {
                    let Some(text) = fsutil::read_text_if_exists(file)? else {
                        return Ok(false);
                    };
                    let parsed = base::toml_parse(&text)?;
                    let model_id = if index == 0 {
                        format!("{}/{}", PROVIDER_ID, default_model)
                    } else {
                        PROVIDER_ID.to_string()
                    };
                    Ok(base::toml_nested_str(&parsed, &["models", "default"])
                        == Some(model_id.as_str())
                        && base::toml_nested_str(&parsed, &["model", &model_id, "base_url"])
                            == Some(base_url.as_str())
                        && base::toml_nested_str(&parsed, &["model", &model_id, "api_key"])
                            == Some(key.as_str())
                        && base::toml_nested_str(&parsed, &["model", &model_id, "model"])
                            == Some(default_model))
                })();
                if !file_ok.unwrap_or(false) {
                    ok = false;
                    break;
                }
            }
            Ok(if ok { "ok" } else if managed { "drift" } else { "todo" })
        };
        check().unwrap_or("error")
    }

    fn plan(&self, env: &EnvCtx) -> Result<Plan> {
        let paths = Self::paths(env);
        let mut files = Vec::new();
        for path in paths.all() {
            files.push(fsutil::snapshot(path)?);
        }
        for file in &files {
            if let Some(original) = &file.original {
                if !original.trim().is_empty() && base::toml_parse(original).is_err() {
                    bail!("现有配置无法读取，未覆盖：{}", file.path.to_string_lossy());
                }
            }
        }
        Ok(Plan {
            files,
            summary: "替换 TokenPlus 模型段并更新默认模型，其他 TOML 内容保留".to_string(),
        })
    }

    fn describe(&self, config: &Value, plan: &Plan) -> (Vec<String>, Option<String>) {
        let masked = base::masked_config(config);
        let available = match grok_models(&masked) {
            Ok(v) => v,
            Err(_) => return (vec![plan.summary.clone()], None),
        };
        let mut shown = available[..available.len().min(4)].join("、");
        if available.len() > 4 {
            shown.push('…');
        }
        let model = base::py_display(masked.get("model"));
        let mut preview = format!(
            "[models]\ndefault = \"{}/{}\"\n\n{}",
            PROVIDER_ID,
            model,
            match cli_sections(&masked) {
                Ok(s) => s,
                Err(_) => return (vec![plan.summary.clone()], None),
            }
        );
        if plan.files.len() > 1 {
            match app_section(&masked) {
                Ok(section) => preview.push_str(&format!(
                    "\n# —— {} ——\n{}",
                    plan.files[1].path.to_string_lossy(),
                    section
                )),
                Err(_) => return (vec![plan.summary.clone()], None),
            }
        }
        let changes = vec![
            plan.summary.clone(),
            format!("默认模型 default = \"{}/{}\"", PROVIDER_ID, model),
            format!("写入 {} 个 TokenPlus 模型段：{}", available.len(), shown),
            format!("base_url = {}", required_base(&masked).unwrap_or_default()),
            "api_key = sk-***".to_string(),
        ];
        (changes, Some(base::clip(&preview, 60)))
    }

    fn apply(&self, _env: &EnvCtx, config: &Value, plan: &Plan) -> Result<()> {
        fsutil::assert_plan_current(plan)?;
        let model = config.get("model").and_then(Value::as_str).unwrap_or("");
        let mut outputs = vec![merge_grok_toml(
            plan.files[0].original.as_deref().unwrap_or(""),
            &format!("{}/{}", PROVIDER_ID, model),
            &cli_sections(config)?,
        )];
        if plan.files.len() > 1 {
            outputs.push(merge_grok_toml(
                plan.files[1].original.as_deref().unwrap_or(""),
                PROVIDER_ID,
                &app_section(config)?,
            ));
        }
        for output in &outputs {
            if base::toml_parse(output).is_err() {
                bail!("合并后的 Grok 配置无效，未写入");
            }
        }
        for (output, file) in outputs.iter().zip(plan.files.iter()) {
            fsutil::atomic_write(&file.path, output)?;
        }
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
        if !base::truthy(config) {
            return vec![];
        }
        grok_models(config).unwrap_or_default()
    }

    fn set_model(&self, config: &Value, model: &str) -> Result<Value> {
        if !grok_models(config)?.iter().any(|m| m == model) {
            bail!("这个模型不在账号已开通的列表中");
        }
        let mut out = config.clone();
        if let Some(obj) = out.as_object_mut() {
            obj.insert("model".to_string(), Value::String(model.to_string()));
        }
        Ok(out)
    }
}
