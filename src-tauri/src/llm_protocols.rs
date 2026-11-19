//! Multi-protocol HTTP transport, transient retry/backoff, native SSE streaming turns
//! (OpenAI-compatible, Anthropic Claude, Google Gemini), one-shot multimodal vision calls,
//! and endpoint connectivity verification.

use crate::agent_tools::{anthropic_tools_json, gemini_tools_json, openai_tools_json};
use crate::mcp::McpToolInfo;
use crate::ai_client::InlineImageFrame;
use crate::settings::ResolvedModelConfig;
use crate::thinking::{
    apply_native_thinking, clamp_thinking_level, classify_protocol,
    infer_thinking_capability_with_mode, is_thinking_param_rejection, normalize_thinking_level,
    strip_native_thinking, ApiProtocol, ThinkingBackend, ThoughtTagSplitter,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter};

#[derive(Clone, Serialize)]
pub struct AiChunkPayload {
    pub stream_id: String,
    pub delta: String,
}

#[derive(Clone, Serialize)]
pub struct AiThoughtPayload {
    pub stream_id: String,
    pub delta: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ParsedToolCall {
    pub id: String,
    pub name: String,
    pub args: Value,
    pub raw_arguments: String,
}

pub(crate) enum TurnOutcome {
    Finished {
        turn_text: String,
        turn_thought: String,
    },
    ToolCalls {
        turn_text: String,
        turn_thought: String,
        native_reasoning: String,
        calls: Vec<ParsedToolCall>,
        gemini_raw_parts: Vec<Value>,
        anthropic_raw_blocks: Vec<Value>,
    },
}

pub(crate) fn emit_chunk(app: &AppHandle, stream_id: &str, delta: &str) {
    if !delta.is_empty() {
        let _ = app.emit(
            "buddy://ai-chunk",
            AiChunkPayload {
                stream_id: stream_id.to_string(),
                delta: delta.to_string(),
            },
        );
    }
}

pub(crate) fn emit_thought(app: &AppHandle, stream_id: &str, delta: &str) {
    if !delta.is_empty() {
        let _ = app.emit(
            "buddy://ai-thought",
            AiThoughtPayload {
                stream_id: stream_id.to_string(),
                delta: delta.to_string(),
            },
        );
    }
}

fn record_split_output(
    app: &AppHandle,
    stream_id: &str,
    turn_thought: &mut String,
    turn_text: &mut String,
    thought_part: &str,
    answer_part: &str,
) {
    if !thought_part.is_empty() {
        turn_thought.push_str(thought_part);
        emit_thought(app, stream_id, thought_part);
    }
    if !answer_part.is_empty() {
        turn_text.push_str(answer_part);
        emit_chunk(app, stream_id, answer_part);
    }
}

fn retry_delay_ms(attempt: usize, retry_after_secs: Option<u64>) -> u64 {
    let base_ms: u64 = match attempt {
        0 => 1200,
        1 => 2600,
        _ => 5200,
    };
    let jitter_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.subsec_nanos() % 350) as u64)
        .unwrap_or(0);
    let computed = base_ms + jitter_ms;
    retry_after_secs
        .map(|s| s.clamp(1, 15) * 1000)
        .unwrap_or(0)
        .max(computed)
        .min(15_000)
}

