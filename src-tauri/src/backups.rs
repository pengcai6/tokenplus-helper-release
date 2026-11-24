//! Backup store — port of helper/backups.py.

use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::fsutil::{atomic_write, json_text, now_ms, remove_file};
use crate::types::FileSnap;

pub struct BackupService {
    /// <data_root>/backups — exposed like Python's `backups.root`.
    pub dir: PathBuf,
}
impl BackupService {
    pub fn new(root: &Path) -> Self {
        let dir = root.join("backups");
        let _ = fs::create_dir_all(&dir);
        Self { dir }
    }

    pub fn create(&self, tool: &str, files: &[FileSnap]) -> Result<Value> {
        self.create_inner(tool, files, false)
    }

    /// Python signature has `original: bool = False` (kept for parity).
    pub fn create_inner(&self, tool: &str, files: &[FileSnap], original: bool) -> Result<Value> {
        let backup_id = format!("{}-{}", now_ms(), uuid::Uuid::new_v4());
        let directory = self.dir.join(tool).join(&backup_id);
        fs::create_dir_all(&directory)?;
        let manifest = json!({
            "id": backup_id,
            "tool": tool,
            "createdAt": now_ms(),
            "original": original,
            "files": files
                .iter()
                .enumerate()
                .map(|(index, file)| json!({
                    "path": file.path.to_string_lossy(),
                    "name": format!(
                        "{}-{}",
                        index,
                        file.path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    ),
                    "existed": file.original.is_some(),
                }))
                .collect::<Vec<_>>(),
        });
        let write_result = (|| -> Result<()> {
            for (index, file) in files.iter().enumerate() {
                if let Some(original) = &file.original {
                    let target = directory.join(
                        manifest["files"][index]["name"]
                            .as_str()
                            .unwrap_or_default(),
                    );
                    fs::write(&target, original)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(&target, fs::Permissions::from_mode(0o600))?;
                    }
                }
            }
            fs::write(directory.join("manifest.json"), json_text(&manifest))?;
            Ok(())
        })();
        if let Err(error) = write_result {
            let _ = fs::remove_dir_all(&directory);
            return Err(error);
        }
        self.prune(tool);
        Ok(to_view(&manifest))
    }

    pub fn list(&self, tool: Option<&str>) -> Vec<Value> {
        let tools: Vec<String> = match tool {
            Some(tool) => vec![tool.to_string()],
            None => match fs::read_dir(&self.dir) {
                Ok(entries) => entries
                    .flatten()
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect(),
                Err(_) => Vec::new(),
            },
        };
        let mut backups = Vec::new();
        for tool_id in tools {
            let directory = self.dir.join(&tool_id);
            if !directory.is_dir() {
                continue;
            }
            if let Ok(entries) = fs::read_dir(&directory) {
                for entry in entries.flatten() {
                    if !entry.path().is_dir() {
                        continue;
                    }
                    let id = entry.file_name().to_string_lossy().into_owned();
                    if let Ok(manifest) = self.read_manifest(&tool_id, &id) {
                        backups.push(to_view(&manifest));
                    }
                }
            }
        }
        backups.sort_by_key(|item| {
            -item["createdAt"].as_i64().unwrap_or(0)
        });
        backups
    }

    pub fn restore(&self, tool: &str, backup_id: &str) -> Result<()> {
        let manifest = self.read_manifest(tool, backup_id)?;
        let directory = self.dir.join(tool).join(backup_id);
        for file in manifest["files"].as_array().cloned().unwrap_or_default() {
            let backup = directory.join(file["name"].as_str().unwrap_or_default());
            if file["existed"].as_bool().unwrap_or(false) {
                if !backup.exists() {
                    bail!("备份文件不完整，未还原");
                }
                atomic_write(
                    Path::new(file["path"].as_str().unwrap_or_default()),
                    &fs::read_to_string(&backup)?,
                )?;
            } else {
                remove_file(Path::new(file["path"].as_str().unwrap_or_default()));
            }
        }
        Ok(())
    }

    fn read_manifest(&self, tool: &str, backup_id: &str) -> Result<Value> {
        static ID_RE: OnceLock<regex::Regex> = OnceLock::new();
        let id_re = ID_RE.get_or_init(|| regex::Regex::new(r"^\d+-[a-f0-9-]+$").unwrap());
        if !id_re.is_match(backup_id) {
            bail!("无效的备份编号");
        }
        let manifest: Value = serde_json::from_str(&fs::read_to_string(
            self.dir.join(tool).join(backup_id).join("manifest.json"),
        )?)?;
        if manifest["id"].as_str() != Some(backup_id)
            || manifest["tool"].as_str() != Some(tool)
            || !manifest["files"].is_array()
        {
            bail!("备份记录无效");
        }
        Ok(manifest)
    }

    fn prune(&self, tool: &str) {
        let records = self.list(Some(tool));
        let originals: std::collections::HashSet<String> = records
            .iter()
            .filter(|r| r["original"].as_bool().unwrap_or(false))
            .filter_map(|r| r["id"].as_str().map(String::from))
            .collect();
        let mut kept = 0;
        for record in records {
            let id = record["id"].as_str().unwrap_or_default();
            if originals.contains(id) || kept < 10 {
                if !originals.contains(id) {
                    kept += 1;
                }
                continue;
            }
            let _ = fs::remove_dir_all(self.dir.join(tool).join(id));
        }
    }
}

fn to_view(manifest: &Value) -> Value {
    json!({
        "id": manifest["id"],
        "tool": manifest["tool"],
        "createdAt": manifest["createdAt"],
        "original": manifest["original"],
        "files": manifest["files"]
            .as_array()
            .map(|files| files.iter().map(|f| f["path"].clone()).collect::<Vec<_>>())
            .unwrap_or_default(),
    })
}
