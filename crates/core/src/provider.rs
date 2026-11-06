//! Talking to a [`Backend`]: OpenAI-compatible and Anthropic HTTP APIs (streamed
//! over SSE), Claude Code in print mode, and plain stdin/stdout commands.

use crate::backends::{Backend, ChildEnv, OpenAiWire, Secret, Transport};
use crate::config::Provider;
use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Generation settings shared by every backend.
#[derive(Debug, Clone)]
pub struct Gen {
    pub max_tokens: u32,
    pub temperature: f64,
    /// HTTP: give up when nothing arrives for this long. Programs: total time.
    pub timeout: Duration,
}

impl From<&Provider> for Gen {
    fn from(p: &Provider) -> Self {
        Gen {
            max_tokens: p.max_tokens,
            temperature: p.temperature,
            timeout: Duration::from_secs(p.timeout_secs.max(1)),
        }
    }
}

/// Stream a completion. Each text delta is passed to `on_chunk`; the full text
/// is returned. A cut-off (`max_tokens`) or empty answer is an error, so it is
/// never pasted over the user's text.
pub async fn stream(
    b: &Backend,
    gen: &Gen,
    system: &str,
    user: &str,
    on_chunk: impl FnMut(&str),
) -> Result<String> {
    let system = system.trim();
    let out = match &b.transport {
        Transport::OpenAi(w) => openai(w, &b.model, gen, system, user, on_chunk).await,
        Transport::Anthropic {
            base_url,
            api_key,
            local,
        } => {
            anthropic(
                base_url, api_key, *local, &b.model, gen, system, user, on_chunk,
            )
            .await
        }
        Transport::ClaudeCode { bin, env } => {
            let run = claude_code(bin, env, &b.model, system, user, on_chunk);
            with_deadline(&b.label, gen.timeout, run).await
        }
        Transport::Command { argv, env } => {
            let run = command(argv, env, &b.model, system, user, on_chunk);
            with_deadline(&b.label, gen.timeout, run).await
        }
    }
    .with_context(|| format!("{} ({})", b.label, b.model_label()))?;
    if out.trim().is_empty() {
        bail!("{} ({}) returned an empty answer", b.label, b.model_label());
    }
    Ok(out)
}

async fn with_deadline(
    label: &str,
    limit: Duration,
    fut: impl std::future::Future<Output = Result<String>>,
) -> Result<String> {
    tokio::time::timeout(limit, fut)
        .await
        .map_err(|_| anyhow!("{label} did not answer within {}s", limit.as_secs()))?
}

/// Convenience for a non-streaming call.
pub async fn complete(b: &Backend, gen: &Gen, system: &str, user: &str) -> Result<String> {
    stream(b, gen, system, user, |_| {}).await
}

fn http_client(gen: &Gen, local: bool) -> Result<reqwest::Client> {
    // No total timeout: a long answer that keeps streaming is fine. Only a
    // stall (nothing received for `timeout`) aborts.
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(gen.timeout);
    if local {
        builder = builder.no_proxy();
    }
    Ok(builder.build()?)
}

fn http_error(e: reqwest::Error, gen: &Gen) -> anyhow::Error {
    if e.is_timeout() {
        anyhow!(
            "no response for {}s (provider.timeout_secs)",
            gen.timeout.as_secs()
        )
    } else {
        anyhow::Error::new(e)
    }
}

/// The JSON body for `/chat/completions`.
pub fn openai_body(w: &OpenAiWire, model: &str, gen: &Gen, system: &str, user: &str) -> Value {
    let mut body = json!({
        "model": model,
        "stream": true,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
    });
    body[w.max_tokens_field] = json!(gen.max_tokens);
    if w.temperature {
        body["temperature"] = json!(gen.temperature);
    }
    if let (Some(dst), Some(extra)) = (body.as_object_mut(), w.extra.as_object()) {
        for (k, v) in extra {
            dst.insert(k.clone(), v.clone());
        }
    }
    body
}

/// What one SSE payload contributes.
#[derive(Debug, Default, PartialEq)]
pub struct Piece {
    pub text: Option<String>,
    /// Why the model stopped, when this payload says so.
    pub stop: Option<String>,
}

