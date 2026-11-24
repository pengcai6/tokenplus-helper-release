//! Filesystem helpers — port of helper/fsutil.py.

use anyhow::{bail, Result};
use serde_json::{Map, Value};
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use crate::types::{FileSnap, Plan};

/// Current unix epoch in milliseconds (time.time() * 1000).
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn read_text_if_exists(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    // lstat semantics: symlink_metadata does not follow the link, so a
    // symlink reports is_file() == false — same as the Python
    // `not isfile or islink` check.
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        bail!("配置不是普通文件，未修改：{}", path.display());
    }
    if meta.permissions().readonly() {
        bail!("配置没有写入权限，未修改：{}", path.display());
    }
    Ok(Some(fs::read_to_string(path)?))
}

pub fn snapshot(path: &Path) -> Result<FileSnap> {
    Ok(FileSnap {
        path: path.to_path_buf(),
        original: read_text_if_exists(path)?,
    })
}

pub fn parse_json_object(text: Option<&str>, path: &Path) -> Result<Map<String, Value>> {
    let text = match text {
        Some(t) if !t.trim().is_empty() => t,
        _ => return Ok(Map::new()),
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let value: Value = serde_json::from_str(text)
        .map_err(|_| anyhow::anyhow!("现有配置无法读取，未覆盖：{}", path.display()))?;
    match value {
        Value::Object(map) => Ok(map),
        _ => bail!("现有配置必须是 JSON 对象，未覆盖：{}", path.display()),
    }
}

pub fn assert_plan_current(plan: &Plan) -> Result<()> {
    for file in &plan.files {
        if read_text_if_exists(&file.path)? != file.original {
            bail!(
                "配置刚刚被其他程序修改，请完全退出目标应用后重试：{}",
                file.path.display()
            );
        }
    }
    Ok(())
}

pub fn atomic_write(path: &Path, content: &str) -> Result<()> {
    atomic_write_bytes(path, content.as_bytes())
}

/// Binary variant of atomic_write (Python accepts str | bytes).
pub fn atomic_write_bytes(path: &Path, content: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".yituo-{}-{}.tmp",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| -> Result<()> {
        {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            file.write_all(content)?;
            file.sync_all()?;
        }
        fs::rename(&temp, path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub fn remove_file(path: &Path) {
    // Python: os.unlink, FileNotFoundError swallowed; other errors would
    // propagate, but this port's signature returns ().
    let _ = fs::remove_file(path);
}

pub fn json_text(value: &Value) -> String {
    // json.dumps(value, indent=2, ensure_ascii=False) + "\n"
    format!("{}\n", serde_json::to_string_pretty(value).unwrap_or_default())
}
