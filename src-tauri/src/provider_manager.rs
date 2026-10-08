use anyhow::{bail, Context, Result};
use serde::Serialize;
use toml_edit::{value, DocumentMut, Item, Table};

#[derive(Serialize)]
pub struct Status {
    pub external: bool,
    pub base_url: String,
}

pub fn status() -> Result<Status> {
    let path = crate::config_writer::codex_config_path()?;
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let doc: DocumentMut = text.parse()?;
    let url = doc
        .get("model_providers")
        .and_then(|providers| providers.get("jokerdeck"))
        .and_then(|provider| provider.get("base_url"))
        .and_then(Item::as_str)
        .unwrap_or("")
        .to_owned();
    let external = doc
        .get("model_providers")
        .and_then(|providers| providers.get("jokerdeck"))
        .and_then(|provider| provider.get("name"))
        .and_then(Item::as_str)
        == Some("External provider (no support)");
    Ok(Status {
        external,
        base_url: url,
    })
}

fn external_config(
    mut doc: DocumentMut,
    url: &str,
    token: &str,
    model: &str,
) -> Result<DocumentMut> {
    let parsed = reqwest::Url::parse(url).context("服务商 URL 无效")?;
    let loopback = matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    if !(parsed.scheme() == "https" || (parsed.scheme() == "http" && loopback))
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        bail!("仅支持 HTTPS 或本机 HTTP URL，不允许 URL 内含密钥、参数或 fragment");
    }
    if token.trim().is_empty() || token.len() > 8192 || token.chars().any(char::is_control) {
        bail!("API Key 无效");
    }
    if model.trim().is_empty() || model.len() > 256 || model.chars().any(char::is_control) {
        bail!("Model 无效");
    }
    if doc.get("profile").is_some() {
        bail!("请先关闭 Codex profile 覆盖配置");
    }
    if doc
        .get("model_providers")
        .is_some_and(|item| !item.is_table())
    {
        bail!("model_providers 配置无效");
    }
    if doc.get("model_providers").is_none() {
        doc["model_providers"] = Item::Table(Table::new());
    }
    // Keep the stable provider identity and CODEX_HOME: threads never need rewriting or copying.
    doc["model_provider"] = value("jokerdeck");
    doc.remove("preferred_auth_method");
    doc["model"] = value(model.trim());
    doc["review_model"] = value(model.trim());
    let mut provider = Table::new();
    provider["name"] = value("External provider (no support)");
    provider["base_url"] = value(url.trim_end_matches('/'));
    provider["wire_api"] = value("responses");
    provider["requires_openai_auth"] = value(false);
    provider["experimental_bearer_token"] = value(token.trim());
    provider["supports_websockets"] = value(false);
    doc["model_providers"]["jokerdeck"] = Item::Table(provider);
    if let Some(catalog) = doc.get("model_catalog_json").and_then(Item::as_str) {
        if catalog == crate::config_writer::codex_catalog_path()?.to_string_lossy() {
            doc.remove("model_catalog_json");
        }
    }
    Ok(doc)
}

pub fn switch(url: &str, token: &str, model: &str) -> Result<Status> {
    let path = crate::config_writer::codex_config_path()?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let doc = external_config(text.parse()?, url, token, model)?;
    crate::config_writer::write_with_backup(&path, doc.to_string().as_bytes())?;
    status()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn switching_preserves_identity_and_other_configuration() {
        let doc: DocumentMut = "model_provider='jokerdeck'\napproval_policy='on-request'\n[mcp_servers.mine]\ncommand='mine'\n[model_providers.other]\nname='other'\n".parse().unwrap();
        let result =
            external_config(doc, "https://example.com/v1/", "test-key", "external-model").unwrap();
        assert_eq!(result["model_provider"].as_str(), Some("jokerdeck"));
        assert_eq!(result["approval_policy"].as_str(), Some("on-request"));
        assert_eq!(
            result["mcp_servers"]["mine"]["command"].as_str(),
            Some("mine")
        );
        assert_eq!(
            result["model_providers"]["other"]["name"].as_str(),
            Some("other")
        );
    }
    #[test]
    fn refuses_invalid_credentials_or_transport() {
        for url in [
            "http://external.com/v1",
            "https://key@example.com/v1",
            "https://example.com/v1?key=x",
            "file:///config",
        ] {
            assert!(external_config(DocumentMut::new(), url, "key", "model").is_err());
        }
        assert!(external_config(
            DocumentMut::new(),
            "https://example.com/v1",
            "key\nInjected",
            "model"
        )
        .is_err());
    }
}
