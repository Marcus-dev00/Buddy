//! Multi-protocol SSE streaming AI & Auto-Agent client (OpenAI-compatible, Anthropic Claude, Google Gemini),
//! 5-level thinking controller (off / low / medium / high / max) supporting both native reasoning deltas
//! and universal `<buddy_thought>...</buddy_thought>` streaming separation,
//! multimodal image compression, native tool-calling loop with interactive approval,
//! and live endpoint verification.

use crate::agent_tools::{
    anthropic_tools_json, build_agent_system_appendix, execute_fetch_web_page, execute_tool,
    execute_web_search, gemini_tools_json, openai_tools_json, plan_tool_execution,
    resolve_user_path, AgentApprovalManager, AgentStepEvent, ApprovalDecision,
};
use crate::db::MemoryDb;
use crate::plugins::{run_media_download, run_offline_ocr, PluginManager};
use crate::prompts::PromptsManager;
use crate::settings::{ResolvedModelConfig, SettingsManager};
use crate::skills::{search_github_skills, SkillsManager};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use image::{codecs::jpeg::JpegEncoder, imageops::FilterType, DynamicImage};
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

#[derive(Clone, Debug)]
pub struct InlineImageFrame {
    pub label: String,
    pub jpeg_b64: String,
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
        native_reasoning: String,
        calls: Vec<ParsedToolCall>,
        gemini_raw_parts: Vec<Value>,
        anthropic_raw_blocks: Vec<Value>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApiProtocol {
    OpenAi,
    Anthropic,
    Gemini,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThinkingBackend {
    PromptOnly,
    GeminiBudget { pro_min: bool },
    GeminiLevel { allow_minimal: bool },
    AnthropicBudget,
    AnthropicAdaptive,
    OpenAiEffort { allow_minimal: bool, allow_xhigh: bool },
    DeepSeekChat,
    BinaryThinkingToggle,
}

#[derive(Clone, Copy, Debug)]
struct ThinkingCapability {
    levels: &'static [&'static str],
    backend: ThinkingBackend,
}

const LEVELS_ALL: &[&str] = &["off", "low", "medium", "high", "max"];
const LEVELS_FLASH_3: &[&str] = &["off", "low", "medium", "high"];
const LEVELS_NO_OFF_4: &[&str] = &["low", "medium", "high", "max"];
const LEVELS_3_TIER: &[&str] = &["low", "medium", "high"];
const LEVELS_DEEPSEEK_CHAT: &[&str] = &["off", "low", "high", "max"];
const LEVELS_DEEPSEEK_R1: &[&str] = &["low", "high", "max"];
const LEVELS_BINARY: &[&str] = &["off", "high"];

fn strip_vendor_prefix(model_lower: &str) -> &str {
    let trimmed = model_lower.trim().trim_start_matches("models/");
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}

fn parse_claude_version(model_lower: &str) -> Option<(u8, u8)> {
    // Handles patterns like claude-sonnet-4-5, claude-3-7-sonnet, claude-opus-4.7, claude-sonnet-5
    let m = strip_vendor_prefix(model_lower);
    if !m.contains("claude") && !m.contains("opus") && !m.contains("sonnet") && !m.contains("haiku") {
        return None;
    }
    let tokens: Vec<&str> = m.split(|c: char| !c.is_ascii_alphanumeric()).collect();
    for i in 0..tokens.len() {
        let Ok(major) = tokens[i].parse::<u8>() else {
            continue;
        };
        if !(3..=9).contains(&major) {
            continue;
        }
        let minor = tokens
            .get(i + 1)
            .and_then(|t| t.parse::<u8>().ok())
            .filter(|&n| n <= 20)
            .unwrap_or(0);
        return Some((major, minor));
    }
    None
}

fn has_gemini_gen3_plus(model_lower: &str) -> bool {
    let m = strip_vendor_prefix(model_lower);
    m.contains("gemini-3")
        || m.contains("gemini3")
        || m.contains("gemini-4")
        || m.contains("gemini4")
}

fn parse_gpt5_minor(model_lower: &str) -> Option<(u8, bool)> {
    let m = strip_vendor_prefix(model_lower);
    let idx = m.find("gpt-5")?;
    let rest = &m[idx + "gpt-5".len()..];
    if let Some(after_dot) = rest.strip_prefix('.') {
        let minor: u8 = after_dot
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap_or(0);
        return Some((minor, m.contains("-pro")));
    }
    if rest.is_empty() || rest.starts_with('-') {
        return Some((0, m.contains("-pro")));
    }
    None
}

fn is_openai_o_series(model_lower: &str) -> bool {
    let m = strip_vendor_prefix(model_lower);
    if m == "o1-mini" || m == "o1-preview" {
        return false;
    }
    m == "o1"
        || m.starts_with("o1-")
        || m == "o3"
        || m.starts_with("o3-")
        || m == "o4"
        || m.starts_with("o4-")
}

fn infer_thinking_capability(
    api_provider: &str,
    base_url: &str,
    model_id: &str,
) -> ThinkingCapability {
    let (_, is_anthropic, is_gemini, clean_url) = classify_protocol(api_provider, base_url);
    let url_lower = clean_url.to_ascii_lowercase();
    let model_lower = model_id.trim().to_ascii_lowercase();
    let model_short = strip_vendor_prefix(&model_lower);

    if is_gemini {
        if has_gemini_gen3_plus(&model_lower) {
            if model_short.contains("pro") {
                return ThinkingCapability {
                    levels: LEVELS_3_TIER,
                    backend: ThinkingBackend::GeminiLevel {
                        allow_minimal: false,
                    },
                };
            }
            return ThinkingCapability {
                levels: LEVELS_FLASH_3,
                backend: ThinkingBackend::GeminiLevel {
                    allow_minimal: true,
                },
            };
        }
        if model_short.contains("2.5") || model_short.contains("thinking") {
            let is_pro = model_short.contains("pro");
            return ThinkingCapability {
                levels: if is_pro { LEVELS_NO_OFF_4 } else { LEVELS_ALL },
                backend: ThinkingBackend::GeminiBudget { pro_min: is_pro },
            };
        }
        return ThinkingCapability {
            levels: LEVELS_ALL,
            backend: ThinkingBackend::PromptOnly,
        };
    }

    if is_anthropic {
        if let Some((major, minor)) = parse_claude_version(&model_lower) {
            if (major, minor) >= (4, 7) {
                return ThinkingCapability {
                    levels: LEVELS_ALL,
                    backend: ThinkingBackend::AnthropicAdaptive,
                };
            }
            if (major, minor) >= (3, 7) {
                return ThinkingCapability {
                    levels: LEVELS_ALL,
                    backend: ThinkingBackend::AnthropicBudget,
                };
            }
        }
        return ThinkingCapability {
            levels: LEVELS_ALL,
            backend: ThinkingBackend::PromptOnly,
        };
    }

    if url_lower.contains("xiaomimimo.com") || model_short.starts_with("mimo") {
        return ThinkingCapability {
            levels: LEVELS_BINARY,
            backend: ThinkingBackend::BinaryThinkingToggle,
        };
    }

    if url_lower.contains("deepseek.com") || model_short.starts_with("deepseek") {
        let is_reasoner = model_short.contains("reasoner") || model_short.contains("r1");
        if url_lower.contains("deepseek.com") {
            if is_reasoner {
                return ThinkingCapability {
                    levels: LEVELS_DEEPSEEK_R1,
                    backend: ThinkingBackend::OpenAiEffort {
                        allow_minimal: false,
                        allow_xhigh: false,
                    },
                };
            }
            return ThinkingCapability {
                levels: LEVELS_DEEPSEEK_CHAT,
                backend: ThinkingBackend::DeepSeekChat,
            };
        }
        return ThinkingCapability {
            levels: if is_reasoner {
                LEVELS_NO_OFF_4
            } else {
                LEVELS_ALL
            },
            backend: ThinkingBackend::PromptOnly,
        };
    }

    if is_openai_o_series(&model_lower) {
        return ThinkingCapability {
            levels: LEVELS_3_TIER,
            backend: ThinkingBackend::OpenAiEffort {
                allow_minimal: false,
                allow_xhigh: false,
            },
        };
    }

    if let Some((minor, is_pro)) = parse_gpt5_minor(&model_lower) {
        let supports_xhigh = minor >= 2 || is_pro;
        return ThinkingCapability {
            levels: if supports_xhigh {
                LEVELS_ALL
            } else {
                LEVELS_FLASH_3
            },
            backend: ThinkingBackend::OpenAiEffort {
                allow_minimal: true,
                allow_xhigh: supports_xhigh,
            },
        };
    }

    ThinkingCapability {
        levels: LEVELS_ALL,
        backend: ThinkingBackend::PromptOnly,
    }
}

fn infer_thinking_capability_with_mode(
    api_provider: &str,
    base_url: &str,
    model_id: &str,
    thinking_mode: &str,
) -> ThinkingCapability {
    let mode = thinking_mode.trim().to_ascii_lowercase();
    if mode == "prompt" {
        return ThinkingCapability {
            levels: LEVELS_ALL,
            backend: ThinkingBackend::PromptOnly,
        };
    }
    let auto = infer_thinking_capability(api_provider, base_url, model_id);
    if mode == "native" && auto.backend == ThinkingBackend::PromptOnly {
        let (_, is_anthropic, is_gemini, _) = classify_protocol(api_provider, base_url);
        if is_gemini {
            return ThinkingCapability {
                levels: LEVELS_FLASH_3,
                backend: ThinkingBackend::GeminiLevel {
                    allow_minimal: true,
                },
            };
        }
        if is_anthropic {
            return ThinkingCapability {
                levels: LEVELS_ALL,
                backend: ThinkingBackend::AnthropicAdaptive,
            };
        }
        return ThinkingCapability {
            levels: LEVELS_FLASH_3,
            backend: ThinkingBackend::OpenAiEffort {
                allow_minimal: true,
                allow_xhigh: false,
            },
        };
    }
    auto
}

fn level_rank(level: &str) -> i32 {
    match level {
        "off" => 0,
        "low" => 1,
        "medium" => 2,
        "high" => 3,
        "max" => 4,
        _ => 2,
    }
}

fn clamp_thinking_level(requested: &str, cap: &ThinkingCapability) -> &'static str {
    let norm = normalize_thinking_level(requested);
    for &lvl in cap.levels {
        if lvl == norm {
            return lvl;
        }
    }
    let req_rank = level_rank(norm);
    // Never downgrade an explicitly active thinking level to "off" if an active level exists.
    let candidates: Vec<&'static str> = if norm != "off" {
        let non_off: Vec<&'static str> = cap
            .levels
            .iter()
            .copied()
            .filter(|&l| l != "off")
            .collect();
        if non_off.is_empty() {
            cap.levels.to_vec()
        } else {
            non_off
        }
    } else {
        cap.levels.to_vec()
    };

    candidates
        .into_iter()
        .min_by_key(|&lvl| ((level_rank(lvl) - req_rank).abs(), level_rank(lvl)))
        .unwrap_or("medium")
}

