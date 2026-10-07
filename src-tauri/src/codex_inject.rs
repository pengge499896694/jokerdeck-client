use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

const MARKER: &str = "\n// jokerdeck-desktop-features-v1";
const CSP_MARKER: &str = "<!-- jokerdeck-bridge-origin:";

fn index(data: &[u8]) -> Result<(Value, usize)> {
    if data.len() < 16 {
        bail!("Codex ASAR 头部过短");
    }
    let number = |at| u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize;
    let header_size = number(4);
    let length = number(12);
    let payload = header_size.checked_add(8).context("ASAR 大小溢出")?;
    if number(0) != 4
        || length == 0
        || length > 32 * 1024 * 1024
        || payload > data.len()
        || 16 + length > payload
    {
        bail!("Codex ASAR 头部格式无效");
    }
    Ok((serde_json::from_slice(&data[16..16 + length])?, payload))
}

fn content<'a>(data: &'a [u8], payload: usize, entry: &Value) -> Result<&'a str> {
    if entry["unpacked"] == true {
        bail!("不支持 unpacked Codex WebView");
    }
    let offset: usize = entry["offset"]
        .as_str()
        .context("ASAR offset 无效")?
        .parse()?;
    let size = usize::try_from(entry["size"].as_u64().context("ASAR size 无效")?)?;
    let start = payload.checked_add(offset).context("ASAR offset 溢出")?;
    Ok(std::str::from_utf8(
        data.get(start..start.checked_add(size).context("ASAR size 溢出")?)
            .context("Codex WebView 超出 ASAR 边界")?,
    )?)
}

fn replace_entry(entry: &mut Value, payload: &mut Vec<u8>, text: &str) -> Result<()> {
    let bytes = text.as_bytes();
    entry["offset"] = json!(payload.len().to_string());
    entry["size"] = json!(bytes.len());
    let block_size = entry["integrity"]["blockSize"]
        .as_u64()
        .unwrap_or(4 * 1024 * 1024);
    if block_size == 0 || block_size > 64 * 1024 * 1024 {
        bail!("ASAR integrity blockSize 无效");
    }
    let hash = |data: &[u8]| format!("{:x}", Sha256::digest(data));
    entry["integrity"] = json!({
        "algorithm": "SHA256",
        "hash": hash(bytes),
        "blockSize": block_size,
        "blocks": bytes.chunks(block_size as usize).map(hash).collect::<Vec<_>>()
    });
    payload.extend_from_slice(bytes);
    Ok(())
}

fn pack(tree: &Value, payload: &[u8]) -> Result<Vec<u8>> {
    let header = serde_json::to_vec(tree)?;
    let padded = (header.len() + 3) & !3;
    let header_size = u32::try_from(8 + padded)?;
    let mut data = Vec::with_capacity(16 + padded + payload.len());
    data.extend_from_slice(&4u32.to_le_bytes());
    data.extend_from_slice(&header_size.to_le_bytes());
    data.extend_from_slice(&(header_size - 4).to_le_bytes());
    data.extend_from_slice(&u32::try_from(header.len())?.to_le_bytes());
    data.extend_from_slice(&header);
    data.resize(16 + padded, 0);
    data.extend_from_slice(payload);
    Ok(data)
}

fn patch_csp(original: &str, origin: &str) -> Result<String> {
    let url = reqwest::Url::parse(origin)?;
    if url.scheme() != "http" || url.host_str() != Some("127.0.0.1") || url.port().is_none() {
        bail!("统计服务地址无效");
    }
    let mut html = original.to_owned();
    if let Some(start) = html.find(CSP_MARKER) {
        let end = start + html[start..].find(" -->").context("旧 CSP 标记无效")?;
        let old = html[start + CSP_MARKER.len()..end].to_owned();
        html.replace_range(start..end + 4, "");
        html = html.replacen(&format!("connect-src {old} "), "connect-src ", 1);
    }
    if html.matches("connect-src ").count() != 1 {
        bail!("Codex CSP 结构已变化；未修改副本");
    }
    html = html.replacen("connect-src ", &format!("connect-src {origin} "), 1);
    html.push_str(&format!("{CSP_MARKER}{origin} -->"));
    Ok(html)
}

