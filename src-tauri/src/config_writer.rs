use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use toml_edit::{value, DocumentMut, Item, Table};

const CODEX_PROVIDER: &str = "jokerdeck";

fn home() -> Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| anyhow!("无法定位用户主目录"))
}

pub fn claude_settings_path() -> Result<PathBuf> {
    Ok(home()?.join(".claude").join("settings.json"))
}
pub fn codex_config_path() -> Result<PathBuf> {
    Ok(codex_home()?.join("config.toml"))
}
fn codex_home() -> Result<PathBuf> {
    match std::env::var_os("CODEX_HOME").filter(|v| !v.is_empty()) {
        Some(path) => Ok(PathBuf::from(path)),
        None => Ok(home()?.join(".codex")),
    }
}

pub fn codex_catalog_path() -> Result<PathBuf> {
    Ok(codex_home()?.join("jokerdeck-models.json"))
}

pub struct ConfigView {
    pub path: PathBuf,
    pub content: String,
}

fn sensitive_key(key: &str) -> bool {
    let name = key.to_ascii_lowercase();
    ["token", "api_key", "password", "secret", "authorization"]
        .iter()
        .any(|part| name.contains(part))
}

fn is_managed_claude_model_env(key: &str) -> bool {
    key == "ANTHROPIC_MODEL"
        || key == "ANTHROPIC_SMALL_FAST_MODEL"
        || key == "ANTHROPIC_CUSTOM_MODEL_OPTION"
        || (key.starts_with("ANTHROPIC_DEFAULT_")
            && (key.ends_with("_MODEL") || key.ends_with("_MODEL_NAME")))
}

fn redact_json(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for (key, entry) in fields.iter_mut() {
                if sensitive_key(&key) {
                    *entry = json!("***");
                } else {
                    redact_json(entry);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact_json),
        _ => {}
    }
}

fn redact_toml_value(value: &mut toml_edit::Value) {
    match value {
        toml_edit::Value::InlineTable(fields) => {
            for (key, entry) in fields.iter_mut() {
                if sensitive_key(&key) {
                    *entry = toml_edit::Value::from("***");
                } else {
                    redact_toml_value(entry);
                }
            }
        }
        toml_edit::Value::Array(items) => items.iter_mut().for_each(redact_toml_value),
        _ => {}
    }
}

fn redact_toml_item(item: &mut Item, key: &str) {
    if sensitive_key(key) {
        *item = value("***");
        return;
    }
    match item {
        Item::Table(table) => {
            for (name, child) in table.iter_mut() {
                redact_toml_item(child, &name);
            }
        }
        Item::ArrayOfTables(tables) => {
            for table in tables.iter_mut() {
                for (name, child) in table.iter_mut() {
                    redact_toml_item(child, &name);
                }
            }
        }
        Item::Value(value) => redact_toml_value(value),
        Item::None => {}
    }
}

pub fn read_config(which: &str) -> Result<ConfigView> {
    let (path, json) = match which {
        "claude" => (claude_settings_path()?, true),
        "codex" => (codex_config_path()?, false),
        "catalog" => (codex_catalog_path()?, true),
        _ => return Err(anyhow!("不支持的配置文件")),
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|err| anyhow!("无法读取 {}：{err}", path.display()))?;
    let content = if json {
        let mut data: Value = serde_json::from_str(&text)?;
        redact_json(&mut data);
        serde_json::to_string_pretty(&data)?
    } else {
        let mut doc = text.parse::<DocumentMut>()?;
        for (key, item) in doc.iter_mut() {
            redact_toml_item(item, &key);
        }
        doc.to_string()
    };
    Ok(ConfigView { path, content })
}