/// Fail on stop reasons that mean the answer is not usable as a replacement.
fn check_stop(stop: Option<&str>, gen: &Gen) -> Result<()> {
    match stop {
        Some("length") | Some("max_tokens") => bail!(
            "the answer was cut off at {} tokens; raise provider.max_tokens",
            gen.max_tokens
        ),
        Some("content_filter") | Some("refusal") => bail!("the model declined to answer"),
        _ => Ok(()),
    }
}

async fn openai(
    w: &OpenAiWire,
    model: &str,
    gen: &Gen,
    system: &str,
    user: &str,
    mut on_chunk: impl FnMut(&str),
) -> Result<String> {
    let url = format!("{}/chat/completions", w.base_url.trim_end_matches('/'));
    let mut req = http_client(gen, w.local)?
        .post(&url)
        .json(&openai_body(w, model, gen, system, user));
    if let Some(Secret(key)) = &w.api_key {
        req = req.bearer_auth(key);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| http_error(e, gen))
        .with_context(|| format!("POST {url}"))?;
    let resp = check_status(resp).await?;
    if is_plain_json(&resp) {
        let v: Value = resp.json().await.map_err(|e| http_error(e, gen))?;
        if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
            bail!("{}", err["message"].as_str().unwrap_or("provider error"));
        }
        let choice = &v["choices"][0];
        check_stop(choice["finish_reason"].as_str(), gen)?;
        let text = choice["message"]["content"].as_str().unwrap_or_default();
        on_chunk(text);
        return Ok(text.to_string());
    }

    let mut full = String::new();
    sse(resp, gen, |data| {
        let piece = openai_delta(data)?;
        if let Some(t) = piece.text {
            full.push_str(&t);
            on_chunk(&t);
        }
        check_stop(piece.stop.as_deref(), gen)
    })
    .await?;
    Ok(full)
}

/// Parse one OpenAI-style SSE `data:` payload. Errors that arrive inside a 200
/// stream (OpenRouter does this) become `Err`.
pub fn openai_delta(data: &str) -> Result<Piece> {
    if data == "[DONE]" {
        return Ok(Piece::default());
    }
    let v: Value = serde_json::from_str(data).with_context(|| format!("bad SSE data: {data}"))?;
    if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
        let msg = err["message"].as_str().map(str::to_string);
        bail!("{}", msg.unwrap_or_else(|| err.to_string()));
    }
    let choice = &v["choices"][0];
    Ok(Piece {
        text: choice["delta"]["content"]
            .as_str()
            .filter(|t| !t.is_empty())
            .map(str::to_string),
        stop: choice["finish_reason"].as_str().map(str::to_string),
    })
}

#[allow(clippy::too_many_arguments)]
async fn anthropic(
    base_url: &str,
    key: &Secret,
    local: bool,
    model: &str,
    gen: &Gen,
    system: &str,
    user: &str,
    mut on_chunk: impl FnMut(&str),
) -> Result<String> {
    let url = format!("{}/v1/messages", base_url.trim_end_matches('/'));
    // No `temperature`: current Claude models reject it. Thinking off: this is a
    // quick rewrite, and thinking tokens would eat into max_tokens.
    let body = json!({
        "model": model,
        "stream": true,
        "max_tokens": gen.max_tokens,
        "system": system,
        "thinking": {"type": "disabled"},
        "messages": [{"role": "user", "content": user}],
    });
    let resp = http_client(gen, local)?
        .post(&url)
        .header("x-api-key", &key.0)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| http_error(e, gen))
        .with_context(|| format!("POST {url}"))?;
    let resp = check_status(resp).await?;
    if is_plain_json(&resp) {
        let v: Value = resp.json().await.map_err(|e| http_error(e, gen))?;
        check_stop(v["stop_reason"].as_str(), gen)?;
        let text: String = v["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect();
        on_chunk(&text);
        return Ok(text);
    }

    let mut full = String::new();
    sse(resp, gen, |data| {
        let piece = anthropic_delta(data)?;
        if let Some(t) = piece.text {
            full.push_str(&t);
            on_chunk(&t);
        }
        check_stop(piece.stop.as_deref(), gen)
    })
    .await?;
    Ok(full)
}

