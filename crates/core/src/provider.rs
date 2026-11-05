use crate::config::{Provider, ProviderKind};
use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::time::Duration;

/// Stream a completion. Each text delta is passed to `on_chunk`; the full text is returned.
pub async fn stream(
    p: &Provider,
    system: &str,
    user: &str,
    on_chunk: impl FnMut(&str),
) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(p.timeout_secs))
        .build()?;
    match p.kind {
        ProviderKind::OpenAi => openai(&client, p, system, user, on_chunk).await,
        ProviderKind::Anthropic => anthropic(&client, p, system, user, on_chunk).await,
    }
}

/// Convenience for a non-streaming call.
pub async fn complete(p: &Provider, system: &str, user: &str) -> Result<String> {
    stream(p, system, user, |_| {}).await
}

async fn openai(
    client: &reqwest::Client,
    p: &Provider,
    system: &str,
    user: &str,
    mut on_chunk: impl FnMut(&str),
) -> Result<String> {
    let url = format!("{}/chat/completions", p.base_url.trim_end_matches('/'));
    let body = json!({
        "model": p.model,
        "stream": true,
        "max_tokens": p.max_tokens,
        "temperature": p.temperature,
        "messages": [
            {"role": "system", "content": system.trim()},
            {"role": "user", "content": user},
        ],
    });
    let mut req = client.post(&url).json(&body);
    if let Some(key) = p.api_key() {
        req = req.bearer_auth(key);
    }
    let resp = req.send().await.with_context(|| format!("POST {url}"))?;
    let resp = check_status(resp).await?;

    let mut full = String::new();
    sse(resp, |data| {
        if data == "[DONE]" {
            return Ok(());
        }
        let v: Value = serde_json::from_str(data)?;
        if let Some(t) = v["choices"][0]["delta"]["content"].as_str() {
            full.push_str(t);
            on_chunk(t);
        }
        Ok(())
    })
    .await?;
    Ok(full)
}

async fn anthropic(
    client: &reqwest::Client,
    p: &Provider,
    system: &str,
    user: &str,
    mut on_chunk: impl FnMut(&str),
) -> Result<String> {
    let base = if p.base_url.is_empty() {
        "https://api.anthropic.com"
    } else {
        p.base_url.trim_end_matches('/')
    };
    let url = format!("{base}/v1/messages");
    let key = p
        .api_key()
        .ok_or_else(|| anyhow!("Anthropic provider needs an API key (api_key or {})", p.api_key_env))?;
    let body = json!({
        "model": p.model,
        "stream": true,
        "max_tokens": p.max_tokens,
        "temperature": p.temperature,
        "system": system.trim(),
        "messages": [{"role": "user", "content": user}],
    });
    let resp = client
        .post(&url)
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    let resp = check_status(resp).await?;

    let mut full = String::new();
    sse(resp, |data| {
        let v: Value = serde_json::from_str(data)?;
        if v["type"] == "content_block_delta" {
            if let Some(t) = v["delta"]["text"].as_str() {
                full.push_str(t);
                on_chunk(t);
            }
        }
        Ok(())
    })
    .await?;
    Ok(full)
}

async fn check_status(resp: reqwest::Response) -> Result<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let text = resp.text().await.unwrap_or_default();
    Err(anyhow!("provider returned {status}: {}", text.chars().take(500).collect::<String>()))
}

/// Minimal server-sent-events reader: calls `on_data` with the payload of each `data:` line.
async fn sse(resp: reqwest::Response, mut on_data: impl FnMut(&str) -> Result<()>) -> Result<()> {
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buf.find('\n') {
            let line = buf[..pos].trim_end_matches('\r').to_string();
            buf.drain(..=pos);
            if let Some(data) = line.strip_prefix("data:") {
                on_data(data.trim())?;
            }
        }
    }
    Ok(())
}