/// Applies provider-native thinking fields to `body` and reports whether any native fields were injected.
fn apply_native_thinking(body: &mut Value, backend: ThinkingBackend, effective_level: &str) -> bool {
    match backend {
        ThinkingBackend::PromptOnly => false,
        ThinkingBackend::GeminiBudget { pro_min } => {
            let cfg = match effective_level {
                "off" if !pro_min => json!({ "thinkingBudget": 0, "includeThoughts": false }),
                "off" => json!({ "thinkingBudget": 128, "includeThoughts": false }),
                "low" => json!({ "thinkingBudget": 1024, "includeThoughts": true }),
                "high" => json!({ "thinkingBudget": 10240, "includeThoughts": true }),
                "max" => json!({ "thinkingBudget": 24576, "includeThoughts": true }),
                _ => json!({ "thinkingBudget": 4096, "includeThoughts": true }),
            };
            body["generationConfig"]["thinkingConfig"] = cfg;
            true
        }
        ThinkingBackend::GeminiLevel { allow_minimal } => {
            let (lvl_str, include_thoughts) = match effective_level {
                "off" if allow_minimal => ("minimal", false),
                "off" | "low" => ("low", effective_level != "off"),
                "high" | "max" => ("high", true),
                _ => ("medium", true),
            };
            body["generationConfig"]["thinkingConfig"] = json!({
                "thinkingLevel": lvl_str,
                "includeThoughts": include_thoughts
            });
            true
        }
        ThinkingBackend::AnthropicBudget => {
            if effective_level == "off" {
                return false;
            }
            let (budget, max_tok) = match effective_level {
                "low" => (1024_u64, 8192_u64),
                "high" => (10240_u64, 20480_u64),
                "max" => (20480_u64, 32000_u64),
                _ => (4096_u64, 12288_u64),
            };
            body["max_tokens"] = json!(max_tok);
            body["thinking"] = json!({ "type": "enabled", "budget_tokens": budget });
            true
        }
        ThinkingBackend::AnthropicAdaptive => {
            if effective_level == "off" {
                return false;
            }
            let effort = match effective_level {
                "low" => "low",
                "high" => "high",
                "max" => "max",
                _ => "medium",
            };
            body["max_tokens"] = json!(16384_u64);
            body["thinking"] = json!({ "type": "adaptive" });
            body["output_config"] = json!({ "effort": effort });
            true
        }
        ThinkingBackend::OpenAiEffort {
            allow_minimal,
            allow_xhigh,
        } => {
            let effort = match effective_level {
                "off" if allow_minimal => "minimal",
                "off" | "low" => "low",
                "high" => "high",
                "max" if allow_xhigh => "xhigh",
                "max" => "high",
                _ => "medium",
            };
            body["reasoning_effort"] = json!(effort);
            true
        }
        ThinkingBackend::DeepSeekChat => {
            if effective_level == "off" {
                body["thinking"] = json!({ "type": "disabled" });
            } else {
                let effort = match effective_level {
                    "low" => "low",
                    "max" => "max",
                    _ => "high",
                };
                body["thinking"] = json!({ "type": "enabled" });
                body["reasoning_effort"] = json!(effort);
            }
            true
        }
        ThinkingBackend::BinaryThinkingToggle => {
            body["thinking"] = json!({
                "type": if effective_level == "off" { "disabled" } else { "enabled" }
            });
            true
        }
    }
}

