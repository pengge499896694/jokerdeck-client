use crate::{config_writer, state::AppState};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Serialize)]
pub struct DiagItem {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}
#[derive(Serialize)]
pub struct DiagReport {
    pub items: Vec<DiagItem>,
    pub overall_ok: bool,
}
fn item(name: &str, ok: bool, detail: impl Into<String>) -> DiagItem {
    DiagItem {
        name: name.into(),
        ok,
        detail: detail.into(),
    }
}

fn managed_url(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|parsed| {
        parsed.scheme() == "http"
            && parsed.host_str() == Some("127.0.0.1")
            && parsed.port().is_some()
    })
}

/// Test each configured tool at its own group-pinned URL. Never use the first
/// /models entry for both protocols or diagnose an unconfigured tool as broken.
pub async fn run(state: &AppState) -> DiagReport {
    let mut items = Vec::new();
    let running = state.proxy.read().await.running;
    items.push(item(
        "本地代理",
        running,
        if running {
            "运行中"
        } else {
            "未启动，请重新应用配置"
        },
    ));
    if !running {
        return DiagReport {
            items,
            overall_ok: false,
        };
    }
    let mut targets: Vec<(&str, String, String, Value)> = Vec::new();
    if let Ok(path) = config_writer::claude_settings_path() {
        if let Ok(text) = std::fs::read_to_string(path) {
            match serde_json::from_str::<Value>(&text) {
                Ok(settings) => {
                    let env = &settings["env"];
                    if let Some(url) = env["ANTHROPIC_BASE_URL"]
                        .as_str()
                        .filter(|u| managed_url(u))
                    {
                        let model = settings["model"]
                            .as_str()
                            .or_else(|| env["ANTHROPIC_MODEL"].as_str())
                            .unwrap_or("");
                        items.push(item(
                            "Claude Code 配置",
                            !model.is_empty(),
                            format!("{url} · 模型 {model}"),
                        ));
                        if !model.is_empty() {
                            targets.push(("Claude Code 接口", format!("{}/v1/messages", url.trim_end_matches('/')), model.into(),
                                json!({"model": model, "max_tokens": 1, "messages": [{"role": "user", "content": "ping"}]})));
                        }
                    }
                }
                Err(_) => items.push(item(
                    "Claude Code 配置",
                    false,
                    "settings.json 格式无效，请先修复 JSON",
                )),
            }
        }
    }
    for warning in config_writer::claude_config_warnings() {
        items.push(item("Claude Code 配置冲突提示", true, warning));
    }
    if let Ok(path) = config_writer::codex_config_path() {
        if let Ok(text) = std::fs::read_to_string(path) {
            match text.parse::<toml_edit::DocumentMut>() {
                Ok(doc)
                    if matches!(
                        doc.get("model_provider").and_then(|v| v.as_str()),
                        Some("jokerdeck")
                    ) =>
                {
                    let provider_name = doc["model_provider"].as_str().unwrap_or("jokerdeck");
                    let provider = &doc["model_providers"][provider_name];
                    let model = doc["model"].as_str().unwrap_or("");
                    let url = provider["base_url"].as_str().unwrap_or("");
                    let valid = managed_url(url)
                        && !model.is_empty()
                        && provider["experimental_bearer_token"].as_str().is_some();
                    items.push(item("Codex 配置", valid, format!("{url} · 模型 {model}")));
                    let catalog = doc
                        .get("model_catalog_json")
                        .and_then(|v| v.as_str())
                        .and_then(|path| std::fs::read_to_string(path).ok())
                        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
                    let models = catalog.as_ref().and_then(|v| v["models"].as_array());
                    let has_model = models.is_some_and(|models| {
                        models.iter().any(|m| m["slug"].as_str() == Some(model))
                    });
                    items.push(item(
                        "Codex 模型目录",
                        has_model,
                        if has_model {
                            "已同步，包含所选模型"
                        } else {
                            "模型目录缺失或模型不匹配，请重新应用配置"
                        },
                    ));
                    if valid {
                        targets.push(("Codex 接口", format!("{}/responses", url.trim_end_matches('/')), model.into(),
                            json!({"model":model,"input":"ping","max_output_tokens":128,"stream":true})));
                    }
                }
                Ok(_) => {}
                Err(_) => items.push(item(
                    "Codex 配置",
                    false,
                    "config.toml 格式无效，请先修复 TOML",
                )),
            }
        }
    }
    if targets.is_empty() {
        items.push(item(
            "工具配置",
            false,
            "未发现客户端管理的工具配置，请选择分组并应用",
        ));
    }
    for (name, url, model, payload) in targets {
        let result = state
            .http
            .post(url)
            .bearer_auth("managed-by-jokerdeck")
            .header("anthropic-version", "2023-06-01")
            .json(&payload)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await;
        items.push(match result {
            Ok(response) => match crate::model_probe::read(response).await {
                Ok((ok, detail)) => item(name, ok, format!("{detail} · 模型 {model}")),
                Err(error) => item(name, false, error),
            },
            Err(_) => item(name, false, "请求超时或代理不可达，请重新应用配置后重试"),
        });
    }
    let overall_ok = items.iter().all(|i| i.ok);
    DiagReport { items, overall_ok }
}

#[cfg(test)]
mod tests {
    #[test]
    fn diagnostics_only_call_local_proxy() {
        assert!(super::managed_url("http://127.0.0.1:8788/groups/1/v1"));
        assert!(!super::managed_url("https://example.com/v1"));
        assert!(!super::managed_url("http://127.0.0.1.evil.test:8788"));
    }
}
