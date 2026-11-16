//! Multi-protocol SSE streaming AI & Auto-Agent client (OpenAI-compatible, Anthropic Claude, Google Gemini),
//! 5-level thinking controller (off / low / medium / high / max) supporting both native reasoning deltas
//! and universal `<buddy_thought>...</buddy_thought>` streaming separation,
//! multimodal image compression, native tool-calling loop with interactive approval,
//! and live endpoint verification.

use crate::agent_tools::{
    anthropic_tools_json, build_agent_system_appendix, execute_tool, gemini_tools_json,
    openai_tools_json, plan_tool_execution, resolve_user_path, AgentApprovalManager,
    AgentStepEvent, ApprovalDecision,
};
use crate::db::MemoryDb;
use crate::plugins::{run_offline_ocr, PluginManager};
use crate::prompts::PromptsManager;
use crate::settings::SettingsManager;
use crate::skills::{search_github_skills, SkillsManager};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use image::{codecs::jpeg::JpegEncoder, imageops::FilterType};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AttachmentPayload {
    pub kind: String, // "image" | "text" | "folder" | "file"
    pub label: String,
    pub text_content: Option<String>,
    pub file_path: Option<String>,
}

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

#[derive(Clone, Serialize)]
pub struct AiDonePayload {
    pub stream_id: String,
    pub full_text: String,
    pub full_thought: String,
}

#[derive(Clone, Serialize)]
pub struct AiErrorPayload {
    pub stream_id: String,
    pub error: String,
}

#[derive(Default)]
pub struct AiStreamController {
    pub cancel_flag: Arc<AtomicBool>,
}

#[derive(Clone, Debug)]
struct ParsedToolCall {
    id: String,
    name: String,
    args: Value,
    raw_arguments: String,
}

enum TurnOutcome {
    Finished {
        turn_text: String,
        turn_thought: String,
    },
    ToolCalls {
        turn_text: String,
        turn_thought: String,
        calls: Vec<ParsedToolCall>,
        gemini_raw_parts: Vec<Value>,
    },
}

const OPEN_TAG: &str = "<buddy_thought>";
const CLOSE_TAG: &str = "</buddy_thought>";

#[derive(Clone, Copy, PartialEq, Eq)]
enum SplitPhase {
    Preamble,
    InsideThought,
    Answer,
}

/// Splits `<buddy_thought>...</buddy_thought>` out of streamed text deltas cleanly in real time.
struct ThoughtTagSplitter {
    enabled: bool,
    phase: SplitPhase,
    buf: String,
}

impl ThoughtTagSplitter {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            phase: if enabled {
                SplitPhase::Preamble
            } else {
                SplitPhase::Answer
            },
            buf: String::new(),
        }
    }

    fn push(&mut self, delta: &str) -> (String, String) {
        if delta.is_empty() {
            return (String::new(), String::new());
        }
        if !self.enabled || self.phase == SplitPhase::Answer {
            return (String::new(), delta.to_string());
        }

        self.buf.push_str(delta);
        let mut thought_out = String::new();
        let mut answer_out = String::new();

        loop {
            match self.phase {
                SplitPhase::Preamble => {
                    let trimmed = self.buf.trim_start();
                    if trimmed.is_empty() {
                        break;
                    }
                    if let Some(after) = trimmed.strip_prefix(OPEN_TAG) {
                        let rest = after.trim_start_matches(['\r', '\n']).to_string();
                        self.buf = rest;
                        self.phase = SplitPhase::InsideThought;
                        continue;
                    }
                    if OPEN_TAG.starts_with(trimmed) {
                        break;
                    }
                    answer_out.push_str(&self.buf);
                    self.buf.clear();
                    self.phase = SplitPhase::Answer;
                    break;
                }
                SplitPhase::InsideThought => {
                    if let Some(pos) = self.buf.find(CLOSE_TAG) {
                        thought_out.push_str(&self.buf[..pos]);
                        let after = self.buf[pos + CLOSE_TAG.len()..]
                            .trim_start_matches(['\r', '\n'])
                            .to_string();
                        self.buf.clear();
                        self.phase = SplitPhase::Answer;
                        if !after.is_empty() {
                            answer_out.push_str(&after);
                        }
                        break;
                    }
                    let hold = trailing_tag_prefix_len(&self.buf, CLOSE_TAG);
                    let emit_len = self.buf.len().saturating_sub(hold);
                    if emit_len > 0 {
                        // Make sure emit_len lands on UTF-8 char boundary
                        let mut safe_len = emit_len;
                        while safe_len > 0 && !self.buf.is_char_boundary(safe_len) {
                            safe_len -= 1;
                        }
                        if safe_len > 0 {
                            thought_out.push_str(&self.buf[..safe_len]);
                            self.buf.drain(..safe_len);
                        }
                    }
                    break;
                }
                SplitPhase::Answer => {
                    if !self.buf.is_empty() {
                        answer_out.push_str(&self.buf);
                        self.buf.clear();
                    }
                    break;
                }
            }
        }

        (thought_out, answer_out)
    }

    fn finish(&mut self) -> (String, String) {
        if self.buf.is_empty() {
            return (String::new(), String::new());
        }
        let leftover = std::mem::take(&mut self.buf);
        if self.phase == SplitPhase::InsideThought {
            (leftover, String::new())
        } else {
            (String::new(), leftover)
        }
    }
}

