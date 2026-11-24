//! TokenPlus API client — port of helper/tpapi.py.

use anyhow::Result;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::types::Probe;

pub const GATEWAY_BASE: &str = "https://api.tokenplus.cn/v1";
pub const PORTAL_ORIGIN: &str = "https://www.tokenplus.cn";
pub const PORTAL_API: &str = "https://www.tokenplus.cn/portal/api/v1";
const PORTAL_PREFIX: &str = "/portal/api/v1/";
const MARKET_PREFIX: &str = "/api/v1/";

/// Port of tpapi.ApiError — carries HTTP status + portal error code.
#[derive(Debug)]
pub struct ApiError {
    pub message: String,
    pub status: u32,
    pub code: String,
}
impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for ApiError {}

pub fn api_error(message: impl Into<String>) -> anyhow::Error {
    ApiError {
        message: message.into(),
        status: 0,
        code: String::new(),
    }
    .into()
}

/// HTTP status of an error raised by this client (probe needs 401/402).
pub fn error_status(error: &anyhow::Error) -> Option<u32> {
    error.downcast_ref::<ApiError>().map(|e| e.status)
}

/// (message, code) pulled from a gateway error body like {"error": {...}}.
fn extract_error(result: &Value, status: u32) -> (String, String) {
    let mut message = String::new();
    let mut code = String::new();
    if let Some(error) = result.get("error").and_then(Value::as_object) {
        message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        code = error
            .get("code")
            .map(|c| match c {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default();
    }
    if message.is_empty() {
        message = result["message"].as_str().unwrap_or_default().to_string();
    }
    if message.is_empty() {
        message = format!("请求失败（HTTP {status}）");
    }
    (message, code)
}

/// Same user-facing mapping probe() applies to gateway failures.
fn map_probe_error(error: &anyhow::Error) -> String {
    match error_status(error) {
        Some(401) => "专用密钥已失效，请重新接入".to_string(),
        Some(402) => "余额不足，请充值后重试".to_string(),
        _ => {
            if error.is::<ApiError>() {
                error.to_string()
            } else {
                "连接失败，请检查网络".to_string()
            }
        }
    }
}

fn join(base: &str, suffix: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), suffix)
}

/// Python truthiness for JSON values (None/False/0/""/[]/{} are falsy).
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// urllib.parse.quote(path segment, safe="/").
fn quote(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'_' | b'.' | b'-' | b'~' | b'/' => out.push(byte as char),
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}

pub fn clean_model(s: &str) -> Result<String> {
    if s.is_empty() || s.chars().count() > 200 || s.contains(['\r', '\n', '"', '\\']) {
        anyhow::bail!("模型名称无效");
    }
    Ok(s.to_string())
}

pub fn required_key(config: &Value) -> Result<String> {
    static KEY_RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = KEY_RE.get_or_init(|| regex::Regex::new(r"^sk-[A-Za-z0-9_-]+$").unwrap());
    let key = config["api_key"].as_str().unwrap_or_default();
    if !re.is_match(key) {
        anyhow::bail!("服务器返回的专用密钥无效，已停止接入");
    }
    Ok(key.to_string())
}

pub fn required_base(config: &Value) -> Result<String> {
    required_base_inner(config, false)
}

/// required_base(config, gateway=True) — reads gateway_base_url.
pub fn required_base_gateway(config: &Value) -> Result<String> {
    required_base_inner(config, true)
}

fn required_base_inner(config: &Value, gateway: bool) -> Result<String> {
    let field = if gateway { "gateway_base_url" } else { "base_url" };
    let value = config[field].as_str().unwrap_or_default();
    if value.is_empty() {
        anyhow::bail!("线路地址不完整");
    }
    assert_service_url(value, &["", "/", "/v1"])
}