async fn sleep_cancellable(ms: u64, cancel_flag: Option<&AtomicBool>) -> bool {
    let mut elapsed = 0_u64;
    while elapsed < ms {
        if cancel_flag.is_some_and(|f| f.load(Ordering::Relaxed)) {
            return false;
        }
        let step = (ms - elapsed).min(120);
        tokio::time::sleep(std::time::Duration::from_millis(step)).await;
        elapsed += step;
    }
    !cancel_flag.is_some_and(|f| f.load(Ordering::Relaxed))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_post_with_retry(
    client: &reqwest::Client,
    protocol: ApiProtocol,
    endpoint: &str,
    api_key: &str,
    body: &mut Value,
    native_thinking_injected: bool,
    cancel_flag: Option<&AtomicBool>,
    max_attempts: usize,
) -> Result<reqwest::Response, String> {
    let mut downgraded = false;
    let mut attempt = 0_usize;
    let max_tries = max_attempts.max(1);

    loop {
        if cancel_flag.is_some_and(|f| f.load(Ordering::Relaxed)) {
            return Err("Cancelled by user.".to_string());
        }

        let mut req = client.post(endpoint);
        match protocol {
            ApiProtocol::Gemini => {
                req = req.header("x-goog-api-key", api_key.trim());
            }
            ApiProtocol::Anthropic => {
                req = req
                    .header("x-api-key", api_key.trim())
                    .header("anthropic-version", "2023-06-01");
            }
            ApiProtocol::OpenAi => {
                if !api_key.trim().is_empty() {
                    req = req.bearer_auth(api_key.trim());
                }
            }
        }
        let resp_res = req.json(body).send().await;

        match resp_res {
            Err(e) => {
                if (e.is_timeout() || e.is_connect()) && attempt + 1 < max_tries {
                    let wait_ms = retry_delay_ms(attempt, None);
                    attempt += 1;
                    crate::logger::log_warn(
                        "ai",
                        &format!("transient network error ({e}); retrying in {wait_ms}ms"),
                    );
                    if !sleep_cancellable(wait_ms, cancel_flag).await {
                        return Err("Cancelled by user.".to_string());
                    }
                    continue;
                }
                return Err(format!("Request failed: {e}"));
            }
            Ok(resp) => {
                if resp.status().is_success() {
                    return Ok(resp);
                }
                let status = resp.status();
                let retry_after = resp
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.trim().parse::<u64>().ok());
                let err_txt = resp.text().await.unwrap_or_default();

                if native_thinking_injected
                    && !downgraded
                    && is_thinking_param_rejection(status, &err_txt)
                {
                    strip_native_thinking(body, protocol);
                    downgraded = true;
                    crate::logger::log_warn(
                        "ai",
                        &format!(
                            "API rejected native thinking params ({status}); retrying without native thinking"
                        ),
                    );
                    continue;
                }

                let is_transient = matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504);
                if is_transient && attempt + 1 < max_tries {
                    let wait_ms = retry_delay_ms(attempt, retry_after);
                    attempt += 1;
                    crate::logger::log_warn(
                        "ai",
                        &format!(
                            "transient HTTP {status} on attempt {attempt}; retrying in {wait_ms}ms"
                        ),
                    );
                    if !sleep_cancellable(wait_ms, cancel_flag).await {
                        return Err("Cancelled by user.".to_string());
                    }
                    continue;
                }

                return Err(format!(
                    "API returned {status}: {}",
                    err_txt.chars().take(240).collect::<String>()
                ));
            }
        }
    }
}