fn trailing_tag_prefix_len(text: &str, tag: &str) -> usize {
    let max_check = tag.len().saturating_sub(1).min(text.len());
    for len in (1..=max_check).rev() {
        let start = text.len() - len;
        if text.is_char_boundary(start) && tag.starts_with(&text[start..]) {
            return len;
        }
    }
    0
}

fn normalize_thinking_level(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "off" => "off",
        "low" => "low",
        "high" => "high",
        "max" => "max",
        _ => "medium",
    }
}

fn build_thinking_appendix(level: &str) -> String {
    match normalize_thinking_level(level) {
        "off" => "\n\n---\n\n# Thinking Level: OFF\n\
                  - Respond immediately and directly with zero deliberation.\n\
                  - Do NOT output `<buddy_thought>` tags."
            .to_string(),
        "low" => "\n\n---\n\n# Thinking Level: LOW\n\
                  - Before answering, do a quick 1-2 line sanity check inside `<buddy_thought>...</buddy_thought>` (check core facts, catch obvious errors or unclear intent; skip the tag if you emit native model reasoning blocks).\n\
                  - Keep the final response outside the tag concise and accurate."
            .to_string(),
        "high" => "\n\n---\n\n# Thinking Level: HIGH\n\
                   - Think thoroughly step-by-step inside `<buddy_thought>...</buddy_thought>` before writing your final reply (skip the tag if using native model reasoning blocks):\n\
                     1) Verify what the user actually wants (clarify first if ambiguous).\n\
                     2) Check factual accuracy, technical feasibility, and edge cases — prepare honest pushback if the premise is flawed or unrealistic.\n\
                     3) Structure the cleanest, verified answer.\n\
                   - Write ONLY the polished final reply outside `</buddy_thought>`."
            .to_string(),
        "max" => "\n\n---\n\n# Thinking Level: MAX (Exhaustive Verification)\n\
                  - Think exhaustively inside `<buddy_thought>...</buddy_thought>` before answering (skip the tag if using native model reasoning blocks):\n\
                    1) Explore every plausible interpretation and edge case.\n\
                    2) Simulate each possible outcome / solution branch and check for failures, contradictions, or hard feasibility limits.\n\
                    3) Double-check every calculation, fact, path, and assumption; identify if you need to ask clarifying questions first or push back honestly on faulty claims.\n\
                    4) Verify the planned reply is 100% sound before writing a single word to the user.\n\
                  - Deliver ONLY the confirmed, well-structured final response after `</buddy_thought>`."
            .to_string(),
        _ => "\n\n---\n\n# Thinking Level: MEDIUM\n\
              - Before answering, think cleanly inside `<buddy_thought>...</buddy_thought>` (identify true intent, check whether clarification or honest pushback is needed, verify facts/steps; skip the tag if using native model reasoning blocks).\n\
              - Put ONLY the final user-facing reply after `</buddy_thought>`."
            .to_string(),
    }
}

/// Resizes image longest edge to <= 1536px and encodes JPEG (q=82) returning raw base64.
pub fn compress_image_to_jpeg_base64(path_str: &str) -> Result<String, String> {
    let path = Path::new(path_str);
    let img = image::open(path).map_err(|e| format!("open image failed: {e}"))?;
    let (w, h) = (img.width(), img.height());
    let max_dim = w.max(h);

    let resized = if max_dim > 1536 {
        let scale = 1536.0 / (max_dim as f32);
        let nw = ((w as f32) * scale).round().max(1.0) as u32;
        let nh = ((h as f32) * scale).round().max(1.0) as u32;
        img.resize(nw, nh, FilterType::Triangle)
    } else {
        img
    };

    let rgb = resized.to_rgb8();
    let mut out = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut out, 82);
    encoder
        .encode_image(&rgb)
        .map_err(|e| format!("encode jpeg failed: {e}"))?;

    Ok(B64.encode(&out))
}

fn format_attachment_text_blocks(prompt: &str, attachments: &[AttachmentPayload]) -> Vec<String> {
    let mut text_blocks: Vec<String> = Vec::new();
    if !prompt.trim().is_empty() {
        text_blocks.push(prompt.trim().to_string());
    }

    for att in attachments {
        let path_note = att
            .file_path
            .as_deref()
            .filter(|p| !p.trim().is_empty())
            .map(|p| format!(" | Path: {p}"))
            .unwrap_or_default();

        match att.kind.as_str() {
            "folder" => {
                text_blocks.push(format!("[Attached Folder: {}{}]", att.label, path_note));
            }
            "file" => {
                let info = att.text_content.as_deref().unwrap_or("Binary/Document file");
                text_blocks.push(format!(
                    "[Attached File: {}{} — {}]",
                    att.label, path_note, info
                ));
            }
            "text" => {
                if let Some(txt) = &att.text_content {
                    text_blocks.push(format!(
                        "[Attached Text: {}{}]\n{}",
                        att.label, path_note, txt
                    ));
                }
            }
            "image" => {
                if !path_note.is_empty() {
                    text_blocks.push(format!("[Attached Image: {}{}]", att.label, path_note));
                }
            }
            _ => {}
        }
    }
    text_blocks
}