/// Port of tpapi.assert_service_url — only https://*.tokenplus.cn.
pub fn assert_service_url(value: &str, allowed_paths: &[&str]) -> Result<String> {
    let rest = match value.split_once("://") {
        Some(("https", rest)) => rest,
        _ => anyhow::bail!("线路地址不受信任，已停止接入"),
    };
    let (authority, tail) = match rest.find(|c| c == '/' || c == '?' || c == '#') {
        Some(index) => rest.split_at(index),
        None => (rest, ""),
    };
    if authority.contains('@') {
        // url.username / url.password must be empty
        anyhow::bail!("线路地址不受信任，已停止接入");
    }
    let host = authority
        .split_once(':')
        .map(|(host, _)| host)
        .unwrap_or(authority)
        .to_lowercase();
    let (path, query_fragment) = match tail.find(|c| c == '?' || c == '#') {
        Some(index) => tail.split_at(index),
        None => (tail, ""),
    };
    let bad_query = query_fragment
        .trim_start_matches(['?', '#'])
        .split('#')
        .any(|part| !part.is_empty());
    if !(host == "tokenplus.cn" || host.ends_with(".tokenplus.cn"))
        || bad_query
        || !allowed_paths.contains(&path)
    {
        anyhow::bail!("线路地址不受信任，已停止接入");
    }
    Ok(value.trim_end_matches('/').to_string())
}

