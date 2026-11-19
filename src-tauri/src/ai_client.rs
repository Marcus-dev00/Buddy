//! Multi-protocol SSE streaming AI & Auto-Agent client (OpenAI-compatible, Anthropic Claude, Google Gemini),
//! 5-level thinking controller (off / low / medium / high / max) supporting both native reasoning deltas
//! and universal `<buddy_thought>...</buddy_thought>` streaming separation,
//! multimodal image compression, native tool-calling loop with interactive approval,
//! and live endpoint verification.

use crate::agent_tools::{
    build_agent_system_appendix, execute_fetch_web_page, execute_tool, execute_web_search,
    plan_tool_execution, resolve_user_path, AgentApprovalManager, AgentPermissionMode,
    AgentStepEvent, ApprovalDecision,
};
use crate::browser::BrowserManager;
use crate::db::{now_ms, MemoryDb};
use crate::llm_protocols::{
    emit_chunk, run_anthropic_turn, run_gemini_turn, run_openai_turn, ParsedToolCall, TurnOutcome,
};
pub use crate::llm_protocols::{run_oneshot_vision, verify_model_connection};
use crate::mcp::McpManager;
use crate::plugins::{run_media_download, run_offline_ocr, PluginManager};
use crate::prompts::PromptsManager;
use crate::reminders::{format_local_datetime, local_utc_offset_ms, parse_fire_at_ms};
use crate::settings::{ResolvedModelConfig, SettingsManager};
use crate::skills::{search_github_skills, SkillsManager};
use crate::thinking::{
    build_thinking_appendix_for_lang, clamp_thinking_level, classify_protocol, detect_thinking_lang,
    detect_thinking_language_hint, infer_thinking_capability_with_mode, ThinkingLang,
};
use crate::win32_sensors::AppSensorState;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use image::{codecs::jpeg::JpegEncoder, imageops::FilterType, DynamicImage};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AttachmentPayload {
    pub kind: String, // "image" | "text" | "folder" | "file"
    pub label: String,
    pub text_content: Option<String>,
    pub file_path: Option<String>,
}

#[derive(Clone, Debug)]
pub struct InlineImageFrame {
    pub label: String,
    pub jpeg_b64: String,
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
    pub is_streaming: Arc<AtomicBool>,
    pub steer_queue: Mutex<Vec<String>>,
}

impl AiStreamController {
    pub fn push_steer(&self, msg: String) {
        let trimmed = msg.trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        if let Ok(mut q) = self.steer_queue.lock() {
            q.push(trimmed);
        }
    }

    pub fn has_pending_steers(&self) -> bool {
        self.steer_queue
            .lock()
            .map(|q| !q.is_empty())
            .unwrap_or(false)
    }

    pub fn drain_steers(&self) -> Vec<String> {
        self.steer_queue
            .lock()
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default()
    }

    pub fn clear_steers(&self) {
        if let Ok(mut q) = self.steer_queue.lock() {
            q.clear();
        }
    }
}

fn clamp_history_turn_text(text: &str) -> String {
    const MAX_HISTORY_TURN_CHARS: usize = 3600;
    let total = text.chars().count();
    if total <= MAX_HISTORY_TURN_CHARS {
        return text.to_string();
    }
    let kept: String = text.chars().take(MAX_HISTORY_TURN_CHARS).collect();
    format!("{kept}\n... [earlier long turn truncated]")
}

/// Resizes `DynamicImage` longest edge to `<= max_dim` and encodes JPEG returning raw base64.
pub fn encode_dynamic_image_jpeg_base64(
    img: &DynamicImage,
    max_dim_limit: u32,
    quality: u8,
) -> Result<String, String> {
    let (w, h) = (img.width(), img.height());
    let max_dim = w.max(h);

    let resized = if max_dim > max_dim_limit {
        let scale = (max_dim_limit as f32) / (max_dim as f32);
        let nw = ((w as f32) * scale).round().max(1.0) as u32;
        let nh = ((h as f32) * scale).round().max(1.0) as u32;
        img.resize(nw, nh, FilterType::CatmullRom)
    } else {
        img.clone()
    };

    let rgb = resized.to_rgb8();
    let mut out = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut out, quality);
    encoder
        .encode_image(&rgb)
        .map_err(|e| format!("encode jpeg failed: {e}"))?;

    Ok(B64.encode(&out))
}

/// Resizes image longest edge to <= 1536px and encodes JPEG (q=82) returning raw base64.
pub fn compress_image_to_jpeg_base64(path_str: &str) -> Result<String, String> {
    let path = Path::new(path_str);
    let img = image::open(path).map_err(|e| format!("open image failed: {e}"))?;
    encode_dynamic_image_jpeg_base64(&img, 2048, 86)
}

fn collect_all_user_jpegs(
    attachments: &[AttachmentPayload],
    live_frames: &[InlineImageFrame],
) -> Vec<String> {
    let mut jpegs = Vec::new();
    for att in attachments {
        if att.kind == "image" {
            if let Some(fp) = &att.file_path {
                if let Ok(raw_b64) = compress_image_to_jpeg_base64(fp) {
                    jpegs.push(raw_b64);
                }
            }
        }
    }
    for lf in live_frames {
        if !lf.jpeg_b64.is_empty() {
            jpegs.push(lf.jpeg_b64.clone());
        }
    }
    jpegs
}

