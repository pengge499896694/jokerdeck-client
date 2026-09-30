use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::{
    fs::{self, File},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

#[derive(Serialize)]
pub struct Session {
    pub id: String,
    pub updated_at: u64,
    pub size: u64,
    pub title: String,
}

#[derive(Serialize)]
pub struct SessionDetail {
    pub messages: Vec<SessionMessage>,
    pub truncated: bool,
}

#[derive(Serialize)]
pub struct SessionMessage {
    pub role: String,
    pub text: String,
}

fn sessions_root() -> Result<PathBuf> {
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .context("无法定位 Codex 数据目录")?;
    let root = if std::env::var_os("CODEX_HOME").is_some() {
        home
    } else {
        home.join(".codex")
    };
    Ok(root.join("sessions"))
}

fn files(root: &Path) -> Result<Vec<(PathBuf, fs::Metadata)>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut pending = vec![(root.to_path_buf(), 0)];
    let mut found = Vec::new();
    while let Some((dir, depth)) = pending.pop() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() && depth < 5 {
                pending.push((entry.path(), depth + 1));
            } else if kind.is_file() && entry.path().extension().is_some_and(|e| e == "jsonl") {
                found.push((entry.path(), entry.metadata()?));
            }
            if found.len() > 20_000 {
                bail!("会话数量超过安全扫描上限");
            }
        }
    }
    Ok(found)
}

fn timestamp(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_secs())
}

fn text(value: &serde_json::Value) -> Option<String> {
    Some(
        value
            .get("content")?
            .as_array()?
            .iter()
            .filter_map(|part| part.get("text").and_then(|v| v.as_str()))
            .take(4)
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string(),
    )
}

fn messages(path: &Path, limit: u64) -> Result<(Vec<SessionMessage>, bool)> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut line = String::new();
    let mut consumed = 0u64;
    let mut result = Vec::new();
    loop {
        line.clear();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            break;
        }
        consumed += bytes as u64;
        if consumed > limit {
            break;
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
            if value.get("type").and_then(|v| v.as_str()) != Some("response_item") {
                continue;
            }
            let payload = &value["payload"];
            let role = payload["role"].as_str().unwrap_or("");
            if !matches!(role, "user" | "assistant") {
                continue;
            }
            if let Some(body) = text(payload).filter(|body| !body.is_empty()) {
                result.push(SessionMessage {
                    role: role.into(),
                    text: body.chars().take(4000).collect(),
                });
            }
        }
    }
    Ok((result, consumed > limit))
}

pub fn list() -> Result<Vec<Session>> {
    let mut entries = files(&sessions_root()?)?;
    entries.sort_unstable_by_key(|(_, meta)| std::cmp::Reverse(timestamp(meta)));
    entries
        .into_iter()
        .take(100)
        .map(|(path, meta)| {
            let id = path
                .file_stem()
                .context("会话文件名无效")?
                .to_string_lossy()
                .into_owned();
            let (items, _) = messages(&path, 512 * 1024)?;
            let title = items
                .iter()
                .find(|item| item.role == "user" && !item.text.starts_with("# AGENTS.md"))
                .or_else(|| items.iter().find(|item| item.role == "user"))
                .map(|item| {
                    item.text
                        .lines()
                        .next()
                        .unwrap_or("")
                        .chars()
                        .take(100)
                        .collect()
                })
                .unwrap_or_else(|| id.clone());
            Ok(Session {
                id,
                updated_at: timestamp(&meta),
                size: meta.len(),
                title,
            })
        })
        .collect()
}

pub fn detail(id: &str) -> Result<SessionDetail> {
    if !id.starts_with("rollout-")
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
    {
        bail!("无效的会话 ID");
    }
    let path = files(&sessions_root()?)?
        .into_iter()
        .find(|(path, _)| path.file_stem().is_some_and(|stem| stem == id))
        .map(|(path, _)| path)
        .context("会话不存在")?;
    let (mut items, truncated) = messages(&path, 8 * 1024 * 1024)?;
    if items.len() > 60 {
        items.drain(..items.len() - 60);
    }
    Ok(SessionDetail {
        messages: items,
        truncated,
    })
}

fn session_uuid(id: &str) -> Result<&str> {
    if !id.starts_with("rollout-")
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
    {
        bail!("会话 ID 无效");
    }
    let uuid = &id[id.len().saturating_sub(36)..];
    if uuid.len() != 36
        || !uuid.bytes().enumerate().all(|(index, c)| {
            if [8, 13, 18, 23].contains(&index) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
    {
        bail!("会话 UUID 无效");
    }
    Ok(uuid)
}

pub async fn delete(id: &str) -> Result<()> {
    let uuid = session_uuid(id)?;
    // Resolve against the session directory first; never pass an arbitrary name to the CLI.
    if !files(&sessions_root()?)?
        .iter()
        .any(|(path, _)| path.file_stem().is_some_and(|stem| stem == id))
    {
        bail!("会话不存在");
    }
    let mut command = tokio::process::Command::new(
        std::env::var_os("CODEX_CLI_PATH").unwrap_or_else(|| "codex".into()),
    );
    command.args(["delete", uuid, "--force"]).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let output =
        tokio::time::timeout(std::time::Duration::from_secs(30), command.output()).await??;
    if !output.status.success() {
        bail!(
            "Codex 删除失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extracts_text_from_message() {
        let value = serde_json::json!({"content":[{"type":"input_text","text":"hello"}]});
        assert_eq!(text(&value).as_deref(), Some("hello"));
    }

    #[test]
    fn validates_session_uuid_before_cli_use() {
        let id = "rollout-2026-10-01T06-37-11-01a0f476-f4c7-7523-b71e-32dd083b2b5b";
        assert_eq!(
            session_uuid(id).unwrap(),
            "01a0f476-f4c7-7523-b71e-32dd083b2b5b"
        );
        assert!(session_uuid("rollout-../../01a0f476-f4c7-7523-b71e-32dd083b2b5b").is_err());
        assert!(session_uuid("rollout-2026-invalid").is_err());
    }
}
