use serde_json::Value;

fn error(body: &Value) -> Option<String> {
    if let Some(value) = body.get("error").filter(|value| !value.is_null()) {
        return Some(
            value
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| value.as_str())
                .unwrap_or("中转返回错误")
                .chars()
                .take(160)
                .collect(),
        );
    }
    if matches!(body["type"].as_str(), Some("error" | "response.failed"))
        || body["status"] == "failed"
    {
        return Some(
            body["response"]["error"]["message"]
                .as_str()
                .unwrap_or("模型响应失败")
                .chars()
                .take(160)
                .collect(),
        );
    }
    None
}

/// Accept JSON and SSE, but never count a truncated stream as a successful probe.
pub(crate) fn assess(status: u16, text: &str) -> (bool, String) {
    let result = if let Ok(body) = serde_json::from_str::<Value>(text) {
        match error(&body) {
            Some(reason) => Err(reason),
            None if body.is_object() => Ok(()),
            None => Err("返回内容不是模型响应".into()),
        }
    } else {
        let mut completed = false;
        let mut failure = None;
        for line in text.lines().filter_map(|line| line.strip_prefix("data:")) {
            let data = line.trim();
            if data == "[DONE]" {
                continue;
            }
            match serde_json::from_str::<Value>(data) {
                Ok(body) => {
                    if let Some(reason) = error(&body) {
                        failure = Some(reason);
                        break;
                    }
                    completed |= matches!(
                        body["type"].as_str(),
                        Some("response.completed" | "response.incomplete" | "message_stop")
                    );
                }
                Err(_) => {
                    failure = Some("SSE 返回内容无效".into());
                    break;
                }
            }
        }
        match failure {
            Some(reason) => Err(reason),
            None if completed => Ok(()),
            None => Err("返回内容无效或流式响应未完成".into()),
        }
    };
    match result {
        Ok(()) if (200..300).contains(&status) => (
            true,
            format!("HTTP {status} · 模型可用（本次测试可能产生费用）"),
        ),
        Ok(()) => (false, format!("HTTP {status} · 中转返回错误")),
        Err(reason) => (false, format!("HTTP {status} · {reason}")),
    }
}

pub(crate) async fn read(response: reqwest::Response) -> Result<(bool, String), String> {
    let status = response.status().as_u16();
    let mut response = response;
    let mut bytes = Vec::new();
    // Keep malformed upstream responses from consuming unbounded memory.
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err("测试响应超过 2 MB 上限".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(assess(status, &String::from_utf8_lossy(&bytes)))
}

#[cfg(test)]
mod tests {
    use super::assess;
    #[test]
    fn accepts_null_error_and_completed_streams() {
        assert!(assess(200, r#"{"error":null,"output":[]}"#).0);
        assert!(
            assess(
                200,
                "data: {\"type\":\"response.completed\",\"response\":{\"error\":null}}\n\n"
            )
            .0
        );
        assert!(assess(200, "data: {\"type\":\"message_stop\"}\n\n").0);
    }
    #[test]
    fn rejects_errors_and_truncated_streams() {
        assert!(!assess(200, r#"{"error":{"message":"failed"}}"#).0);
        assert!(!assess(200, "data: {\"type\":\"response.failed\"}\n\n").0);
        assert!(!assess(200, "data: {\"type\":\"response.created\"}\n\n").0);
        assert!(!assess(502, r#"{"error":null}"#).0);
        assert!(!assess(200, "<html>error</html>").0);
    }
}