fn format_attachment_text_blocks(
    prompt: &str,
    attachments: &[AttachmentPayload],
    live_frames: &[InlineImageFrame],
) -> Vec<String> {
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

    for lf in live_frames {
        text_blocks.push(format!("[Live Screen Frame: {}]", lf.label));
    }

    text_blocks
}

fn build_openai_user_content(
    prompt: &str,
    attachments: &[AttachmentPayload],
    live_frames: &[InlineImageFrame],
) -> Value {
    let mut parts: Vec<Value> = Vec::new();
    let text_blocks = format_attachment_text_blocks(prompt, attachments, live_frames);

    for raw_b64 in collect_all_user_jpegs(attachments, live_frames) {
        parts.push(json!({
            "type": "image_url",
            "image_url": { "url": format!("data:image/jpeg;base64,{raw_b64}"), "detail": "high" }
        }));
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

fn build_anthropic_user_content(
    prompt: &str,
    attachments: &[AttachmentPayload],
    live_frames: &[InlineImageFrame],
) -> Value {
    let mut parts: Vec<Value> = Vec::new();
    let text_blocks = format_attachment_text_blocks(prompt, attachments, live_frames);

    for raw_b64 in collect_all_user_jpegs(attachments, live_frames) {
        parts.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": "image/jpeg",
                "data": raw_b64
            }
        }));
    }

    let combined_text = text_blocks.join("\n\n");
    if parts.is_empty() {
        Value::String(combined_text)
    } else {
        parts.push(json!({ "type": "text", "text": combined_text }));
        Value::Array(parts)
    }
}

