//! Bridge dispatch — port of helper/gui/api.py channel table.
//! The renderer calls `dispatch(channel, args)` (was POST /invoke {channel, args})
//! and listens to channel-named events via `TAURI.event.listen` (was /events SSE).

use anyhow::Result;
use serde_json::{json, Map, Value};
use std::sync::{Mutex, MutexGuard, OnceLock};

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;

use crate::catalog::TOOL_ORDER;
use crate::fsutil::now_ms;
use crate::pets::PetService;
use crate::service::HelperService;
use crate::tpapi::{truthy, PORTAL_ORIGIN};

const VERSION: &str = env!("CARGO_PKG_VERSION"); // 0.1.4

fn guides(tool: &str) -> &'static str {
    match tool {
        "claude-code" => "https://docs.anthropic.com/en/docs/claude-code/setup",
        "claude-desktop" => "https://claude.com/download",
        "codex" => "https://developers.openai.com/codex/cli",
        "grok" => "https://x.ai/",
        "deepseek" => "https://github.com/DeepSeek-ai",
        "workbuddy" => "https://www.workbuddy.ai/",
        _ => PORTAL_ORIGIN,
    }
}

pub struct AppState {
    pub service: Mutex<HelperService>,
    pub diagnostics: Mutex<Vec<Value>>,
    pub connect_running: Mutex<bool>,
}

impl AppState {
    pub fn new() -> Self {
        let root = crate::store::data_root();
        Self {
            service: Mutex::new(HelperService::new(&root)),
            diagnostics: Mutex::new(Vec::new()),
            connect_running: Mutex::new(false),
        }
    }
}

fn lock<'a, T>(mutex: &'a Mutex<T>) -> MutexGuard<'a, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or(Value::Null)
}

fn arg_str(args: &[Value], index: usize) -> String {
    arg(args, index).as_str().unwrap_or_default().to_string()
}

// ---------- views (helpers from api.py) ----------

fn tools_view(service: &HelperService) -> Vec<Value> {
    let managed_map = service.state.managed();
    let mut out = Vec::new();
    for item in service.tool_catalog() {
        let managed = managed_map
            .get(item["id"].as_str().unwrap_or_default())
            .cloned()
            .unwrap_or_else(|| json!({}));
        out.push(json!({
            "id": item["id"],
            "status": item["status"],
            "installed": item["installed"],
            "supported": true,
            "running": false,
            "version": item["version"].as_str().unwrap_or_default(),
            "files": item["files"].as_array().cloned().unwrap_or_default(),
            "model": managed["model"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| item["model"].as_str())
                .unwrap_or_default(),
            "models": item["models"].as_array().cloned().unwrap_or_default(),
            "message": item["message"].as_str().unwrap_or_default(),
            "lastProbe": if managed["lastProbeAt"].is_number() {
                json!({
                    "at": managed["lastProbeAt"],
                    "ok": managed["lastProbeOk"].as_bool().unwrap_or(false),
                    "message": managed["lastProbeMessage"].as_str().unwrap_or_default(),
                    "latency": managed["lastProbeLatency"],
                    "model": managed["lastProbeModel"].as_str().unwrap_or_default(),
                    "firstToken": managed["lastProbeFirstToken"],
                    "reply": managed["lastProbeReply"].as_str().unwrap_or_default(),
                })
            } else {
                Value::Null
            },
        }));
    }
    out
}

fn auth_view(service: &HelperService) -> Value {
    if service.has_key() {
        let key = service.get_key().unwrap_or_default();
        let hint = if key.chars().count() >= 4 {
            let tail: String = key
                .chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("sk-…{tail}")
        } else {
            String::new()
        };
        return json!({
            "phase": "in",
            "key_hint": hint,
            "user": {"username": "tokenplus", "display_name": "TokenPlus 用户"},
        });
    }
    json!({"phase": "out"})
}

/// (account, error). Best effort from portal session.
pub(crate) fn account(service: &HelperService) -> (Option<Value>, String) {
    if !service.has_key() {
        return (None, "未接入密钥".to_string());
    }
    let mut base = json!({
        "user": {"username": "tokenplus", "display_name": "TokenPlus 用户"},
        "balance": {"amount": "—", "currency": "CNY"},
        "today": {"amount": "0.00", "currency": "CNY"},
        "recent": [],
        "daily": [],
        "topup_url": PORTAL_ORIGIN,
        "billing_url": PORTAL_ORIGIN,
    });
    if service.api.has_session_cookies() {
        if let Ok(overview) = service.api.account_overview() {
            let account = &overview["account"];
            let balance = &account["fund"]["balance"];
            let amount = balance
                .as_f64()
                .or_else(|| balance.as_str().and_then(|s| s.parse::<f64>().ok()));
            if let Some(amount) = amount {
                base["balance"] =
                    json!({"amount": format!("{amount:.2}"), "currency": "CNY"});
            }
            let name = account["name"].as_str().unwrap_or_default();
            if !name.is_empty() {
                base["user"]["display_name"] = json!(name);
            }
        }
    }
    (Some(base), String::new())
}