fn strip_native_thinking(body: &mut Value, protocol: ApiProtocol) {
    match protocol {
        ApiProtocol::Gemini => {
            if let Some(gen_cfg) = body.get_mut("generationConfig").and_then(|v| v.as_object_mut())
            {
                gen_cfg.remove("thinkingConfig");
                if gen_cfg.is_empty() {
                    if let Some(root) = body.as_object_mut() {
                        root.remove("generationConfig");
                    }
                }
            }
        }
        ApiProtocol::Anthropic => {
            if let Some(root) = body.as_object_mut() {
                root.remove("thinking");
                root.remove("output_config");
                if root
                    .get("max_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0)
                    > 4096
                {
                    root.insert("max_tokens".to_string(), json!(4096));
                }
            }
        }
        ApiProtocol::OpenAi => {
            if let Some(root) = body.as_object_mut() {
                root.remove("reasoning_effort");
                root.remove("thinking");
            }
        }
    }
}

fn is_thinking_param_rejection(status: reqwest::StatusCode, err_txt: &str) -> bool {
    let code = status.as_u16();
    if code != 400 && code != 422 {
        return false;
    }
    let low = err_txt.to_ascii_lowercase();
    let mentions_field = low.contains("thinking")
        || low.contains("reasoning")
        || low.contains("budget")
        || low.contains("effort")
        || low.contains("output_config")
        || low.contains("thought");
    let mentions_invalid = low.contains("unknown")
        || low.contains("unsupported")
        || low.contains("unrecognized")
        || low.contains("invalid")
        || low.contains("extra")
        || low.contains("unexpected");
    mentions_field && mentions_invalid
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
async fn send_post_with_retry(
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
            api_messages.push(json!({ "role": role, "content": content }));
        }
    }
    api_messages.push(json!({
        "role": "user",
        "content": build_openai_user_content(prompt, attachments, live_frames)
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
    text: String,
    thinking_text: String,
    signature: String,
    redacted_data: String,
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
    include_media_downloader: bool,
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
        body["tools"] = openai_tools_json(include_media_downloader);
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
            native_reasoning,
            calls,
            gemini_raw_parts: Vec::new(),
            anthropic_raw_blocks: Vec::new(),
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
    include_media_downloader: bool,
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
        body["tools"] = anthropic_tools_json(include_media_downloader);
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
async fn run_gemini_turn(
    app: &AppHandle,
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    model_id: &str,
    sys_prompt: &str,
    contents: &[Value],
    agent_mode: bool,
    include_media_downloader: bool,
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
        body["tools"] = gemini_tools_json(include_media_downloader);
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
            native_reasoning: String::new(),
            calls,
            gemini_raw_parts: raw_model_parts,
            anthropic_raw_blocks: Vec::new(),
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
    plugin_mgr: &PluginManager,
    db: &Arc<MemoryDb>,
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
        "download_media" => {
            let url = call.args["url"].as_str().unwrap_or("");
            let fmt = call.args["format"].as_str().unwrap_or("audio_best");
            let out_dir = call.args["output_dir"].as_str();
            match run_media_download(app, plugin_mgr, url, fmt, out_dir).await {
                Ok(out) => {
                    let saved = out.outputs.first().cloned().unwrap_or_default();
                    (
                        true,
                        format!("{}\nSaved Path: {saved}", out.summary),
                        out.summary,
                    )
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

fn classify_protocol(api_provider: &str, base_url: &str) -> (bool, bool, bool, String) {
    let clean_url = base_url.trim().trim_end_matches('/').to_string();
    let is_local = clean_url.contains("localhost") || clean_url.contains("127.0.0.1");
    let is_anthropic = api_provider == "anthropic" || clean_url.contains("anthropic.com");
    let is_gemini = api_provider == "gemini"
        || (clean_url.contains("generativelanguage.googleapis.com")
            && !clean_url.contains("/openai"));
    (is_local, is_anthropic, is_gemini, clean_url)
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
    let lang_rule = if ui_language == "zh" {
        "Reply in concise natural Simplified Chinese."
    } else {
        "Reply in concise natural English (unless the user's goal text is in another language, then match it)."
    };

    let goal_line = if goal.trim().is_empty() {
        "General co-working observation (UI, layout, errors, or helpful clarification)."
    } else {
        goal.trim()
    };

    let mut anti_repeat = String::new();
    if !recent_whispers.is_empty() {
        anti_repeat.push_str("\nAlready mentioned recently (DO NOT repeat these):\n");
        for w in recent_whispers {
            anti_repeat.push_str(&format!("- {w}\n"));
        }
    }

    let posture_rule = if cadence == "active" {
        "The user chose ACTIVE co-work mode and specifically wants you to speak up like a sharp design/coding partner. Whenever you see concrete visual content, layout/typography/color/hierarchy details to improve, an error/ambiguity, or a natural clarifying question about their goal, share 1 crisp specific suggestion or question! Only reply `[SILENT]` if the screen shows nothing meaningful or nothing new since your already-mentioned remarks."
    } else {
        "Output ONLY `[SILENT]` unless you spot a clear, actionable design/layout/contrast/alignment/typography flaw, a visible error/blocker, OR you have one genuinely useful clarifying question or suggestion that helps the user's work right now."
    };

    let sys_prompt = format!(
        "You are Buddy quietly co-working alongside the user while watching their shared screen ({target_label}).\n\
        Collaboration focus: {goal_line}\n\
        Rules:\n\
        1. Compare the current screen against the previous snapshot (if any).\n\
        2. {posture_rule}\n\
        3. Reference concrete visible UI elements, labels, code/error text, or layout areas you actually read on screen. Never give empty generic praise ('looks good') or guess unreadable tiny text, and never repeat recent remarks.\n\
        4. When speaking, write ONLY 1-2 short, direct plain-text sentences (max ~90 Chinese chars or ~30 English words). No markdown headers or bullet lists.\n\
        5. {lang_rule}{anti_repeat}"
    );

    let mut frames = Vec::new();
    let mut user_desc = format!("Shared target: {target_label}.");
    if let Some(pf) = prev_frame {
        frames.push(pf.clone());
        user_desc.push_str(" Image 1 is the earlier frame; Image 2 is the settled current frame.");
    } else {
        user_desc.push_str(" Attached is the initial screen snapshot.");
    }
    frames.push(curr_frame.clone());
    user_desc.push_str(" Respond with [SILENT] or 1-2 concise sentences.");

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
    conversation_summary: Option<String>,
    live_frames: Vec<InlineImageFrame>,
    screen_goal: Option<String>,
    watch_profile_override: Option<String>,
) {
    controller.cancel_flag.store(false, Ordering::Relaxed);
    let cancel_flag = controller.cancel_flag.clone();

    tauri::async_runtime::spawn(async move {
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
        let mut sys_prompt = prompts_mgr.build_system_prompt(&cfg.persona_id);
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
                .map(|g| format!(" Co-working goal: {g}."))
                .unwrap_or_default();
            sys_prompt.push_str(&format!(
                "\n\n[LIVE SCREEN SHARE ACTIVE]\nThe user is actively sharing their screen with you right now. Use the attached live frame(s) to answer their question directly with specific visual feedback.{goal_note}"
            ));
        }
        sys_prompt.push_str(&build_thinking_appendix(thinking_level));

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
                    break;
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
                        let (result_text, step_rec) = run_tool_with_approval(
                            &app,
                            &approval_mgr,
                            &skills_mgr,
                            &plugin_mgr,
                            &db,
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

    #[test]
    fn infers_and_clamps_model_specific_thinking_capabilities() {
        // Gemini 3.5 Flash Lite -> 4 native tiers, clamps max -> high
        let g35 = infer_thinking_capability(
            "gemini",
            "https://generativelanguage.googleapis.com",
            "gemini-3.5-flash-lite",
        );
        assert_eq!(g35.levels, &["off", "low", "medium", "high"]);
        assert_eq!(
            g35.backend,
            ThinkingBackend::GeminiLevel {
                allow_minimal: true
            }
        );
        assert_eq!(clamp_thinking_level("max", &g35), "high");
        assert_eq!(clamp_thinking_level("off", &g35), "off");

        // Gemini 2.5 Pro -> cannot turn off completely
        let g25p = infer_thinking_capability(
            "gemini",
            "https://generativelanguage.googleapis.com",
            "gemini-2.5-pro",
        );
        assert_eq!(g25p.levels, &["low", "medium", "high", "max"]);
        assert_eq!(clamp_thinking_level("off", &g25p), "low");
        assert_eq!(clamp_thinking_level("max", &g25p), "max");

        // Claude 4.5 (budget_tokens) vs Claude 4.7+ (adaptive) vs Claude 3.5 Haiku (prompt)
        let c45 = infer_thinking_capability(
            "anthropic",
            "https://api.anthropic.com",
            "claude-sonnet-4-5",
        );
        assert_eq!(c45.backend, ThinkingBackend::AnthropicBudget);

        let c47 = infer_thinking_capability(
            "anthropic",
            "https://api.anthropic.com",
            "claude-opus-4-7",
        );
        assert_eq!(c47.backend, ThinkingBackend::AnthropicAdaptive);

        let c35h = infer_thinking_capability(
            "anthropic",
            "https://api.anthropic.com",
            "claude-3-5-haiku-latest",
        );
        assert_eq!(c35h.backend, ThinkingBackend::PromptOnly);

        // OpenAI o3 vs GPT-5 vs GPT-5.2 vs GPT-4o
        let o3 = infer_thinking_capability("openai", "https://api.openai.com/v1", "o3");
        assert_eq!(o3.levels, &["low", "medium", "high"]);
        assert_eq!(clamp_thinking_level("off", &o3), "low");
        assert_eq!(clamp_thinking_level("max", &o3), "high");

        let gpt5 = infer_thinking_capability("openai", "https://api.openai.com/v1", "gpt-5-mini");
        assert_eq!(gpt5.levels, &["off", "low", "medium", "high"]);
        assert_eq!(clamp_thinking_level("max", &gpt5), "high");

        let gpt52 = infer_thinking_capability("openai", "https://api.openai.com/v1", "gpt-5.2");
        assert_eq!(gpt52.levels, &["off", "low", "medium", "high", "max"]);

        let gpt4o = infer_thinking_capability("openai", "https://api.openai.com/v1", "gpt-4o-mini");
        assert_eq!(gpt4o.backend, ThinkingBackend::PromptOnly);

        // MiMo binary thinking toggle: active low preference must NOT clamp down to off
        let mimo = infer_thinking_capability(
            "openai",
            "https://api.xiaomimimo.com/v1",
            "mimo-v2-flash",
        );
        assert_eq!(mimo.levels, &["off", "high"]);
        assert_eq!(clamp_thinking_level("low", &mimo), "high");
        assert_eq!(clamp_thinking_level("off", &mimo), "off");

        // DeepSeek Chat V3
        let ds = infer_thinking_capability(
            "openai",
            "https://api.deepseek.com/v1",
            "deepseek-chat",
        );
        assert_eq!(ds.levels, &["off", "low", "high", "max"]);
        assert_eq!(clamp_thinking_level("medium", &ds), "low");
    }

    #[test]
    fn native_thinking_payload_and_strip_work_cleanly() {
        // Gemini 3 uses thinkingLevel and never sets thinkingBudget
        let mut g_body = json!({ "contents": [] });
        assert!(apply_native_thinking(
            &mut g_body,
            ThinkingBackend::GeminiLevel {
                allow_minimal: true
            },
            "high"
        ));
        assert_eq!(
            g_body["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "high"
        );
        assert!(g_body["generationConfig"]["thinkingConfig"]
            .get("thinkingBudget")
            .is_none());
        strip_native_thinking(&mut g_body, ApiProtocol::Gemini);
        assert!(g_body.get("generationConfig").is_none());

        // Anthropic budget ensures max_tokens > budget_tokens and cleans up on downgrade
        let mut a_body = json!({ "model": "claude-sonnet-4-5", "max_tokens": 4096 });
        assert!(apply_native_thinking(
            &mut a_body,
            ThinkingBackend::AnthropicBudget,
            "max"
        ));
        assert!(
            a_body["max_tokens"].as_u64().unwrap()
                > a_body["thinking"]["budget_tokens"].as_u64().unwrap()
        );
        strip_native_thinking(&mut a_body, ApiProtocol::Anthropic);
        assert!(a_body.get("thinking").is_none());
        assert_eq!(a_body["max_tokens"], 4096);

        // OpenAI strip cleans reasoning_effort & thinking
        let mut o_body = json!({ "model": "deepseek-chat" });
        assert!(apply_native_thinking(
            &mut o_body,
            ThinkingBackend::DeepSeekChat,
            "max"
        ));
        assert_eq!(o_body["reasoning_effort"], "max");
        assert_eq!(o_body["thinking"]["type"], "enabled");
        strip_native_thinking(&mut o_body, ApiProtocol::OpenAi);
        assert!(o_body.get("reasoning_effort").is_none());
        assert!(o_body.get("thinking").is_none());
    }

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

    #[test]
    fn manual_thinking_mode_overrides_auto_detection() {
        let prompt_forced = infer_thinking_capability_with_mode(
            "gemini",
            "https://generativelanguage.googleapis.com",
            "gemini-3.5-flash-lite",
            "prompt",
        );
        assert_eq!(prompt_forced.backend, ThinkingBackend::PromptOnly);
        assert_eq!(prompt_forced.levels, &["off", "low", "medium", "high", "max"]);

        let native_forced = infer_thinking_capability_with_mode(
            "openai",
            "https://my-proxy.example.com/v1",
            "custom-reasoning-alias",
            "native",
        );
        assert_eq!(
            native_forced.backend,
            ThinkingBackend::OpenAiEffort {
                allow_minimal: true,
                allow_xhigh: false
            }
        );
        assert_eq!(native_forced.levels, &["off", "low", "medium", "high"]);
    }
}