#[derive(Default)]
struct OpenAiPartialTool {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
struct AnthropicBlock {
    kind: String,
    id: String,
    name: String,
    partial_json: String,
    text: String,
    thinking_text: String,
    signature: String,
    redacted_data: String,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_openai_turn(
    app: &AppHandle,
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    model_id: &str,
    messages: &[Value],
    agent_mode: bool,
    include_media_downloader: bool,
    mcp_tools: &[McpToolInfo],
    thinking_level: &str,
    thinking_backend: ThinkingBackend,
    stream_id: &str,
    turn_idx: usize,
    cancel_flag: &AtomicBool,
) -> Result<TurnOutcome, String> {
    let endpoint = format!("{base_url}/chat/completions");
    let mut body = json!({
        "model": model_id,
        "stream": true,
        "messages": messages
    });
    let native_thinking = apply_native_thinking(&mut body, thinking_backend, thinking_level);
    if agent_mode {
        body["tools"] = openai_tools_json(include_media_downloader, mcp_tools);
        body["tool_choice"] = json!("auto");
    }

    let mut resp = send_post_with_retry(
        client,
        ApiProtocol::OpenAi,
        &endpoint,
        api_key,
        &mut body,
        native_thinking,
        Some(cancel_flag),
        4,
    )
    .await?;

    let mut turn_text = String::new();
    let mut turn_thought = String::new();
    let mut native_reasoning = String::new();
    let mut splitter = ThoughtTagSplitter::new(thinking_level != "off");
    let mut sse_buffer = String::new();
    let mut partial_tools: BTreeMap<usize, OpenAiPartialTool> = BTreeMap::new();
    let mut stream_finished = false;

    while !stream_finished {
        if cancel_flag.load(Ordering::Relaxed) {
            break;
        }
        match resp.chunk().await {
            Ok(Some(bytes)) => {
                sse_buffer.push_str(&String::from_utf8_lossy(&bytes));
                while let Some(newline_pos) = sse_buffer.find('\n') {
                    let line = sse_buffer[..newline_pos].trim().to_string();
                    sse_buffer.drain(..=newline_pos);
                    if !line.starts_with("data:") {
                        continue;
                    }
                    let data = line.trim_start_matches("data:").trim();
                    if data == "[DONE]" {
                        stream_finished = true;
                        break;
                    }
                    if let Ok(parsed) = serde_json::from_str::<Value>(data) {
                        if let Some(err_msg) = parsed["error"]["message"].as_str() {
                            return Err(err_msg.to_string());
                        }
                        let delta_obj = &parsed["choices"][0]["delta"];
                        // Native OpenAI/DeepSeek/Qwen reasoning_content stream
                        if let Some(rc) = delta_obj["reasoning_content"]
                            .as_str()
                            .or_else(|| delta_obj["reasoning"].as_str())
                        {
                            if !rc.is_empty() {
                                native_reasoning.push_str(rc);
                                if thinking_level != "off" {
                                    turn_thought.push_str(rc);
                                    emit_thought(app, stream_id, rc);
                                }
                            }
                        }
                        if let Some(delta) = delta_obj["content"].as_str() {
                            let (th, ans) = splitter.push(delta);
                            record_split_output(
                                app,
                                stream_id,
                                &mut turn_thought,
                                &mut turn_text,
                                &th,
                                &ans,
                            );
                        }
                        if let Some(tcs) = delta_obj["tool_calls"].as_array() {
                            for (fallback_idx, tc) in tcs.iter().enumerate() {
                                let idx =
                                    tc["index"].as_u64().unwrap_or(fallback_idx as u64) as usize;
                                let entry = partial_tools.entry(idx).or_default();
                                if let Some(id) = tc["id"].as_str() {
                                    if !id.is_empty() {
                                        entry.id = id.to_string();
                                    }
                                }
                                if let Some(name) = tc["function"]["name"].as_str() {
                                    entry.name.push_str(name);
                                }
                                if let Some(args) = tc["function"]["arguments"].as_str() {
                                    entry.arguments.push_str(args);
                                }
                            }
                        }
                    }
                }
            }
            Ok(None) => break,
            Err(e) => return Err(format!("Stream interrupted: {e}")),
        }
    }

    let (th_tail, ans_tail) = splitter.finish();
    record_split_output(
        app,
        stream_id,
        &mut turn_thought,
        &mut turn_text,
        &th_tail,
        &ans_tail,
    );

    if partial_tools.is_empty() {
        Ok(TurnOutcome::Finished {
            turn_text,
            turn_thought,
        })
    } else {
        let calls = partial_tools
            .into_iter()
            .filter(|(_, t)| !t.name.trim().is_empty())
            .map(|(idx, t)| {
                let raw_arguments = if t.arguments.trim().is_empty() {
                    "{}".to_string()
                } else {
                    t.arguments.clone()
                };
                let args: Value =
                    serde_json::from_str(&raw_arguments).unwrap_or_else(|_| json!({}));
                let id = if t.id.is_empty() {
                    format!("call_{turn_idx}_{idx}")
                } else {
                    t.id
                };
                ParsedToolCall {
                    id,
                    name: t.name.trim().to_string(),
                    args,
                    raw_arguments,
                }
            })
            .collect();
        Ok(TurnOutcome::ToolCalls {
            turn_text,
            turn_thought,
            native_reasoning,
            calls,
            gemini_raw_parts: Vec::new(),
            anthropic_raw_blocks: Vec::new(),
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_anthropic_turn(
    app: &AppHandle,
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    model_id: &str,
    sys_prompt: &str,
    messages: &[Value],
    agent_mode: bool,
    include_media_downloader: bool,
    mcp_tools: &[McpToolInfo],
    thinking_level: &str,
    thinking_backend: ThinkingBackend,
    stream_id: &str,
    turn_idx: usize,
    cancel_flag: &AtomicBool,
) -> Result<TurnOutcome, String> {
    let endpoint = if base_url.ends_with("/v1") {
        format!("{base_url}/messages")
    } else {
        format!("{base_url}/v1/messages")
    };
    let mut body = json!({
        "model": model_id,
        "max_tokens": 4096,
        "stream": true,
        "system": sys_prompt,
        "messages": messages
    });
    let native_thinking = apply_native_thinking(&mut body, thinking_backend, thinking_level);
    if agent_mode {
        body["tools"] = anthropic_tools_json(include_media_downloader, mcp_tools);
    }

    let mut resp = send_post_with_retry(
        client,
        ApiProtocol::Anthropic,
        &endpoint,
        api_key,
        &mut body,
        native_thinking,
        Some(cancel_flag),
        4,
    )
    .await?;

    let mut turn_text = String::new();
    let mut turn_thought = String::new();
    let mut splitter = ThoughtTagSplitter::new(thinking_level != "off");
    let mut sse_buffer = String::new();
    let mut blocks: BTreeMap<usize, AnthropicBlock> = BTreeMap::new();
    let mut stream_finished = false;

    while !stream_finished {
        if cancel_flag.load(Ordering::Relaxed) {
            break;
        }
        match resp.chunk().await {
            Ok(Some(bytes)) => {
                sse_buffer.push_str(&String::from_utf8_lossy(&bytes));
                while let Some(newline_pos) = sse_buffer.find('\n') {
                    let line = sse_buffer[..newline_pos].trim().to_string();
                    sse_buffer.drain(..=newline_pos);
                    if !line.starts_with("data:") {
                        continue;
                    }
                    let data = line.trim_start_matches("data:").trim();
                    if let Ok(parsed) = serde_json::from_str::<Value>(data) {
                        let evt_type = parsed["type"].as_str().unwrap_or("");
                        match evt_type {
                            "content_block_start" => {
                                let idx = parsed["index"].as_u64().unwrap_or(0) as usize;
                                let cb = &parsed["content_block"];
                                let btype = cb["type"].as_str().unwrap_or("").to_string();
                                blocks.insert(
                                    idx,
                                    AnthropicBlock {
                                        kind: btype,
                                        id: cb["id"].as_str().unwrap_or("").to_string(),
                                        name: cb["name"].as_str().unwrap_or("").to_string(),
                                        partial_json: String::new(),
                                        text: cb["text"].as_str().unwrap_or("").to_string(),
                                        thinking_text: cb["thinking"]
                                            .as_str()
                                            .unwrap_or("")
                                            .to_string(),
                                        signature: cb["signature"]
                                            .as_str()
                                            .unwrap_or("")
                                            .to_string(),
                                        redacted_data: cb["data"].as_str().unwrap_or("").to_string(),
                                    },
                                );
                            }
                            "content_block_delta" => {
                                let idx = parsed["index"].as_u64().unwrap_or(0) as usize;
                                let dtype = parsed["delta"]["type"].as_str().unwrap_or("");
                                if dtype == "thinking_delta" {
                                    if let Some(th) = parsed["delta"]["thinking"].as_str() {
                                        if let Some(b) = blocks.get_mut(&idx) {
                                            b.thinking_text.push_str(th);
                                        }
                                        if !th.is_empty() && thinking_level != "off" {
                                            turn_thought.push_str(th);
                                            emit_thought(app, stream_id, th);
                                        }
                                    }
                                } else if dtype == "signature_delta" {
                                    if let Some(sig) = parsed["delta"]["signature"].as_str() {
                                        if let Some(b) = blocks.get_mut(&idx) {
                                            b.signature.push_str(sig);
                                        }
                                    }
                                } else if dtype == "text_delta" || parsed["delta"]["text"].is_string() {
                                    if let Some(delta) = parsed["delta"]["text"].as_str() {
                                        if let Some(b) = blocks.get_mut(&idx) {
                                            b.text.push_str(delta);
                                        }
                                        let (th, ans) = splitter.push(delta);
                                        record_split_output(
                                            app,
                                            stream_id,
                                            &mut turn_thought,
                                            &mut turn_text,
                                            &th,
                                            &ans,
                                        );
                                    }
                                } else if dtype == "input_json_delta" {
                                    if let Some(part) = parsed["delta"]["partial_json"].as_str() {
                                        if let Some(b) = blocks.get_mut(&idx) {
                                            b.partial_json.push_str(part);
                                        }
                                    }
                                }
                            }
                            "message_stop" => {
                                stream_finished = true;
                                break;
                            }
                            "error" => {
                                let msg = parsed["error"]["message"]
                                    .as_str()
                                    .unwrap_or("Anthropic stream error");
                                return Err(msg.to_string());
                            }
                            _ => {}
                        }
                    }
                }
            }
            Ok(None) => break,
            Err(e) => return Err(format!("Stream interrupted: {e}")),
        }
    }

    let (th_tail, ans_tail) = splitter.finish();
    record_split_output(
        app,
        stream_id,
        &mut turn_thought,
        &mut turn_text,
        &th_tail,
        &ans_tail,
    );

    let mut anthropic_raw_blocks: Vec<Value> = Vec::new();
    let mut tool_calls: Vec<ParsedToolCall> = Vec::new();
    for (idx, b) in blocks {
        match b.kind.as_str() {
            "thinking" => {
                if !b.thinking_text.is_empty() || !b.signature.is_empty() {
                    anthropic_raw_blocks.push(json!({
                        "type": "thinking",
                        "thinking": b.thinking_text,
                        "signature": b.signature
                    }));
                }
            }
            "redacted_thinking" => {
                if !b.redacted_data.is_empty() {
                    anthropic_raw_blocks.push(json!({
                        "type": "redacted_thinking",
                        "data": b.redacted_data
                    }));
                }
            }
            "text" => {
                if !b.text.is_empty() {
                    anthropic_raw_blocks.push(json!({
                        "type": "text",
                        "text": b.text
                    }));
                }
            }
            "tool_use" if !b.name.trim().is_empty() => {
                let raw_arguments = if b.partial_json.trim().is_empty() {
                    "{}".to_string()
                } else {
                    b.partial_json
                };
                let args: Value =
                    serde_json::from_str(&raw_arguments).unwrap_or_else(|_| json!({}));
                let id = if b.id.is_empty() {
                    format!("toolu_{turn_idx}_{idx}")
                } else {
                    b.id
                };
                anthropic_raw_blocks.push(json!({
                    "type": "tool_use",
                    "id": id,
                    "name": b.name,
                    "input": args
                }));
                tool_calls.push(ParsedToolCall {
                    id,
                    name: b.name,
                    args,
                    raw_arguments,
                });
            }
            _ => {}
        }
    }

    if tool_calls.is_empty() {
        Ok(TurnOutcome::Finished {
            turn_text,
            turn_thought,
        })
    } else {
        Ok(TurnOutcome::ToolCalls {
            turn_text,
            turn_thought,
            native_reasoning: String::new(),
            calls: tool_calls,
            gemini_raw_parts: Vec::new(),
            anthropic_raw_blocks,
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_gemini_turn(
    app: &AppHandle,
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    model_id: &str,
    sys_prompt: &str,
    contents: &[Value],
    agent_mode: bool,
    include_media_downloader: bool,
    mcp_tools: &[McpToolInfo],
    thinking_level: &str,
    thinking_backend: ThinkingBackend,
    stream_id: &str,
    turn_idx: usize,
    cancel_flag: &AtomicBool,
) -> Result<TurnOutcome, String> {
    let model_clean = model_id.trim().trim_start_matches("models/");
    let endpoint = if base_url.ends_with("/v1beta") || base_url.ends_with("/v1") {
        format!("{base_url}/models/{model_clean}:streamGenerateContent?alt=sse")
    } else {
        format!("{base_url}/v1beta/models/{model_clean}:streamGenerateContent?alt=sse")
    };
    let mut body = json!({
        "systemInstruction": {
            "parts": [{ "text": sys_prompt }]
        },
        "contents": contents
    });

    let lvl = normalize_thinking_level(thinking_level);
    let native_thinking = apply_native_thinking(&mut body, thinking_backend, lvl);

    if agent_mode {
        body["tools"] = gemini_tools_json(include_media_downloader, mcp_tools);
    }

    let mut resp = send_post_with_retry(
        client,
        ApiProtocol::Gemini,
        &endpoint,
        api_key,
        &mut body,
        native_thinking,
        Some(cancel_flag),
        4,
    )
    .await?;

    let mut turn_text = String::new();
    let mut turn_thought = String::new();
    let mut splitter = ThoughtTagSplitter::new(lvl != "off");
    let mut sse_buffer = String::new();
    let mut raw_model_parts: Vec<Value> = Vec::new();
    let mut calls: Vec<ParsedToolCall> = Vec::new();

    loop {
        if cancel_flag.load(Ordering::Relaxed) {
            break;
        }
        match resp.chunk().await {
            Ok(Some(bytes)) => {
                sse_buffer.push_str(&String::from_utf8_lossy(&bytes));
                while let Some(newline_pos) = sse_buffer.find('\n') {
                    let line = sse_buffer[..newline_pos].trim().to_string();
                    sse_buffer.drain(..=newline_pos);
                    if !line.starts_with("data:") {
                        continue;
                    }
                    let data = line.trim_start_matches("data:").trim();
                    if data == "[DONE]" {
                        break;
                    }
                    if let Ok(parsed) = serde_json::from_str::<Value>(data) {
                        if let Some(err_msg) = parsed["error"]["message"].as_str() {
                            return Err(err_msg.to_string());
                        }
                        if let Some(parts) = parsed["candidates"][0]["content"]["parts"].as_array() {
                            for part in parts {
                                raw_model_parts.push(part.clone());
                                if part["thought"].as_bool() == Some(true) {
                                    if let Some(th) = part["text"].as_str() {
                                        if !th.is_empty() && lvl != "off" {
                                            turn_thought.push_str(th);
                                            emit_thought(app, stream_id, th);
                                        }
                                    }
                                    continue;
                                }
                                if let Some(fc) = part.get("functionCall") {
                                    if let Some(name) = fc["name"].as_str() {
                                        let args =
                                            fc.get("args").cloned().unwrap_or_else(|| json!({}));
                                        let idx = calls.len();
                                        calls.push(ParsedToolCall {
                                            id: format!("gem_{turn_idx}_{idx}"),
                                            name: name.to_string(),
                                            raw_arguments: args.to_string(),
                                            args,
                                        });
                                    }
                                    continue;
                                }
                                if let Some(delta) = part["text"].as_str() {
                                    let (th, ans) = splitter.push(delta);
                                    record_split_output(
                                        app,
                                        stream_id,
                                        &mut turn_thought,
                                        &mut turn_text,
                                        &th,
                                        &ans,
                                    );
                                }
                            }
                        }
                    }
                }
            }
            Ok(None) => break,
            Err(e) => return Err(format!("Stream interrupted: {e}")),
        }
    }

    let (th_tail, ans_tail) = splitter.finish();
    record_split_output(
        app,
        stream_id,
        &mut turn_thought,
        &mut turn_text,
        &th_tail,
        &ans_tail,
    );

    if calls.is_empty() {
        Ok(TurnOutcome::Finished {
            turn_text,
            turn_thought,
        })
    } else {
        Ok(TurnOutcome::ToolCalls {
            turn_text,
            turn_thought,
            native_reasoning: String::new(),
            calls,
            gemini_raw_parts: raw_model_parts,
            anthropic_raw_blocks: Vec::new(),
        })
    }
}

/// Non-streaming one-shot multimodal call used by the background screen sentinel & text-model fallback.
pub async fn run_oneshot_vision(
    client: &reqwest::Client,
    model: &ResolvedModelConfig,
    api_key: &str,
    sys_prompt: &str,
    user_prompt: &str,
    frames: &[InlineImageFrame],
    max_tokens: u32,
) -> Result<String, String> {
    let (is_local, is_anthropic, is_gemini, base_url) =
        classify_protocol(&model.api_provider, &model.base_url);

    if api_key.trim().is_empty() && !is_local {
        return Err("No API Key configured for Watch model.".to_string());
    }

    if is_gemini {
        let model_clean = model.model_id.trim().trim_start_matches("models/");
        let endpoint = if base_url.ends_with("/v1beta") || base_url.ends_with("/v1") {
            format!("{base_url}/models/{model_clean}:generateContent")
        } else {
            format!("{base_url}/v1beta/models/{model_clean}:generateContent")
        };
        let mut parts: Vec<Value> = Vec::new();
        for f in frames {
            parts.push(json!({
                "inlineData": { "mimeType": "image/jpeg", "data": f.jpeg_b64 }
            }));
        }
        parts.push(json!({ "text": user_prompt }));

        let mut body = json!({
            "contents": [{ "role": "user", "parts": parts }],
            "generationConfig": { "maxOutputTokens": max_tokens }
        });
        let cap = infer_thinking_capability_with_mode(
            &model.api_provider,
            &model.base_url,
            &model.model_id,
            &model.thinking_mode,
        );
        let native_thinking = match cap.backend {
            ThinkingBackend::GeminiBudget { .. } | ThinkingBackend::GeminiLevel { .. } => {
                let min_lvl = clamp_thinking_level("off", &cap);
                apply_native_thinking(&mut body, cap.backend, min_lvl)
            }
            _ => false,
        };
        if !sys_prompt.trim().is_empty() {
            body["systemInstruction"] = json!({ "parts": [{ "text": sys_prompt }] });
        }

        let resp = send_post_with_retry(
            client,
            ApiProtocol::Gemini,
            &endpoint,
            api_key,
            &mut body,
            native_thinking,
            None,
            2,
        )
        .await
        .map_err(|e| format!("Watch {e}"))?;
        let val: Value = resp
            .json()
            .await
            .map_err(|e| format!("Watch parse failed: {e}"))?;
        let mut out = String::new();
        if let Some(arr) = val
            .get("candidates")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.as_array())
        {
            for p in arr {
                if p.get("thought").and_then(|t| t.as_bool()).unwrap_or(false) {
                    continue;
                }
                if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                    out.push_str(t);
                }
            }
        }
        Ok(out.trim().to_string())
    } else if is_anthropic {
        let endpoint = if base_url.ends_with("/v1") {
            format!("{base_url}/messages")
        } else {
            format!("{base_url}/v1/messages")
        };
        let mut content_parts: Vec<Value> = Vec::new();
        for f in frames {
            content_parts.push(json!({
                "type": "image",
                "source": { "type": "base64", "media_type": "image/jpeg", "data": f.jpeg_b64 }
            }));
        }
        content_parts.push(json!({ "type": "text", "text": user_prompt }));
        let mut body = json!({
            "model": model.model_id.trim(),
            "max_tokens": max_tokens,
            "system": sys_prompt,
            "messages": [{ "role": "user", "content": content_parts }]
        });
        let resp = send_post_with_retry(
            client,
            ApiProtocol::Anthropic,
            &endpoint,
            api_key,
            &mut body,
            false,
            None,
            2,
        )
        .await
        .map_err(|e| format!("Watch {e}"))?;
        let val: Value = resp
            .json()
            .await
            .map_err(|e| format!("Watch parse failed: {e}"))?;
        let mut out = String::new();
        if let Some(arr) = val.get("content").and_then(|c| c.as_array()) {
            for blk in arr {
                if blk.get("type").and_then(|t| t.as_str()) == Some("text") {
                    if let Some(t) = blk.get("text").and_then(|v| v.as_str()) {
                        out.push_str(t);
                    }
                }
            }
        }
        Ok(out.trim().to_string())
    } else {
        let endpoint = format!("{base_url}/chat/completions");
        let mut user_parts: Vec<Value> = vec![json!({ "type": "text", "text": user_prompt })];
        for f in frames {
            user_parts.push(json!({
                "type": "image_url",
                "image_url": { "url": format!("data:image/jpeg;base64,{raw}", raw = f.jpeg_b64), "detail": "high" }
            }));
        }
        let mut body = json!({
            "model": model.model_id.trim(),
            "max_tokens": max_tokens,
            "messages": [
                { "role": "system", "content": sys_prompt },
                { "role": "user", "content": user_parts }
            ]
        });
        let resp = send_post_with_retry(
            client,
            ApiProtocol::OpenAi,
            &endpoint,
            api_key,
            &mut body,
            false,
            None,
            2,
        )
        .await
        .map_err(|e| format!("Watch {e}"))?;
        let val: Value = resp
            .json()
            .await
            .map_err(|e| format!("Watch parse failed: {e}"))?;
        let text = val
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        Ok(text)
    }
}

/// Lightweight live connectivity probe for OpenAI-compatible, Anthropic Claude, and Google Gemini endpoints.
/// Returns latency in milliseconds on success.
pub async fn verify_model_connection(
    api_provider: &str,
    base_url_raw: &str,
    model_id: &str,
    api_key: &str,
) -> Result<u64, String> {
    let base_url = base_url_raw.trim().trim_end_matches('/').to_string();
    if base_url.is_empty() {
        return Err("Base URL cannot be empty".to_string());
    }
    let is_local = base_url.contains("localhost") || base_url.contains("127.0.0.1");
    if api_key.trim().is_empty() && !is_local {
        return Err("API Key is required for remote endpoints".to_string());
    }

    let is_anthropic = api_provider == "anthropic" || base_url.contains("anthropic.com");
    let is_gemini = api_provider == "gemini"
        || (base_url.contains("generativelanguage.googleapis.com") && !base_url.contains("/openai"));

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(|e| format!("HTTP client init failed: {e}"))?;

    let started = std::time::Instant::now();

    if is_gemini {
        let model_clean = model_id.trim().trim_start_matches("models/");
        let endpoint = if base_url.ends_with("/v1beta") || base_url.ends_with("/v1") {
            format!("{base_url}/models/{model_clean}")
        } else {
            format!("{base_url}/v1beta/models/{model_clean}")
        };
        let resp = client
            .get(&endpoint)
            .header("x-goog-api-key", api_key.trim())
            .send()
            .await
            .map_err(|e| format!("Connection failed: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(format!(
                "HTTP {status}: {}",
                txt.chars().take(120).collect::<String>()
            ));
        }
    } else if is_anthropic {
        let endpoint = if base_url.ends_with("/v1") {
            format!("{base_url}/messages")
        } else {
            format!("{base_url}/v1/messages")
        };
        let body = json!({
            "model": model_id.trim(),
            "max_tokens": 1,
            "messages": [{ "role": "user", "content": "hi" }]
        });
        let resp = client
            .post(&endpoint)
            .header("x-api-key", api_key.trim())
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Connection failed: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(format!(
                "HTTP {status}: {}",
                txt.chars().take(120).collect::<String>()
            ));
        }
    } else {
        let models_url = format!("{base_url}/models");
        let mut req = client.get(&models_url);
        if !api_key.trim().is_empty() {
            req = req.bearer_auth(api_key.trim());
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("Connection failed: {e}"))?;
        let status = resp.status();
        if status.as_u16() == 404 || status.as_u16() == 405 {
            let chat_url = format!("{base_url}/chat/completions");
            let body = json!({
                "model": model_id.trim(),
                "max_tokens": 1,
                "messages": [{ "role": "user", "content": "hi" }]
            });
            let mut creq = client.post(&chat_url).json(&body);
            if !api_key.trim().is_empty() {
                creq = creq.bearer_auth(api_key.trim());
            }
            let cresp = creq
                .send()
                .await
                .map_err(|e| format!("Connection failed: {e}"))?;
            let cstatus = cresp.status();
            if !cstatus.is_success() {
                let txt = cresp.text().await.unwrap_or_default();
                return Err(format!(
                    "HTTP {cstatus}: {}",
                    txt.chars().take(120).collect::<String>()
                ));
            }
        } else if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(format!(
                "HTTP {status}: {}",
                txt.chars().take(120).collect::<String>()
            ));
        }
    }

    Ok(started.elapsed().as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_classifiers_and_retry_delays_are_bounded() {
        assert!(is_thinking_param_rejection(
            reqwest::StatusCode::BAD_REQUEST,
            "Invalid value for 'reasoning_effort': unsupported"
        ));
        assert!(!is_thinking_param_rejection(
            reqwest::StatusCode::BAD_REQUEST,
            "Invalid API key provided"
        ));
        assert!(!is_thinking_param_rejection(
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
            "This model is currently experiencing high demand"
        ));

        let d0 = retry_delay_ms(0, None);
        let d2 = retry_delay_ms(2, Some(60));
        assert!((1200..=1600).contains(&d0));
        assert_eq!(d2, 15_000);
    }
}