fn push_auth(app: &AppHandle, service: &HelperService) {
    let _ = app.emit("auth:changed", auth_view(service));
}

fn push_tools(app: &AppHandle, service: &HelperService) {
    let _ = app.emit("tools:changed", tools_view(service));
}

// ---------- bootstrap / refresh ----------

/// platform.node() — machine name, "这台电脑" as fallback.
fn device_name() -> String {
    for var in ["HOSTNAME", "COMPUTERNAME"] {
        if let Ok(name) = std::env::var(var) {
            let name = name.trim().to_string();
            if !name.is_empty() {
                return name;
            }
        }
    }
    if let Ok(output) = std::process::Command::new("hostname").output() {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !name.is_empty() {
            return name;
        }
    }
    "这台电脑".to_string()
}

fn bootstrap(service: &HelperService) -> Value {
    let (account, error) = account(service);
    json!({
        "auth": auth_view(service),
        "tools": tools_view(service),
        "account": account,
        "accountError": error,
        "settings": service.settings(),
        "backups": service.backups.list(None),
        "version": VERSION,
        "platform": service.env.platform,
        "deviceName": device_name(),
        "selfCheck": crate::selfcheck::last(service).unwrap_or_else(|| json!({})),
        "models": match service.keys.get_json("models") {
            Some(m) => match m["list"].as_array() {
                Some(list) => json!({"count": list.len(), "savedAt": m["savedAt"]}),
                None => Value::Null,
            },
            None => Value::Null,
        },
    })
}

// ---------- url helpers ----------

fn open_url(url: &str) {
    static HTTP_RE: OnceLock<regex::Regex> = OnceLock::new();
    static SCHEME_RE: OnceLock<regex::Regex> = OnceLock::new();
    let http = HTTP_RE.get_or_init(|| regex::Regex::new(r"^https?://").unwrap());
    let scheme =
        SCHEME_RE.get_or_init(|| regex::Regex::new(r"^[a-z][a-z0-9+.-]*://").unwrap());
    if http.is_match(url) || scheme.is_match(url) {
        let _ = tauri_plugin_opener::open_url(url, None::<&str>);
    }
}

// ---------- tools:connect (api.py connect flow) ----------