fn build_gemini_user_parts(
    prompt: &str,
    attachments: &[AttachmentPayload],
    live_frames: &[InlineImageFrame],
) -> Vec<Value> {
    let mut parts: Vec<Value> = Vec::new();
    let text_blocks = format_attachment_text_blocks(prompt, attachments, live_frames);

    for raw_b64 in collect_all_user_jpegs(attachments, live_frames) {
        parts.push(json!({
            "inlineData": {
                "mimeType": "image/jpeg",
                "data": raw_b64
            }
        }));
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
    live_frames: &[InlineImageFrame],
) -> Vec<Value> {
    let mut turns: Vec<(String, Vec<Value>)> = Vec::new();

    for (role, content) in history
        .into_iter()
        .rev()
        .take(20)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        if content.trim().is_empty() {
            continue;
        }
        let capped = clamp_history_turn_text(&content);
        let g_role = if role == "assistant" { "model" } else { "user" };
        if let Some((last_role, last_parts)) = turns.last_mut() {
            if last_role == g_role {
                last_parts.push(json!({ "text": capped }));
                continue;
            }
        }
        turns.push((g_role.to_string(), vec![json!({ "text": capped })]));
    }

    while turns.last().map(|(r, _)| r.as_str()) == Some("user") {
        turns.pop();
    }
    while turns.first().map(|(r, _)| r.as_str()) == Some("model") {
        turns.remove(0);
    }

    turns.push((
        "user".to_string(),
        build_gemini_user_parts(prompt, attachments, live_frames),
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
    live_frames: &[InlineImageFrame],
) -> Vec<Value> {
    let mut msgs: Vec<(String, Value)> = Vec::new();

    for (role, content) in history
        .into_iter()
        .rev()
        .take(20)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        if content.trim().is_empty() {
            continue;
        }
        let capped = clamp_history_turn_text(&content);
        let norm_role = if role == "assistant" { "assistant" } else { "user" };
        if let Some((last_role, last_val)) = msgs.last_mut() {
            if last_role == norm_role {
                if let Some(prev_s) = last_val.as_str() {
                    *last_val = Value::String(format!("{prev_s}\n\n{capped}"));
                    continue;
                }
            }
        }
        msgs.push((norm_role.to_string(), Value::String(capped)));
    }

    while msgs.last().map(|(r, _)| r.as_str()) == Some("user") {
        msgs.pop();
    }
    while msgs.first().map(|(r, _)| r.as_str()) == Some("assistant") {
        msgs.remove(0);
    }

    msgs.push((
        "user".to_string(),
        build_anthropic_user_content(prompt, attachments, live_frames),
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
    live_frames: &[InlineImageFrame],
) -> Vec<Value> {
    let mut api_messages = vec![json!({
        "role": "system",
        "content": sys_prompt
    })];
    for (role, content) in history
        .into_iter()
        .rev()
        .take(20)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        if !content.trim().is_empty() {
            api_messages.push(json!({ "role": role, "content": clamp_history_turn_text(&content) }));
        }
    }
    api_messages.push(json!({
        "role": "user",
        "content": build_openai_user_content(prompt, attachments, live_frames)
    }));
    api_messages
}

fn emit_step(app: &AppHandle, payload: &AgentStepEvent) {
    let _ = app.emit("buddy://agent-step", payload);
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

#[allow(clippy::too_many_arguments)]
async fn run_tool_with_approval(
    app: &AppHandle,
    approval_mgr: &AgentApprovalManager,
    skills_mgr: &SkillsManager,
    plugin_mgr: &PluginManager,
    browser_mgr: &Arc<BrowserManager>,
    sensor_state: &Arc<AppSensorState>,
    mcp_mgr: &Arc<McpManager>,
    settings_mgr: &SettingsManager,
    controller: &AiStreamController,
    http_client: &reqwest::Client,
    chat_model: &ResolvedModelConfig,
    chat_api_key: &str,
    db: &Arc<MemoryDb>,
    mirror_prefix: &str,
    stream_id: &str,
    turn_idx: usize,
    call_idx: usize,
    call: &ParsedToolCall,
    workspace_hint: Option<&Path>,
    permission_mode: AgentPermissionMode,
    approve_all_turn: &mut bool,
    cancel_flag: &AtomicBool,
) -> (String, AgentStepEvent) {
    let mut plan = plan_tool_execution(&call.name, &call.args, workspace_hint, permission_mode);
    if call.name.starts_with("mcp__") {
        if let Some(meta) = mcp_mgr.find_tool(&call.name) {
            if meta.server_trusted && permission_mode != AgentPermissionMode::Ask {
                plan.requires_approval = false;
            }
        }
    }
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
        "download_media" => {
            let url = call.args["url"].as_str().unwrap_or("");
            let fmt = call.args["format"].as_str().unwrap_or("audio_best");
            let out_dir = call.args["output_dir"].as_str();
            match run_media_download(app, plugin_mgr, url, fmt, out_dir).await {
                Ok(out) => {
                    let saved = out.outputs.first().cloned().unwrap_or_default();
                    if !saved.is_empty() {
                        plan.preview = Some(format!("Saved: {saved}"));
                    }
                    (
                        true,
                        format!("{}\nSaved Path: {saved}", out.summary),
                        out.summary,
                    )
                }
                Err(e) => (false, format!("Error: {e}"), e),
            }
        }
        "browser_open" | "browser_read" | "browser_click" | "browser_type" | "browser_scroll" | "browser_screenshot" | "browser_console" | "browser_close" => {
            // If user paused AI for manual interaction ("我来操作"), wait cleanly until resumed or steered
            if browser_mgr.is_user_takeover() && call.name != "browser_close" {
                let wait_start = std::time::Instant::now();
                while browser_mgr.is_user_takeover()
                    && !cancel_flag.load(Ordering::Relaxed)
                    && !controller.has_pending_steers()
                    && wait_start.elapsed() < std::time::Duration::from_secs(300)
                {
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
                if browser_mgr.is_user_takeover() && controller.has_pending_steers() {
                    browser_mgr.set_user_takeover(app, false);
                }
            }
            if cancel_flag.load(Ordering::Relaxed) {
                (false, "Cancelled by user.".to_string(), "Cancelled".to_string())
            } else {
                match call.name.as_str() {
                    "browser_open" => {
                        let url = call.args["url"].as_str().unwrap_or("");
                        let wait_ms = call.args["wait_ms"].as_u64().unwrap_or(5500);
                        match browser_mgr.execute_open(app, sensor_state, url, wait_ms).await {
                            Ok((out, det)) => (true, out, det),
                            Err(e) => (false, format!("Error: {e}"), e),
                        }
                    }
                    "browser_read" => {
                        let max_chars = call.args["max_chars"].as_u64().unwrap_or(10_000).clamp(500, 16_000) as usize;
                        match browser_mgr.execute_read(app, sensor_state, max_chars).await {
                            Ok((out, det)) => (true, out, det),
                            Err(e) => (false, format!("Error: {e}"), e),
                        }
                    }
                    "browser_click" => {
                        let target = call.args["target"].as_str().unwrap_or("");
                        match browser_mgr.execute_click(app, sensor_state, target).await {
                            Ok((out, det)) => (true, out, det),
                            Err(e) => (false, format!("Error: {e}"), e),
                        }
                    }
                    "browser_type" => {
                        let target = call.args["target"].as_str().unwrap_or("");
                        let text = call.args["text"].as_str().unwrap_or("");
                        let submit = call.args["submit"].as_bool().unwrap_or(true);
                        match browser_mgr.execute_type(app, sensor_state, target, text, submit).await {
                            Ok((out, det)) => (true, out, det),
                            Err(e) => (false, format!("Error: {e}"), e),
                        }
                    }
                    "browser_scroll" => {
                        let dir = call.args["direction"].as_str().unwrap_or("down");
                        let amount = call.args["amount"].as_i64().unwrap_or(650) as i32;
                        match browser_mgr.execute_scroll(app, sensor_state, dir, amount).await {
                            Ok((out, det)) => (true, out, det),
                            Err(e) => (false, format!("Error: {e}"), e),
                        }
                    }
                    "browser_screenshot" => match browser_mgr.execute_screenshot_b64(app).await {
                        Ok(b64) => {
                            let (vis_model, vis_key) = if chat_model.supports_vision {
                                (chat_model.clone(), chat_api_key.to_string())
                            } else {
                                settings_mgr.resolve_watch_target(None)
                            };
                            if vis_model.supports_vision {
                                let frames = vec![InlineImageFrame {
                                    label: "Built-in Browser Viewport".to_string(),
                                    jpeg_b64: b64,
                                }];
                                match run_oneshot_vision(
                                    http_client,
                                    &vis_model,
                                    &vis_key,
                                    "Describe the key visual layout, playable media items, titles, buttons, errors, or prompts shown in this browser viewport concisely for another AI assistant.",
                                    "Inspect this browser screenshot.",
                                    &frames,
                                    520,
                                )
                                .await
                                {
                                    Ok(desc) => (true, format!("[Browser Visual Snapshot]\n{desc}"), "Viewport inspected".to_string()),
                                    Err(e) => (false, format!("Vision inspection failed: {e}"), "Vision error".to_string()),
                                }
                            } else {
                                (
                                    true,
                                    "Visual inspection skipped (no Vision-enabled model configured); rely on `browser_read` text & element tags.".to_string(),
                                    "No vision model".to_string(),
                                )
                            }
                        }
                        Err(e) => (false, format!("Error: {e}"), e),
                    },
                    "browser_console" => {
                        let clear = call.args["clear"].as_bool().unwrap_or(false);
                        match browser_mgr.execute_console(app, clear).await {
                            Ok((out, det)) => (true, out, det),
                            Err(e) => (false, format!("Error: {e}"), e),
                        }
                    }
                    _ => {
                        browser_mgr.close_browser(app, sensor_state);
                        (true, "Built-in browser closed and memory freed.".to_string(), "Closed".to_string())
                    }
                }
            }
        }
        "create_reminder" => {
            let title = call.args["title"].as_str().unwrap_or("").trim();
            let delay_min = call.args["delay_minutes"].as_f64();
            let at_time = call.args["at_time"].as_str();
            let repeat = call.args["repeat"].as_str();
            let tz = local_utc_offset_ms();
            match parse_fire_at_ms(delay_min, at_time, now_ms(), tz)
                .and_then(|fire_ms| db.create_reminder(title, fire_ms, repeat))
            {
                Ok(rem) => {
                    let _ = app.emit("buddy://reminders-changed", ());
                    let when_str = format_local_datetime(rem.fire_at_ms, tz);
                    let rep_str = rem
                        .repeat_rule
                        .as_deref()
                        .map(|r| format!(" (repeat: {r})"))
                        .unwrap_or_default();
                    (
                        true,
                        format!("Reminder #{} scheduled for {}{}: {}", rem.id, when_str, rep_str, rem.title),
                        when_str,
                    )
                }
                Err(e) => (false, format!("Error: {e}"), e),
            }
        }
        "list_reminders" => {
            let include_done = call.args["include_completed"].as_bool().unwrap_or(false);
            let tz = local_utc_offset_ms();
            match db.list_reminders(include_done) {
                Ok(items) if items.is_empty() => (
                    true,
                    "No reminders scheduled.".to_string(),
                    "0 reminders".to_string(),
                ),
                Ok(items) => {
                    let count = items.len();
                    let lines = items
                        .into_iter()
                        .map(|r| {
                            let when_str = format_local_datetime(r.fire_at_ms, tz);
                            let rep = r.repeat_rule.as_deref().unwrap_or("once");
                            let st = if r.enabled { "active" } else { "done" };
                            format!("- [id={} {st} {rep}] {when_str} — {}", r.id, r.title)
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    (true, lines, format!("{count} reminders"))
                }
                Err(e) => (false, format!("Error: {e}"), e),
            }
        }
        "delete_reminder" => {
            let id = call.args["id"].as_i64().unwrap_or(0);
            match db.delete_reminder(id) {
                Ok(()) => {
                    let _ = app.emit("buddy://reminders-changed", ());
                    (true, format!("Deleted reminder #{id}."), "Deleted".to_string())
                }
                Err(e) => (false, format!("Error: {e}"), e),
            }
        }
        "search_memories" => {
            let q = call.args["query"].as_str().unwrap_or("").trim();
            let lim = call.args["limit"].as_u64().unwrap_or(8).clamp(1, 20) as usize;
            if q.is_empty() {
                (false, "Error: `query` cannot be empty".into(), "Empty query".into())
            } else {
                match db.list_memories(Some(q), lim) {
                    Ok(items) if items.is_empty() => (
                        true,
                        format!("No memory matches found for `{q}`."),
                        "0 hits".into(),
                    ),
                    Ok(items) => {
                        let count = items.len();
                        let text = items
                            .into_iter()
                            .map(|it| {
                                let body = it
                                    .content_text
                                    .as_deref()
                                    .unwrap_or(it.file_path.as_deref().unwrap_or("(image)"))
                                    .chars()
                                    .take(360)
                                    .collect::<String>();
                                format!("- [id={} kind={} tag={}] {}", it.id, it.kind, it.app_name, body)
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        (true, text, format!("{count} memory hits"))
                    }
                    Err(e) => (false, format!("Error: {e}"), e),
                }
            }
        }
        "save_memory_fact" => {
            let content = call.args["content"].as_str().unwrap_or("");
            let category = call.args["category"].as_str();
            match db.save_fact(content, category) {
                Ok(item) => {
                    let _ = app.emit("buddy://memory-cleaned", ());
                    crate::logger::log_info("memory", &format!("saved fact id={}", item.id));
                    (
                        true,
                        format!("Saved permanent memory fact [id={} tag={}]: {}", item.id, item.app_name, content.trim()),
                        "Fact saved".into(),
                    )
                }
                Err(e) => (false, format!("Error: {e}"), e),
            }
        }
        "web_search" => {
            let q = call.args["query"].as_str().unwrap_or("");
            let lim = call.args["limit"].as_u64().unwrap_or(5).clamp(1, 8) as usize;
            match execute_web_search(q, lim).await {
                Ok(out) => (true, out, "Search done".into()),
                Err(e) => (false, format!("Error: {e}"), e),
            }
        }
        "fetch_web_page" => {
            let url = call.args["url"].as_str().unwrap_or("");
            let max_chars = call.args["max_chars"].as_u64().unwrap_or(8000).clamp(500, 16000) as usize;
            match execute_fetch_web_page(url, max_chars).await {
                Ok(out) => (true, out, "Page fetched".into()),
                Err(e) => (false, format!("Error: {e}"), e),
            }
        }
        name if name.starts_with("mcp__") => match mcp_mgr.call_tool(name, call.args.clone()).await {
            Ok(out) => (true, out, "MCP tool done".into()),
            Err(e) => (false, format!("Error: {e}"), e),
        },
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

fn build_screen_watch_prompts(
    target_label: &str,
    goal: &str,
    cadence: &str,
    ui_language: &str,
    recent_whispers: &[String],
    has_prev_frame: bool,
) -> (String, String) {
    let goal_trimmed = goal.trim();
    let has_goal = !goal_trimmed.is_empty();
    let lang_rule = if has_goal {
        let hint = detect_thinking_language_hint(goal_trimmed, ui_language);
        format!("Reply naturally in {hint}.")
    } else if ui_language == "zh" {
        "Reply in concise natural Simplified Chinese.".to_string()
    } else {
        "Reply in concise natural English.".to_string()
    };

    let mut anti_repeat = String::new();
    if !recent_whispers.is_empty() {
        anti_repeat.push_str("\nAlready mentioned recently (DO NOT repeat these):\n");
        for w in recent_whispers {
            anti_repeat.push_str(&format!("- {w}\n"));
        }
    }

    let mission_block = if has_goal {
        format!(
            "PRIMARY MISSION (STRICT): Everything you say must directly advance the user's active goal: \"{goal_trimmed}\".\n\
             - Treat this goal as your filter for what matters on screen.\n\
             - NEVER passively describe, narrate, or summarize what app, page, or UI elements you see.\n\
             - NEVER offer unrelated design/layout/color tips unless the user's goal itself asks for design/UI feedback.\n\
             - If nothing visible right now needs action, insight, or follow-up for \"{goal_trimmed}\", output ONLY `[SILENT]`."
        )
    } else {
        "PRIMARY MISSION: Act as a quiet, practical co-worker.\n\
         - NEVER merely describe or narrate what app or content is on screen.\n\
         - Output `[SILENT]` unless you spot an actual error, blocker, or high-value actionable tip."
            .to_string()
    };

    let posture_rule = match (cadence, has_goal) {
        ("active", true) => format!(
            "The user chose ACTIVE mode for their goal \"{goal_trimmed}\". Whenever current or changed screen content gives you something concrete to help with THAT goal (answer, bug/error clue, next step, translation, or 1 sharp clarifying question about the goal), share 1-2 crisp sentences. Output `[SILENT]` if the screen has nothing new relevant to the goal."
        ),
        (_, true) => format!(
            "Output ONLY `[SILENT]` unless something on screen directly impacts \"{goal_trimmed}\" with an actionable diagnosis, blocker, answer, or key question."
        ),
        ("active", false) => {
            "The user chose ACTIVE co-work mode. Share 1 crisp specific suggestion, issue spotted, or helpful next step if something genuinely useful stands out; otherwise output `[SILENT]`.".to_string()
        }
        (_, false) => {
            "Output ONLY `[SILENT]` unless you spot a clear error, blocker, or genuinely high-value suggestion.".to_string()
        }
    };

    let sys_prompt = format!(
        "You are Buddy co-working alongside the user while watching their shared screen ({target_label}).\n\
         {mission_block}\n\
         Rules:\n\
         1. Compare the current screen against the previous snapshot (if provided) to see what the user is actively doing now.\n\
         2. {posture_rule}\n\
         3. Ground every remark in exact text, code, errors, or UI states visible on screen — never guess unreadable tiny text, never give generic praise ('looks good'), and never repeat recent remarks.\n\
         4. Keep replies to 1-2 natural, everyday plain-text sentences (max ~90 Chinese chars or ~30 English words). No markdown headers or bullet lists.\n\
         5. {lang_rule}{anti_repeat}"
    );

    let frame_note = if has_prev_frame {
        "Image 1 is the earlier frame; Image 2 is the settled current frame."
    } else {
        "Attached is the current screen snapshot."
    };

    let user_desc = if has_goal {
        format!(
            "Shared target: {target_label}.\n\
             USER'S ACTIVE GOAL: \"{goal_trimmed}\"\n\
             {frame_note}\n\
             Do NOT describe the screen. Respond strictly toward the Active Goal with [SILENT] or 1-2 short actionable sentences."
        )
    } else {
        format!(
            "Shared target: {target_label}.\n\
             {frame_note}\n\
             Do NOT merely narrate what is on screen. Respond with [SILENT] or 1-2 concise actionable sentences."
        )
    };

    (sys_prompt, user_desc)
}

/// Proactive low-noise screen watch check. Returns `Ok(None)` when `[SILENT]`.
#[allow(clippy::too_many_arguments)]
pub async fn run_screen_watch_check(
    client: &reqwest::Client,
    model: &ResolvedModelConfig,
    api_key: &str,
    target_label: &str,
    goal: &str,
    cadence: &str,
    ui_language: &str,
    recent_whispers: &[String],
    prev_frame: Option<&InlineImageFrame>,
    curr_frame: &InlineImageFrame,
) -> Result<Option<String>, String> {
    let (sys_prompt, user_desc) = build_screen_watch_prompts(
        target_label,
        goal,
        cadence,
        ui_language,
        recent_whispers,
        prev_frame.is_some(),
    );

    let mut frames = Vec::new();
    if let Some(pf) = prev_frame {
        frames.push(pf.clone());
    }
    frames.push(curr_frame.clone());

    let reply = run_oneshot_vision(client, model, api_key, &sys_prompt, &user_desc, &frames, 1024).await?;
    let trimmed = reply.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("[SILENT]")
        || trimmed.contains("[SILENT]")
    {
        return Ok(None);
    }
    Ok(Some(trimmed.to_string()))
}

struct StreamCleanupGuard {
    app: AppHandle,
    controller: Arc<AiStreamController>,
    browser_mgr: Arc<BrowserManager>,
    sensor_state: Arc<AppSensorState>,
}

impl Drop for StreamCleanupGuard {
    fn drop(&mut self) {
        self.controller.is_streaming.store(false, Ordering::Release);
        self.controller.clear_steers();
        self.browser_mgr.on_agent_turn_finished(&self.app, &self.sensor_state);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_chat_stream(
    app: AppHandle,
    settings_mgr: Arc<SettingsManager>,
    prompts_mgr: Arc<PromptsManager>,
    skills_mgr: Arc<SkillsManager>,
    plugin_mgr: Arc<PluginManager>,
    browser_mgr: Arc<BrowserManager>,
    mcp_mgr: Arc<McpManager>,
    sensor_state: Arc<AppSensorState>,
    db: Arc<MemoryDb>,
    controller: Arc<AiStreamController>,
    approval_mgr: Arc<AgentApprovalManager>,
    conversation_id: i64,
    stream_id: String,
    prompt: String,
    attachments: Vec<AttachmentPayload>,
    history: Vec<(String, String)>,
    conversation_summary: Option<String>,
    live_frames: Vec<InlineImageFrame>,
    screen_goal: Option<String>,
    watch_profile_override: Option<String>,
) {
    controller.cancel_flag.store(false, Ordering::Relaxed);
    controller.clear_steers();
    controller.is_streaming.store(true, Ordering::Release);
    let cancel_flag = controller.cancel_flag.clone();

    tauri::async_runtime::spawn(async move {
        let _cleanup_guard = StreamCleanupGuard {
            app: app.clone(),
            controller: controller.clone(),
            browser_mgr: browser_mgr.clone(),
            sensor_state: sensor_state.clone(),
        };
        let cfg = settings_mgr.get();
        let (chat_model, api_key) = settings_mgr.resolve_chat_target();
        let (is_local, is_anthropic, is_gemini, base_url) =
            classify_protocol(&chat_model.api_provider, &chat_model.base_url);

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

        let thinking_cap = infer_thinking_capability_with_mode(
            &chat_model.api_provider,
            &chat_model.base_url,
            &chat_model.model_id,
            &chat_model.thinking_mode,
        );
        let thinking_level = clamp_thinking_level(&cfg.thinking_level, &thinking_cap);
        let thinking_backend = thinking_cap.backend;
        let mirror_prefix = plugin_mgr.list().mirror_prefix;
        let media_dl_ready = plugin_mgr.is_engine_ready("media_downloader");
        let active_mcp_tools = mcp_mgr.list_active_tools();
        let thinking_lang = detect_thinking_lang(&prompt, &history, &cfg.ui_language);
        let mut sys_prompt = if thinking_lang == ThinkingLang::Zh {
            format!(
                "【全局最高优先级语言规则】用户当前使用简体中文交流。你在原生思考区（`reasoning_content` / `thinking`）以及 `<buddy_thought>` 中的每一句内部推理，都必须从第一句话开始全程使用自然、口语化、接地气的简体中文，严禁使用英文自言自语或英文思维模板（仅路径、命令与代码标识保留原样）。\n\n{}",
                prompts_mgr.build_system_prompt(&cfg.persona_id)
            )
        } else {
            prompts_mgr.build_system_prompt(&cfg.persona_id)
        };
        if let Ok(facts) = db.list_active_facts(8) {
            if !facts.is_empty() {
                let fact_lines = facts
                    .into_iter()
                    .filter_map(|f| {
                        f.content_text
                            .map(|txt| format!("- [{}] {}", f.app_name, txt.trim()))
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if !fact_lines.is_empty() {
                    sys_prompt.push_str(&format!(
                        "\n\n[Saved User Facts & Preferences]\n{}",
                        fact_lines
                    ));
                }
            }
        }
        if let Some(summary) = conversation_summary
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            sys_prompt.push_str(&format!(
                "\n\n[Earlier Conversation Summary]\n{}",
                summary
            ));
        }
        if cfg.agent_mode {
            sys_prompt.push_str(&build_agent_system_appendix(&cfg.agent_workspace, media_dl_ready));
            sys_prompt.push_str(&skills_mgr.build_prompt_appendix());
        }
        sys_prompt.push_str(&skills_mgr.build_explicit_slash_appendix(&prompt));
        if !live_frames.is_empty() {
            let goal_note = screen_goal
                .as_deref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|g| {
                    format!(
                        " The user's active screen-sharing goal is: \"{g}\". Prioritize helping fulfill this goal directly from the attached live frame(s) rather than passively describing the screen."
                    )
                })
                .unwrap_or_default();
            sys_prompt.push_str(&format!(
                "\n\n[LIVE SCREEN SHARE ACTIVE]\nThe user is actively sharing their screen with you right now. Use the attached live frame(s) to answer their question directly with specific visual feedback.{goal_note}"
            ));
        }
        sys_prompt.push_str(&build_thinking_appendix_for_lang(thinking_level, thinking_lang));

        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(150))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                crate::logger::log_error("ai", &format!("HTTP client error: {e}"));
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

        // Fallback for text-only Chat models while Screen Share is active: ask Watch Vision model to transcribe live screen
        let mut effective_prompt = prompt.clone();
        if thinking_lang == ThinkingLang::Zh && thinking_level != "off" {
            effective_prompt.push_str(
                "\n\n[内部思考语言提示：请从第一句思考起全程使用自然、接地气的简体中文推演，禁止英文内部思考。]",
            );
        }
        let effective_live_frames = if !live_frames.is_empty() && !chat_model.supports_vision {
            let (watch_model, watch_key) =
                settings_mgr.resolve_watch_target(watch_profile_override.as_deref());
            if watch_model.supports_vision {
                let vision_sys = "Summarize the key layout, visible UI text, colors, errors, and details on this screen concisely and accurately for another AI assistant.";
                if let Ok(summary) = run_oneshot_vision(
                    &client,
                    &watch_model,
                    &watch_key,
                    vision_sys,
                    "Describe the current screen state accurately.",
                    &live_frames,
                    420,
                )
                .await
                {
                    if !summary.trim().is_empty() {
                        effective_prompt.push_str(&format!(
                            "\n\n[Live Screen Summary from Watch Vision Model]\n{}",
                            summary.trim()
                        ));
                    }
                }
            }
            Vec::new()
        } else {
            live_frames
        };

        let workspace_hint = derive_workspace_hint(&cfg.agent_workspace, &attachments);
        let mut full_text = String::new();
        let mut full_thought = String::new();
        let mut recorded_steps: Vec<AgentStepEvent> = Vec::new();
        let mut approve_all_turn = false;

        let mut openai_msgs = if !is_anthropic && !is_gemini {
            build_openai_history(
                &sys_prompt,
                history.clone(),
                &effective_prompt,
                &attachments,
                &effective_live_frames,
            )
        } else {
            Vec::new()
        };
        let mut anthropic_msgs = if is_anthropic {
            build_anthropic_history(
                history.clone(),
                &effective_prompt,
                &attachments,
                &effective_live_frames,
            )
        } else {
            Vec::new()
        };
        let mut gemini_contents = if is_gemini {
            build_gemini_contents(
                history,
                &effective_prompt,
                &attachments,
                &effective_live_frames,
            )
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
                    &chat_model.model_id,
                    &sys_prompt,
                    &gemini_contents,
                    cfg.agent_mode,
                    media_dl_ready,
                    &active_mcp_tools,
                    thinking_level,
                    thinking_backend,
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
                    &chat_model.model_id,
                    &sys_prompt,
                    &anthropic_msgs,
                    cfg.agent_mode,
                    media_dl_ready,
                    &active_mcp_tools,
                    thinking_level,
                    thinking_backend,
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
                    &chat_model.model_id,
                    &openai_msgs,
                    cfg.agent_mode,
                    media_dl_ready,
                    &active_mcp_tools,
                    thinking_level,
                    thinking_backend,
                    &stream_id,
                    turn_idx,
                    &cancel_flag,
                )
                .await
            };

            match turn_res {
                Err(err) => {
                    approval_mgr.reject_for_stream(&stream_id);
                    if cancel_flag.load(Ordering::Relaxed) {
                        break;
                    }
                    crate::logger::log_error("ai", &format!("chat turn failed: {err}"));
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
                    let pending_steers = controller.drain_steers();
                    if pending_steers.is_empty() || cancel_flag.load(Ordering::Relaxed) {
                        break;
                    }
                    let combined_steer = format!(
                        "[LIVE USER INSTRUCTION DURING TASK]\n{}",
                        pending_steers.join("\n")
                    );
                    if is_gemini {
                        if !turn_text.is_empty() {
                            gemini_contents.push(json!({
                                "role": "model",
                                "parts": [{ "text": turn_text }]
                            }));
                        }
                        gemini_contents.push(json!({
                            "role": "user",
                            "parts": [{ "text": combined_steer }]
                        }));
                    } else if is_anthropic {
                        if !turn_text.is_empty() {
                            anthropic_msgs.push(json!({ "role": "assistant", "content": turn_text }));
                        }
                        anthropic_msgs.push(json!({ "role": "user", "content": combined_steer }));
                    } else {
                        if !turn_text.is_empty() {
                            openai_msgs.push(json!({ "role": "assistant", "content": turn_text }));
                        }
                        openai_msgs.push(json!({ "role": "user", "content": combined_steer }));
                    }
                }
                Ok(TurnOutcome::ToolCalls {
                    turn_text,
                    turn_thought,
                    native_reasoning,
                    calls,
                    gemini_raw_parts,
                    anthropic_raw_blocks,
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
                        let blocks = if anthropic_raw_blocks.is_empty() {
                            let mut fallback: Vec<Value> = Vec::new();
                            if !turn_text.is_empty() {
                                fallback.push(json!({ "type": "text", "text": turn_text }));
                            }
                            for c in &calls {
                                fallback.push(json!({
                                    "type": "tool_use",
                                    "id": c.id,
                                    "name": c.name,
                                    "input": c.args
                                }));
                            }
                            fallback
                        } else {
                            anthropic_raw_blocks
                        };
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
                        let mut assistant_msg = json!({
                            "role": "assistant",
                            "content": if turn_text.is_empty() { Value::Null } else { Value::String(turn_text) },
                            "tool_calls": tc_vals
                        });
                        if (base_url.contains("deepseek.com") || base_url.contains("xiaomimimo.com"))
                            && !native_reasoning.is_empty()
                        {
                            assistant_msg["reasoning_content"] = json!(native_reasoning);
                        }
                        openai_msgs.push(assistant_msg);
                    }

                    let mut anthropic_results: Vec<Value> = Vec::new();
                    let mut gemini_results: Vec<Value> = Vec::new();

                    for (call_idx, call) in calls.iter().enumerate() {
                        if cancel_flag.load(Ordering::Relaxed) {
                            break;
                        }
                        let permission_mode = AgentPermissionMode::from_str(
                            &settings_mgr.get().agent_permission_mode,
                        );
                        let (result_text, step_rec) = run_tool_with_approval(
                            &app,
                            &approval_mgr,
                            &skills_mgr,
                            &plugin_mgr,
                            &browser_mgr,
                            &sensor_state,
                            &mcp_mgr,
                            &settings_mgr,
                            &controller,
                            &client,
                            &chat_model,
                            &api_key,
                            &db,
                            &mirror_prefix,
                            &stream_id,
                            turn_idx,
                            call_idx,
                            call,
                            workspace_hint.as_deref(),
                            permission_mode,
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

                    let live_steers = controller.drain_steers();
                    if !live_steers.is_empty() {
                        let steer_msg = format!(
                            "[LIVE USER INSTRUCTION DURING TASK]\n{}",
                            live_steers.join("\n")
                        );
                        if is_gemini {
                            gemini_contents.push(json!({
                                "role": "user",
                                "parts": [{ "text": steer_msg }]
                            }));
                        } else if is_anthropic {
                            anthropic_msgs.push(json!({
                                "role": "user",
                                "content": steer_msg
                            }));
                        } else {
                            openai_msgs.push(json!({
                                "role": "user",
                                "content": steer_msg
                            }));
                        }
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
        let _ = app.emit(
            "buddy://conversations-changed",
            serde_json::json!({ "conversation_id": conversation_id }),
        );

        // Background rolling compaction when uncompacted conversation messages exceed threshold
        if !cancel_flag.load(Ordering::Relaxed) {
            if let Ok(Some((prev_summary, old_msgs, up_to_id))) =
                db.list_messages_to_compact(conversation_id, 22, 12)
            {
                let db_compact = db.clone();
                let compact_client = client.clone();
                let compact_model = chat_model.clone();
                let compact_key = api_key.clone();
                tauri::async_runtime::spawn(async move {
                    let msg_count = old_msgs.len();
                    let transcript = old_msgs
                        .into_iter()
                        .map(|m| {
                            let body: String = m.content.chars().take(500).collect();
                            format!("{}: {}", m.role, body)
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    let user_body = match prev_summary {
                        Some(prev) => format!(
                            "[Existing Summary]\n{prev}\n\n[Additional Older Turns]\n{transcript}"
                        ),
                        None => format!("[Older Conversation Turns]\n{transcript}"),
                    };
                    let compact_sys = "Summarize this conversation segment concisely (max ~220 words). Preserve key user goals, file paths, decisions, constraints, and unresolved questions verbatim.";
                    match run_oneshot_vision(
                        &compact_client,
                        &compact_model,
                        &compact_key,
                        compact_sys,
                        &user_body,
                        &[],
                        1536,
                    )
                    .await
                    {
                        Ok(summary) if !summary.trim().is_empty() => {
                            if let Err(e) = db_compact.update_conversation_summary(
                                conversation_id,
                                up_to_id,
                                &summary,
                            ) {
                                crate::logger::log_warn(
                                    "compaction",
                                    &format!("failed saving summary for conv={conversation_id}: {e}"),
                                );
                            } else {
                                crate::logger::log_info(
                                    "compaction",
                                    &format!(
                                        "compacted {msg_count} messages in conv={conversation_id} up_to_id={up_to_id}"
                                    ),
                                );
                            }
                        }
                        Err(e) => {
                            crate::logger::log_warn(
                                "compaction",
                                &format!("background summary skipped for conv={conversation_id}: {e}"),
                            );
                        }
                        _ => {}
                    }
                });
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_turn_clamping_caps_huge_turns_cleanly() {
        let short = "hello buddy";
        assert_eq!(clamp_history_turn_text(short), short);
        let huge = "a".repeat(5000);
        let clamped = clamp_history_turn_text(&huge);
        assert!(clamped.chars().count() < 3700);
        assert!(clamped.contains("truncated"));
    }

    #[test]
    fn screen_watch_prompt_anchors_user_goal_and_forbids_narration() {
        let (sys_prompt, user_desc) = build_screen_watch_prompts(
            "VS Code",
            "帮我盯着终端报错并给修复命令",
            "active",
            "en",
            &[],
            true,
        );
        assert!(sys_prompt.contains("帮我盯着终端报错并给修复命令"));
        assert!(sys_prompt.contains("NEVER passively describe"));
        assert!(sys_prompt.contains("Simplified Chinese"));
        assert!(user_desc.contains("USER'S ACTIVE GOAL: \"帮我盯着终端报错并给修复命令\""));
        assert!(user_desc.contains("Do NOT describe the screen"));
    }
}