/// Parse one Anthropic SSE `data:` payload (thinking blocks are ignored).
pub fn anthropic_delta(data: &str) -> Result<Piece> {
    let v: Value = serde_json::from_str(data).with_context(|| format!("bad SSE data: {data}"))?;
    match v["type"].as_str() {
        Some("content_block_delta") if v["delta"]["type"] == "text_delta" => Ok(Piece {
            text: v["delta"]["text"].as_str().map(str::to_string),
            stop: None,
        }),
        Some("message_delta") => Ok(Piece {
            text: None,
            stop: v["delta"]["stop_reason"].as_str().map(str::to_string),
        }),
        Some("error") => bail!(
            "{}",
            v["error"]["message"].as_str().unwrap_or("provider error")
        ),
        _ => Ok(Piece::default()),
    }
}

async fn check_status(resp: reqwest::Response) -> Result<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let text = resp.text().await.unwrap_or_default();
    Err(anyhow!(
        "provider returned {status}: {}",
        text.chars().take(500).collect::<String>()
    ))
}

/// Minimal server-sent-events reader: calls `on_data` with the payload of each
/// `data:` line. Comment lines (`: keep-alive`) and other fields are skipped.
async fn sse(
    resp: reqwest::Response,
    gen: &Gen,
    mut on_data: impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        buf.extend_from_slice(&chunk.map_err(|e| http_error(e, gen))?);
        // Split on bytes so a multi-byte character cut between chunks survives.
        while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\n', '\r']);
            if let Some(data) = line.strip_prefix("data:") {
                on_data(data.trim())?;
            }
        }
    }
    // A last line without a trailing newline.
    let rest = String::from_utf8_lossy(&buf);
    if let Some(data) = rest.trim_end().strip_prefix("data:") {
        on_data(data.trim())?;
    }
    Ok(())
}

/// Some servers ignore `"stream": true` and answer with one JSON document.
fn is_plain_json(resp: &reqwest::Response) -> bool {
    resp.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("application/json"))
}

// ---------------------------------------------------------------------------
// Local programs
// ---------------------------------------------------------------------------

/// Claude Code in print mode: no tools, no MCP servers, no hooks, no session
/// saved, run from a private empty dir so no project CLAUDE.md is picked up.
/// Uses the user's own Claude login (subscription or API key).
pub fn claude_code_argv(bin: &Path, model: &str, system: &str) -> Vec<String> {
    let mut argv: Vec<String> = [
        &bin.display().to_string(),
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--tools",
        "",
        "--system-prompt",
        system,
        "--no-session-persistence",
        "--strict-mcp-config",
        "--disable-slash-commands",
        // Empty: no user/project settings, so ~/.claude/CLAUDE.md is not
        // injected. The login still works (checked with `claude -p`).
        "--setting-sources",
        "",
        "--settings",
        r#"{"disableAllHooks":true}"#,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if !model.is_empty() {
        argv.push("--model".into());
        argv.push(model.into());
    }
    argv
}

/// What one line of Claude Code's `stream-json` output means for us.
#[derive(Debug, PartialEq)]
pub enum ClaudeEvent {
    Delta(String),
    /// Final `result` line: the whole answer.
    Done(String),
    Failed(String),
    Other,
}

pub fn claude_code_event(line: &str) -> ClaudeEvent {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return ClaudeEvent::Other;
    };
    match v["type"].as_str() {
        Some("stream_event") => {
            let e = &v["event"];
            if e["type"] == "content_block_delta" && e["delta"]["type"] == "text_delta" {
                if let Some(t) = e["delta"]["text"].as_str() {
                    return ClaudeEvent::Delta(t.to_string());
                }
            }
            if e["type"] == "message_delta" && e["delta"]["stop_reason"] == "max_tokens" {
                return ClaudeEvent::Failed("the answer was cut off (max_tokens)".into());
            }
            ClaudeEvent::Other
        }
        Some("result") => {
            let text = v["result"].as_str().unwrap_or_default().to_string();
            if v["is_error"].as_bool().unwrap_or(false) {
                let msg = if text.is_empty() {
                    v["subtype"].as_str().unwrap_or("error").to_string()
                } else {
                    text
                };
                ClaudeEvent::Failed(msg)
            } else {
                ClaudeEvent::Done(text)
            }
        }
        _ => ClaudeEvent::Other,
    }
}