fn connect(state: &AppState, app: &AppHandle, tools: &[Value]) -> Result<Value> {
    {
        let mut running = lock(&state.connect_running);
        if *running {
            anyhow::bail!("已有接入任务正在进行，请稍候");
        }
        *running = true;
    }
    struct Reset<'a>(&'a Mutex<bool>);
    impl Drop for Reset<'_> {
        fn drop(&mut self) {
            *lock(self.0) = false;
        }
    }
    let _reset = Reset(&state.connect_running);

    let queue: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        tools
            .iter()
            .filter_map(|t| t.as_str())
            .filter(|t| TOOL_ORDER.contains(t) && seen.insert(t.to_string()))
            .map(String::from)
            .collect()
    };
    if queue.is_empty() {
        return Ok(Value::Null);
    }
    let service = lock(&state.service);
    if !service.has_key() {
        anyhow::bail!("请先接入 TokenPlus 密钥");
    }
    let mut failures: Vec<Value> = Vec::new();
    for (index, tool_id) in queue.iter().enumerate() {
        let last_step = std::cell::Cell::new(-1i64);
        let progress = |step: i64, message: &str| {
            last_step.set(step);
            let _ = app.emit(
                "connect:progress",
                json!({
                    "state": "run", "queue": queue, "index": index,
                    "step": step, "tool": tool_id, "message": message,
                    "failures": failures,
                }),
            );
        };
        let outcome: Result<()> = (|| {
            let adapter = service
                .adapters
                .get(tool_id.as_str())
                .ok_or_else(|| anyhow::anyhow!("未知工具：{tool_id}"))?;
            let detected = adapter.detect(&service.env);
            if !detected.installed {
                anyhow::bail!(if detected.message.is_empty() {
                    "请先安装这个工具".to_string()
                } else {
                    detected.message
                });
            }
            progress(0, "");
            let plan = adapter.plan(&service.env)?;
            progress(1, "");
            service.backups.create(tool_id, &plan.files)?;
            progress(2, "");
            let config = service.config_for(tool_id, "", None);
            progress(3, "");
            if let Err(error) = adapter.apply(&service.env, &config, &plan) {
                for f in &plan.files {
                    if let Some(original) = &f.original {
                        let _ = crate::fsutil::atomic_write(&f.path, original);
                    } else {
                        crate::fsutil::remove_file(&f.path);
                    }
                }
                return Err(error);
            }
            progress(4, "已跳过验证，稍后可点「测试连接」");
            service.state.set_managed(
                tool_id,
                &json!({
                    "managed": true, "expected": config,
                    "model": adapter.current_model(&config),
                    "connectedAt": now_ms(),
                }),
            )?;
            Ok(())
        })();
        if let Err(error) = outcome {
            eprintln!("[dispatch] tools:connect {tool_id} -> {error}");
            let _ = app.emit(
                "connect:progress",
                json!({
                    "state": "run", "queue": queue, "index": index,
                    "step": last_step.get().max(0), "tool": tool_id,
                    "toolState": "fail",
                    "message": error.to_string(),
                    "failures": failures,
                }),
            );
            failures.push(json!({
                "tool": tool_id,
                "message": error.to_string(),
            }));
        }
        push_tools(app, &service);
    }
    let message = failures
        .iter()
        .filter_map(|f| f["message"].as_str().map(String::from))
        .collect::<Vec<_>>()
        .join("；");
    let _ = app.emit(
        "connect:progress",
        json!({
            "state": if failures.len() == queue.len() { "fail" } else { "done" },
            "queue": queue,
            "index": queue.len() - 1,
            "step": 4,
            "tool": queue.last(),
            "message": message,
            "failures": failures,
        }),
    );
    Ok(Value::Null)
}

// ---------- dispatch ----------

fn run(channel: &str, args: &[Value], state: &AppState, app: &AppHandle) -> Result<Value> {
    let result = run_inner(channel, args, state, app);
    if let Err(e) = &result {
        eprintln!("[dispatch] {channel} -> {e}");
    }
    result
}

