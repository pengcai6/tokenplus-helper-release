//! Tool adapters — port of helper/adapters/*.py.
//! Each adapter knows how to detect / status / plan / apply / verify the
//! configuration of one AI tool against the TokenPlus gateway.

pub mod base;
pub mod claude_code;
pub mod claude_desktop;
pub mod codex;
pub mod deepseek;
pub mod grok;
pub mod workbuddy;

use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::tpapi::TokenPlusClient;
use crate::types::{Detected, EnvCtx, Plan, Probe};

pub use base::*;

/// Port of the Python adapter surface (helper/service.py calls these).
/// `status` returns one of: "ok" | "todo" | "drift" | "missing" | "error".
pub trait Adapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn restart(&self) -> &'static str;
    fn detect(&self, env: &EnvCtx) -> Detected;
    fn status(&self, env: &EnvCtx, expected: Option<&Value>, managed: bool) -> &'static str;
    fn plan(&self, env: &EnvCtx) -> Result<Plan>;
    /// (changes, preview) — keys must already be masked via base::masked_config.
    fn describe(&self, config: &Value, plan: &Plan) -> (Vec<String>, Option<String>);
    fn apply(&self, env: &EnvCtx, config: &Value, plan: &Plan) -> Result<()>;
    fn verify(&self, env: &EnvCtx, config: &Value) -> Result<Probe>;
    fn current_model(&self, config: &Value) -> String;
    fn available_models(&self, config: &Value) -> Vec<String>;
    fn set_model(&self, config: &Value, model: &str) -> Result<Value>;
}

/// Build the adapter registry — same order-independent map as Python.
pub fn all(api: Arc<TokenPlusClient>) -> HashMap<String, Box<dyn Adapter>> {
    let mut map: HashMap<String, Box<dyn Adapter>> = HashMap::new();
    map.insert("claude-code".into(), Box::new(claude_code::ClaudeCodeAdapter::new(api.clone())));
    map.insert("claude-desktop".into(), Box::new(claude_desktop::ClaudeDesktopAdapter::new(api.clone())));
    map.insert("codex".into(), Box::new(codex::CodexAdapter::new(api.clone())));
    map.insert("grok".into(), Box::new(grok::GrokAdapter::new(api.clone())));
    map.insert("deepseek".into(), Box::new(deepseek::DeepSeekAdapter::new(api.clone())));
    map.insert("workbuddy".into(), Box::new(workbuddy::WorkBuddyAdapter::new(api)));
    map
}