pub struct TokenPlusClient {
    /// gateway client — allow_redirects=False like Python.
    gateway: reqwest::blocking::Client,
    /// portal/market client — follows redirects like the default session.
    portal: reqwest::blocking::Client,
    cookies: Mutex<HashMap<String, String>>,
    csrf: Mutex<String>,
}
impl TokenPlusClient {
    pub fn new() -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::ACCEPT, "application/json".parse().unwrap());
        headers.insert(reqwest::header::ACCEPT_LANGUAGE, "zh-CN".parse().unwrap());
        let gateway = reqwest::blocking::Client::builder()
            .default_headers(headers.clone())
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        let portal = reqwest::blocking::Client::builder()
            .default_headers(headers)
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        Self {
            gateway,
            portal,
            cookies: Mutex::new(HashMap::new()),
            csrf: Mutex::new(String::new()),
        }
    }

    fn lock_cookies(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.cookies.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_csrf(&self) -> std::sync::MutexGuard<'_, String> {
        self.csrf.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---------- gateway (sk- key) ----------

    pub fn gateway_request(
        &self,
        path: &str,
        key: &str,
        method: &str,
        body: Option<&Value>,
        timeout_secs: u64,
        headers: &[(&str, &str)],
    ) -> Result<Value> {
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            join(GATEWAY_BASE, path)
        };
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .unwrap_or(reqwest::Method::GET);
        let mut request = self
            .gateway
            .request(method, &url)
            .timeout(Duration::from_secs(timeout_secs));
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        if body.is_some() {
            request = request.header(reqwest::header::CONTENT_TYPE, "application/json");
        }
        let has_api_key_header = headers.iter().any(|(n, _)| n.eq_ignore_ascii_case("x-api-key"));
        if !key.is_empty() && !has_api_key_header {
            request = request.header(reqwest::header::AUTHORIZATION, format!("Bearer {key}"));
        }
        if let Some(body) = body {
            request = request.body(serde_json::to_string(body)?);
        }
        let response = request
            .send()
            .map_err(|_| api_error("连不上 TokenPlus 服务器，请检查网络后重试"))?;
        let status = response.status().as_u16() as u32;
        let result: Value = response.json().map_err(|_| ApiError {
            message: "服务器返回了无法读取的结果".to_string(),
            status,
            code: String::new(),
        })?;
        if status >= 400 {
            let (message, code) = extract_error(&result, status);
            return Err(ApiError {
                message,
                status,
                code,
            }
            .into());
        }
        Ok(result)
    }

    /// Throws when the key cannot list models.
    pub fn validate_key(&self, key: &str) -> Result<()> {
        self.gateway_request("/models", key, "GET", None, 15, &[])?;
        Ok(())
    }

    /// Model ids usable for tool configs, from GET /v1/models.
    pub fn list_models(&self, key: &str) -> Result<Vec<String>> {
        let result = self.gateway_request("/models", key, "GET", None, 20, &[])?;
        if let Some(items) = result.get("data").and_then(Value::as_array) {
            let ids: Vec<String> = items
                .iter()
                .filter_map(|item| item["id"].as_str().map(String::from))
                .collect();
            if !ids.is_empty() {
                return Ok(ids);
            }
        }
        Err(api_error("没有读取到可用模型列表"))
    }

    /// Public catalog slugs shown on tokenplus.cn/models (no key needed).
    pub fn marketplace_models(&self) -> Vec<String> {
        let result = (|| -> Result<Value> {
            Ok(self
                .portal
                .get(format!("{PORTAL_ORIGIN}{MARKET_PREFIX}models"))
                .query(&[("page_size", "200")])
                .timeout(Duration::from_secs(15))
                .send()?
                .json()?)
        })();
        result
            .ok()
            .and_then(|v| v["data"]["items"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|item| item["model_slug"].as_str().map(String::from))
            .collect()
    }

    // ---------- portal (web session cookie) ----------

    pub fn set_cookies(&self, cookies: &Map<String, Value>) {
        let mut jar = self.lock_cookies();
        jar.clear();
        for (name, value) in cookies {
            if let Some(text) = value.as_str() {
                jar.insert(name.clone(), text.to_string());
            }
        }
        drop(jar);
        self.lock_csrf().clear();
    }

    pub fn has_session_cookies(&self) -> bool {
        !self.lock_cookies().is_empty()
    }

    fn portal_request(
        &self,
        path: &str,
        method: &str,
        body: Option<&Value>,
        timeout_secs: u64,
        retry: bool,
    ) -> Result<Value> {
        let path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        let url = join(PORTAL_ORIGIN, &path);
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .unwrap_or(reqwest::Method::GET);
        if method != reqwest::Method::GET && self.lock_csrf().is_empty() {
            self.session()?;
        }
        let mut request = self
            .portal
            .request(method.clone(), &url)
            .timeout(Duration::from_secs(timeout_secs))
            .header(reqwest::header::ACCEPT, "application/json");
        {
            let cookie = self
                .lock_cookies()
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            if !cookie.is_empty() {
                request = request.header(reqwest::header::COOKIE, cookie);
            }
        }
        if body.is_some() {
            request = request.header(reqwest::header::CONTENT_TYPE, "application/json");
        }
        if method != reqwest::Method::GET {
            request = request.header("x-csrf-token", self.lock_csrf().clone());
        }
        if let Some(body) = body {
            request = request.body(serde_json::to_string(body)?);
        }
        let response = request
            .send()
            .map_err(|_| api_error("连不上 TokenPlus 控制台，请检查网络后重试"))?;
        let status = response.status().as_u16() as u32;
        let result: Value = response.json().map_err(|_| ApiError {
            message: "控制台返回了无法读取的结果".to_string(),
            status,
            code: String::new(),
        })?;
        if retry
            && status == 403
            && result["error"]["code"].as_str() == Some("csrf_invalid")
        {
            self.lock_csrf().clear();
            self.session()?;
            return self.portal_request(&path, method.as_str(), body, timeout_secs, false);
        }
        if status >= 400 || result["status"].as_str() == Some("error") {
            let error = &result["error"];
            let code = error["code"].as_str().unwrap_or_default().to_string();
            let message = error["message"]
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| format!("请求失败（HTTP {status}）"));
            return Err(ApiError {
                message,
                status,
                code,
            }
            .into());
        }
        Ok(match result.get("data") {
            Some(data) => data.clone(),
            None => result,
        })
    }

    /// GET auth/session: csrf token + user + current account.
    pub fn session(&self) -> Result<Value> {
        let data = self.portal_request(
            &format!("{PORTAL_PREFIX}auth/session"),
            "GET",
            None,
            15,
            true,
        )?;
        let token = data["csrf_token"].as_str().unwrap_or_default().trim();
        if token.is_empty() {
            return Err(api_error("未获取到登录信息，请先在网页里登录 TokenPlus"));
        }
        *self.lock_csrf() = token.to_string();
        Ok(data)
    }

    pub fn logged_in(&self, session: &Value) -> bool {
        truthy(&session["user"]) || truthy(&session["current_account"])
    }

    pub fn account_overview(&self) -> Result<Value> {
        self.portal_request(
            &format!("{PORTAL_PREFIX}accounts/current/overview"),
            "GET",
            None,
            15,
            true,
        )
    }

    pub fn list_projects(&self, account_id: &str) -> Result<Vec<Value>> {
        let data = self.portal_request(
            &format!(
                "{PORTAL_PREFIX}accounts/{}/projects?joined_only=true&page=1&page_size=100",
                quote(account_id)
            ),
            "GET",
            None,
            15,
            true,
        )?;
        Ok(data["items"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|item| truthy(&item["is_enabled"]))
            .collect())
    }

    /// POST .../api-keys — same call the dashboard's 新建 API 密钥 makes.
    pub fn create_api_key(
        &self,
        account_id: &str,
        project_id: &str,
        name: &str,
    ) -> Result<Value> {
        let body = json!({
            "name": name,
            "key_kind": "project",
            "budget": {"enabled": false},
        });
        self.portal_request(
            &format!(
                "{PORTAL_PREFIX}accounts/{}/projects/{}/api-keys",
                quote(account_id),
                quote(project_id)
            ),
            "POST",
            Some(&body),
            30,
            true,
        )
    }

    pub fn fund(&self, account_id: &str) -> Result<Value> {
        self.portal_request(
            &format!("{PORTAL_PREFIX}accounts/{}/fund", quote(account_id)),
            "GET",
            None,
            15,
            true,
        )
    }

    // ---------- connectivity probe ----------

    /// One cheap request against the gateway, like src/main/probe.ts.
    pub fn probe(&self, tool: &str, config: &Value) -> Result<Probe> {
        let started = Instant::now();
        let outcome = self.probe_inner(tool, config);
        let latency = (started.elapsed().as_secs_f64() * 1000.0).trunc();
        match outcome {
            Ok(_) => Ok(Probe {
                ok: true,
                latency,
                message: "连接正常".to_string(),
            }),
            Err(error) => Ok(Probe {
                ok: false,
                latency,
                message: map_probe_error(&error),
            }),
        }
    }

    fn probe_inner(&self, tool: &str, config: &Value) -> Result<()> {
        let target = self.probe_target(tool, config, "ping", 16, false);
        let headers: Vec<(&str, &str)> = target
            .headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        self.gateway_request(
            &target.url,
            &target.key,
            "POST",
            Some(&target.body),
            90,
            &headers,
        )?;
        Ok(())
    }

    /// Builds the probe request for `tool` — endpoint, key, headers, body.
    fn probe_target(
        &self,
        tool: &str,
        config: &Value,
        prompt: &str,
        max_tokens: u32,
        stream: bool,
    ) -> ProbeTarget {
        if tool == "claude-code" || tool == "claude-desktop" {
            let base = config["base_url"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| config["gateway_base_url"].as_str())
                .unwrap_or_default()
                .trim_end_matches('/');
            let model = config["model"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(String::from)
                .unwrap_or_else(|| claude_default(config));
            let mut body = json!({
                "model": model,
                "max_tokens": max_tokens,
                "messages": [{"role": "user", "content": prompt}],
            });
            if stream {
                body["stream"] = json!(true);
            }
            return ProbeTarget {
                url: join(base, "/v1/messages"),
                key: String::new(),
                headers: vec![
                    ("x-api-key".to_string(), config["api_key"].as_str().unwrap_or_default().to_string()),
                    ("anthropic-version".to_string(), "2023-06-01".to_string()),
                ],
                body,
                kind: ProbeKind::Anthropic,
                model,
            };
        }
        let provider = selected_provider(config);
        let empty = json!({});
        let provider = provider.unwrap_or(&empty);
        let base = provider["base_url"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| config["base_url"].as_str())
            .unwrap_or_default()
            .trim_end_matches('/');
        let model = provider["model"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| config["model"].as_str().filter(|s| !s.is_empty()))
            .map(String::from)
            .unwrap_or_else(|| claude_default(config));
        let key = provider["api_key"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| config["api_key"].as_str())
            .unwrap_or_default()
            .to_string();
        let responses = tool == "codex"
            || tool == "grok"
            || (tool == "workbuddy" && provider["api"].as_str() == Some("openai-responses"));
        let mut body = if responses {
            json!({"model": model, "max_output_tokens": max_tokens, "input": prompt})
        } else {
            json!({
                "model": model,
                "max_tokens": max_tokens,
                "messages": [{"role": "user", "content": prompt}],
            })
        };
        if stream {
            body["stream"] = json!(true);
        }
        ProbeTarget {
            url: join(base, if responses { "/responses" } else { "/chat/completions" }),
            key,
            headers: vec![],
            body,
            kind: if responses { ProbeKind::Responses } else { ProbeKind::Chat },
            model,
        }
    }

    /// Streaming "hi" probe — never Err; failures live in the returned struct.
    pub fn probe_stream(
        &self,
        tool: &str,
        config: &Value,
        on_delta: &mut dyn FnMut(&str),
    ) -> StreamProbe {
        let started = Instant::now();
        let elapsed = || (started.elapsed().as_secs_f64() * 1000.0).trunc();
        let target = self.probe_target(tool, config, "hi", 256, true);
        let mut probe = StreamProbe {
            ok: false,
            message: String::new(),
            latency: 0.0,
            first_token: None,
            reply: String::new(),
            model: target.model.clone(),
        };
        let mut request = self
            .gateway
            .post(&target.url)
            .timeout(Duration::from_secs(60))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, "text/event-stream");
        for (name, value) in &target.headers {
            request = request.header(name, value);
        }
        let has_api_key_header = target
            .headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("x-api-key"));
        if !target.key.is_empty() && !has_api_key_header {
            request = request.header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", target.key),
            );
        }
        request = request.body(serde_json::to_string(&target.body).unwrap_or_default());
        let response = match request.send() {
            Ok(r) => r,
            Err(_) => {
                probe.latency = elapsed();
                probe.message = "连接失败，请检查网络".to_string();
                return probe;
            }
        };
        let status = response.status().as_u16() as u32;
        if status >= 400 {
            let result: Value = response.json().unwrap_or_else(|_| json!({}));
            let (message, code) = extract_error(&result, status);
            let error: anyhow::Error = ApiError { message, status, code }.into();
            probe.latency = elapsed();
            probe.message = map_probe_error(&error);
            return probe;
        }
        let is_sse = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.contains("text/event-stream"))
            .unwrap_or(false);
        if !is_sse {
            // Gateway ignored stream:true — pull the full text from the JSON body.
            let result: Value = response.json().unwrap_or_else(|_| json!({}));
            let text = full_reply_text(target.kind, &result);
            if !text.is_empty() {
                probe.reply = text.clone();
                on_delta(&text);
            }
        } else {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(response);
            'lines: for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => break,
                };
                let data = match line.strip_prefix("data:") {
                    Some(d) => d.trim_start_matches(' ').trim(),
                    None => continue,
                };
                if data.is_empty() {
                    continue;
                }
                if data == "[DONE]" {
                    break;
                }
                let value: Value = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                match sse_delta(target.kind, &value) {
                    SseEv::Text(text) => {
                        if probe.first_token.is_none() {
                            probe.first_token = Some(elapsed());
                        }
                        probe.reply.push_str(&text);
                        on_delta(&text);
                    }
                    SseEv::Error(message) => {
                        probe.latency = elapsed();
                        probe.message = message;
                        break 'lines;
                    }
                    SseEv::Skip => {}
                }
            }
        }
        probe.latency = elapsed();
        if probe.message.is_empty() {
            probe.ok = true;
            if !is_sse && probe.first_token.is_none() {
                probe.first_token = Some(probe.latency);
            }
            probe.message = if probe.reply.is_empty() {
                "连接正常（模型没有返回文本）".to_string()
            } else {
                "连接正常".to_string()
            };
        }
        probe
    }
}