async fn claude_code(
    bin: &Path,
    env: &ChildEnv,
    model: &str,
    system: &str,
    user: &str,
    mut on_chunk: impl FnMut(&str),
) -> Result<String> {
    let argv = claude_code_argv(bin, model, system);
    let mut full = String::new();
    let mut streamed = false;
    let mut failure: Option<String> = None;
    let mut pending = Vec::new();
    let out = run_process(&argv, env, user, |bytes| {
        pending.extend_from_slice(bytes);
        while let Some(pos) = pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = pending.drain(..=pos).collect();
            match claude_code_event(&String::from_utf8_lossy(&line)) {
                ClaudeEvent::Delta(t) => {
                    streamed = true;
                    full.push_str(&t);
                    on_chunk(&t);
                }
                ClaudeEvent::Done(t) if !streamed => {
                    full.push_str(&t);
                    on_chunk(&t);
                }
                ClaudeEvent::Failed(m) => failure = Some(m),
                _ => {}
            }
        }
    })
    .await?;
    if let Some(m) = failure {
        bail!("{m}");
    }
    out.check()?;
    Ok(full)
}

/// `{system}` and `{model}` are substituted in argv. Without `{system}` the
/// system prompt goes on stdin before the text. A `{model}` with no model set
/// is dropped together with the flag in front of it.
pub fn command_invocation(
    template: &[String],
    model: &str,
    system: &str,
    user: &str,
) -> (Vec<String>, String) {
    let mut argv = Vec::with_capacity(template.len());
    let mut system_in_argv = false;
    for (i, arg) in template.iter().enumerate() {
        if arg.contains("{model}") && model.is_empty() {
            if i > 0 && template[i - 1].starts_with('-') && argv.len() > 1 {
                argv.pop();
            }
            continue;
        }
        if arg.contains("{system}") {
            system_in_argv = true;
        }
        argv.push(arg.replace("{system}", system).replace("{model}", model));
    }
    let stdin = if system_in_argv || system.is_empty() {
        user.to_string()
    } else {
        format!("{system}\n\n---\n\n{user}")
    };
    (argv, stdin)
}

/// Decode as much of `pending` as is complete UTF-8, replacing invalid bytes;
/// an incomplete character at the end stays in `pending` for the next read.
fn take_utf8(pending: &mut Vec<u8>) -> String {
    let mut out = String::new();
    loop {
        match std::str::from_utf8(pending) {
            Ok(s) => {
                out.push_str(s);
                pending.clear();
                return out;
            }
            Err(e) => {
                let valid = e.valid_up_to();
                out.push_str(std::str::from_utf8(&pending[..valid]).unwrap_or_default());
                match e.error_len() {
                    // Truly invalid bytes: replace and keep going.
                    Some(bad) => {
                        out.push(char::REPLACEMENT_CHARACTER);
                        pending.drain(..valid + bad);
                    }
                    // A character split across reads: wait for the rest.
                    None => {
                        pending.drain(..valid);
                        return out;
                    }
                }
            }
        }
    }
}

async fn command(
    template: &[String],
    env: &ChildEnv,
    model: &str,
    system: &str,
    user: &str,
    mut on_chunk: impl FnMut(&str),
) -> Result<String> {
    let (argv, stdin) = command_invocation(template, model, system, user);
    let mut full = String::new();
    let mut pending: Vec<u8> = Vec::new();
    let out = run_process(&argv, env, &stdin, |bytes| {
        pending.extend_from_slice(bytes);
        let text = take_utf8(&mut pending);
        if !text.is_empty() {
            full.push_str(&text);
            on_chunk(&text);
        }
    })
    .await?;
    if !pending.is_empty() {
        let rest = String::from_utf8_lossy(&pending).into_owned();
        full.push_str(&rest);
        on_chunk(&rest);
    }
    out.check()?;
    Ok(full)
}

struct ProcessOutput {
    program: String,
    status: std::process::ExitStatus,
    stderr: String,
}