fn patched_archive(data: &[u8], script: &str, origin: &str) -> Result<Option<Vec<u8>>> {
    let (mut tree, payload_start) = index(data)?;
    let assets = tree["files"]["webview"]["files"]["assets"]["files"]
        .as_object()
        .context("Codex WebView assets 缺失")?;
    let candidates: Vec<_> = assets
        .keys()
        .filter(|name| name.starts_with("app-initial-") && name.ends_with(".js"))
        .cloned()
        .collect();
    if candidates.len() != 1 {
        bail!("无法唯一定位 Codex WebView bundle；未修改副本");
    }
    let name = &candidates[0];
    let original_js = content(
        data,
        payload_start,
        &tree["files"]["webview"]["files"]["assets"]["files"][name],
    )?;
    let base = original_js
        .split_once(MARKER)
        .map_or(original_js, |(base, _)| base);
    let patched_js = format!("{base}{script}");
    let original_html = content(
        data,
        payload_start,
        &tree["files"]["webview"]["files"]["index.html"],
    )?;
    let patched_html = patch_csp(original_html, origin)?;
    if patched_js == original_js && patched_html == original_html {
        return Ok(None);
    }
    // Retain the original payload boundary so repeated launches do not grow the archive.
    let base_length = tree["jokerdeck_payload_base"]
        .as_u64()
        .map(usize::try_from)
        .transpose()?
        .unwrap_or(data.len() - payload_start);
    if base_length > data.len() - payload_start {
        bail!("Codex ASAR payload 边界无效");
    }
    tree["jokerdeck_payload_base"] = json!(base_length);
    let mut payload = data[payload_start..payload_start + base_length].to_vec();
    replace_entry(
        &mut tree["files"]["webview"]["files"]["index.html"],
        &mut payload,
        &patched_html,
    )?;
    replace_entry(
        &mut tree["files"]["webview"]["files"]["assets"]["files"][name],
        &mut payload,
        &patched_js,
    )?;
    Ok(Some(pack(&tree, &payload)?))
}

pub async fn apply(archive: PathBuf) -> Result<()> {
    let origin = crate::desktop_features::bridge_origin().context("统计服务尚未启动")?;
    let script = crate::desktop_features::runtime_script();
    tokio::task::spawn_blocking(move || {
        let data = fs::read(&archive).with_context(|| format!("无法读取 {}", archive.display()))?;
        let Some(patched) = patched_archive(&data, &script, &origin)? else {
            return Ok(());
        };
        let backup = archive.with_extension("asar.jokerdeck-feature-backup");
        if !backup.exists() {
            fs::copy(&archive, &backup)?;
        }
        let staging = archive.with_extension("asar.jokerdeck-feature-tmp");
        fs::write(&staging, patched)?;
        fs::rename(&staging, &archive).context("无法写入 Codex 增强副本")?;
        Ok(())
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csp_replaces_previous_bridge_without_growing() {
        let html = r#"<meta http-equiv="Content-Security-Policy" content="connect-src 'self'">"#;
        let first = patch_csp(html, "http://127.0.0.1:1234").unwrap();
        let second = patch_csp(&first, "http://127.0.0.1:5678").unwrap();
        assert!(!second.contains("127.0.0.1:1234"));
        assert_eq!(second.matches("127.0.0.1:5678").count(), 2);
        assert!(patch_csp(html, "http://evil.example:5678").is_err());
    }

    #[test]
    fn archive_injection_is_idempotent_and_replaces_bridge() {
        let js = b"console.log('codex');";
        let html = b"<meta content=\"connect-src 'self'\">";
        let entry = |offset: usize, size: usize| {
            json!({
                "offset": offset.to_string(), "size": size, "integrity": {"blockSize": 4096}
            })
        };
        let tree = json!({"files": {"webview": {"files": {
            "index.html": entry(js.len(), html.len()),
            "assets": {"files": {"app-initial-test.js": entry(0, js.len())}}
        }}}});
        let original = pack(&tree, &[js.as_slice(), html.as_slice()].concat()).unwrap();
        let script = format!("{MARKER}\n(function() {{}})();");
        let first = patched_archive(&original, &script, "http://127.0.0.1:1234")
            .unwrap()
            .unwrap();
        assert!(patched_archive(&first, &script, "http://127.0.0.1:1234")
            .unwrap()
            .is_none());
        let second = patched_archive(&first, &script, "http://127.0.0.1:5678")
            .unwrap()
            .unwrap();
        let (tree, payload) = index(&second).unwrap();
        let html = content(
            &second,
            payload,
            &tree["files"]["webview"]["files"]["index.html"],
        )
        .unwrap();
        assert!(html.contains("127.0.0.1:5678"));
        assert!(!html.contains("127.0.0.1:1234"));
        assert_eq!(first.len(), second.len());
    }

    #[cfg(windows)]
    #[test]
    fn installed_managed_copy_is_compatible_when_present() {
        let Some(app) = crate::codex_desktop::patched_app_dir() else {
            return;
        };
        let archive = std::fs::read(app.join("resources/app.asar")).unwrap();
        let script = crate::desktop_features::runtime_script();
        let result = patched_archive(&archive, &script, "http://127.0.0.1:39877");
        assert!(result.is_ok(), "managed copy incompatible: {result:?}");
    }
}