/// Which API dialect a probe target speaks (drives SSE/JSON decoding).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProbeKind {
    Anthropic,
    Chat,
    Responses,
}

struct ProbeTarget {
    url: String,
    key: String,
    headers: Vec<(String, String)>,
    body: Value,
    kind: ProbeKind,
    model: String,
}

/// probe_stream() outcome — ok/message/latency plus stream metrics.
pub struct StreamProbe {
    pub ok: bool,
    pub message: String,
    pub latency: f64,
    pub first_token: Option<f64>,
    pub reply: String,
    pub model: String,
}

enum SseEv {
    Text(String),
    Error(String),
    Skip,
}

fn text_or_skip(s: Option<&str>) -> SseEv {
    match s {
        Some(t) if !t.is_empty() => SseEv::Text(t.to_string()),
        _ => SseEv::Skip,
    }
}

/// One `data:` JSON payload -> text delta, stream error, or skip.
fn sse_delta(kind: ProbeKind, v: &Value) -> SseEv {
    match kind {
        ProbeKind::Anthropic => {
            let ev_type = v["type"].as_str().unwrap_or_default();
            if ev_type == "content_block_delta"
                && v["delta"]["type"].as_str() == Some("text_delta")
            {
                return text_or_skip(v["delta"]["text"].as_str());
            }
            if ev_type == "error" {
                let message = v["error"]["message"]
                    .as_str()
                    .or_else(|| v["message"].as_str())
                    .unwrap_or("模型返回了错误");
                return SseEv::Error(message.to_string());
            }
            SseEv::Skip
        }
        ProbeKind::Chat => {
            if let Some(message) = v["error"]["message"].as_str() {
                return SseEv::Error(message.to_string());
            }
            match v["choices"].as_array().and_then(|c| c.first()) {
                Some(choice) => text_or_skip(choice["delta"]["content"].as_str()),
                None => SseEv::Skip,
            }
        }
        ProbeKind::Responses => match v["type"].as_str().unwrap_or_default() {
            "response.output_text.delta" => text_or_skip(v["delta"].as_str()),
            "error" => {
                let message = v["message"]
                    .as_str()
                    .or_else(|| v["error"]["message"].as_str())
                    .unwrap_or("模型返回了错误");
                SseEv::Error(message.to_string())
            }
            "response.failed" => {
                let message = v["response"]["error"]["message"]
                    .as_str()
                    .unwrap_or("模型返回了错误");
                SseEv::Error(message.to_string())
            }
            _ => SseEv::Skip,
        },
    }
}