fn build_openai_user_content(prompt: &str, attachments: &[AttachmentPayload]) -> Value {
    let mut parts: Vec<Value> = Vec::new();
    let text_blocks = format_attachment_text_blocks(prompt, attachments);

    for att in attachments {
        if att.kind == "image" {
            if let Some(fp) = &att.file_path {
                if let Ok(raw_b64) = compress_image_to_jpeg_base64(fp) {
                    parts.push(json!({
                        "type": "image_url",
                        "image_url": { "url": format!("data:image/jpeg;base64,{raw_b64}") }
                    }));
                }
            }
        }
    }

    let combined_text = text_blocks.join("\n\n");
    if parts.is_empty() {
        Value::String(combined_text)
    } else {
        let mut all_parts = vec![json!({ "type": "text", "text": combined_text })];
        all_parts.extend(parts);
        Value::Array(all_parts)
    }
}

fn build_anthropic_user_content(prompt: &str, attachments: &[AttachmentPayload]) -> Value {
    let mut parts: Vec<Value> = Vec::new();
    let text_blocks = format_attachment_text_blocks(prompt, attachments);

    for att in attachments {
        if att.kind == "image" {
            if let Some(fp) = &att.file_path {
                if let Ok(raw_b64) = compress_image_to_jpeg_base64(fp) {
                    parts.push(json!({
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": "image/jpeg",
                            "data": raw_b64
                        }
                    }));
                }
            }
        }
    }

    let combined_text = text_blocks.join("\n\n");
    if parts.is_empty() {
        Value::String(combined_text)
    } else {
        parts.push(json!({ "type": "text", "text": combined_text }));
        Value::Array(parts)
    }
}

fn build_gemini_user_parts(prompt: &str, attachments: &[AttachmentPayload]) -> Vec<Value> {
    let mut parts: Vec<Value> = Vec::new();
    let text_blocks = format_attachment_text_blocks(prompt, attachments);

    for att in attachments {
        if att.kind == "image" {
            if let Some(fp) = &att.file_path {
                if let Ok(raw_b64) = compress_image_to_jpeg_base64(fp) {
                    parts.push(json!({
                        "inlineData": {
                            "mimeType": "image/jpeg",
                            "data": raw_b64
                        }
                    }));
                }
            }
        }
    }

    let combined_text = text_blocks.join("\n\n");
    if !combined_text.is_empty() {
        parts.push(json!({ "text": combined_text }));
    }
    parts
}

fn build_gemini_contents(
    history: Vec<(String, String)>,
    prompt: &str,
    attachments: &[AttachmentPayload],
) -> Vec<Value> {
    let mut turns: Vec<(String, Vec<Value>)> = Vec::new();

    for (role, content) in history
        .into_iter()
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        if content.trim().is_empty() {
            continue;
        }
        let g_role = if role == "assistant" { "model" } else { "user" };
        if let Some((last_role, last_parts)) = turns.last_mut() {
            if last_role == g_role {
                last_parts.push(json!({ "text": content }));
                continue;
            }
        }
        turns.push((g_role.to_string(), vec![json!({ "text": content })]));
    }

    while turns.last().map(|(r, _)| r.as_str()) == Some("user") {
        turns.pop();
    }
    while turns.first().map(|(r, _)| r.as_str()) == Some("model") {
        turns.remove(0);
    }

    turns.push((
        "user".to_string(),
        build_gemini_user_parts(prompt, attachments),
    ));

    turns
        .into_iter()
        .map(|(role, parts)| json!({ "role": role, "parts": parts }))
        .collect()
}

fn build_anthropic_history(
    history: Vec<(String, String)>,
    prompt: &str,
    attachments: &[AttachmentPayload],
) -> Vec<Value> {
    let mut msgs: Vec<(String, Value)> = Vec::new();

    for (role, content) in history
        .into_iter()
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        if content.trim().is_empty() {
            continue;
        }
        let norm_role = if role == "assistant" { "assistant" } else { "user" };
        if let Some((last_role, last_val)) = msgs.last_mut() {
            if last_role == norm_role {
                if let Some(prev_s) = last_val.as_str() {
                    *last_val = Value::String(format!("{prev_s}\n\n{content}"));
                    continue;
                }
            }
        }
        msgs.push((norm_role.to_string(), Value::String(content)));
    }

    while msgs.last().map(|(r, _)| r.as_str()) == Some("user") {
        msgs.pop();
    }
    while msgs.first().map(|(r, _)| r.as_str()) == Some("assistant") {
        msgs.remove(0);
    }

    msgs.push((
        "user".to_string(),
        build_anthropic_user_content(prompt, attachments),
    ));

    msgs.into_iter()
        .map(|(role, content)| json!({ "role": role, "content": content }))
        .collect()
}

