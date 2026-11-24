//! Diagnostics — port of the run_diagnostics/copy_report logic that lives
//! in helper/gui/api.py in the current build.

use serde_json::{json, Value};
use std::time::Duration;

use crate::service::HelperService;
use crate::tpapi::PORTAL_ORIGIN;

/// urllib.request.urlopen(url, timeout=10) — True for *any* HTTP response
/// (Python counts HTTPError.status too), False on network failure.
fn http_ok(url: &str) -> bool {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };
    client.get(url).send().is_ok()
}

/// Run the 6 connectivity checks; emits `diagnostics:changed` per step via
/// the callback. Returns the final item list.
pub fn run<F>(mut emit: F, ctx: &HelperService) -> Vec<Value>
where
    F: FnMut(Vec<Value>),
{
    let mut checks = vec![
        json!({"id": "network", "name": "网络", "description": "能连上 TokenPlus"}),
        json!({"id": "login", "name": "登录", "description": "本机保存的密钥有效"}),
        json!({"id": "balance", "name": "余额", "description": "TokenPlus 账户可用"}),
        json!({"id": "claude", "name": "Claude 线路",
            "description": "Claude Code、Claude 桌面版使用"}),
        json!({"id": "codex", "name": "Codex 线路", "description": "Codex、WorkBuddy 使用"}),
        json!({"id": "general", "name": "通用线路", "description": "Grok、DeepSeek 等使用"}),
    ];
    emit(checks.clone());
    let mut step = |index: usize, result: Value| {
        if let Some(obj) = checks[index].as_object_mut() {
            if let Some(update) = result.as_object() {
                for (k, v) in update {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }
        emit(checks.clone());
    };

    step(0, json!({"state": "run", "result": "正在连接…"}));
    let site_ok = http_ok(PORTAL_ORIGIN);
    step(
        0,
        json!({
            "state": if site_ok { "done" } else { "fail" },
            "result": if site_ok { "连接正常" } else { "无法连接 TokenPlus" },
        }),
    );

    step(1, json!({"state": "run", "result": "正在检查…"}));
    let key = ctx.get_key();
    let mut valid = false;
    if let Some(key) = &key {
        valid = ctx.api.validate_key(key).is_ok();
    }
    step(
        1,
        json!({
            "state": if valid { "done" } else { "fail" },
            "result": if valid { "密钥有效" } else { "没有可用密钥" },
        }),
    );

    step(2, json!({"state": "run", "result": "正在检查…"}));
    let (account, error) = crate::dispatch::account(ctx);
    let account_ok = account.is_some() && key.is_some();
    step(
        2,
        json!({
            "state": if account_ok { "done" } else { "fail" },
            "result": if account_ok {
                "账户可用".to_string()
            } else if error.is_empty() {
                "未接入".to_string()
            } else {
                error
            },
        }),
    );

    let managed = ctx.state.managed();
    for (index, tool_ids) in [
        (3usize, vec!["claude-code"]),
        (4usize, vec!["codex"]),
        (5usize, vec!["grok"]),
    ] {
        step(index, json!({"state": "run", "result": "正在测试…"}));
        let target = tool_ids.iter().find(|t| {
            crate::tpapi::truthy(&managed.get(**t).cloned().unwrap_or(json!({}))["expected"])
        });
        let target = match target {
            Some(target) => *target,
            None => {
                step(index, json!({"state": "done", "result": "未接入，跳过"}));
                continue;
            }
        };
        let expected = managed.get(target).cloned().unwrap_or(json!({}))["expected"].clone();
        match ctx.adapters.get(target).map(|a| a.verify(&ctx.env, &expected)) {
            Some(Ok(result)) => step(
                index,
                json!({
                    "state": if result.ok { "done" } else { "fail" },
                    "result": result.message,
                }),
            ),
            Some(Err(error)) => step(
                index,
                json!({"state": "fail", "result": error.to_string()}),
            ),
            None => step(index, json!({"state": "done", "result": "未接入，跳过"})),
        }
    }
    checks
}

/// Copyable plain-text report (no secrets).
pub fn report(items: &[Value]) -> String {
    let mut lines = vec![
        "TokenPlus 助手连接诊断报告".to_string(),
        format!(
            "时间：{}",
            chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%z")
        ),
    ];
    for c in items {
        let state = match c["state"].as_str() {
            Some("done") => "正常",
            Some("fail") => "失败",
            _ => "未检查",
        };
        lines.push(format!(
            "{}：{} — {}",
            c["name"].as_str().unwrap_or_default(),
            state,
            c["result"].as_str().unwrap_or_default()
        ));
    }
    lines.push("本报告不包含登录密钥或工具专用密钥。".to_string());
    lines.join("\n")
}