/// Full reply text from a non-streaming JSON body (same shape per dialect).
fn full_reply_text(kind: ProbeKind, v: &Value) -> String {
    match kind {
        ProbeKind::Anthropic => v["content"]
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| p["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default(),
        ProbeKind::Chat => v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        ProbeKind::Responses => {
            let direct = v["output_text"].as_str().unwrap_or_default();
            if !direct.is_empty() {
                return direct.to_string();
            }
            v["output"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .flat_map(|item| {
                            item["content"]
                                .as_array()
                                .cloned()
                                .unwrap_or_default()
                        })
                        .filter_map(|p| p["text"].as_str().map(String::from))
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default()
        }
    }
}

fn claude_default(config: &Value) -> String {
    if let Some(default) = config["default"].as_str() {
        return default.to_string();
    }
    config["models"]["default"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn selected_provider(config: &Value) -> Option<&Value> {
    let providers = config["providers"].as_array()?;
    for provider in providers {
        if provider["provider_id"].as_str() == config["default_provider_id"].as_str() {
            return Some(provider);
        }
    }
    providers.first()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(ev: SseEv) -> Option<String> {
        match ev {
            SseEv::Text(t) => Some(t),
            _ => None,
        }
    }

    #[test]
    fn sse_anthropic_text_delta() {
        let v = json!({"type": "content_block_delta", "delta": {"type": "text_delta", "text": "Hel"}});
        assert_eq!(text_of(sse_delta(ProbeKind::Anthropic, &v)), Some("Hel".into()));
    }

    #[test]
    fn sse_anthropic_skips_non_text_events() {
        for v in [
            json!({"type": "message_start", "message": {}}),
            json!({"type": "ping"}),
            json!({"type": "content_block_delta", "delta": {"type": "input_json_delta", "partial_json": "{}"}}),
        ] {
            assert!(matches!(sse_delta(ProbeKind::Anthropic, &v), SseEv::Skip));
        }
    }

    #[test]
    fn sse_anthropic_error_event() {
        let v = json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}});
        assert!(matches!(sse_delta(ProbeKind::Anthropic, &v), SseEv::Error(m) if m == "Overloaded"));
    }

    #[test]
    fn sse_chat_text_and_skip() {
        let v = json!({"choices": [{"delta": {"content": "wor"}}]});
        assert_eq!(text_of(sse_delta(ProbeKind::Chat, &v)), Some("wor".into()));
        // usage chunk: choices present but empty
        let usage = json!({"choices": [], "usage": {"total_tokens": 9}});
        assert!(matches!(sse_delta(ProbeKind::Chat, &usage), SseEv::Skip));
        // reasoning-only delta: content empty/missing
        let reasoning = json!({"choices": [{"delta": {"reasoning_content": "hmm"}}]});
        assert!(matches!(sse_delta(ProbeKind::Chat, &reasoning), SseEv::Skip));
        let empty = json!({"choices": [{"delta": {"content": ""}}]});
        assert!(matches!(sse_delta(ProbeKind::Chat, &empty), SseEv::Skip));
    }

    #[test]
    fn sse_chat_error() {
        let v = json!({"error": {"message": "invalid key"}});
        assert!(matches!(sse_delta(ProbeKind::Chat, &v), SseEv::Error(m) if m == "invalid key"));
    }

    #[test]
    fn sse_responses_delta_and_skip() {
        let v = json!({"type": "response.output_text.delta", "delta": "Hi"});
        assert_eq!(text_of(sse_delta(ProbeKind::Responses, &v)), Some("Hi".into()));
        let created = json!({"type": "response.created", "response": {}});
        assert!(matches!(sse_delta(ProbeKind::Responses, &created), SseEv::Skip));
    }

    #[test]
    fn sse_responses_errors() {
        let v = json!({"type": "error", "message": "rate limited"});
        assert!(matches!(sse_delta(ProbeKind::Responses, &v), SseEv::Error(m) if m == "rate limited"));
        let v = json!({"type": "error", "error": {"message": "nested"}});
        assert!(matches!(sse_delta(ProbeKind::Responses, &v), SseEv::Error(m) if m == "nested"));
        let v = json!({"type": "response.failed", "response": {"error": {"message": "boom"}}});
        assert!(matches!(sse_delta(ProbeKind::Responses, &v), SseEv::Error(m) if m == "boom"));
    }
}