pub fn validate_existing(claude: bool, codex: bool) -> Result<()> {
    if claude {
        read_json_object(&claude_settings_path()?)?;
    }
    if codex {
        let path = codex_config_path()?;
        match std::fs::read_to_string(path) {
            Ok(text) => {
                text.parse::<DocumentMut>()?;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }
    Ok(())
}

fn read_json_object(path: &Path) -> Result<Value> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let v: Value = serde_json::from_str(&text)?;
            if v.is_object() {
                Ok(v)
            } else {
                Err(anyhow!("配置文件不是 JSON object: {}", path.display()))
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(err) => Err(err.into()),
    }
}

pub fn write_with_backup(path: &Path, bytes: &[u8]) -> Result<()> {
    let backup = backup_path(path);
    if path.exists() && !backup.exists() {
        std::fs::copy(path, backup)?;
    }
    write_atomic(path, bytes)
}

fn backup_path(path: &Path) -> PathBuf {
    path.with_extension(format!(
        "{}.jokerdeck.bak",
        path.extension()
            .and_then(|s| s.to_str())
            .unwrap_or("config")
    ))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Stage first so serialization or a full disk cannot truncate the old file.
    let temporary = path.with_extension("jokerdeck.tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

pub struct RestoreResult {
    pub files: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn restore_item(doc: &mut DocumentMut, original: Option<&DocumentMut>, key: &str) {
    if let Some(item) = original.and_then(|saved| saved.get(key)) {
        doc[key] = item.clone();
    } else {
        doc.remove(key);
    }
}

pub fn restore_configs() -> Result<RestoreResult> {
    restore_configs_at(
        &claude_settings_path()?,
        &codex_config_path()?,
        &codex_catalog_path()?,
    )
}

fn restore_configs_at(claude: &Path, codex: &Path, catalog: &Path) -> Result<RestoreResult> {
    let mut changes: Vec<(PathBuf, Option<Vec<u8>>)> = Vec::new();
    let mut warnings = Vec::new();
    let mut consumed_backups = Vec::new();

    if let Some(current) = read_optional(claude)? {
        let mut root: Value = serde_json::from_slice(&current)?;
        let owned = root["env"]["ANTHROPIC_AUTH_TOKEN"] == "managed-by-jokerdeck";
        if owned {
            let backup = backup_path(claude);
            let saved = read_optional(&backup)?;
            let original: Value = saved
                .as_deref()
                .map(serde_json::from_slice)
                .transpose()?
                .unwrap_or_else(|| json!({}));
            let env = root["env"]
                .as_object_mut()
                .ok_or_else(|| anyhow!("Claude Code env 配置无效"))?;
            for key in [
                "ANTHROPIC_BASE_URL",
                "ANTHROPIC_AUTH_TOKEN",
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_MODEL",
                "ANTHROPIC_SMALL_FAST_MODEL",
            ] {
                if let Some(value) = original.get("env").and_then(|v| v.get(key)) {
                    env.insert(key.into(), value.clone());
                } else {
                    env.remove(key);
                }
            }
            let stale_model_keys: Vec<String> = env
                .keys()
                .filter(|key| is_managed_claude_model_env(key))
                .cloned()
                .collect();
            for key in stale_model_keys {
                if let Some(value) = original.get("env").and_then(|v| v.get(&key)) {
                    env.insert(key, value.clone());
                } else {
                    env.remove(&key);
                }
            }
            if env.is_empty() && original.get("env").is_none() {
                root.as_object_mut().unwrap().remove("env");
            }
            for key in ["model", "modelPicker"] {
                if let Some(value) = original.get(key) {
                    root[key] = value.clone();
                } else {
                    root.as_object_mut().unwrap().remove(key);
                }
            }
            let restored = if saved.is_none() && root.as_object().is_some_and(|obj| obj.is_empty())
            {
                None
            } else {
                Some(serde_json::to_vec_pretty(&root)?)
            };
            changes.push((claude.to_path_buf(), restored));
            if saved.is_some() {
                consumed_backups.push(backup);
            }
        } else if backup_path(claude).exists() {
            warnings.push("Claude Code 配置已被其他工具修改，未覆盖".into());
        }
    }

    if let Some(current) = read_optional(codex)? {
        let mut doc = String::from_utf8(current)?.parse::<DocumentMut>()?;
        let provider_name = doc.get("model_provider").and_then(Item::as_str);
        let owned = provider_name == Some(CODEX_PROVIDER);
        if owned {
            let active_provider = CODEX_PROVIDER;
            let owned_catalog = doc
                .get("model_catalog_json")
                .and_then(Item::as_str)
                .is_some_and(|path| Path::new(path) == catalog);
            let backup = backup_path(codex);
            let saved = read_optional(&backup)?;
            let original = saved
                .as_deref()
                .map(|bytes| {
                    std::str::from_utf8(bytes)?
                        .parse::<DocumentMut>()
                        .map_err(anyhow::Error::from)
                })
                .transpose()?;
            for key in [
                "model_provider",
                "preferred_auth_method",
                "model",
                "review_model",
                "model_reasoning_effort",
                "model_catalog_json",
            ] {
                restore_item(&mut doc, original.as_ref(), key);
            }
            let old_provider = original
                .as_ref()
                .and_then(|saved| saved.get("model_providers"))
                .and_then(|providers| providers.get(active_provider))
                .cloned();
            if let Some(provider) = old_provider {
                doc["model_providers"][active_provider] = provider;
            } else if let Some(providers) = doc.get_mut("model_providers") {
                if let Some(table) = providers.as_table_mut() {
                    table.remove(active_provider);
                    if table.is_empty()
                        && original
                            .as_ref()
                            .and_then(|saved| saved.get("model_providers"))
                            .is_none()
                    {
                        doc.remove("model_providers");
                    }
                }
            }
            let restored = if saved.is_none() && doc.as_table().is_empty() {
                None
            } else {
                Some(doc.to_string().into_bytes())
            };
            changes.push((codex.to_path_buf(), restored));
            if saved.is_some() {
                consumed_backups.push(backup);
            }
            let catalog_backup = backup_path(catalog);
            let catalog_original = read_optional(&catalog_backup)?;
            if owned_catalog && (catalog_original.is_some() || catalog.exists()) {
                changes.push((catalog.to_path_buf(), catalog_original.clone()));
                if catalog_original.is_some() {
                    consumed_backups.push(catalog_backup);
                }
            }
        } else if backup_path(codex).exists()
            || doc
                .get("model_providers")
                .and_then(|providers| providers.get(CODEX_PROVIDER))
                .is_some()
        {
            warnings.push("Codex 配置已被其他工具修改，未覆盖".into());
        }
    }

    let transaction =
        ConfigTransaction::begin(changes.iter().map(|(path, _)| path.clone()).collect())?;
    for (path, bytes) in &changes {
        match bytes {
            Some(bytes) => write_atomic(path, bytes)?,
            None if path.exists() => std::fs::remove_file(path)?,
            None => {}
        }
    }
    transaction.commit();
    for backup in consumed_backups {
        if let Err(err) = std::fs::remove_file(&backup) {
            warnings.push(format!("旧备份未清理：{} ({err})", backup.display()));
        }
    }
    Ok(RestoreResult {
        files: changes.into_iter().map(|(path, _)| path).collect(),
        warnings,
    })
}

pub struct ConfigTransaction {
    originals: Vec<(PathBuf, Option<Vec<u8>>)>,
    committed: bool,
}

impl ConfigTransaction {
    pub fn begin(paths: Vec<PathBuf>) -> Result<Self> {
        let mut originals = Vec::new();
        for path in paths {
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => Some(bytes),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
                Err(err) => return Err(err.into()),
            };
            originals.push((path, bytes));
        }
        Ok(Self {
            originals,
            committed: false,
        })
    }
    pub fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for ConfigTransaction {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        for (path, bytes) in &self.originals {
            let result = match bytes {
                Some(bytes) => write_with_backup(path, bytes),
                None if path.exists() => std::fs::remove_file(path).map_err(anyhow::Error::from),
                None => Ok(()),
            };
            if let Err(err) = result {
                tracing::error!(
                    "configuration rollback failed for {}: {err}",
                    path.display()
                );
            }
        }
    }
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    write_with_backup(path, serde_json::to_string_pretty(value)?.as_bytes())
}
/// Point Claude Code at the local proxy. The real key never lands in CC config:
/// the token is a sentinel and the proxy injects the actual key upstream.
/// `base_url` must NOT include `/v1` — Claude Code appends `/v1/messages` itself.
/// `model` (optional) pins the main model; the relay routes it to the active group.
pub fn write_claude(base_url: &str, model: &str, models: &[String]) -> Result<PathBuf> {
    let path = claude_settings_path()?;
    write_claude_at(&path, base_url, Some(model), models)?;
    Ok(path)
}

fn write_claude_at(
    path: &Path,
    base_url: &str,
    model: Option<&str>,
    models: &[String],
) -> Result<()> {
    let mut root = read_json_object(&path)?;
    let obj = root.as_object_mut().unwrap();
    obj.remove("modelPicker");
    if let Some(m) = model.filter(|m| !m.is_empty()) {
        obj.insert("model".into(), json!(m));
        if !models.is_empty() {
            obj.insert(
                "modelPicker".into(),
                json!({
                    "options": models.iter().map(|name| json!({"model": name, "label": name})).collect::<Vec<_>>(),
                    "replaceBuiltInOptions": true,
                }),
            );
        }
    }
    let env = obj.entry("env").or_insert_with(|| json!({}));
    if !env.is_object() {
        *env = json!({});
    }
    let env = env.as_object_mut().unwrap();
    env.insert("ANTHROPIC_BASE_URL".into(), json!(base_url));
    env.insert("ANTHROPIC_AUTH_TOKEN".into(), json!("managed-by-jokerdeck"));
    // Remove stale provider/model overrides so they cannot override the selected
    // group or lock Claude Code's /model picker to an unrelated model.
    env.retain(|key, _| !is_managed_claude_model_env(key) && key != "ANTHROPIC_API_KEY");
    if env.is_empty() {
        obj.remove("env");
    }
    write_json(&path, &root)?;
    let written = read_json_object(&path)?;
    if written["env"]["ANTHROPIC_BASE_URL"].as_str() != Some(base_url)
        || written["env"]["ANTHROPIC_AUTH_TOKEN"].as_str() != Some("managed-by-jokerdeck")
        || model.is_some_and(|model| written["model"].as_str() != Some(model))
    {
        return Err(anyhow!(
            "Claude Code 配置写入后验证失败，请检查设置文件是否被其他工具覆盖"
        ));
    }
    Ok(())
}

pub fn claude_config_warnings() -> Vec<String> {
    let mut warnings = Vec::new();
    if let Some(app_data) = std::env::var_os("APPDATA") {
        let app_data = PathBuf::from(app_data);
        if [
            "com.ccswitch.desktop",
            "com.ccswitch.plus.desktop",
            "com.ccswitch-plus-plus.desktop",
        ]
        .iter()
        .any(|name| app_data.join(name).is_dir())
        {
            warnings.push("检测到 CC Switch：切换供应商可能覆盖 Claude Code 配置。请保持客户端配置为当前配置并重新打开 Claude Code。".into());
        }
    }
    let overrides: Vec<String> = std::env::vars()
        .map(|(name, _)| name)
        .filter(|name| {
            name == "ANTHROPIC_BASE_URL"
                || name == "ANTHROPIC_API_KEY"
                || name == "ANTHROPIC_AUTH_TOKEN"
                || is_managed_claude_model_env(name)
        })
        .collect();
    if !overrides.is_empty() {
        warnings.push(format!(
            "检测到客户端进程环境变量 {}；它们可能覆盖 settings.json，请检查终端环境。",
            overrides.join(", ")
        ));
    }
    warnings
}

/// Point Codex at the local proxy via a custom provider using the Responses API.
/// `base_url_v1` should be `http://127.0.0.1:<port>/v1`.
/// `model` — when given, it is written as the default model (so the client's
/// model picker actually takes effect); when `None` an existing choice is kept.
pub fn write_codex(
    base_url_v1: &str,
    model: Option<&str>,
    catalog: Option<&Value>,
) -> Result<Vec<PathBuf>> {
    let cfg_path = codex_config_path()?;
    write_codex_at(
        &cfg_path,
        &codex_catalog_path()?,
        base_url_v1,
        model,
        catalog,
    )
}

fn write_codex_at(
    cfg_path: &Path,
    catalog_path: &Path,
    base_url_v1: &str,
    model: Option<&str>,
    catalog: Option<&Value>,
) -> Result<Vec<PathBuf>> {
    let existing = match std::fs::read_to_string(&cfg_path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err.into()),
    };
    let mut doc = existing.parse::<DocumentMut>()?;

    doc["model_provider"] = value(CODEX_PROVIDER);
    doc.remove("preferred_auth_method");
    if doc.get("profile").is_some() {
        return Err(anyhow!(
            "Codex 当前启用了 profile；请先在 config.toml 中检查 profile 覆盖项，再应用配置"
        ));
    }
    match model {
        Some(m) if !m.is_empty() => {
            doc["model"] = value(m);
            doc["review_model"] = value(m);
        }
        _ => {
            if doc.get("model").is_none() {
                doc["model"] = value("gpt-5-codex");
            }
        }
    }
    if !doc.contains_key("model_providers") {
        doc["model_providers"] = Item::Table(Table::new());
    }
    let mut provider = Table::new();
    provider["name"] = value("jokerdeck 中转");
    provider["base_url"] = value(base_url_v1);
    provider["wire_api"] = value("responses");
    // A local bearer token works in Desktop too, without inheriting a stale
    // OPENAI_API_KEY or replacing the user's ChatGPT OAuth auth.json.
    provider["requires_openai_auth"] = value(false);
    provider["experimental_bearer_token"] = value("managed-by-jokerdeck");
    provider["supports_websockets"] = value(false);
    doc["model_providers"][CODEX_PROVIDER] = Item::Table(provider);

    let mut files = Vec::new();
    if let Some(catalog) = catalog {
        let models = catalog
            .get("models")
            .and_then(Value::as_array)
            .filter(|m| !m.is_empty())
            .ok_or_else(|| anyhow!("中转未返回有效的 Codex model manifest"))?;
        if let Some(model) = model {
            if !models.iter().any(|m| {
                m.get("slug").and_then(Value::as_str) == Some(model)
                    && m["visibility"]
                        .as_str()
                        .is_none_or(|visibility| visibility == "list")
            }) {
                return Err(anyhow!(
                    "当前分组的 Codex model manifest 不包含模型 {model}"
                ));
            }
        }
        if let Some(metadata) = models.iter().find(|m| m["slug"].as_str() == model) {
            let supported: Vec<&str> = metadata["supported_reasoning_levels"]
                .as_array()
                .map(|levels| levels.iter().filter_map(|v| v["effort"].as_str()).collect())
                .unwrap_or_default();
            let effort = doc.get("model_reasoning_effort").and_then(Item::as_str);
            if !effort.is_some_and(|effort| supported.contains(&effort)) {
                let default = metadata["default_reasoning_level"]
                    .as_str()
                    .filter(|s| supported.contains(s));
                match default {
                    Some(effort) => {
                        doc["model_reasoning_effort"] = value(effort);
                    }
                    None => {
                        doc.remove("model_reasoning_effort");
                    }
                }
            }
        }
        write_json(catalog_path, catalog)?;
        doc["model_catalog_json"] = value(catalog_path.to_string_lossy().as_ref());
        files.push(catalog_path.to_path_buf());
    }
    write_with_backup(&cfg_path, doc.to_string().as_bytes())?;
    files.push(cfg_path.to_path_buf());

    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("jokerdeck-config-test-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn claude_sets_proxy_and_selected_model_without_discarding_other_settings() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{"permissions":{"allow":["Read"]},"env":{"ANTHROPIC_API_KEY":"old"}}"#,
        )
        .unwrap();
        write_claude_at(
            &path,
            "http://127.0.0.1:8788/groups/7",
            Some("claude-relay"),
            &["claude-relay".into(), "claude-other".into()],
        )
        .unwrap();
        let data = read_json_object(&path).unwrap();
        assert_eq!(
            data["env"]["ANTHROPIC_BASE_URL"],
            "http://127.0.0.1:8788/groups/7"
        );
        assert_eq!(data["model"], "claude-relay");
        assert_eq!(data["modelPicker"]["options"][1]["model"], "claude-other");
        assert!(data["env"].get("ANTHROPIC_MODEL").is_none());
        assert!(data["env"].get("ANTHROPIC_DEFAULT_HAIKU_MODEL").is_none());
        assert!(data["env"].get("ANTHROPIC_API_KEY").is_none());
        assert_eq!(data["permissions"]["allow"][0], "Read");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn config_preview_redacts_nested_credentials() {
        let mut json = json!({
            "env": {"ANTHROPIC_AUTH_TOKEN": "secret", "ANTHROPIC_BASE_URL": "http://127.0.0.1:8788"},
            "model": "relay"
        });
        redact_json(&mut json);
        assert_eq!(json["env"]["ANTHROPIC_AUTH_TOKEN"], "***");
        assert_eq!(json["model"], "relay");

        let mut doc = "model = 'relay'\n[model_providers.jokerdeck]\nexperimental_bearer_token = 'secret'\nhttp_headers = { Authorization = 'Bearer secret' }\nbase_url = 'http://127.0.0.1:8788/v1'\n"
            .parse::<DocumentMut>().unwrap();
        for (name, item) in doc.iter_mut() {
            redact_toml_item(item, &name);
        }
        let rendered = doc.to_string();
        assert!(!rendered.contains("secret"));
        assert!(rendered.contains("relay"));
        assert!(rendered.contains("127.0.0.1"));
    }

    #[test]
    fn restore_preserves_unrelated_changes_and_rearms_first_apply_backup() {
        let dir = temp_dir();
        let claude = dir.join("settings.json");
        let codex = dir.join("config.toml");
        let catalog = dir.join("jokerdeck-models.json");
        std::fs::write(
            &claude,
            r#"{"env":{"ANTHROPIC_API_KEY":"original"},"permissions":{"allow":["Read"]}}"#,
        )
        .unwrap();
        std::fs::write(
            &codex,
            "model = 'original'\n[projects.test]\ntrust_level = 'trusted'\n",
        )
        .unwrap();
        std::fs::write(&catalog, r#"{"models":["original"]}"#).unwrap();
        write_claude_at(
            &claude,
            "http://127.0.0.1:8788/groups/7",
            Some("relay"),
            &["relay".into()],
        )
        .unwrap();
        write_codex_at(
            &codex,
            &catalog,
            "http://127.0.0.1:8788/groups/7/v1",
            Some("relay"),
            Some(&json!({"models":[{"slug":"relay"}]})),
        )
        .unwrap();
        let mut current = read_json_object(&claude).unwrap();
        current["permissions"]["allow"]
            .as_array_mut()
            .unwrap()
            .push(json!("Write"));
        std::fs::write(&claude, serde_json::to_vec(&current).unwrap()).unwrap();
        let current = std::fs::read_to_string(&codex).unwrap();
        std::fs::write(&codex, format!("custom_option = true\n{current}")).unwrap();

        let restored = restore_configs_at(&claude, &codex, &catalog).unwrap();
        assert_eq!(restored.files.len(), 3);
        assert!(restored.warnings.is_empty());
        let data = read_json_object(&claude).unwrap();
        assert_eq!(data["env"]["ANTHROPIC_API_KEY"], "original");
        assert!(data["env"].get("ANTHROPIC_BASE_URL").is_none());
        assert!(data.get("model").is_none());
        assert!(data.get("modelPicker").is_none());
        assert_eq!(data["permissions"]["allow"][1], "Write");
        let doc = std::fs::read_to_string(&codex)
            .unwrap()
            .parse::<DocumentMut>()
            .unwrap();
        assert_eq!(doc["model"].as_str(), Some("original"));
        assert_eq!(doc["custom_option"].as_bool(), Some(true));
        assert!(doc.get("model_catalog_json").is_none());
        assert!(doc
            .get("model_providers")
            .and_then(|providers| providers.get("jokerdeck"))
            .is_none());
        assert_eq!(
            std::fs::read_to_string(&catalog).unwrap(),
            r#"{"models":["original"]}"#
        );
        assert!(!backup_path(&claude).exists());
        assert!(!backup_path(&codex).exists());
        assert!(!backup_path(&catalog).exists());
        write_claude_at(
            &claude,
            "http://127.0.0.1:8788/groups/8",
            Some("new"),
            &["new".into()],
        )
        .unwrap();
        assert_eq!(
            read_json_object(&backup_path(&claude)).unwrap()["permissions"]["allow"][1],
            "Write"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn restore_removes_generated_files_but_skips_external_provider() {
        let dir = temp_dir();
        let claude = dir.join("settings.json");
        let codex = dir.join("config.toml");
        let catalog = dir.join("jokerdeck-models.json");
        write_claude_at(
            &claude,
            "http://127.0.0.1:8788/groups/7",
            Some("relay"),
            &["relay".into()],
        )
        .unwrap();
        write_codex_at(
            &codex,
            &catalog,
            "http://127.0.0.1:8788/groups/7/v1",
            Some("relay"),
            Some(&json!({"models":[{"slug":"relay"}]})),
        )
        .unwrap();
        let mut doc = std::fs::read_to_string(&codex)
            .unwrap()
            .parse::<DocumentMut>()
            .unwrap();
        doc["model_provider"] = value("someone-else");
        std::fs::write(&codex, doc.to_string()).unwrap();
        let restored = restore_configs_at(&claude, &codex, &catalog).unwrap();
        assert_eq!(restored.files, vec![claude.clone()]);
        assert!(!claude.exists());
        assert!(codex.exists());
        assert!(catalog.exists());
        assert!(!restored.warnings.is_empty());
        doc["model_provider"] = value("jokerdeck");
        std::fs::write(&codex, doc.to_string()).unwrap();
        restore_configs_at(&claude, &codex, &catalog).unwrap();
        assert!(!codex.exists());
        assert!(!catalog.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn codex_preserves_settings_and_syncs_catalog_without_auth() {
        let dir = temp_dir();
        let cfg = dir.join("config.toml");
        let catalog = dir.join("models.json");
        let original = "approval_policy = 'on-request'\nmodel_reasoning_effort = 'ultra'\n[projects.'test']\ntrust_level = 'trusted'\n";
        std::fs::write(&cfg, original).unwrap();
        let models = json!({"models":[{"slug":"relay-model","visibility":"list","default_reasoning_level":"high","supported_reasoning_levels":[{"effort":"high"}]}]});
        let files = write_codex_at(
            &cfg,
            &catalog,
            "http://127.0.0.1:8788/groups/42/v1",
            Some("relay-model"),
            Some(&models),
        )
        .unwrap();
        let doc = std::fs::read_to_string(&cfg)
            .unwrap()
            .parse::<DocumentMut>()
            .unwrap();
        assert_eq!(doc["model"].as_str(), Some("relay-model"));
        assert_eq!(doc["review_model"].as_str(), Some("relay-model"));
        assert_eq!(doc["model_reasoning_effort"].as_str(), Some("high"));
        assert_eq!(doc["approval_policy"].as_str(), Some("on-request"));
        assert_eq!(
            doc["projects"]["test"]["trust_level"].as_str(),
            Some("trusted")
        );
        assert!(doc["model_providers"]["jokerdeck"].get("env_key").is_none());
        assert_eq!(
            doc["model_providers"]["jokerdeck"]["requires_openai_auth"].as_bool(),
            Some(false)
        );
        assert_eq!(
            std::fs::read_to_string(cfg.with_extension("toml.jokerdeck.bak")).unwrap(),
            original
        );
        assert_eq!(files.len(), 2);
        assert!(!dir.join("auth.json").exists());
        write_codex_at(
            &cfg,
            &catalog,
            "http://127.0.0.1:8788/groups/42/v1",
            Some("relay-model"),
            Some(&models),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(cfg.with_extension("toml.jokerdeck.bak")).unwrap(),
            original
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn invalid_config_and_missing_model_do_not_overwrite() {
        let dir = temp_dir();
        let cfg = dir.join("config.toml");
        let catalog = dir.join("models.json");
        std::fs::write(&cfg, "invalid = [").unwrap();
        assert!(write_codex_at(&cfg, &catalog, "http://local/v1", Some("test"), None).is_err());
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), "invalid = [");
        std::fs::write(&cfg, "profile = 'custom'").unwrap();
        assert!(write_codex_at(&cfg, &catalog, "http://local/v1", Some("test"), None).is_err());
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), "profile = 'custom'");
        std::fs::write(&cfg, "model = 'old'").unwrap();
        assert!(write_codex_at(
            &cfg,
            &catalog,
            "http://local/v1",
            Some("missing"),
            Some(&json!({"models":[{"slug":"other"}]}))
        )
        .is_err());
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), "model = 'old'");
        assert!(!catalog.exists());
        std::fs::write(dir.join("broken.json"), "[]").unwrap();
        assert!(read_json_object(&dir.join("broken.json")).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