fn build_openai_history(
    sys_prompt: &str,
    history: Vec<(String, String)>,
    prompt: &str,
    attachments: &[AttachmentPayload],
) -> Vec<Value> {
    let mut api_messages = vec![json!({
        "role": "system",
        "content": sys_prompt
    })];
    for (role, content) in history
        .into_iter()
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        if !content.trim().is_empty() {
            api_messages.push(json!({ "role": role, "content": content }));
        }
    }
    api_messages.push(json!({
        "role": "user",
        "content": build_openai_user_content(prompt, attachments)
    }));
    api_messages
}

fn emit_chunk(app: &AppHandle, stream_id: &str, delta: &str) {
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

fn emit_thought(app: &AppHandle, stream_id: &str, delta: &str) {
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

fn emit_step(app: &AppHandle, payload: &AgentStepEvent) {
    let _ = app.emit("buddy://agent-step", payload);
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
}

#[allow(clippy::too_many_arguments)]
async fn run_openai_turn(
    app: &AppHandle,
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    model_id: &str,
    messages: &[Value],
    agent_mode: bool,
    thinking_level: &str,
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
    if agent_mode {
        body["tools"] = openai_tools_json();
        body["tool_choice"] = json!("auto");
    }

    let mut req = client.post(&endpoint).json(&body);
    if !api_key.is_empty() {
        req = req.bearer_auth(api_key);
    }

    let mut resp = req.send().await.map_err(|e| format!("Request failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let err_txt = resp.text().await.unwrap_or_default();
        return Err(format!(
            "API returned {status}: {}",
            err_txt.chars().take(240).collect::<String>()
        ));
    }

    let mut turn_text = String::new();
    let mut turn_thought = String::new();
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
                            if !rc.is_empty() && thinking_level != "off" {
                                turn_thought.push_str(rc);
                                emit_thought(app, stream_id, rc);
                            }
                        }
                        if let Some(delta) = delta_obj["content"].as_str() {
                            let (th, ans) = splitter.push(delta);
                            if !th.is_empty() {
                                turn_thought.push_str(&th);
                                emit_thought(app, stream_id, &th);
                            }
                            if !ans.is_empty() {
                                turn_text.push_str(&ans);
                                emit_chunk(app, stream_id, &ans);
                            }
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
    if !th_tail.is_empty() {
        turn_thought.push_str(&th_tail);
        emit_thought(app, stream_id, &th_tail);
    }
    if !ans_tail.is_empty() {
        turn_text.push_str(&ans_tail);
        emit_chunk(app, stream_id, &ans_tail);
    }

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
            calls,
            gemini_raw_parts: Vec::new(),
        })
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_anthropic_turn(
    app: &AppHandle,
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    model_id: &str,
    sys_prompt: &str,
    messages: &[Value],
    agent_mode: bool,
    thinking_level: &str,
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
    if agent_mode {
        body["tools"] = anthropic_tools_json();
    }

    let mut resp = client
        .post(&endpoint)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_txt = resp.text().await.unwrap_or_default();
        return Err(format!(
            "API returned {status}: {}",
            err_txt.chars().take(240).collect::<String>()
        ));
    }

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
                                    },
                                );
                            }
                            "content_block_delta" => {
                                let idx = parsed["index"].as_u64().unwrap_or(0) as usize;
                                let dtype = parsed["delta"]["type"].as_str().unwrap_or("");
                                if dtype == "thinking_delta" {
                                    if let Some(th) = parsed["delta"]["thinking"].as_str() {
                                        if !th.is_empty() && thinking_level != "off" {
                                            turn_thought.push_str(th);
                                            emit_thought(app, stream_id, th);
                                        }
                                    }
                                } else if dtype == "text_delta" || parsed["delta"]["text"].is_string() {
                                    if let Some(delta) = parsed["delta"]["text"].as_str() {
                                        let (th, ans) = splitter.push(delta);
                                        if !th.is_empty() {
                                            turn_thought.push_str(&th);
                                            emit_thought(app, stream_id, &th);
                                        }
                                        if !ans.is_empty() {
                                            turn_text.push_str(&ans);
                                            emit_chunk(app, stream_id, &ans);
                                        }
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
    if !th_tail.is_empty() {
        turn_thought.push_str(&th_tail);
        emit_thought(app, stream_id, &th_tail);
    }
    if !ans_tail.is_empty() {
        turn_text.push_str(&ans_tail);
        emit_chunk(app, stream_id, &ans_tail);
    }

    let tool_calls: Vec<ParsedToolCall> = blocks
        .into_iter()
        .filter(|(_, b)| b.kind == "tool_use" && !b.name.trim().is_empty())
        .map(|(idx, b)| {
            let raw_arguments = if b.partial_json.trim().is_empty() {
                "{}".to_string()
            } else {
                b.partial_json
            };
            let args: Value = serde_json::from_str(&raw_arguments).unwrap_or_else(|_| json!({}));
            let id = if b.id.is_empty() {
                format!("toolu_{turn_idx}_{idx}")
            } else {
                b.id
            };
            ParsedToolCall {
                id,
                name: b.name,
                args,
                raw_arguments,
            }
        })
        .collect();

    if tool_calls.is_empty() {
        Ok(TurnOutcome::Finished {
            turn_text,
            turn_thought,
        })
    } else {
        Ok(TurnOutcome::ToolCalls {
            turn_text,
            turn_thought,
            calls: tool_calls,
            gemini_raw_parts: Vec::new(),
        })
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_gemini_turn(
    app: &AppHandle,
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    model_id: &str,
    sys_prompt: &str,
    contents: &[Value],
    agent_mode: bool,
    thinking_level: &str,
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

    // Configure native Gemini 2.5 thinking budget cleanly
    let lvl = normalize_thinking_level(thinking_level);
    let is_gemini_25 = model_clean.contains("2.5") || model_clean.contains("thinking");
    if is_gemini_25 {
        let thinking_cfg = match lvl {
            "off" if !model_clean.contains("pro") => json!({ "thinkingBudget": 0, "includeThoughts": false }),
            "off" => json!({ "thinkingBudget": 128, "includeThoughts": false }),
            "low" => json!({ "thinkingBudget": 1024, "includeThoughts": true }),
            "high" => json!({ "thinkingBudget": 10240, "includeThoughts": true }),
            "max" => json!({ "thinkingBudget": 24576, "includeThoughts": true }),
            _ => json!({ "thinkingBudget": 4096, "includeThoughts": true }),
        };
        body["generationConfig"] = json!({ "thinkingConfig": thinking_cfg });
    }

    if agent_mode {
        body["tools"] = gemini_tools_json();
    }

    let mut resp = client
        .post(&endpoint)
        .header("x-goog-api-key", api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_txt = resp.text().await.unwrap_or_default();
        return Err(format!(
            "API returned {status}: {}",
            err_txt.chars().take(240).collect::<String>()
        ));
    }

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
                                    if !th.is_empty() {
                                        turn_thought.push_str(&th);
                                        emit_thought(app, stream_id, &th);
                                    }
                                    if !ans.is_empty() {
                                        turn_text.push_str(&ans);
                                        emit_chunk(app, stream_id, &ans);
                                    }
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
    if !th_tail.is_empty() {
        turn_thought.push_str(&th_tail);
        emit_thought(app, stream_id, &th_tail);
    }
    if !ans_tail.is_empty() {
        turn_text.push_str(&ans_tail);
        emit_chunk(app, stream_id, &ans_tail);
    }

    if calls.is_empty() {
        Ok(TurnOutcome::Finished {
            turn_text,
            turn_thought,
        })
    } else {
        Ok(TurnOutcome::ToolCalls {
            turn_text,
            turn_thought,
            calls,
            gemini_raw_parts: raw_model_parts,
        })
    }
}

fn derive_workspace_hint(configured: &str, attachments: &[AttachmentPayload]) -> Option<PathBuf> {
    if !configured.trim().is_empty() {
        return Some(PathBuf::from(configured.trim()));
    }
    for att in attachments {
        if let Some(fp) = &att.file_path {
            let p = PathBuf::from(fp);
            if att.kind == "folder" || p.is_dir() {
                return Some(p);
            }
            if let Some(parent) = p.parent() {
                return Some(parent.to_path_buf());
            }
        }
    }
    None
}

async fn run_tool_with_approval(
    app: &AppHandle,
    approval_mgr: &AgentApprovalManager,
    skills_mgr: &SkillsManager,
    mirror_prefix: &str,
    stream_id: &str,
    turn_idx: usize,
    call_idx: usize,
    call: &ParsedToolCall,
    workspace_hint: Option<&Path>,
    approve_all_turn: &mut bool,
    cancel_flag: &AtomicBool,
) -> (String, AgentStepEvent) {
    let plan = plan_tool_execution(&call.name, &call.args, workspace_hint);
    let step_id = format!("{stream_id}_{turn_idx}_{call_idx}");

    let needs_prompt = plan.requires_approval && !(plan.allow_approve_all && *approve_all_turn);

    if needs_prompt {
        let mut rx = approval_mgr.register(stream_id, &step_id);
        emit_step(
            app,
            &AgentStepEvent {
                stream_id: stream_id.to_string(),
                step_id: step_id.clone(),
                tool_name: call.name.clone(),
                summary: plan.summary.clone(),
                status: "waiting_approval".to_string(),
                detail: if plan.allow_approve_all {
                    Some("allow_all".to_string())
                } else {
                    None
                },
                preview: plan.preview.clone(),
                is_destructive: plan.is_destructive,
            },
        );

        let decision = loop {
            if cancel_flag.load(Ordering::Relaxed) {
                break ApprovalDecision::Reject;
            }
            match rx.try_recv() {
                Ok(d) => break d,
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    break ApprovalDecision::Reject;
                }
            }
        };

        if decision == ApprovalDecision::Reject || cancel_flag.load(Ordering::Relaxed) {
            let step = AgentStepEvent {
                stream_id: stream_id.to_string(),
                step_id,
                tool_name: call.name.clone(),
                summary: plan.summary,
                status: "rejected".to_string(),
                detail: Some("Rejected by user".to_string()),
                preview: plan.preview,
                is_destructive: plan.is_destructive,
            };
            emit_step(app, &step);
            return (
                "Action rejected by user. Do not retry this exact action unless the user asks."
                    .to_string(),
                step,
            );
        }

        if decision == ApprovalDecision::ApproveAllTurn && plan.allow_approve_all {
            *approve_all_turn = true;
        }
    }

    emit_step(
        app,
        &AgentStepEvent {
            stream_id: stream_id.to_string(),
            step_id: step_id.clone(),
            tool_name: call.name.clone(),
            summary: plan.summary.clone(),
            status: "running".to_string(),
            detail: None,
            preview: plan.preview.clone(),
            is_destructive: plan.is_destructive,
        },
    );

    let (ok, output, detail) = match call.name.as_str() {
        "list_installed_skills" => {
            let list = skills_mgr.list();
            if list.is_empty() {
                (
                    true,
                    "No skills installed yet. You can find one via `search_github_skills` and propose installing it via `install_github_skill`.".into(),
                    "0 skills".into(),
                )
            } else {
                let count = list.len();
                let formatted = list
                    .into_iter()
                    .map(|s| {
                        format!(
                            "- id=`{}` | enabled={} | name=\"{}\" | dir=`{}`\n  {}",
                            s.id, s.enabled, s.name, s.dir_path, s.description
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                (true, formatted, format!("{count} skills"))
            }
        }
        "read_skill" => {
            let sid = call.args["skill_id"].as_str().unwrap_or("");
            match skills_mgr.get_detail(sid) {
                Ok(det) => {
                    let files_str = det.files.join(", ");
                    let out = format!(
                        "Skill `{}` ({})\nDir: `{}`\nFiles: [{}]\n\n{}",
                        det.info.id, det.info.name, det.info.dir_path, files_str, det.skill_md
                    );
                    (true, out, format!("{} files", det.files.len()))
                }
                Err(e) => (false, format!("Error: {e}"), e),
            }
        }
        "search_github_skills" => {
            let q = call.args["query"].as_str().unwrap_or("");
            match search_github_skills(q).await {
                Ok(cands) if cands.is_empty() => (
                    true,
                    format!("No matching GitHub Skills found for `{q}`."),
                    "0 results".into(),
                ),
                Ok(cands) => {
                    let count = cands.len();
                    let text = cands
                        .into_iter()
                        .map(|c| {
                            format!(
                                "- **{}** (★{}) -> install_spec: `{}`\n  {}",
                                c.title, c.stars, c.install_spec, c.description
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    (true, text, format!("{count} found"))
                }
                Err(e) => (false, format!("Error: {e}"), e),
            }
        }
        "install_github_skill" => {
            let src = call.args["source"].as_str().unwrap_or("");
            match skills_mgr.install_from_github(src, mirror_prefix).await {
                Ok(installed) => {
                    let _ = app.emit("buddy://skills-updated", ());
                    let count = installed.len();
                    let lines = installed
                        .into_iter()
                        .map(|s| format!("- `{}` (**{}**) -> `{}`", s.id, s.name, s.dir_path))
                        .collect::<Vec<_>>()
                        .join("\n");
                    (
                        true,
                        format!("Installed {count} skill(s):\n{lines}"),
                        format!("{count} installed"),
                    )
                }
                Err(e) => (false, format!("Error: {e}"), e),
            }
        }
        "offline_ocr" => {
            let raw_src = call.args["source"].as_str().unwrap_or("");
            let resolved = resolve_user_path(raw_src, workspace_hint)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| raw_src.to_string());
            match run_offline_ocr(resolved).await {
                Ok(res) => {
                    let text = if res.text.trim().is_empty() {
                        "(No text detected in image)".to_string()
                    } else {
                        res.text.clone()
                    };
                    (
                        true,
                        format!("OCR ({}) result:\n{}", res.language, text),
                        format!("{} lines", res.line_count),
                    )
                }
                Err(e) => (false, format!("Error: {e}"), e),
            }
        }
        _ => {
            let name_owned = call.name.clone();
            let args_owned = call.args.clone();
            let ws_owned = workspace_hint.map(|p| p.to_path_buf());

            tauri::async_runtime::spawn_blocking(move || {
                execute_tool(&name_owned, &args_owned, ws_owned.as_deref())
            })
            .await
            .unwrap_or_else(|e| (false, format!("Worker panic: {e}"), "Panic".to_string()))
        }
    };

    let step = AgentStepEvent {
        stream_id: stream_id.to_string(),
        step_id,
        tool_name: call.name.clone(),
        summary: plan.summary,
        status: if ok { "done" } else { "error" }.to_string(),
        detail: Some(detail),
        preview: plan.preview,
        is_destructive: plan.is_destructive,
    };
    emit_step(app, &step);
    (output, step)
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_chat_stream(
    app: AppHandle,
    settings_mgr: Arc<SettingsManager>,
    prompts_mgr: Arc<PromptsManager>,
    skills_mgr: Arc<SkillsManager>,
    plugin_mgr: Arc<PluginManager>,
    db: Arc<MemoryDb>,
    controller: Arc<AiStreamController>,
    approval_mgr: Arc<AgentApprovalManager>,
    conversation_id: i64,
    stream_id: String,
    prompt: String,
    attachments: Vec<AttachmentPayload>,
    history: Vec<(String, String)>,
) {
    controller.cancel_flag.store(false, Ordering::Relaxed);
    let cancel_flag = controller.cancel_flag.clone();

    tauri::async_runtime::spawn(async move {
        let cfg = settings_mgr.get();
        let api_key = settings_mgr.load_api_key().unwrap_or_default();
        let base_url = cfg.base_url.trim().trim_end_matches('/').to_string();
        let is_local = base_url.contains("localhost") || base_url.contains("127.0.0.1");
        let is_anthropic = cfg.api_provider == "anthropic" || base_url.contains("anthropic.com");
        let is_gemini = cfg.api_provider == "gemini"
            || (base_url.contains("generativelanguage.googleapis.com")
                && !base_url.contains("/openai"));

        if api_key.is_empty() && !is_local {
            let _ = app.emit(
                "buddy://ai-error",
                AiErrorPayload {
                    stream_id,
                    error: "No API Key configured yet. Open Settings to enter your Base URL & API Key (all local features continue working offline).".to_string(),
                },
            );
            return;
        }

        let thinking_level = normalize_thinking_level(&cfg.thinking_level);
        let mirror_prefix = plugin_mgr.list().mirror_prefix;
        let mut sys_prompt = prompts_mgr.build_system_prompt(&cfg.persona_id);
        if cfg.agent_mode {
            sys_prompt.push_str(&build_agent_system_appendix(&cfg.agent_workspace));
            sys_prompt.push_str(&skills_mgr.build_prompt_appendix());
        }
        sys_prompt.push_str(&skills_mgr.build_explicit_slash_appendix(&prompt));
        sys_prompt.push_str(&build_thinking_appendix(thinking_level));

        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(150))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = app.emit(
                    "buddy://ai-error",
                    AiErrorPayload {
                        stream_id,
                        error: format!("HTTP client error: {e}"),
                    },
                );
                return;
            }
        };

        let workspace_hint = derive_workspace_hint(&cfg.agent_workspace, &attachments);
        let mut full_text = String::new();
        let mut full_thought = String::new();
        let mut recorded_steps: Vec<AgentStepEvent> = Vec::new();
        let mut approve_all_turn = false;

        let mut openai_msgs = if !is_anthropic && !is_gemini {
            build_openai_history(&sys_prompt, history.clone(), &prompt, &attachments)
        } else {
            Vec::new()
        };
        let mut anthropic_msgs = if is_anthropic {
            build_anthropic_history(history.clone(), &prompt, &attachments)
        } else {
            Vec::new()
        };
        let mut gemini_contents = if is_gemini {
            build_gemini_contents(history, &prompt, &attachments)
        } else {
            Vec::new()
        };

        const MAX_TURNS: usize = 16;

        for turn_idx in 0..MAX_TURNS {
            if cancel_flag.load(Ordering::Relaxed) {
                break;
            }

            if turn_idx > 0 && !full_text.is_empty() && !full_text.ends_with('\n') {
                full_text.push_str("\n\n");
                emit_chunk(&app, &stream_id, "\n\n");
            }

            let turn_res = if is_gemini {
                run_gemini_turn(
                    &app,
                    &client,
                    &base_url,
                    &api_key,
                    &cfg.model_id,
                    &sys_prompt,
                    &gemini_contents,
                    cfg.agent_mode,
                    thinking_level,
                    &stream_id,
                    turn_idx,
                    &cancel_flag,
                )
                .await
            } else if is_anthropic {
                run_anthropic_turn(
                    &app,
                    &client,
                    &base_url,
                    &api_key,
                    &cfg.model_id,
                    &sys_prompt,
                    &anthropic_msgs,
                    cfg.agent_mode,
                    thinking_level,
                    &stream_id,
                    turn_idx,
                    &cancel_flag,
                )
                .await
            } else {
                run_openai_turn(
                    &app,
                    &client,
                    &base_url,
                    &api_key,
                    &cfg.model_id,
                    &openai_msgs,
                    cfg.agent_mode,
                    thinking_level,
                    &stream_id,
                    turn_idx,
                    &cancel_flag,
                )
                .await
            };

            match turn_res {
                Err(err) => {
                    approval_mgr.reject_for_stream(&stream_id);
                    let _ = app.emit(
                        "buddy://ai-error",
                        AiErrorPayload {
                            stream_id,
                            error: err,
                        },
                    );
                    return;
                }
                Ok(TurnOutcome::Finished {
                    turn_text,
                    turn_thought,
                }) => {
                    full_text.push_str(&turn_text);
                    if !turn_thought.is_empty() {
                        if !full_thought.is_empty() {
                            full_thought.push_str("\n\n");
                        }
                        full_thought.push_str(&turn_thought);
                    }
                    break;
                }
                Ok(TurnOutcome::ToolCalls {
                    turn_text,
                    turn_thought,
                    calls,
                    gemini_raw_parts,
                }) => {
                    full_text.push_str(&turn_text);
                    if !turn_thought.is_empty() {
                        if !full_thought.is_empty() {
                            full_thought.push_str("\n\n");
                        }
                        full_thought.push_str(&turn_thought);
                    }

                    if cancel_flag.load(Ordering::Relaxed) {
                        break;
                    }

                    if is_gemini {
                        gemini_contents.push(json!({
                            "role": "model",
                            "parts": gemini_raw_parts
                        }));
                    } else if is_anthropic {
                        let mut blocks: Vec<Value> = Vec::new();
                        if !turn_text.is_empty() {
                            blocks.push(json!({ "type": "text", "text": turn_text }));
                        }
                        for c in &calls {
                            blocks.push(json!({
                                "type": "tool_use",
                                "id": c.id,
                                "name": c.name,
                                "input": c.args
                            }));
                        }
                        anthropic_msgs.push(json!({ "role": "assistant", "content": blocks }));
                    } else {
                        let tc_vals: Vec<Value> = calls
                            .iter()
                            .map(|c| {
                                json!({
                                    "id": c.id,
                                    "type": "function",
                                    "function": {
                                        "name": c.name,
                                        "arguments": c.raw_arguments
                                    }
                                })
                            })
                            .collect();
                        openai_msgs.push(json!({
                            "role": "assistant",
                            "content": if turn_text.is_empty() { Value::Null } else { Value::String(turn_text) },
                            "tool_calls": tc_vals
                        }));
                    }

                    let mut anthropic_results: Vec<Value> = Vec::new();
                    let mut gemini_results: Vec<Value> = Vec::new();

                    for (call_idx, call) in calls.iter().enumerate() {
                        if cancel_flag.load(Ordering::Relaxed) {
                            break;
                        }
                        let (result_text, step_rec) = run_tool_with_approval(
                            &app,
                            &approval_mgr,
                            &skills_mgr,
                            &mirror_prefix,
                            &stream_id,
                            turn_idx,
                            call_idx,
                            call,
                            workspace_hint.as_deref(),
                            &mut approve_all_turn,
                            &cancel_flag,
                        )
                        .await;
                        recorded_steps.push(step_rec.clone());

                        if is_gemini {
                            gemini_results.push(json!({
                                "functionResponse": {
                                    "name": call.name,
                                    "response": { "result": result_text }
                                }
                            }));
                        } else if is_anthropic {
                            anthropic_results.push(json!({
                                "type": "tool_result",
                                "tool_use_id": call.id,
                                "content": result_text,
                                "is_error": step_rec.status == "error" || step_rec.status == "rejected"
                            }));
                        } else {
                            openai_msgs.push(json!({
                                "role": "tool",
                                "tool_call_id": call.id,
                                "content": result_text
                            }));
                        }
                    }

                    if is_gemini && !gemini_results.is_empty() {
                        gemini_contents.push(json!({
                            "role": "user",
                            "parts": gemini_results
                        }));
                    } else if is_anthropic && !anthropic_results.is_empty() {
                        anthropic_msgs.push(json!({
                            "role": "user",
                            "content": anthropic_results
                        }));
                    }
                }
            }
        }

        approval_mgr.reject_for_stream(&stream_id);

        let steps_json = if recorded_steps.is_empty() {
            None
        } else {
            serde_json::to_string(&recorded_steps).ok()
        };
        let thought_opt = if full_thought.trim().is_empty() {
            None
        } else {
            Some(full_thought.trim().to_string())
        };

        if !full_text.is_empty() || steps_json.is_some() || thought_opt.is_some() {
            let stored_text = if full_text.is_empty() {
                "Done.".to_string()
            } else {
                full_text.clone()
            };
            let _ = db.append_chat_message(
                conversation_id,
                "assistant",
                &stored_text,
                None,
                steps_json.as_deref(),
                thought_opt.as_deref(),
            );
        }

        let _ = app.emit(
            "buddy://ai-done",
            AiDonePayload {
                stream_id,
                full_text,
                full_thought,
            },
        );
    });
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
    fn thought_splitter_separates_chunked_tags_without_leaking() {
        let mut s = ThoughtTagSplitter::new(true);
        let chunks = [
            "  <budd",
            "y_thought>\nSte",
            "p 1: check fact",
            "</bud",
            "dy_thought>\n\n1+1 ",
            "= 2.",
        ];
        let mut th_all = String::new();
        let mut ans_all = String::new();
        for c in chunks {
            let (th, ans) = s.push(c);
            th_all.push_str(&th);
            ans_all.push_str(&ans);
        }
        let (th_t, ans_t) = s.finish();
        th_all.push_str(&th_t);
        ans_all.push_str(&ans_t);

        assert_eq!(th_all, "Step 1: check fact");
        assert_eq!(ans_all, "1+1 = 2.");
    }

    #[test]
    fn thought_splitter_passes_plain_text_when_no_tag_or_off() {
        let mut s = ThoughtTagSplitter::new(true);
        let (th, ans) = s.push("Hello world!");
        assert_eq!(th, "");
        assert_eq!(ans, "Hello world!");

        let mut s_off = ThoughtTagSplitter::new(false);
        let (th2, ans2) = s_off.push("<buddy_thought>raw");
        assert_eq!(th2, "");
        assert_eq!(ans2, "<buddy_thought>raw");
    }
}