fn run_inner(channel: &str, args: &[Value], state: &AppState, app: &AppHandle) -> Result<Value> {
    match channel {
        "app:bootstrap" => Ok(bootstrap(&lock(&state.service))),
        "tools:refresh" => {
            let service = lock(&state.service);
            let tools = tools_view(&service);
            let _ = app.emit("tools:changed", tools.clone());
            Ok(json!(tools))
        }
        "tools:plan" => Ok(lock(&state.service).tools_plan()),
        "account:refresh" => {
            let service = lock(&state.service);
            let (account, error) = account(&service);
            if !error.is_empty() {
                anyhow::bail!(error);
            }
            let _ = app.emit("account:changed", account.clone());
            Ok(account.unwrap_or(Value::Null))
        }
        "auth:start" | "auth:reopen" | "auth:password" | "auth:verify-code"
        | "auth:register" => {
            open_url(PORTAL_ORIGIN);
            Ok(json!({"external": true}))
        }
        "auth:cancel" | "auth:slider-challenge" | "auth:slider-verify" => {
            Ok(Value::Null)
        }
        "auth:logout" => {
            let service = lock(&state.service);
            service.logout();
            push_auth(app, &service);
            Ok(Value::Null)
        }
        "auth:import-key" => {
            let service = lock(&state.service);
            let result = service.import_key(&arg_str(args, 0))?;
            if !service.keys.get_json("user").map(|u| truthy(&u)).unwrap_or(false) {
                let _ = service.keys.set_json(
                    "user",
                    &json!({"username": "tokenplus", "display_name": "TokenPlus 用户"}),
                );
            }
            push_auth(app, &service);
            let _ = app.emit("app:bootstrap", bootstrap(&service));
            Ok(result)
        }
        "tools:connect" => {
            let tools = arg(args, 0).as_array().cloned().unwrap_or_default();
            connect(state, app, &tools)
        }
        "tools:model" => {
            let service = lock(&state.service);
            service.set_model(&arg_str(args, 0), &arg_str(args, 1))?;
            push_tools(app, &service);
            Ok(Value::Null)
        }
        "models:refresh" => {
            let service = lock(&state.service);
            let result = service.refresh_models()?;
            push_tools(app, &service);
            Ok(result)
        }
        "tools:test" => {
            let tool = arg_str(args, 0);
            let run_id = arg_str(args, 1);
            // Resolve under the lock, then drop it — the probe does network I/O.
            let (api, expected, model) = {
                let service = lock(&state.service);
                let adapter = service
                    .adapters
                    .get(tool.as_str())
                    .ok_or_else(|| anyhow::anyhow!("未知工具：{tool}"))?;
                let record = service.state.get_managed(&tool).unwrap_or_else(|| json!({}));
                let expected = record.get("expected").cloned().unwrap_or(Value::Null);
                if !truthy(&expected) {
                    anyhow::bail!("请先接入这个工具");
                }
                let model = adapter.current_model(&expected);
                (service.api.clone(), expected, model)
            };
            let emit = |payload: Value| {
                let mut p = payload;
                p["tool"] = json!(tool);
                p["runId"] = json!(run_id);
                let _ = app.emit("tools:test-chunk", p);
            };
            emit(json!({"state": "start", "model": model}));
            let result = api.probe_stream(&tool, &expected, &mut |delta| {
                emit(json!({"delta": delta}));
            });
            emit(json!({
                "state": "end", "ok": result.ok, "message": result.message,
                "latency": result.latency, "firstToken": result.first_token,
                "model": result.model,
            }));
            {
                let service = lock(&state.service);
                // re-read — the record may have changed while the lock was dropped
                let mut record = service.state.get_managed(&tool).unwrap_or_else(|| json!({}));
                record["lastProbeAt"] = json!(now_ms());
                record["lastProbeOk"] = json!(result.ok);
                record["lastProbeMessage"] = json!(result.message);
                record["lastProbeLatency"] = json!(result.latency);
                record["lastProbeModel"] = json!(result.model);
                record["lastProbeFirstToken"] = json!(result.first_token);
                record["lastProbeReply"] =
                    json!(result.reply.chars().take(200).collect::<String>());
                let _ = service.state.set_managed(&tool, &record);
                push_tools(app, &service);
            }
            if !result.ok {
                anyhow::bail!(if result.message.is_empty() {
                    "连接失败".to_string()
                } else {
                    result.message
                });
            }
            Ok(json!({
                "ok": true,
                "message": result.message,
                "latency": result.latency,
                "firstToken": result.first_token,
                "model": result.model,
                "reply": result.reply,
            }))
        }
        "selfcheck:run" => {
            let service = lock(&state.service);
            let report = crate::selfcheck::run(&service);
            let _ = app.emit("selfcheck:changed", report.clone());
            Ok(report)
        }
        "selfcheck:reset" => {
            let service = lock(&state.service);
            service.env_fix(None)?;
            let _ = app.emit(
                "selfcheck:changed",
                crate::selfcheck::last(&service).unwrap_or_else(|| json!({})),
            );
            let managed = service.state.managed();
            let ids: Vec<Value> = service
                .tool_catalog()
                .iter()
                .filter(|t| {
                    t["installed"].as_bool().unwrap_or(false)
                        && t["id"]
                            .as_str()
                            .map(|id| truthy(&managed.get(id).cloned().unwrap_or(json!({}))))
                            .unwrap_or(false)
                })
                .map(|t| t["id"].clone())
                .collect();
            Ok(json!(ids))
        }
        "tools:open-folder" => {
            let service = lock(&state.service);
            let tool = arg_str(args, 0);
            let item = service
                .tool_catalog()
                .into_iter()
                .find(|t| t["id"].as_str() == Some(tool.as_str()));
            let files = item
                .as_ref()
                .and_then(|i| i["files"].as_array().cloned())
                .unwrap_or_default();
            let first = files
                .first()
                .and_then(Value::as_str)
                .unwrap_or_default();
            if item.is_none() || first.is_empty() {
                anyhow::bail!("当前平台没有可打开的配置位置");
            }
            let home = service.env.home.to_string_lossy().to_string();
            let path = first.replacen('~', &home, 1);
            let target = if std::path::Path::new(&path).exists() {
                std::path::PathBuf::from(path)
            } else {
                std::path::Path::new(&path)
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| std::path::PathBuf::from(path))
            };
            drop(service);
            let _ = tauri_plugin_opener::reveal_item_in_dir(&target);
            Ok(Value::Null)
        }
        "tools:install-guide" => {
            open_url(guides(&arg_str(args, 0)));
            Ok(Value::Null)
        }
        "pets:list" => {
            let home = lock(&state.service).env.home.clone();
            Ok(PetService::new(home).list())
        }
        "pets:use" => {
            let home = lock(&state.service).env.home.clone();
            PetService::new(home).use_pet(&arg_str(args, 0))
        }
        "pets:preview" => {
            let home = lock(&state.service).env.home.clone();
            PetService::new(home).preview(&arg_str(args, 0))
        }
        "pets:download" => {
            let home = lock(&state.service).env.home.clone();
            let pet_id = arg_str(args, 0);
            let target = app
                .dialog()
                .file()
                .set_file_name(format!("{pet_id}.codex-pet.zip"))
                .add_filter("Codex 宠物包", &["zip"])
                .blocking_save_file();
            match target {
                Some(path) => {
                    PetService::new(home).download_zip(&pet_id, &path.to_string())?;
                    Ok(json!(true))
                }
                None => Ok(json!(false)),
            }
        }
        "pets:import" => {
            let home = lock(&state.service).env.home.clone();
            let picked = app
                .dialog()
                .file()
                .set_title("导入并启用宠物")
                .add_filter("Codex 宠物包", &["zip"])
                .blocking_pick_file();
            match picked {
                Some(path) => PetService::new(home).import_zip(&path.to_string()),
                None => Ok(Value::Null),
            }
        }
        "pets:community" => {
            open_url("https://codex-pet.org/zh/collections/ai/");
            // Python returns webbrowser.open()'s result (True)
            Ok(json!(true))
        }
        "pets:codex-settings" => {
            open_url("codex://settings");
            Ok(Value::Null)
        }
        "shell:open" => {
            let url = match arg_str(args, 0).as_str() {
                "topup" => PORTAL_ORIGIN.to_string(),
                "billing" => PORTAL_ORIGIN.to_string(),
                "download" => format!("{PORTAL_ORIGIN}/docs/chat-completions"),
                "tokens" => format!("{PORTAL_ORIGIN}/workspace"),
                _ => PORTAL_ORIGIN.to_string(),
            };
            open_url(&url);
            Ok(Value::Null)
        }
        "settings:update" => {
            let service = lock(&state.service);
            let allowed = ["boot", "tray", "watch", "update"];
            let mut clean = Map::new();
            let settings_arg = arg(args, 0);
            if let Some(obj) = settings_arg.as_object() {
                for (key, value) in obj {
                    if allowed.contains(&key.as_str()) {
                        clean.insert(key.clone(), json!(truthy(value)));
                    }
                }
            }
            let result = service.set_settings(&Value::Object(clean))?;
            let _ = app.emit("app:bootstrap", json!({"settings": result}));
            Ok(Value::Null)
        }
        "diagnostics:run" => {
            let service = lock(&state.service);
            let items = crate::diagnostics::run(
                |items| {
                    *lock(&state.diagnostics) = items.clone();
                    let _ = app.emit("diagnostics:changed", items);
                },
                &service,
            );
            Ok(json!(items))
        }
        "diagnostics:copy" => {
            let items = lock(&state.diagnostics).clone();
            let report = crate::diagnostics::report(&items);
            app.clipboard()
                .write_text(report)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(Value::Null)
        }
        "updates:check" => Ok(json!({
            "available": false,
            "version": VERSION,
            "message": format!("已是最新版本 {VERSION}"),
        })),
        "updates:install" => anyhow::bail!("没有可安装的更新"),
        "backups:list" => Ok(json!(lock(&state.service).backups.list(None))),
        "backups:restore" => {
            let service = lock(&state.service);
            service.restore(&arg_str(args, 0), &arg_str(args, 1))?;
            push_tools(app, &service);
            Ok(json!({"ok": true}))
        }
        "window:control" => {
            let action = arg_str(args, 0);
            let window = app
                .get_webview_window("main")
                .ok_or_else(|| anyhow::anyhow!("窗口不存在"))?;
            match action.as_str() {
                "minimize" => window.minimize()?,
                "maximize" => {
                    if window.is_maximized()? {
                        window.unmaximize()?
                    } else {
                        window.maximize()?
                    }
                }
                "close" => window.close()?,
                _ => anyhow::bail!("无效的窗口操作"),
            }
            Ok(Value::Null)
        }
        _ => anyhow::bail!("未知的调用：{channel}"),
    }
}

#[tauri::command]
pub async fn dispatch(
    channel: String,
    args: Vec<Value>,
    app: AppHandle,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        run(&channel, &args, &state, &app).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
