use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use toml_edit::{value, DocumentMut, Item, Table};

#[derive(Serialize)]
pub struct Status {
    pub plugins_enabled: bool,
    pub cache_available: bool,
    pub cache_registered: bool,
    pub cached_plugins: usize,
    pub model_catalog_active: bool,
    pub model_count: usize,
}

fn root() -> Result<PathBuf> {
    Ok(crate::config_writer::codex_config_path()?
        .parent()
        .context("Codex 配置目录无效")?
        .to_path_buf())
}

fn cache_path() -> Result<PathBuf> {
    Ok(root()?.join(".tmp").join("plugins-remote"))
}

fn read_config() -> Result<DocumentMut> {
    let path = crate::config_writer::codex_config_path()?;
    match std::fs::read_to_string(path) {
        Ok(text) => text.parse().context("Codex config.toml 格式无效"),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(DocumentMut::new()),
        Err(err) => Err(err.into()),
    }
}

fn cache_count(path: &Path) -> Result<usize> {
    let manifest = path.join(".agents/plugins/marketplace.json");
    if std::fs::metadata(&manifest)?.len() > 2 * 1024 * 1024 {
        bail!("远端插件缓存 manifest 过大");
    }
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(manifest)?)?;
    let plugins = data["plugins"]
        .as_array()
        .context("远端插件缓存 manifest 格式无效")?;
    if plugins.is_empty() {
        bail!("远端插件缓存为空");
    }
    for plugin in plugins {
        let source = plugin["source"]["path"]
            .as_str()
            .context("插件缺少本地路径")?;
        let relative = Path::new(source);
        if relative.is_absolute()
            || relative
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            bail!("插件缓存包含非法路径");
        }
        if !path.join(relative).is_dir() {
            bail!("插件缓存不完整");
        }
    }
    Ok(plugins.len())
}

fn registered(doc: &DocumentMut, path: &Path) -> bool {
    doc.get("marketplaces")
        .and_then(|item| item.get("codex-plus-curated"))
        .and_then(|item| item.get("source"))
        .and_then(Item::as_str)
        .and_then(|source| std::fs::canonicalize(source).ok())
        .zip(std::fs::canonicalize(path).ok())
        .is_some_and(|(left, right)| left == right)
}

fn plugins_enabled(doc: &DocumentMut) -> bool {
    doc.get("features")
        .and_then(|item| item.get("plugins"))
        .and_then(Item::as_bool)
        .unwrap_or(false)
}

pub fn status() -> Result<Status> {
    let doc = read_config()?;
    let cache = cache_path()?;
    let cached_plugins = cache_count(&cache).unwrap_or(0);
    let catalog_path = doc.get("model_catalog_json").and_then(Item::as_str);
    let model_count = catalog_path
        .filter(|path| std::fs::metadata(path).is_ok_and(|meta| meta.len() <= 12 * 1024 * 1024))
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|json| json["models"].as_array().map(Vec::len))
        .unwrap_or(0);
    Ok(Status {
        plugins_enabled: plugins_enabled(&doc),
        cache_available: cached_plugins > 0,
        cache_registered: cached_plugins > 0 && registered(&doc, &cache),
        cached_plugins,
        model_catalog_active: model_count > 0,
        model_count,
    })
}

pub fn enable_marketplace() -> Result<String> {
    let mut doc = read_config()?;
    if !doc.contains_key("features") {
        doc["features"] = Item::Table(Table::new());
    }
    if !doc["features"].is_table() && !doc["features"].is_inline_table() {
        bail!("Codex features 配置格式无效");
    }
    doc["features"]["plugins"] = value(true);
    let path = crate::config_writer::codex_config_path()?;
    crate::config_writer::write_with_backup(&path, doc.to_string().as_bytes())?;
    Ok("已启用 Codex 插件功能；API Key 模式的官方在线市场仍受登录权限限制。".into())
}

pub fn register_cache() -> Result<String> {
    let cache = cache_path()?;
    let count = cache_count(&cache).map_err(|_| {
        anyhow!("未发现完整的官方远端插件缓存；请先在已登录的 Codex Desktop 中获取插件缓存")
    })?;
    let mut doc = read_config()?;
    if registered(&doc, &cache) {
        return Ok(format!("已注册 {count} 个缓存插件，无需修复"));
    }
    if !doc.contains_key("marketplaces") {
        doc["marketplaces"] = Item::Table(Table::new());
    }
    if !doc["marketplaces"].is_table() {
        bail!("Codex marketplaces 配置格式无效");
    }
    if let Some(source) = doc["marketplaces"]
        .get("codex-plus-curated")
        .and_then(|item| item.get("source"))
        .and_then(Item::as_str)
    {
        if Path::new(source).exists() {
            bail!("同名插件市场已指向其他有效目录，请先在 Codex 中核对");
        }
    }
    let mut entry = Table::new();
    entry["source_type"] = value("local");
    entry["source"] = value(cache.canonicalize()?.to_string_lossy().as_ref());
    doc["marketplaces"]["codex-plus-curated"] = Item::Table(entry);
    let config = crate::config_writer::codex_config_path()?;
    crate::config_writer::write_with_backup(&config, doc.to_string().as_bytes())?;
    Ok(format!("已注册 {count} 个本地缓存插件，重启 Codex 后生效"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_missing_cache_manifest() {
        assert!(cache_count(Path::new("nonexistent-cache-directory")).is_err());
    }

    #[test]
    fn unconfigured_plugins_remain_available_to_enable() {
        assert!(!plugins_enabled(&DocumentMut::new()));
        let enabled = "[features]\nplugins = true\n".parse().unwrap();
        assert!(plugins_enabled(&enabled));
    }
}