impl ProcessOutput {
    fn check(&self) -> Result<()> {
        if self.status.success() {
            return Ok(());
        }
        let tail: String = {
            let s = self.stderr.trim();
            let start = s.len().saturating_sub(600);
            let start = (start..s.len())
                .find(|&i| s.is_char_boundary(i))
                .unwrap_or(0);
            s[start..].to_string()
        };
        bail!("{} exited with {}: {}", self.program, self.status, tail)
    }
}

/// Kills a whole process group when dropped, so helpers that a program starts
/// (the npm `codex` wrapper starts the real binary, for example) do not keep
/// running after a timeout or a cancel.
pub struct ProcessGroup(Option<i32>);

impl ProcessGroup {
    /// `pid` of a child spawned with `process_group(0)`, so its pgid equals its pid.
    pub fn new(pid: Option<u32>) -> Self {
        ProcessGroup(pid.and_then(|p| i32::try_from(p).ok()).filter(|p| *p > 0))
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if let Some(pgid) = self.0 {
            // SAFETY: plain syscall; a group that is already gone just gives ESRCH.
            unsafe {
                libc::kill(-pgid, libc::SIGKILL);
            }
        }
    }
}

/// A private, empty directory to run agent CLIs in: never the shared /tmp,
/// where anyone could plant an AGENTS.md, GEMINI.md or .env for them to read.
fn work_dir() -> PathBuf {
    use std::os::unix::fs::DirBuilderExt;
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            directories::ProjectDirs::from("", "", "mirzabenevis").map(|d| d.cache_dir().into())
        })
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("mirzabenevis").join("work");
    let _ = std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir);
    dir
}

