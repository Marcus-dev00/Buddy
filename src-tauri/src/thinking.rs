//! Per-model thinking backend inference, native reasoning payload helpers,
//! `<buddy_thought>` tag stream splitter, and language-mirrored thinking prompts.

use serde_json::{json, Value};

const OPEN_TAG: &str = "<buddy_thought>";
const CLOSE_TAG: &str = "</buddy_thought>";

#[derive(Clone, Copy, PartialEq, Eq)]
enum SplitPhase {
    Preamble,
    InsideThought,
    Answer,
}

/// Splits `<buddy_thought>...</buddy_thought>` out of streamed text deltas cleanly in real time.
pub struct ThoughtTagSplitter {
    enabled: bool,
    phase: SplitPhase,
    buf: String,
}

impl ThoughtTagSplitter {
    pub fn new(enabled: bool) -> Self {
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

    pub fn push(&mut self, delta: &str) -> (String, String) {
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

    pub fn finish(&mut self) -> (String, String) {
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
pub enum ApiProtocol {
    OpenAi,
    Anthropic,
    Gemini,
}

pub fn classify_protocol(api_provider: &str, base_url: &str) -> (bool, bool, bool, String) {
    let clean_url = base_url.trim().trim_end_matches('/').to_string();
    let is_local = clean_url.contains("localhost") || clean_url.contains("127.0.0.1");
    let is_anthropic = api_provider == "anthropic" || clean_url.contains("anthropic.com");
    let is_gemini = api_provider == "gemini"
        || (clean_url.contains("generativelanguage.googleapis.com")
            && !clean_url.contains("/openai"));
    (is_local, is_anthropic, is_gemini, clean_url)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThinkingBackend {
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
pub struct ThinkingCapability {
    pub levels: &'static [&'static str],
    pub backend: ThinkingBackend,
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
    let m = strip_vendor_prefix(model_lower);
    if !m.contains("claude") && !m.contains("opus") && !m.contains("sonnet") && !m.contains("haiku")
    {
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

pub fn infer_thinking_capability(
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

pub fn infer_thinking_capability_with_mode(
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

pub fn clamp_thinking_level(requested: &str, cap: &ThinkingCapability) -> &'static str {
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
pub fn apply_native_thinking(
    body: &mut Value,
    backend: ThinkingBackend,
    effective_level: &str,
) -> bool {
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

pub fn strip_native_thinking(body: &mut Value, protocol: ApiProtocol) {
    match protocol {
        ApiProtocol::Gemini => {
            if let Some(gen_cfg) = body
                .get_mut("generationConfig")
                .and_then(|v| v.as_object_mut())
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

pub fn is_thinking_param_rejection(status: reqwest::StatusCode, err_txt: &str) -> bool {
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

pub fn normalize_thinking_level(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "off" => "off",
        "low" => "low",
        "high" => "high",
        "max" => "max",
        _ => "medium",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThinkingLang {
    Zh,
    Ja,
    Ko,
    En,
    MatchUser,
}

fn is_non_prose_prompt(trimmed: &str) -> bool {
    if trimmed.is_empty() {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower == "analyze attached item" || lower == "analyze attached items" {
        return true;
    }
    let has_latin = trimmed.chars().any(|c| c.is_ascii_alphabetic());
    if !has_latin {
        return true;
    }
    !trimmed.contains(' ')
        && (lower.starts_with("http://")
            || lower.starts_with("https://")
            || trimmed.contains(":\\")
            || trimmed.starts_with('/'))
}

fn detect_text_script_lang(text: &str) -> Option<ThinkingLang> {
    if text
        .chars()
        .any(|c| matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF))
    {
        return Some(ThinkingLang::Zh);
    }
    if text
        .chars()
        .any(|c| matches!(c as u32, 0x3040..=0x30FF))
    {
        return Some(ThinkingLang::Ja);
    }
    if text
        .chars()
        .any(|c| matches!(c as u32, 0xAC00..=0xD7AF))
    {
        return Some(ThinkingLang::Ko);
    }
    None
}

pub fn detect_thinking_lang(
    prompt: &str,
    history: &[(String, String)],
    ui_language: &str,
) -> ThinkingLang {
    let trimmed = prompt.trim();
    if let Some(script) = detect_text_script_lang(trimmed) {
        return script;
    }
    if is_non_prose_prompt(trimmed) {
        for (role, content) in history.iter().rev() {
            if role == "user" {
                if let Some(script) = detect_text_script_lang(content) {
                    return script;
                }
            }
        }
        return if ui_language.eq_ignore_ascii_case("zh") {
            ThinkingLang::Zh
        } else {
            ThinkingLang::En
        };
    }
    ThinkingLang::MatchUser
}

pub fn detect_thinking_language_hint(prompt: &str, ui_language: &str) -> &'static str {
    match detect_thinking_lang(prompt, &[], ui_language) {
        ThinkingLang::Zh => "Simplified Chinese (简体中文)",
        ThinkingLang::Ja => "Japanese (日本語)",
        ThinkingLang::Ko => "Korean (한국어)",
        ThinkingLang::En => "English",
        ThinkingLang::MatchUser => "the exact same language as the user's latest message",
    }
}

pub fn build_thinking_appendix_for_lang(level: &str, lang: ThinkingLang) -> String {
    let norm = normalize_thinking_level(level);
    if lang == ThinkingLang::Zh {
        return match norm {
            "off" => "\n\n---\n\n# 思考模式：关闭（OFF）\n\
                      - 直接、自然地回复用户，不输出 `<buddy_thought>`，全程使用简体中文。"
                .to_string(),
            "low" => "\n\n---\n\n# 思考深度：轻量（LOW · 简体中文）\n\
                      - **强制语言规则**：无论原生思考块（`reasoning_content` / `thinking`）还是 `<buddy_thought>...</buddy_thought>`，**从第一句起全程使用接地气的简体中文推演**，严禁英文自言自语（路径/命令/代码标识除外）。\n\
                      - 快速核对核心事实与意图即可；若用 `<buddy_thought>`，正式回复写在 `</buddy_thought>` 后。"
                .to_string(),
            "high" => "\n\n---\n\n# 思考深度：深入（HIGH · 简体中文）\n\
                      - **强制语言规则**：无论原生思考块（`reasoning_content` / `thinking`）还是 `<buddy_thought>...</buddy_thought>`，**从第一个字到最后一个字全程使用自然口语化的简体中文推演**，严禁英文思考（路径/命令/代码标识除外）。\n\
                      - 回答前按步骤过一遍：1）确认真实意图（模糊或高风险先问）；2）核对事实与可行性，不对的地方温和直说；3）理清简洁表达结构后再正式作答。"
                .to_string(),
            "max" => "\n\n---\n\n# 思考深度：极致核验（MAX · 简体中文）\n\
                      - **强制语言规则**：无论原生思考块（`reasoning_content` / `thinking`）还是 `<buddy_thought>...</buddy_thought>`，**从第一句起全程使用自然接地气的简体中文思考**，严禁英文推演（路径/命令/代码标识除外）。\n\
                      - 回答前充分推演：列出边界情况与备选方案、排查报错与矛盾、逐一核对依据并确认完全站得住脚后再输出正式回复。"
                .to_string(),
            _ => "\n\n---\n\n# 思考深度：均衡（MEDIUM · 简体中文）\n\
                  - **强制语言规则**：无论原生思考块（`reasoning_content` / `thinking`）还是 `<buddy_thought>...</buddy_thought>`，**从第一句起全程使用自然、有人情味的简体中文思考**，严禁英文推演（路径/命令/代码标识除外）。\n\
                  - 回答前快速梳理意图、判断需不需要先确认或坦诚纠偏，核实关键步骤后再回复。"
                .to_string(),
        };
    }

    let lang_hint = match lang {
        ThinkingLang::Zh => "Simplified Chinese (简体中文)",
        ThinkingLang::Ja => "Japanese (日本語)",
        ThinkingLang::Ko => "Korean (한국어)",
        ThinkingLang::En => "English",
        ThinkingLang::MatchUser => "the exact same language as the user's latest message",
    };
    let lang_rule = format!(
        "- Write ALL internal reasoning (both native thinking/reasoning blocks and `<buddy_thought>...</buddy_thought>`) in {lang_hint} in a natural inner voice."
    );
    match norm {
        "off" => "\n\n---\n\n# Thinking Level: OFF\n\
                  - Respond directly with zero deliberation and no `<buddy_thought>` tags."
            .to_string(),
        "low" => format!(
            "\n\n---\n\n# Thinking Level: LOW\n\
             {lang_rule}\n\
             - Quick 1-2 line sanity check before replying (skip `<buddy_thought>` when using native reasoning blocks)."
        ),
        "high" => format!(
            "\n\n---\n\n# Thinking Level: HIGH\n\
             {lang_rule}\n\
             - Think step-by-step before replying (skip `<buddy_thought>` when using native reasoning blocks): verify intent, check feasibility/facts, and structure a concise answer."
        ),
        "max" => format!(
            "\n\n---\n\n# Thinking Level: MAX\n\
             {lang_rule}\n\
             - Exhaustively verify edge cases, outcomes, and facts before replying (skip `<buddy_thought>` when using native reasoning blocks)."
        ),
        _ => format!(
            "\n\n---\n\n# Thinking Level: MEDIUM\n\
             {lang_rule}\n\
             - Check intent, factual accuracy, and whether clarification or honest pushback is needed before replying."
        ),
    }
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

        let g25p = infer_thinking_capability(
            "gemini",
            "https://generativelanguage.googleapis.com",
            "gemini-2.5-pro",
        );
        assert_eq!(g25p.levels, &["low", "medium", "high", "max"]);
        assert_eq!(clamp_thinking_level("off", &g25p), "low");
        assert_eq!(clamp_thinking_level("max", &g25p), "max");

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

        let mimo = infer_thinking_capability(
            "openai",
            "https://api.xiaomimimo.com/v1",
            "mimo-v2-flash",
        );
        assert_eq!(mimo.levels, &["off", "high"]);
        assert_eq!(clamp_thinking_level("low", &mimo), "high");
        assert_eq!(clamp_thinking_level("off", &mimo), "off");

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

    #[test]
    fn thinking_appendix_follows_user_language_and_natural_tone() {
        assert_eq!(
            detect_thinking_language_hint("帮我看看这个报错怎么回事", "en"),
            "Simplified Chinese (简体中文)"
        );
        assert_eq!(
            detect_thinking_language_hint("", "zh"),
            "Simplified Chinese (简体中文)"
        );
        assert_eq!(
            detect_thinking_language_hint("Analyze attached item", "zh"),
            "Simplified Chinese (简体中文)"
        );
        assert_eq!(
            detect_thinking_language_hint("https://example.com", "zh"),
            "Simplified Chinese (简体中文)"
        );
        assert_eq!(
            detect_thinking_language_hint("Explain this Rust error", "zh"),
            "the exact same language as the user's latest message"
        );
        let lang = detect_thinking_lang("帮我排查一下", &[], "zh");
        let app_zh = build_thinking_appendix_for_lang("high", lang);
        assert!(app_zh.contains("简体中文"));
        assert!(app_zh.contains("reasoning_content"));
    }
}
