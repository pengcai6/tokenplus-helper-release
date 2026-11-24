//! Self check — port of helper/selfcheck.py.
//!
//! Report shape mirrors the original: {checkedAt, running, issues, fixed}.
//! issues: [{id, tool?, text, fixable}]  fixed: [message strings]

use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::envguard::describe_issue;
use crate::fsutil::{json_text, now_ms};
use crate::service::HelperService;

/// Serializes concurrent runs like Python's `_pending`.
static PENDING: AtomicBool = AtomicBool::new(false);

fn report_file(service: &HelperService) -> std::path::PathBuf {
    // Python: sibling of backups/ dir -> <data_root>/selfcheck.json
    service.root.join("selfcheck.json")
}

/// selfcheck.last() — the previously published report, if any.
pub fn last(service: &HelperService) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(report_file(service)).ok()?).ok()
}

fn publish(service: &HelperService, report: Value) -> Value {
    let file = report_file(service);
    if std::fs::write(&file, json_text(&report)).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
        }
    }
    report
}

/// {checkedAt, running, issues:[{id,text,fixable}], fixed:[str]}
/// Equivalent of SelfCheck.run("manual") — manual runs ignore the
/// `watch` setting gate. Returns {} while another run is in flight.
pub fn run(service: &HelperService) -> Value {
    if PENDING.swap(true, Ordering::SeqCst) {
        return json!({});
    }
    // try/finally — release the flag even if a check panics.
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            PENDING.store(false, Ordering::SeqCst);
        }
    }
    let _reset = Reset;
    check_inner(service)
}

fn check_inner(service: &HelperService) -> Value {
    let result = (|| -> anyhow::Result<Value> {
        let mut catalog = service.tool_catalog();
        let mut env_issues = service.env_scan();
        let mut fixed: Vec<String> = Vec::new();
        if service.has_key() {
            // auto-fix env, like the original
            let env_fixed = service.env_fix(Some(env_issues.clone()))?;
            if !env_fixed.is_empty() {
                fixed.extend(env_fixed.iter().map(|issue| {
                    format!(
                        "已停用 {}（{}）",
                        issue["name"].as_str().unwrap_or_default(),
                        if issue["source"].as_str() == Some("shell") {
                            issue["where"].as_str().unwrap_or_default().to_string()
                        } else {
                            "登录环境".to_string()
                        }
                    )
                }));
                env_issues = service.env_scan();
            }
            // reconnect drifted managed tools silently
            for item in &catalog {
                if item["status"].as_str() != Some("drift")
                    || !item["managed"].as_bool().unwrap_or(false)
                {
                    continue;
                }
                let tool_id = item["id"].as_str().unwrap_or_default();
                let outcome: anyhow::Result<()> = (|| {
                    let record = service.state.get_managed(tool_id).unwrap_or(json!({}));
                    let expected = record.get("expected").cloned().unwrap_or(Value::Null);
                    if !crate::tpapi::truthy(&expected) {
                        return Ok(());
                    }
                    let adapter = service
                        .adapters
                        .get(tool_id)
                        .ok_or_else(|| anyhow::anyhow!("未知工具"))?;
                    let plan = adapter.plan(&service.env)?;
                    service.backups.create(tool_id, &plan.files)?;
                    adapter.apply(&service.env, &expected, &plan)?;
                    fixed.push(format!(
                        "{} 已改回 TokenPlus",
                        item["name"].as_str().unwrap_or_default()
                    ));
                    Ok(())
                })();
                let _ = outcome;
            }
            if !fixed.is_empty() {
                catalog = service.tool_catalog();
            }
        }
        let mut issues: Vec<Value> = catalog
            .iter()
            .filter(|item| {
                item["status"].as_str() == Some("drift")
                    && item["managed"].as_bool().unwrap_or(false)
            })
            .map(|item| {
                json!({
                    "id": format!("tool:{}", item["id"].as_str().unwrap_or_default()),
                    "tool": item["id"],
                    "fixable": true,
                    "text": format!(
                        "{} 的配置被改成了别的服务商",
                        item["name"].as_str().unwrap_or_default()
                    ),
                })
            })
            .collect();
        issues.extend(env_issues.iter().map(|issue| {
            json!({
                "id": issue["id"],
                "text": describe_issue(issue),
                "fixable": issue["fixable"],
            })
        }));
        Ok(json!({
            "checkedAt": now_ms(),
            "running": false,
            "issues": issues,
            "fixed": fixed,
        }))
    })();
    match result {
        Ok(report) => publish(service, report),
        Err(error) => {
            let mut last_report = last(service).unwrap_or_else(|| json!({}));
            last_report["checkedAt"] = json!(now_ms());
            last_report["running"] = json!(false);
            last_report["error"] = json!(if error.to_string().is_empty() {
                "自检没有完成".to_string()
            } else {
                error.to_string()
            });
            publish(service, last_report)
        }
    }
}