/// Run a program in a private dir, feed it `stdin`, and hand stdout to
/// `on_stdout` as it arrives. The program and anything it started are killed
/// if this future is dropped (timeout, cancel) or once it has finished.
async fn run_process(
    argv: &[String],
    env: &ChildEnv,
    stdin: &str,
    mut on_stdout: impl FnMut(&[u8]),
) -> Result<ProcessOutput> {
    let (program, args) = argv.split_first().context("empty command")?;
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .current_dir(work_dir())
        .envs(env.0.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true);
    let mut child = cmd.spawn().with_context(|| format!("starting {program}"))?;
    let _group = ProcessGroup::new(child.id());

    // Write stdin concurrently so a child that prints before reading cannot deadlock.
    let mut child_stdin = child.stdin.take().context("stdin")?;
    let input = stdin.as_bytes().to_vec();
    let writer = tokio::spawn(async move {
        let _ = child_stdin.write_all(&input).await;
        // Dropping closes the pipe: EOF for the child.
    });
    let mut child_stderr = child.stderr.take().context("stderr")?;
    let reader = tokio::spawn(async move {
        let mut s = Vec::new();
        let _ = child_stderr.read_to_end(&mut s).await;
        String::from_utf8_lossy(&s).into_owned()
    });

    let mut stdout = child.stdout.take().context("stdout")?;
    let mut buf = [0u8; 8192];
    loop {
        let n = stdout.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        on_stdout(&buf[..n]);
    }
    let status = child.wait().await?;
    writer.abort();
    // A leftover grandchild may still hold stderr open; don't wait for it forever.
    let stderr = tokio::time::timeout(Duration::from_secs(2), reader)
        .await
        .ok()
        .and_then(|r| r.ok())
        .unwrap_or_default();
    let name = Path::new(program)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| program.clone());
    Ok(ProcessOutput {
        program: name,
        status,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::{Category, Transport};

    fn gen() -> Gen {
        Gen {
            max_tokens: 100,
            temperature: 0.3,
            timeout: Duration::from_secs(10),
        }
    }

    #[test]
    fn openai_body_respects_wire_options() {
        let w = OpenAiWire {
            base_url: "x".into(),
            api_key: None,
            extra: json!({"reasoning_effort": "none"}),
            temperature: false,
            max_tokens_field: "max_completion_tokens",
            local: false,
        };
        let b = openai_body(&w, "m", &gen(), "sys", "hi");
        assert_eq!(b["max_completion_tokens"], 100);
        assert!(b.get("max_tokens").is_none());
        assert!(b.get("temperature").is_none());
        assert_eq!(b["reasoning_effort"], "none");
        assert_eq!(b["messages"][0]["content"], "sys");
        let w = OpenAiWire {
            temperature: true,
            ..w
        };
        // f64 all the way, so 0.3 stays 0.3 on the wire.
        assert_eq!(
            openai_body(&w, "m", &gen(), "s", "u")["temperature"].to_string(),
            "0.3"
        );
    }

    #[test]
    fn sse_payloads() {
        assert_eq!(openai_delta("[DONE]").unwrap(), Piece::default());
        let d = r#"{"choices":[{"delta":{"content":"hé"}}]}"#;
        assert_eq!(openai_delta(d).unwrap().text.as_deref(), Some("hé"));
        let reasoning = r#"{"choices":[{"delta":{"reasoning_content":"hmm"}}]}"#;
        assert_eq!(openai_delta(reasoning).unwrap().text, None);
        let cut = r#"{"choices":[{"delta":{},"finish_reason":"length"}]}"#;
        assert_eq!(openai_delta(cut).unwrap().stop.as_deref(), Some("length"));
        let err = r#"{"error":{"message":"rate limited"},"choices":[{"finish_reason":"error"}]}"#;
        assert!(openai_delta(err)
            .unwrap_err()
            .to_string()
            .contains("rate limited"));

        let a = r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"ok"}}"#;
        assert_eq!(anthropic_delta(a).unwrap().text.as_deref(), Some("ok"));
        let t =
            r#"{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"x"}}"#;
        assert_eq!(anthropic_delta(t).unwrap(), Piece::default());
        let m = r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"}}"#;
        assert_eq!(
            anthropic_delta(m).unwrap().stop.as_deref(),
            Some("max_tokens")
        );
        let e = r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;
        assert!(anthropic_delta(e)
            .unwrap_err()
            .to_string()
            .contains("Overloaded"));

        assert!(check_stop(Some("length"), &gen()).is_err());
        assert!(check_stop(Some("max_tokens"), &gen()).is_err());
        assert!(check_stop(Some("stop"), &gen()).is_ok());
        assert!(check_stop(Some("end_turn"), &gen()).is_ok());
    }

    #[test]
    fn claude_code_events() {
        let delta = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"This is"}}}"#;
        assert_eq!(
            claude_code_event(delta),
            ClaudeEvent::Delta("This is".into())
        );
        let think = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":""}}}"#;
        assert_eq!(claude_code_event(think), ClaudeEvent::Other);
        let done =
            r#"{"type":"result","subtype":"success","is_error":false,"result":"This is a test."}"#;
        assert_eq!(
            claude_code_event(done),
            ClaudeEvent::Done("This is a test.".into())
        );
        let fail = r#"{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login"}"#;
        assert_eq!(
            claude_code_event(fail),
            ClaudeEvent::Failed("Not logged in · Please run /login".into())
        );
        assert_eq!(claude_code_event("not json"), ClaudeEvent::Other);
    }

    #[test]
    fn claude_code_argv_disables_tools_and_hooks() {
        let a = claude_code_argv(Path::new("/bin/claude"), "haiku", "SYS");
        let after = |flag: &str| a[a.iter().position(|x| x == flag).unwrap() + 1].clone();
        assert_eq!(after("--tools"), "");
        assert_eq!(after("--setting-sources"), "");
        assert_eq!(after("--system-prompt"), "SYS");
        assert!(after("--settings").contains("disableAllHooks"));
        assert!(
            !a.contains(&"--bare".to_string()),
            "--bare would ignore the Claude login"
        );
        assert_eq!(&a[a.len() - 2..], ["--model", "haiku"]);
        assert!(!claude_code_argv(Path::new("c"), "", "s").contains(&"--model".to_string()));
    }

    #[test]
    fn command_substitution() {
        let t: Vec<String> = ["llm", "-s", "{system}", "-m", "{model}"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (argv, stdin) = command_invocation(&t, "", "SYS", "text");
        assert_eq!(argv, ["llm", "-s", "SYS"]);
        assert_eq!(stdin, "text");
        let (argv, _) = command_invocation(&t, "gpt", "SYS", "text");
        assert_eq!(argv, ["llm", "-s", "SYS", "-m", "gpt"]);
        let (argv, stdin) = command_invocation(&["cat".to_string()], "", "SYS", "text");
        assert_eq!(argv, ["cat"]);
        assert_eq!(stdin, "SYS\n\n---\n\ntext");
    }

    #[test]
    fn utf8_decoding_keeps_split_chars_and_skips_bad_bytes() {
        let mut p = "hé".as_bytes()[..2].to_vec(); // "h" + first byte of "é"
        assert_eq!(take_utf8(&mut p), "h");
        assert_eq!(p.len(), 1);
        p.extend_from_slice(&"é".as_bytes()[1..]);
        assert_eq!(take_utf8(&mut p), "é");
        let mut p = b"a\xffb".to_vec();
        assert_eq!(take_utf8(&mut p), "a\u{fffd}b");
        assert!(p.is_empty());
    }

    fn cmd_backend(argv: &[&str]) -> Backend {
        Backend {
            id: "custom".into(),
            label: "Custom".into(),
            category: Category::Custom,
            model: String::new(),
            source: String::new(),
            transport: Transport::Command {
                argv: argv.iter().map(|s| s.to_string()).collect(),
                env: ChildEnv::default(),
            },
        }
    }

    #[tokio::test]
    async fn command_backend_streams_and_reports_failures() {
        let b = cmd_backend(&["sh", "-c", "tr a-z A-Z"]);
        let mut chunks = Vec::new();
        let out = stream(&b, &gen(), "", "héllo wörld", |c| {
            chunks.push(c.to_string())
        })
        .await
        .unwrap();
        assert_eq!(out, "HéLLO WöRLD");
        assert_eq!(chunks.concat(), out);

        let b = cmd_backend(&["sh", "-c", "echo boom >&2; exit 3"]);
        let e = format!("{:#}", complete(&b, &gen(), "", "x").await.unwrap_err());
        assert!(e.contains("boom") && e.contains("exit status: 3"), "{e}");

        let b = cmd_backend(&["sh", "-c", "cat >/dev/null; printf '  \\n'"]);
        let e = format!("{:#}", complete(&b, &gen(), "", "x").await.unwrap_err());
        assert!(e.contains("empty answer"), "{e}");

        let b = cmd_backend(&["sh", "-c", "sleep 5"]);
        let mut g = gen();
        g.timeout = Duration::from_millis(200);
        let e = format!("{:#}", complete(&b, &g, "", "x").await.unwrap_err());
        assert!(e.contains("did not answer"), "{e}");
    }

    #[tokio::test]
    async fn timeout_kills_grandchildren() {
        let marker = format!("sleep {}", 900_000 + std::process::id() % 99_999);
        let script = format!("sh -c '{marker}' & wait");
        let b = cmd_backend(&["sh", "-c", &script]);
        let mut g = gen();
        g.timeout = Duration::from_millis(300);
        assert!(complete(&b, &g, "", "x").await.is_err());
        tokio::time::sleep(Duration::from_millis(200)).await;
        let ps = std::process::Command::new("ps")
            .args(["-eo", "args"])
            .output()
            .unwrap();
        let ps = String::from_utf8_lossy(&ps.stdout);
        assert!(!ps.contains(&marker), "grandchild still running");
    }

    #[tokio::test]
    async fn claude_code_transport_with_fake_binary() {
        let dir = std::env::temp_dir().join(format!("mzb-claude-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("claude");
        std::fs::write(
            &bin,
            r#"#!/bin/sh
cat > /dev/null
echo '{"type":"system","subtype":"init"}'
echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"This "}}}'
echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"is fine."}}}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"This is fine."}'
"#,
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let b = Backend {
            id: "claude-code".into(),
            label: "Claude Code".into(),
            category: Category::Agent,
            model: "haiku".into(),
            source: String::new(),
            transport: Transport::ClaudeCode {
                bin,
                env: ChildEnv::default(),
            },
        };
        assert_eq!(
            complete(&b, &gen(), "s", "u").await.unwrap(),
            "This is fine."
        );
    }
}
