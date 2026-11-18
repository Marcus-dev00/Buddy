//! Local Desktop Agent toolset & interactive approval gate.
//! Provides read-only exploration (`list_dir`, `find_files`, `search_text`, `read_file`)
//! and approval-gated filesystem / shell mutations (`create_dir`, `write_file`, `edit_file`,
//! `move_or_rename`, `copy_file`, `delete_path`, `run_shell`).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::oneshot;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    ApproveOnce,
    ApproveAllTurn,
    Reject,
}

#[derive(Default)]
pub struct AgentApprovalManager {
    pending: Mutex<HashMap<String, (String, oneshot::Sender<ApprovalDecision>)>>,
}

impl AgentApprovalManager {
    pub fn register(&self, stream_id: &str, step_id: &str) -> oneshot::Receiver<ApprovalDecision> {
        let (tx, rx) = oneshot::channel();
        if let Ok(mut map) = self.pending.lock() {
            map.insert(step_id.to_string(), (stream_id.to_string(), tx));
        }
        rx
    }

    pub fn resolve(&self, step_id: &str, decision: ApprovalDecision) -> Result<(), String> {
        let entry = self
            .pending
            .lock()
            .map_err(|e| e.to_string())?
            .remove(step_id);
        if let Some((_, tx)) = entry {
            let _ = tx.send(decision);
            Ok(())
        } else {
            Err("Approval request expired or already handled".to_string())
        }
    }

    pub fn reject_for_stream(&self, stream_id: &str) {
        if let Ok(mut map) = self.pending.lock() {
            let keys: Vec<String> = map
                .iter()
                .filter_map(|(k, (sid, _))| if sid == stream_id { Some(k.clone()) } else { None })
                .collect();
            for k in keys {
                if let Some((_, tx)) = map.remove(&k) {
                    let _ = tx.send(ApprovalDecision::Reject);
                }
            }
        }
    }

    pub fn reject_all(&self) {
        if let Ok(mut map) = self.pending.lock() {
            for (_, (_, tx)) in map.drain() {
                let _ = tx.send(ApprovalDecision::Reject);
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentStepEvent {
    pub stream_id: String,
    pub step_id: String,
    pub tool_name: String,
    pub summary: String,
    pub status: String, // "running" | "waiting_approval" | "done" | "rejected" | "error"
    pub detail: Option<String>,
    pub preview: Option<String>,
    pub is_destructive: bool,
}

#[derive(Clone, Debug)]
pub struct ToolExecutionPlan {
    pub summary: String,
    pub preview: Option<String>,
    pub requires_approval: bool,
    pub allow_approve_all: bool,
    pub is_destructive: bool,
}

pub fn user_home_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\"))
}

pub fn resolve_user_path(raw: &str, workspace_hint: Option<&Path>) -> Result<PathBuf, String> {
    let trimmed = raw.trim().trim_matches('"');
    if trimmed.is_empty() {
        return Ok(workspace_hint
            .map(|p| p.to_path_buf())
            .unwrap_or_else(user_home_dir));
    }

    let home = user_home_dir();
    let lower = trimmed.to_ascii_lowercase();

    let expanded = if trimmed == "~" {
        home
    } else if let Some(rest) = trimmed.strip_prefix("~/").or_else(|| trimmed.strip_prefix("~\\")) {
        home.join(rest)
    } else if lower == "desktop" || lower.starts_with("desktop\\") || lower.starts_with("desktop/") {
        let rest = &trimmed[7..].trim_start_matches(['\\', '/']);
        if rest.is_empty() {
            home.join("Desktop")
        } else {
            home.join("Desktop").join(rest)
        }
    } else if lower == "documents"
        || lower.starts_with("documents\\")
        || lower.starts_with("documents/")
    {
        let rest = &trimmed[9..].trim_start_matches(['\\', '/']);
        if rest.is_empty() {
            home.join("Documents")
        } else {
            home.join("Documents").join(rest)
        }
    } else if lower == "downloads"
        || lower.starts_with("downloads\\")
        || lower.starts_with("downloads/")
    {
        let rest = &trimmed[9..].trim_start_matches(['\\', '/']);
        if rest.is_empty() {
            home.join("Downloads")
        } else {
            home.join("Downloads").join(rest)
        }
    } else {
        let p = PathBuf::from(trimmed);
        if p.is_absolute() {
            p
        } else if let Some(base) = workspace_hint {
            base.join(p)
        } else {
            home.join(p)
        }
    };

    Ok(normalize_lexical(&expanded))
}

fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    out.components().next_back(),
                    Some(Component::Prefix(_)) | Some(Component::RootDir) | None
                ) {
                    out.pop();
                }
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}

fn is_blocked_path(path: &Path, is_write: bool) -> bool {
    let s = path.to_string_lossy().to_ascii_lowercase().replace('/', "\\");
    let blocked_prefixes = [
        "c:\\windows",
        "c:\\$recycle.bin",
        "c:\\system volume information",
    ];
    if blocked_prefixes.iter().any(|p| s == *p || s.starts_with(&format!("{p}\\"))) {
        return true;
    }
    if is_write {
        let write_protected = ["c:\\program files", "c:\\program files (x86)"];
        if write_protected
            .iter()
            .any(|p| s == *p || s.starts_with(&format!("{p}\\")))
        {
            return true;
        }
    }
    let sensitive_fragments = [
        "\\buddy\\secret.dpapi",
        "\\.ssh\\",
        "\\.gnupg\\",
        "\\.aws\\credentials",
    ];
    if sensitive_fragments.iter().any(|f| s.contains(f) || s.ends_with(f.trim_end_matches('\\'))) {
        return true;
    }
    false
}

pub(crate) fn check_path_allowed(path: &Path, is_write: bool) -> Result<(), String> {
    if is_blocked_path(path, is_write) {
        return Err(format!(
            "Access blocked for protected OS or credential path: {}",
            path.display()
        ));
    }
    Ok(())
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn should_skip_dir_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        ".git" | "node_modules" | "target" | "$recycle.bin" | "system volume information"
    )
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    let mut out: String = s.chars().take(max_chars).collect();
    if s.chars().count() > max_chars {
        out.push_str("\n...[truncated]");
    }
    out
}

fn decode_text_lossy(bytes: &[u8]) -> Result<String, String> {
    if bytes.starts_with(&[0xFF, 0xFE]) && bytes.len() >= 2 {
        let u16s: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return Ok(String::from_utf16_lossy(&u16s));
    }
    if bytes.starts_with(&[0xFE, 0xFF]) && bytes.len() >= 2 {
        let u16s: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return Ok(String::from_utf16_lossy(&u16s));
    }
    if bytes.iter().take(4096).filter(|&&b| b == 0).count() > 8 {
        return Err("Binary file detected; cannot display as plain UTF-8 text.".to_string());
    }
    String::from_utf8(bytes.to_vec())
        .or_else(|_| Ok(String::from_utf8_lossy(bytes).into_owned()))
}

/// Canonical tool definitions for OpenAI, Anthropic, and Gemini schemas.
fn raw_tool_definitions(include_media_downloader: bool) -> Vec<(&'static str, &'static str, Value)> {
    let mut defs = vec![
        (
            "list_dir",
            "List files and folders inside a directory on Windows. Supports `~`, `Desktop`, `Documents`, `Downloads`, or absolute paths.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path to list." },
                    "recursive": { "type": "boolean", "description": "Whether to traverse subdirectories (default false)." },
                    "max_depth": { "type": "integer", "description": "Max depth when recursive=true (default 2, max 4)." }
                },
                "required": ["path"]
            }),
        ),
        (
            "find_files",
            "Search for files or directories under `root` whose names contain `query` (case-insensitive) or match `*` wildcards.",
            json!({
                "type": "object",
                "properties": {
                    "root": { "type": "string", "description": "Starting directory path." },
                    "query": { "type": "string", "description": "Filename substring or pattern (e.g. '*.pdf', 'invoice')." }
                },
                "required": ["root", "query"]
            }),
        ),
        (
            "search_text",
            "Search inside plain text/code/markdown/csv/json files under `root` for a text substring (case-insensitive).",
            json!({
                "type": "object",
                "properties": {
                    "root": { "type": "string", "description": "Directory or file path to search in." },
                    "query": { "type": "string", "description": "Text to search for." },
                    "extension": { "type": "string", "description": "Optional extension filter without dot, e.g. 'md', 'rs', 'csv'." }
                },
                "required": ["root", "query"]
            }),
        ),
        (
            "read_file",
            "Read a text file with 1-based line numbers. Supports line ranges so large files can be read in chunks.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to read." },
                    "start_line": { "type": "integer", "description": "Optional 1-based start line (default 1)." },
                    "end_line": { "type": "integer", "description": "Optional 1-based end line (default start_line + 200)." }
                },
                "required": ["path"]
            }),
        ),
        (
            "create_dir",
            "Create a directory (and missing parent directories). Requires user approval.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path to create." }
                },
                "required": ["path"]
            }),
        ),
        (
            "write_file",
            "Create a new text file or overwrite an existing one (set overwrite=true to replace an existing file). Requires user approval.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Destination file path." },
                    "content": { "type": "string", "description": "Full text content to write." },
                    "overwrite": { "type": "boolean", "description": "Must be true if replacing an existing file." }
                },
                "required": ["path", "content"]
            }),
        ),
        (
            "edit_file",
            "Edit an existing text file by replacing an exact `target` substring with `replacement`. Requires user approval.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to modify." },
                    "target": { "type": "string", "description": "Exact existing text block to replace (must match uniquely)." },
                    "replacement": { "type": "string", "description": "Replacement text." }
                },
                "required": ["path", "target", "replacement"]
            }),
        ),
        (
            "move_or_rename",
            "Move or rename a file or directory. Creates destination parent directory if needed. Requires user approval.",
            json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Existing source file or directory path." },
                    "destination": { "type": "string", "description": "New file or directory path." }
                },
                "required": ["source", "destination"]
            }),
        ),
        (
            "copy_file",
            "Copy a file to a new destination path. Requires user approval.",
            json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Source file path." },
                    "destination": { "type": "string", "description": "Destination file path." }
                },
                "required": ["source", "destination"]
            }),
        ),
        (
            "delete_path",
            "Send a file or folder safely to the Windows Recycle Bin. Always requires individual user approval.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File or directory path to move to the Recycle Bin." }
                },
                "required": ["path"]
            }),
        ),
        (
            "run_shell",
            "Run a Windows PowerShell command (e.g. for batch operations, archives, scripts, PDF/Office extraction via Python/PowerShell). Always requires individual user approval.",
            json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "PowerShell command or script snippet to execute." },
                    "working_dir": { "type": "string", "description": "Optional working directory path." }
                },
                "required": ["command"]
            }),
        ),
        (
            "list_installed_skills",
            "List all Agent Skills installed in Buddy's local skill directory (`%LOCALAPPDATA%/Buddy/skills/`), including enabled state and descriptions.",
            json!({
                "type": "object",
                "properties": {}
            }),
        ),
        (
            "read_skill",
            "Load the full `SKILL.md` instructions and file list of an installed Agent Skill by its `id` or `name` before executing it.",
            json!({
                "type": "object",
                "properties": {
                    "skill_id": { "type": "string", "description": "Skill ID or name." }
                },
                "required": ["skill_id"]
            }),
        ),
        (
            "search_github_skills",
            "Search GitHub for Agent Skill repositories (by keywords), or pass an `owner/repo` / GitHub URL to discover all `SKILL.md` subfolders inside that repo.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search keywords (e.g. 'pdf', 'ocr') OR GitHub repo (`owner/repo` / URL)." }
                },
                "required": ["query"]
            }),
        ),
        (
            "install_github_skill",
            "Download and install an Agent Skill from GitHub (`owner/repo` or `https://github.com/owner/repo/tree/<branch>/<subpath>`) into Buddy's skills directory. Always requires user approval.",
            json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "GitHub repository or subfolder URL containing `SKILL.md`." }
                },
                "required": ["source"]
            }),
        ),
        (
            "offline_ocr",
            "Extract text from a screenshot or image file using fast Windows native offline OCR (zero AI cost). Use this when the user asks to OCR/extract text from an image or invokes `/ocr`.",
            json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Path to the image file (or screenshot path from attachments)." }
                },
                "required": ["source"]
            }),
        ),
        (
            "search_memories",
            "Search Buddy's SQLite memory database (saved long-term facts, clipboard text/snippets, and screenshot records) by keywords.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search keywords." },
                    "limit": { "type": "integer", "description": "Max items to return (default 8, max 20)." }
                },
                "required": ["query"]
            }),
        ),
        (
            "save_memory_fact",
            "Persist an important long-term user fact, preference, stack choice, or project detail into permanent memory (exempt from 7-day cleanup).",
            json!({
                "type": "object",
                "properties": {
                    "content": { "type": "string", "description": "Concise self-contained fact worth remembering across sessions." },
                    "category": { "type": "string", "description": "Short tag: preference | project | workflow | general." }
                },
                "required": ["content"]
            }),
        ),
        (
            "web_search",
            "Search the public web for up-to-date facts, docs, releases, or answers (read-only).",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Web search query." },
                    "limit": { "type": "integer", "description": "Max results (default 5, max 8)." }
                },
                "required": ["query"]
            }),
        ),
        (
            "fetch_web_page",
            "Fetch a public http:// or https:// web page and extract readable plain text (SSRF-protected against localhost/LAN URLs).",
            json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Public http:// or https:// URL." },
                    "max_chars": { "type": "integer", "description": "Maximum text characters to return (default 8000, max 16000)." }
                },
                "required": ["url"]
            }),
        ),
    ];

    if include_media_downloader {
        defs.push((
            "download_media",
            "Download audio or video from a URL (YouTube, Bilibili, SoundCloud, etc.) using Buddy's yt-dlp + FFmpeg plugin. Prefer 'audio_best' (zero re-encode original audio stream, no secondary loss) or 'audio_flac' (lossless FLAC) for highest audio fidelity, 'mp4_lossless' (zero re-encode stream copy MP4) for video, or 'audio_mp3' (320kbps MP3) when explicit MP3 compatibility is requested. Always requires user approval.",
            json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Full http:// or https:// media/music URL." },
                    "format": {
                        "type": "string",
                        "enum": ["audio_best", "mp4_lossless", "audio_flac", "audio_mp3"],
                        "description": "Output mode. Use audio_best for zero secondary loss original audio, audio_flac for lossless FLAC, mp4_lossless for zero re-encode MP4 video, or audio_mp3 for 320kbps MP3."
                    },
                    "output_dir": {
                        "type": "string",
                        "description": "Optional target directory (defaults to Windows Downloads folder)."
                    }
                },
                "required": ["url", "format"]
            }),
        ));
    }

    defs
}

pub fn openai_tools_json(include_media_downloader: bool) -> Value {
    let arr: Vec<Value> = raw_tool_definitions(include_media_downloader)
        .into_iter()
        .map(|(name, desc, params)| {
            json!({
                "type": "function",
                "function": {
                    "name": name,
                    "description": desc,
                    "parameters": params
                }
            })
        })
        .collect();
    Value::Array(arr)
}

pub fn anthropic_tools_json(include_media_downloader: bool) -> Value {
    let arr: Vec<Value> = raw_tool_definitions(include_media_downloader)
        .into_iter()
        .map(|(name, desc, params)| {
            json!({
                "name": name,
                "description": desc,
                "input_schema": params
            })
        })
        .collect();
    Value::Array(arr)
}

pub fn gemini_tools_json(include_media_downloader: bool) -> Value {
    let decls: Vec<Value> = raw_tool_definitions(include_media_downloader)
        .into_iter()
        .map(|(name, desc, params)| {
            json!({
                "name": name,
                "description": desc,
                "parameters": params
            })
        })
        .collect();
    json!([{ "functionDeclarations": decls }])
}

fn short_path_label(path: &Path) -> String {
    let s = path.to_string_lossy();
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 44 {
        return s.into_owned();
    }
    let tail: String = chars[chars.len() - 36..].iter().collect();
    format!("...{tail}")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentPermissionMode {
    Ask,
    Default,
    Auto,
}

impl AgentPermissionMode {
    pub fn from_str(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "ask" => Self::Ask,
            "auto" => Self::Auto,
            _ => Self::Default,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolRiskKind {
    ReadOnly,
    ReversibleWrite,
    ExternalSideEffect,
    DangerousOrDelete,
}

fn approval_flags_for(mode: AgentPermissionMode, risk: ToolRiskKind) -> (bool, bool, bool) {
    let is_destructive = risk == ToolRiskKind::DangerousOrDelete;
    match (mode, risk) {
        (_, ToolRiskKind::ReadOnly) => (false, false, false),
        (
            AgentPermissionMode::Ask,
            ToolRiskKind::ReversibleWrite
            | ToolRiskKind::ExternalSideEffect
            | ToolRiskKind::DangerousOrDelete,
        ) => (true, false, is_destructive),
        (AgentPermissionMode::Default, ToolRiskKind::ReversibleWrite) => (true, true, false),
        (
            AgentPermissionMode::Default,
            ToolRiskKind::ExternalSideEffect | ToolRiskKind::DangerousOrDelete,
        ) => (true, false, is_destructive),
        (
            AgentPermissionMode::Auto,
            ToolRiskKind::ReversibleWrite | ToolRiskKind::ExternalSideEffect,
        ) => (false, false, false),
        (AgentPermissionMode::Auto, ToolRiskKind::DangerousOrDelete) => (true, false, true),
    }
}

fn is_dangerous_shell_command(cmd: &str) -> bool {
    let normalized: String = cmd
        .to_ascii_lowercase()
        .replace('`', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        return true;
    }

    const SUBSTR_PATTERNS: &[&str] = &[
        "remove-item",
        "clear-content",
        "clear-item",
        "remove-itemproperty",
        "set-itemproperty",
        "new-itemproperty",
        "stop-process",
        "stop-service",
        "set-service",
        "remove-service",
        "restart-computer",
        "stop-computer",
        "format-volume",
        "format-disk",
        "clear-disk",
        "remove-partition",
        "invoke-expression",
        "-encodedcommand",
        "frombase64string",
        "start-bitstransfer",
        "file]::delete",
        "directory]::delete",
        ".delete(",
        "os.remove",
        "os.unlink",
        "shutil.rmtree",
        "fs.unlink",
        "fs.rm",
        "rmsync",
        "rmdirsync",
        "unlinksync",
        "uninstall-package",
        "uninstall-module",
        "winget uninstall",
        "npm uninstall",
        "pip uninstall",
        "cargo clean",
        "git clean",
        "git reset --hard",
        "git push",
        "git branch -d",
        "git checkout --",
        "git restore",
        "git rebase",
        "reg delete",
        "reg add",
        "reg import",
        "reg restore",
        "-verb runas",
        "-verb 'runas'",
        "-verb \"runas\"",
        " -force",
        " --force",
    ];
    if SUBSTR_PATTERNS.iter().any(|p| normalized.contains(p)) {
        return true;
    }
    if normalized.contains("cipher") && normalized.contains("/w") {
        return true;
    }
    if normalized.contains(" -enc ") || normalized.ends_with(" -enc") {
        return true;
    }

    const DANGEROUS_CMDS: &[&str] = &[
        "rm",
        "del",
        "erase",
        "rd",
        "rmdir",
        "ri",
        "clc",
        "kill",
        "taskkill",
        "shutdown",
        "logoff",
        "diskpart",
        "bcdedit",
        "vssadmin",
        "wbadmin",
        "reg",
        "regedit",
        "schtasks",
        "netsh",
        "net",
        "icacls",
        "takeown",
        "iex",
        "runas",
        "format",
        "sfc",
        "dism",
        "sc",
    ];

    for segment in normalized.split(|c| matches!(c, ';' | '|' | '&' | '\n' | '\r' | '(' | '{')) {
        let mut iter = segment.split_whitespace().peekable();
        // Skip shell wrappers / invocation prefixes to inspect the actual verb
        while let Some(raw_tok) = iter.peek().copied() {
            let tok = raw_tok
                .trim_matches(|c| matches!(c, '\'' | '"' | '.'))
                .trim_end_matches(".exe");
            if matches!(
                tok,
                "" | "cmd" | "powershell" | "pwsh" | "/c" | "/k" | "-c" | "-command" | "-noprofile"
                    | "-nologo" | "-noninteractive" | "sudo"
            ) {
                iter.next();
                continue;
            }
            if DANGEROUS_CMDS.contains(&tok) {
                return true;
            }
            break;
        }
    }

    false
}

fn make_plan(
    summary: String,
    preview: Option<String>,
    mode: AgentPermissionMode,
    risk: ToolRiskKind,
) -> ToolExecutionPlan {
    let (requires_approval, allow_approve_all, is_destructive) = approval_flags_for(mode, risk);
    ToolExecutionPlan {
        summary,
        preview,
        requires_approval,
        allow_approve_all,
        is_destructive,
    }
}

/// Builds a user-readable plan and safety classification before executing a tool.
pub fn plan_tool_execution(
    name: &str,
    args: &Value,
    workspace_hint: Option<&Path>,
    permission_mode: AgentPermissionMode,
) -> ToolExecutionPlan {
    match name {
        "list_dir" => {
            let raw = args["path"].as_str().unwrap_or("");
            let label = resolve_user_path(raw, workspace_hint)
                .map(|p| short_path_label(&p))
                .unwrap_or_else(|_| raw.to_string());
            make_plan(
                format!("List folder `{label}`"),
                None,
                permission_mode,
                ToolRiskKind::ReadOnly,
            )
        }
        "find_files" => {
            let q = args["query"].as_str().unwrap_or("");
            let r = args["root"].as_str().unwrap_or("");
            let label = resolve_user_path(r, workspace_hint)
                .map(|p| short_path_label(&p))
                .unwrap_or_else(|_| r.to_string());
            make_plan(
                format!("Find `{q}` in `{label}`"),
                None,
                permission_mode,
                ToolRiskKind::ReadOnly,
            )
        }
        "search_text" => {
            let q = args["query"].as_str().unwrap_or("");
            let r = args["root"].as_str().unwrap_or("");
            let label = resolve_user_path(r, workspace_hint)
                .map(|p| short_path_label(&p))
                .unwrap_or_else(|_| r.to_string());
            make_plan(
                format!("Search \"{q}\" in `{label}`"),
                None,
                permission_mode,
                ToolRiskKind::ReadOnly,
            )
        }
        "read_file" => {
            let raw = args["path"].as_str().unwrap_or("");
            let label = resolve_user_path(raw, workspace_hint)
                .map(|p| short_path_label(&p))
                .unwrap_or_else(|_| raw.to_string());
            make_plan(
                format!("Read `{label}`"),
                None,
                permission_mode,
                ToolRiskKind::ReadOnly,
            )
        }
        "create_dir" => {
            let raw = args["path"].as_str().unwrap_or("");
            let resolved = resolve_user_path(raw, workspace_hint)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| raw.to_string());
            let dir_name = Path::new(&resolved)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&resolved)
                .to_string();
            make_plan(
                format!("Create folder `{dir_name}`"),
                Some(resolved),
                permission_mode,
                ToolRiskKind::ReversibleWrite,
            )
        }
        "write_file" => {
            let raw = args["path"].as_str().unwrap_or("");
            let content = args["content"].as_str().unwrap_or("");
            let overwrite = args["overwrite"].as_bool().unwrap_or(false);
            let resolved = resolve_user_path(raw, workspace_hint)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| raw.to_string());
            let snippet = truncate_chars(content, 360);
            let file_label = Path::new(&resolved)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&resolved)
                .to_string();
            make_plan(
                format!(
                    "{} `{}` ({} bytes)",
                    if overwrite { "Overwrite" } else { "Write" },
                    file_label,
                    content.len()
                ),
                Some(format!("Path: {resolved}\n---\n{snippet}")),
                permission_mode,
                if overwrite {
                    ToolRiskKind::DangerousOrDelete
                } else {
                    ToolRiskKind::ReversibleWrite
                },
            )
        }
        "edit_file" => {
            let raw = args["path"].as_str().unwrap_or("");
            let target = args["target"].as_str().unwrap_or("");
            let replacement = args["replacement"].as_str().unwrap_or("");
            let resolved = resolve_user_path(raw, workspace_hint)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| raw.to_string());
            let preview = format!(
                "Path: {resolved}\n- {}\n+ {}",
                truncate_chars(target, 160),
                truncate_chars(replacement, 160)
            );
            let file_label = Path::new(&resolved)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&resolved)
                .to_string();
            make_plan(
                format!("Edit `{file_label}`"),
                Some(preview),
                permission_mode,
                ToolRiskKind::ReversibleWrite,
            )
        }
        "move_or_rename" => {
            let src = args["source"].as_str().unwrap_or("");
            let dst = args["destination"].as_str().unwrap_or("");
            let src_res = resolve_user_path(src, workspace_hint)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| src.to_string());
            let dst_res = resolve_user_path(dst, workspace_hint)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| dst.to_string());
            let s_name = Path::new(&src_res)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&src_res)
                .to_string();
            let d_name = Path::new(&dst_res)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&dst_res)
                .to_string();
            make_plan(
                format!("Move `{s_name}` → `{d_name}`"),
                Some(format!("From: {src_res}\nTo:   {dst_res}")),
                permission_mode,
                ToolRiskKind::ReversibleWrite,
            )
        }
        "copy_file" => {
            let src = args["source"].as_str().unwrap_or("");
            let dst = args["destination"].as_str().unwrap_or("");
            let src_res = resolve_user_path(src, workspace_hint)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| src.to_string());
            let dst_res = resolve_user_path(dst, workspace_hint)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| dst.to_string());
            let dst_exists = Path::new(&dst_res).exists();
            let s_name = Path::new(&src_res)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&src_res)
                .to_string();
            make_plan(
                if dst_exists {
                    format!("Overwrite `{s_name}` (copy)")
                } else {
                    format!("Copy `{s_name}`")
                },
                Some(format!("From: {src_res}\nTo:   {dst_res}")),
                permission_mode,
                if dst_exists {
                    ToolRiskKind::DangerousOrDelete
                } else {
                    ToolRiskKind::ReversibleWrite
                },
            )
        }
        "delete_path" => {
            let raw = args["path"].as_str().unwrap_or("");
            let resolved = resolve_user_path(raw, workspace_hint)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| raw.to_string());
            let file_label = Path::new(&resolved)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&resolved)
                .to_string();
            make_plan(
                format!("Recycle `{file_label}`"),
                Some(format!("Send to Recycle Bin:\n{resolved}")),
                permission_mode,
                ToolRiskKind::DangerousOrDelete,
            )
        }
        "run_shell" => {
            let cmd = args["command"].as_str().unwrap_or("");
            let wd = args["working_dir"].as_str().unwrap_or("");
            let wd_label = if wd.trim().is_empty() {
                workspace_hint
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| user_home_dir().display().to_string())
            } else {
                resolve_user_path(wd, workspace_hint)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| wd.to_string())
            };
            let shell_risk = if is_dangerous_shell_command(cmd) {
                ToolRiskKind::DangerousOrDelete
            } else {
                ToolRiskKind::ReversibleWrite
            };
            make_plan(
                "Run PowerShell command".to_string(),
                Some(format!("CWD: {wd_label}\n$ {}", truncate_chars(cmd, 420))),
                permission_mode,
                shell_risk,
            )
        }
        "list_installed_skills" => make_plan(
            "List installed Skills".to_string(),
            None,
            permission_mode,
            ToolRiskKind::ReadOnly,
        ),
        "read_skill" => {
            let id = args["skill_id"].as_str().unwrap_or("");
            make_plan(
                format!("Load Skill `{id}`"),
                None,
                permission_mode,
                ToolRiskKind::ReadOnly,
            )
        }
        "search_github_skills" => {
            let q = args["query"].as_str().unwrap_or("");
            make_plan(
                format!("Search GitHub Skills `{q}`"),
                None,
                permission_mode,
                ToolRiskKind::ReadOnly,
            )
        }
        "install_github_skill" => {
            let src = args["source"].as_str().unwrap_or("");
            make_plan(
                format!("Install GitHub Skill `{}`", truncate_chars(src, 42)),
                Some(format!("Source: {src}\nInstall into %LOCALAPPDATA%/Buddy/skills/")),
                permission_mode,
                ToolRiskKind::ExternalSideEffect,
            )
        }
        "offline_ocr" => {
            let src = args["source"].as_str().unwrap_or("");
            let name = Path::new(src)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(src);
            make_plan(
                format!("Offline OCR `{}`", truncate_chars(name, 36)),
                None,
                permission_mode,
                ToolRiskKind::ReadOnly,
            )
        }
        "search_memories" => {
            let q = args["query"].as_str().unwrap_or("");
            make_plan(
                format!("Search memories `{}`", truncate_chars(q, 32)),
                None,
                permission_mode,
                ToolRiskKind::ReadOnly,
            )
        }
        "save_memory_fact" => {
            let fact = args["content"].as_str().unwrap_or("");
            make_plan(
                format!("Save fact: `{}`", truncate_chars(fact, 36)),
                None,
                permission_mode,
                ToolRiskKind::ReadOnly,
            )
        }
        "web_search" => {
            let q = args["query"].as_str().unwrap_or("");
            make_plan(
                format!("Web search `{}`", truncate_chars(q, 34)),
                None,
                permission_mode,
                ToolRiskKind::ReadOnly,
            )
        }
        "fetch_web_page" => {
            let url = args["url"].as_str().unwrap_or("");
            make_plan(
                format!("Fetch `{}`", truncate_chars(url, 38)),
                None,
                permission_mode,
                ToolRiskKind::ReadOnly,
            )
        }
        "download_media" => {
            let url = args["url"].as_str().unwrap_or("");
            let fmt = args["format"].as_str().unwrap_or("audio_best");
            let out_dir = args["output_dir"].as_str().unwrap_or("").trim();
            let dir_note = if out_dir.is_empty() {
                "Windows Downloads folder".to_string()
            } else {
                resolve_user_path(out_dir, workspace_hint)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| out_dir.to_string())
            };
            make_plan(
                format!("Download `{fmt}` via yt-dlp + FFmpeg"),
                Some(format!("URL: {}\nFormat: {fmt}\nSave to: {dir_note}", truncate_chars(url, 180))),
                permission_mode,
                ToolRiskKind::ExternalSideEffect,
            )
        }
        other => make_plan(
            format!("Call `{other}`"),
            None,
            permission_mode,
            ToolRiskKind::DangerousOrDelete,
        ),
    }
}

/// Executes a single validated tool call and returns (ok, output_text, short_detail).
pub fn execute_tool(
    name: &str,
    args: &Value,
    workspace_hint: Option<&Path>,
) -> (bool, String, String) {
    match exec_inner(name, args, workspace_hint) {
        Ok((out, detail)) => (true, out, detail),
        Err(err) => (false, format!("Error: {err}"), err),
    }
}

fn exec_inner(
    name: &str,
    args: &Value,
    workspace_hint: Option<&Path>,
) -> Result<(String, String), String> {
    match name {
        "list_dir" => {
            let raw = args["path"]
                .as_str()
                .ok_or_else(|| "Missing `path`".to_string())?;
            let root = resolve_user_path(raw, workspace_hint)?;
            check_path_allowed(&root, false)?;
            if !root.exists() {
                return Err(format!("Path does not exist: {}", root.display()));
            }
            if !root.is_dir() {
                return Err(format!("Not a directory: {}", root.display()));
            }
            let recursive = args["recursive"].as_bool().unwrap_or(false);
            let max_depth = args["max_depth"].as_u64().unwrap_or(2).clamp(1, 4) as usize;
            let mut lines = Vec::new();
            collect_dir_lines(&root, &root, recursive, 1, max_depth, &mut lines, 150)?;
            let count = lines.len();
            let body = if lines.is_empty() {
                format!("Directory `{}` is empty.", root.display())
            } else {
                format!("Directory: {}\n{}", root.display(), lines.join("\n"))
            };
            Ok((body, format!("{count} items")))
        }
        "find_files" => {
            let raw = args["root"]
                .as_str()
                .ok_or_else(|| "Missing `root`".to_string())?;
            let query = args["query"]
                .as_str()
                .ok_or_else(|| "Missing `query`".to_string())?
                .trim();
            if query.is_empty() {
                return Err("`query` cannot be empty".to_string());
            }
            let root = resolve_user_path(raw, workspace_hint)?;
            check_path_allowed(&root, false)?;
            if !root.is_dir() {
                return Err(format!("Not a directory: {}", root.display()));
            }
            let mut hits = Vec::new();
            walk_find_files(&root, query, 0, 6, &mut hits, 80);
            let count = hits.len();
            let out = if hits.is_empty() {
                format!("No matches for `{query}` under {}", root.display())
            } else {
                format!("Found {count} match(es):\n{}", hits.join("\n"))
            };
            Ok((out, format!("{count} matches")))
        }
        "search_text" => {
            let raw = args["root"]
                .as_str()
                .ok_or_else(|| "Missing `root`".to_string())?;
            let query = args["query"]
                .as_str()
                .ok_or_else(|| "Missing `query`".to_string())?;
            if query.is_empty() {
                return Err("`query` cannot be empty".to_string());
            }
            let ext_filter = args["extension"]
                .as_str()
                .map(|s| s.trim().trim_start_matches('.').to_ascii_lowercase())
                .filter(|s| !s.is_empty());
            let root = resolve_user_path(raw, workspace_hint)?;
            check_path_allowed(&root, false)?;
            let query_lower = query.to_ascii_lowercase();
            let mut hits = Vec::new();
            if root.is_file() {
                search_in_file(&root, &query_lower, &mut hits, 50);
            } else if root.is_dir() {
                walk_search_text(
                    &root,
                    &query_lower,
                    ext_filter.as_deref(),
                    0,
                    6,
                    &mut hits,
                    50,
                );
            } else {
                return Err(format!("Path does not exist: {}", root.display()));
            }
            let count = hits.len();
            let out = if hits.is_empty() {
                format!("No text matches for \"{query}\" under {}", root.display())
            } else {
                format!("Matched {count} line(s):\n{}", hits.join("\n"))
            };
            Ok((out, format!("{count} hits")))
        }
        "read_file" => {
            let raw = args["path"]
                .as_str()
                .ok_or_else(|| "Missing `path`".to_string())?;
            let path = resolve_user_path(raw, workspace_hint)?;
            check_path_allowed(&path, false)?;
            if !path.is_file() {
                return Err(format!("File not found: {}", path.display()));
            }
            let meta = fs::metadata(&path).map_err(|e| e.to_string())?;
            if meta.len() > 8 * 1024 * 1024 {
                return Err(format!(
                    "File too large for direct text read ({}). Use `run_shell` to inspect slices.",
                    format_size(meta.len())
                ));
            }
            let bytes = fs::read(&path).map_err(|e| e.to_string())?;
            let text = decode_text_lossy(&bytes)?;
            let lines: Vec<&str> = text.lines().collect();
            let total = lines.len();
            let start = args["start_line"].as_u64().unwrap_or(1).max(1) as usize;
            let default_end = start.saturating_add(199);
            let end = args["end_line"]
                .as_u64()
                .map(|v| v as usize)
                .unwrap_or(default_end)
                .min(total)
                .min(start.saturating_add(249));
            if total == 0 {
                return Ok((
                    format!("File `{}` is empty (0 lines).", path.display()),
                    "0 lines".to_string(),
                ));
            }
            if start > total {
                return Err(format!("start_line {start} exceeds total lines ({total})"));
            }
            let slice = lines[start - 1..end]
                .iter()
                .enumerate()
                .map(|(i, l)| format!("{}: {}", start + i, l))
                .collect::<Vec<_>>()
                .join("\n");
            let header = format!(
                "File: {} (lines {start}-{end} of {total}, {})",
                path.display(),
                format_size(meta.len())
            );
            Ok((
                format!("{header}\n{}", truncate_chars(&slice, 20_000)),
                format!("L{start}-{end}/{total}"),
            ))
        }
        "create_dir" => {
            let raw = args["path"]
                .as_str()
                .ok_or_else(|| "Missing `path`".to_string())?;
            let path = resolve_user_path(raw, workspace_hint)?;
            check_path_allowed(&path, true)?;
            fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            Ok((
                format!("Created directory: {}", path.display()),
                "Created folder".to_string(),
            ))
        }
        "write_file" => {
            let raw = args["path"]
                .as_str()
                .ok_or_else(|| "Missing `path`".to_string())?;
            let content = args["content"]
                .as_str()
                .ok_or_else(|| "Missing `content`".to_string())?;
            let overwrite = args["overwrite"].as_bool().unwrap_or(false);
            let path = resolve_user_path(raw, workspace_hint)?;
            check_path_allowed(&path, true)?;
            if path.exists() && !overwrite {
                return Err(format!(
                    "File already exists (`{}`). Pass overwrite=true or use `edit_file`.",
                    path.display()
                ));
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(&path, content.as_bytes()).map_err(|e| e.to_string())?;
            Ok((
                format!("Wrote {} bytes to {}", content.len(), path.display()),
                format!("{} bytes", content.len()),
            ))
        }
        "edit_file" => {
            let raw = args["path"]
                .as_str()
                .ok_or_else(|| "Missing `path`".to_string())?;
            let target = args["target"]
                .as_str()
                .ok_or_else(|| "Missing `target`".to_string())?;
            let replacement = args["replacement"]
                .as_str()
                .ok_or_else(|| "Missing `replacement`".to_string())?;
            if target.is_empty() {
                return Err("`target` cannot be empty".to_string());
            }
            let path = resolve_user_path(raw, workspace_hint)?;
            check_path_allowed(&path, true)?;
            if !path.is_file() {
                return Err(format!("File not found: {}", path.display()));
            }
            let orig = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let matches = orig.match_indices(target).count();
            if matches == 0 {
                return Err("`target` block not found in file.".to_string());
            }
            if matches > 1 {
                return Err(format!(
                    "`target` matched {matches} places; provide more surrounding context so it matches uniquely."
                ));
            }
            let updated = orig.replacen(target, replacement, 1);
            fs::write(&path, updated.as_bytes()).map_err(|e| e.to_string())?;
            Ok((
                format!("Edited `{}` successfully.", path.display()),
                "Patched 1 block".to_string(),
            ))
        }
        "move_or_rename" => {
            let src_raw = args["source"]
                .as_str()
                .ok_or_else(|| "Missing `source`".to_string())?;
            let dst_raw = args["destination"]
                .as_str()
                .ok_or_else(|| "Missing `destination`".to_string())?;
            let src = resolve_user_path(src_raw, workspace_hint)?;
            let dst = resolve_user_path(dst_raw, workspace_hint)?;
            check_path_allowed(&src, true)?;
            check_path_allowed(&dst, true)?;
            if !src.exists() {
                return Err(format!("Source does not exist: {}", src.display()));
            }
            if dst.exists() {
                return Err(format!("Destination already exists: {}", dst.display()));
            }
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::rename(&src, &dst).map_err(|e| e.to_string())?;
            Ok((
                format!("Moved `{}` -> `{}`", src.display(), dst.display()),
                "Moved".to_string(),
            ))
        }
        "copy_file" => {
            let src_raw = args["source"]
                .as_str()
                .ok_or_else(|| "Missing `source`".to_string())?;
            let dst_raw = args["destination"]
                .as_str()
                .ok_or_else(|| "Missing `destination`".to_string())?;
            let src = resolve_user_path(src_raw, workspace_hint)?;
            let dst = resolve_user_path(dst_raw, workspace_hint)?;
            check_path_allowed(&src, false)?;
            check_path_allowed(&dst, true)?;
            if !src.is_file() {
                return Err(format!("Source file does not exist: {}", src.display()));
            }
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let copied = fs::copy(&src, &dst).map_err(|e| e.to_string())?;
            Ok((
                format!(
                    "Copied `{}` -> `{}` ({})",
                    src.display(),
                    dst.display(),
                    format_size(copied)
                ),
                format_size(copied),
            ))
        }
        "delete_path" => {
            let raw = args["path"]
                .as_str()
                .ok_or_else(|| "Missing `path`".to_string())?;
            let path = resolve_user_path(raw, workspace_hint)?;
            check_path_allowed(&path, true)?;
            if !path.exists() {
                return Err(format!("Path does not exist: {}", path.display()));
            }
            send_to_recycle_bin(&path)?;
            Ok((
                format!("Moved `{}` to Windows Recycle Bin.", path.display()),
                "Recycled".to_string(),
            ))
        }
        "run_shell" => {
            let cmd = args["command"]
                .as_str()
                .ok_or_else(|| "Missing `command`".to_string())?;
            let wd = if let Some(wd_raw) = args["working_dir"].as_str().filter(|s| !s.trim().is_empty()) {
                let p = resolve_user_path(wd_raw, workspace_hint)?;
                check_path_allowed(&p, false)?;
                p
            } else {
                workspace_hint
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(user_home_dir)
            };
            let (code, stdout, stderr) = run_powershell_with_timeout(cmd, &wd, Duration::from_secs(30))?;
            let mut combined = format!("Exit code: {code}");
            if !stdout.trim().is_empty() {
                combined.push_str(&format!("\n--- STDOUT ---\n{}", truncate_chars(stdout.trim(), 8000)));
            }
            if !stderr.trim().is_empty() {
                combined.push_str(&format!("\n--- STDERR ---\n{}", truncate_chars(stderr.trim(), 4000)));
            }
            Ok((combined, format!("exit {code}")))
        }
        other => Err(format!("Unknown tool: {other}")),
    }
}

fn collect_dir_lines(
    root: &Path,
    dir: &Path,
    recursive: bool,
    depth: usize,
    max_depth: usize,
    out: &mut Vec<String>,
    cap: usize,
) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| e.to_string())?;
    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for entry in entries.flatten() {
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if !should_skip_dir_name(&name) {
                dirs.push((name, p));
            }
        } else {
            files.push((name, p, entry.metadata().ok()));
        }
    }

    dirs.sort_by(|a, b| a.0.to_ascii_lowercase().cmp(&b.0.to_ascii_lowercase()));
    files.sort_by(|a, b| a.0.to_ascii_lowercase().cmp(&b.0.to_ascii_lowercase()));

    for (_, p) in dirs {
        if out.len() >= cap {
            return Ok(());
        }
        let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy();
        out.push(format!("[DIR]  {rel}\\"));
        if recursive && depth < max_depth {
            let _ = collect_dir_lines(root, &p, recursive, depth + 1, max_depth, out, cap);
        }
    }

    for (_, p, meta) in files {
        if out.len() >= cap {
            return Ok(());
        }
        let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy();
        let size_s = meta
            .as_ref()
            .map(|m| format_size(m.len()))
            .unwrap_or_else(|| "?".to_string());
        let age_s = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .map(|d| {
                let mins = d.as_secs() / 60;
                if mins < 60 {
                    format!("{mins}m ago")
                } else if mins < 1440 {
                    format!("{}h ago", mins / 60)
                } else {
                    format!("{}d ago", mins / 1440)
                }
            })
            .unwrap_or_default();
        if age_s.is_empty() {
            out.push(format!("[FILE] {rel} ({size_s})"));
        } else {
            out.push(format!("[FILE] {rel} ({size_s}, {age_s})"));
        }
    }
    Ok(())
}

fn wildcard_match_ci(pattern: &str, text: &str) -> bool {
    let p_lower = pattern.to_ascii_lowercase();
    let t_lower = text.to_ascii_lowercase();
    if !p_lower.contains('*') && !p_lower.contains('?') {
        return t_lower.contains(&p_lower);
    }
    let p: Vec<char> = p_lower.chars().collect();
    let t: Vec<char> = t_lower.chars().collect();
    let (mut pi, mut ti, mut star_pi, mut star_ti) = (0usize, 0usize, None, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_pi = Some(pi);
            pi += 1;
            star_ti = ti;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn walk_find_files(
    dir: &Path,
    query: &str,
    depth: usize,
    max_depth: usize,
    out: &mut Vec<String>,
    cap: usize,
) {
    if out.len() >= cap || depth > max_depth {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= cap {
            break;
        }
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = p.is_dir();
        if is_dir && should_skip_dir_name(&name) {
            continue;
        }
        if wildcard_match_ci(query, &name) {
            out.push(format!(
                "{} {}",
                if is_dir { "[DIR]" } else { "[FILE]" },
                p.display()
            ));
        }
        if is_dir {
            walk_find_files(&p, query, depth + 1, max_depth, out, cap);
        }
    }
}

fn walk_search_text(
    dir: &Path,
    query_lower: &str,
    ext_filter: Option<&str>,
    depth: usize,
    max_depth: usize,
    out: &mut Vec<String>,
    cap: usize,
) {
    if out.len() >= cap || depth > max_depth {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= cap {
            break;
        }
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if !should_skip_dir_name(&name) {
                walk_search_text(&p, query_lower, ext_filter, depth + 1, max_depth, out, cap);
            }
        } else if p.is_file() {
            if let Some(want_ext) = ext_filter {
                let got = p
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_ascii_lowercase())
                    .unwrap_or_default();
                if got != want_ext {
                    continue;
                }
            }
            if entry.metadata().map(|m| m.len()).unwrap_or(0) > 2 * 1024 * 1024 {
                continue;
            }
            search_in_file(&p, query_lower, out, cap);
        }
    }
}

fn search_in_file(path: &Path, query_lower: &str, out: &mut Vec<String>, cap: usize) {
    let Ok(bytes) = fs::read(path) else {
        return;
    };
    let Ok(text) = decode_text_lossy(&bytes) else {
        return;
    };
    for (idx, line) in text.lines().enumerate() {
        if out.len() >= cap {
            break;
        }
        if line.to_ascii_lowercase().contains(query_lower) {
            let trimmed: String = line.trim().chars().take(160).collect();
            out.push(format!("{}:{}: {}", path.display(), idx + 1, trimmed));
        }
    }
}

fn send_to_recycle_bin(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let escaped = path.to_string_lossy().replace('\'', "''");
        let ps_script = if path.is_dir() {
            format!(
                "Add-Type -AssemblyName Microsoft.VisualBasic; [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory('{escaped}', 'OnlyErrorDialogs', 'SendToRecycleBin')"
            )
        } else {
            format!(
                "Add-Type -AssemblyName Microsoft.VisualBasic; [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile('{escaped}', 'OnlyErrorDialogs', 'SendToRecycleBin')"
            )
        };
        let (code, _, stderr) =
            run_powershell_with_timeout(&ps_script, &user_home_dir(), Duration::from_secs(15))?;
        if code != 0 {
            return Err(format!("Recycle Bin move failed: {}", stderr.trim()));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("Recycle Bin only supported on Windows".to_string())
    }
}

fn run_powershell_with_timeout(
    command: &str,
    working_dir: &Path,
    timeout: Duration,
) -> Result<(i32, String, String), String> {
    let wrapped = format!(
        "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; $OutputEncoding = [System.Text.Encoding]::UTF8; {command}"
    );
    let encoded_cmd = encode_powershell_utf16_base64(&wrapped);
    let mut cmd = std::process::Command::new("powershell.exe");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-EncodedCommand",
        &encoded_cmd,
    ])
    .current_dir(working_dir)
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped());

    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(|e| format!("Failed to start PowerShell: {e}"))?;
    let started = std::time::Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child.wait_with_output().map_err(|e| e.to_string())?;
                let code = output.status.code().unwrap_or(-1);
                let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                return Ok((code, stdout, stderr));
            }
            Ok(None) => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("Command timed out after {}s", timeout.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(60));
            }
            Err(e) => return Err(format!("Wait error: {e}")),
        }
    }
}

fn encode_powershell_utf16_base64(script: &str) -> String {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    let utf16_bytes: Vec<u8> = script
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    B64.encode(&utf16_bytes)
}

pub fn build_agent_system_appendix(workspace_cfg: &str, media_downloader_ready: bool) -> String {
    let home = user_home_dir();
    let desktop = home.join("Desktop");
    let documents = home.join("Documents");
    let downloads = home.join("Downloads");
    let ws_line = if workspace_cfg.trim().is_empty() {
        format!("- Default Workspace: {} (and any dragged paths)", home.display())
    } else {
        format!("- Configured Workspace: {}", workspace_cfg.trim())
    };
    let media_line = if media_downloader_ready {
        "\n- Enabled Plugin Tool: `download_media` (yt-dlp + FFmpeg) is active! When user sends a music/video link and asks to save/download audio/MP3/FLAC/MP4, explain briefly whether original stream avoids secondary loss if helpful and call `download_media`."
    } else {
        ""
    };
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    format!(
        "\n\n---\n\n# Desktop Agent Mode (Auto)\n\
         You have local Windows file & PowerShell tools (`list_dir`, `find_files`, `search_text`, `read_file`, `create_dir`, `write_file`, `edit_file`, `move_or_rename`, `copy_file`, `delete_path`, `run_shell`), persistent memory tools (`search_memories`, `save_memory_fact`), and read-only web tools (`web_search`, `fetch_web_page`).{media_line}\n\
         - User Profile: `{}`\n\
         - Desktop: `{}` | Documents: `{}` | Downloads: `{}`\n\
         {}\n\
         - Epoch Seconds: {ts}\n\
         - Operating Protocol:\n\
           1. **Ask & Clarify First**: Before modifying files or committing to complex actions, clarify ambiguous goals, preferences, or constraints so you truly understand what the user wants.\n\
           2. **Push Back Honestly**: If the user's request relies on wrong facts, strange contradictions, or very hard/impractical assumptions, tell them candidly and explain why instead of blindly agreeing.\n\
           3. **Auto Judgment**: Conversational Q&A -> reply directly; clear well-scoped file task -> inspect with read tools first, then execute cleanly (mutations pause for user approval cards).\n\
           4. **Truthfulness**: Never fabricate paths, outputs, or claim success for rejected, blocked, or failed actions.",
        home.display(),
        desktop.display(),
        documents.display(),
        downloads.display(),
        ws_line
    )
}

// ============================================================================
// SSRF-Safe Web Read / Search Helpers
// ============================================================================

fn is_blocked_ipv4(ip: Ipv4Addr) -> bool {
    let oct = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        // CGNAT 100.64.0.0/10
        || (oct[0] == 100 && (oct[1] & 0b1100_0000) == 64)
        // Reserved / protocol / bench ranges
        || (oct[0] == 192 && oct[1] == 0 && oct[2] == 0)
        || (oct[0] == 198 && (oct[1] == 18 || oct[1] == 19))
        || oct[0] >= 240
}

fn is_blocked_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_blocked_ipv4(v4);
    }
    let seg0 = ip.segments()[0];
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (seg0 & 0xfe00) == 0xfc00 // fc00::/7 unique local
        || (seg0 & 0xffc0) == 0xfe80 // fe80::/10 link-local
}

pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_ipv4(v4),
        IpAddr::V6(v6) => is_blocked_ipv6(v6),
    }
}

pub fn validate_public_url(raw_url: &str, resolve_dns: bool) -> Result<reqwest::Url, String> {
    let trimmed = raw_url.trim();
    let parsed = reqwest::Url::parse(trimmed).map_err(|e| format!("Invalid URL: {e}"))?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!("Blocked non-HTTP URL scheme: {scheme}"));
    }
    let host_raw = parsed
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;
    let host = host_raw.trim_matches(|c| c == '[' || c == ']').to_ascii_lowercase();
    if host.is_empty()
        || host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with(".lan")
    {
        return Err(format!("Blocked local/private host: {host}"));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_blocked_ip(ip) {
            return Err(format!("Blocked non-public IP: {ip}"));
        }
        return Ok(parsed);
    }
    if !host.contains('.') {
        return Err(format!("Blocked single-label intranet host: {host}"));
    }
    if resolve_dns {
        let port = parsed.port_or_known_default().unwrap_or(80);
        let addrs: Vec<_> = (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|e| format!("DNS resolution failed for `{host}`: {e}"))?
            .collect();
        if addrs.is_empty() {
            return Err(format!("No DNS addresses resolved for `{host}`"));
        }
        for sa in addrs {
            if is_blocked_ip(sa.ip()) {
                return Err(format!(
                    "Blocked host `{host}` resolving to private/local IP {}",
                    sa.ip()
                ));
            }
        }
    }
    Ok(parsed)
}

fn strip_tag_block(html: &str, tag: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let open_pat = format!("<{tag}");
    let close_pat = format!("</{tag}>");
    let mut out = String::with_capacity(html.len());
    let mut i = 0usize;
    while i < html.len() {
        if let Some(rel_start) = lower[i..].find(&open_pat) {
            let abs_start = i + rel_start;
            out.push_str(&html[i..abs_start]);
            if let Some(rel_end) = lower[abs_start..].find(&close_pat) {
                i = abs_start + rel_end + close_pat.len();
            } else {
                break;
            }
        } else {
            out.push_str(&html[i..]);
            break;
        }
    }
    out
}

fn decode_basic_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&#x27;", "'")
}

pub fn html_to_readable_text(html: &str, max_chars: usize) -> String {
    let mut cleaned = strip_tag_block(html, "script");
    cleaned = strip_tag_block(&cleaned, "style");
    cleaned = strip_tag_block(&cleaned, "noscript");
    cleaned = strip_tag_block(&cleaned, "svg");

    let mut plain = String::with_capacity(cleaned.len() / 2);
    let mut in_tag = false;
    let mut tag_buf = String::new();
    for ch in cleaned.chars() {
        if in_tag {
            if ch == '>' {
                in_tag = false;
                let tag_lower = tag_buf.trim().to_ascii_lowercase();
                let name = tag_lower
                    .trim_start_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or("");
                if matches!(
                    name,
                    "p" | "div" | "br" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "section" | "article"
                ) {
                    plain.push('\n');
                }
                tag_buf.clear();
            } else if tag_buf.len() < 40 {
                tag_buf.push(ch);
            }
        } else if ch == '<' {
            in_tag = true;
            tag_buf.clear();
        } else {
            plain.push(ch);
        }
    }

    let decoded = decode_basic_entities(&plain);
    let mut lines = Vec::new();
    for raw_line in decoded.lines() {
        let collapsed = raw_line.split_whitespace().collect::<Vec<_>>().join(" ");
        if !collapsed.is_empty() {
            lines.push(collapsed);
        }
    }
    truncate_chars(&lines.join("\n"), max_chars)
}

fn url_encode_query(q: &str) -> String {
    let mut out = String::with_capacity(q.len() * 2);
    for b in q.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            b' ' => out.push('+'),
            other => {
                out.push('%');
                out.push_str(&format!("{other:02X}"));
            }
        }
    }
    out
}

fn percent_decode_lossy(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(val) = u8::from_str_radix(hex, 16) {
                    out.push(val);
                    i += 3;
                    continue;
                }
            }
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(decode_basic_entities(&tag[start..end]))
}

fn clean_ddg_href(raw_href: &str) -> String {
    let normalized = if raw_href.starts_with("//") {
        format!("https:{raw_href}")
    } else {
        raw_href.to_string()
    };
    if let Some(pos) = normalized.find("uddg=") {
        let rest = &normalized[pos + 5..];
        let end = rest.find('&').unwrap_or(rest.len());
        return percent_decode_lossy(&rest[..end]);
    }
    normalized
}

pub fn parse_ddg_html(html: &str, limit: usize) -> Vec<(String, String, String)> {
    let mut results = Vec::new();
    let mut cursor = 0usize;
    while results.len() < limit {
        let Some(pos) = html[cursor..].find("result__a") else {
            break;
        };
        let marker = cursor + pos;
        let a_start = html[..marker].rfind("<a").unwrap_or(marker);
        let Some(tag_end_rel) = html[a_start..].find('>') else {
            break;
        };
        let tag_end = a_start + tag_end_rel;
        let a_tag = &html[a_start..=tag_end];
        let Some(close_rel) = html[tag_end + 1..].find("</a>") else {
            break;
        };
        let title_raw = &html[tag_end + 1..tag_end + 1 + close_rel];
        let title = html_to_readable_text(title_raw, 180);
        let url = extract_attr(a_tag, "href")
            .map(|h| clean_ddg_href(&h))
            .unwrap_or_default();

        let after_link = tag_end + 1 + close_rel + 4;
        let window_end = (after_link + 2000).min(html.len());
        let snippet = if let Some(sp) = html[after_link..window_end].find("result__snippet") {
            let s_pos = after_link + sp;
            if let Some(gt) = html[s_pos..window_end].find('>') {
                let body_start = s_pos + gt + 1;
                let end_tag = html[body_start..window_end]
                    .find("</a>")
                    .or_else(|| html[body_start..window_end].find("</div>"))
                    .unwrap_or((window_end - body_start).min(400));
                html_to_readable_text(&html[body_start..body_start + end_tag], 260)
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        if !title.is_empty() && url.starts_with("http") {
            results.push((title, url, snippet));
        }
        cursor = after_link;
    }
    results
}

pub async fn execute_web_search(query: &str, limit: usize) -> Result<String, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err("`query` cannot be empty".to_string());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let url = format!("https://html.duckduckgo.com/html/?q={}", url_encode_query(trimmed));
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Web search request failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("Web search HTTP {status}"));
    }
    let html = resp
        .text()
        .await
        .map_err(|e| format!("Failed reading search response: {e}"))?;
    let hits = parse_ddg_html(&html, limit.clamp(1, 8));
    if hits.is_empty() {
        return Ok(format!("No web search results extracted for `{trimmed}`."));
    }
    let formatted = hits
        .into_iter()
        .enumerate()
        .map(|(idx, (title, link, snippet))| {
            if snippet.is_empty() {
                format!("{}. **{}**\n   {}", idx + 1, title, link)
            } else {
                format!("{}. **{}**\n   {}\n   {}", idx + 1, title, link, snippet)
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    Ok(formatted)
}

pub async fn execute_fetch_web_page(raw_url: &str, max_chars: usize) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(18))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) BuddyDesktop/1.0")
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let mut current = validate_public_url(raw_url, true)?;
    let mut final_resp = None;

    for _hop in 0..=3 {
        let resp = client
            .get(current.clone())
            .send()
            .await
            .map_err(|e| format!("Fetch failed: {e}"))?;
        if resp.status().is_redirection() {
            let loc = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| "Redirect missing Location header".to_string())?;
            let next_url = current
                .join(loc)
                .map_err(|e| format!("Invalid redirect URL: {e}"))?;
            current = validate_public_url(next_url.as_str(), true)?;
            continue;
        }
        final_resp = Some(resp);
        break;
    }

    let resp = final_resp.ok_or_else(|| "Too many redirects".to_string())?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status} from {current}"));
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !content_type.is_empty()
        && !content_type.contains("text/")
        && !content_type.contains("json")
        && !content_type.contains("xml")
        && !content_type.contains("javascript")
    {
        return Err(format!("Unsupported binary content type: {content_type}"));
    }

    const MAX_BODY_BYTES: usize = 1_500_000;
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed reading response body: {e}"))?;
    let slice = if bytes.len() > MAX_BODY_BYTES {
        &bytes[..MAX_BODY_BYTES]
    } else {
        &bytes[..]
    };
    let raw_text = String::from_utf8_lossy(slice);
    let cap = max_chars.clamp(500, 16_000);
    let body = if content_type.contains("json") || content_type.contains("text/plain") {
        truncate_chars(&raw_text, cap)
    } else {
        html_to_readable_text(&raw_text, cap)
    };
    Ok(format!("URL: {current}\n\n{body}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_blocking_protects_os_and_secrets() {
        assert!(is_blocked_path(Path::new("C:\\Windows\\System32\\cmd.exe"), false));
        assert!(is_blocked_path(Path::new("C:\\Program Files\\App\\a.dll"), true));
        assert!(is_blocked_path(Path::new("C:\\Users\\Test\\AppData\\Local\\Buddy\\secret.dpapi"), false));
        assert!(!is_blocked_path(Path::new("C:\\Users\\Test\\Desktop\\notes.md"), true));
    }

    #[test]
    fn agent_file_tools_roundtrip() {
        let base = std::env::temp_dir().join("buddy_agent_tools_test");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        let (ok_dir, _, _) = execute_tool("create_dir", &json!({ "path": "sub" }), Some(&base));
        assert!(ok_dir);

        let (ok_write, _, _) = execute_tool(
            "write_file",
            &json!({ "path": "sub/hello.txt", "content": "line1\nalpha\nline3" }),
            Some(&base),
        );
        assert!(ok_write);

        let (ok_read, read_out, _) = execute_tool("read_file", &json!({ "path": "sub/hello.txt" }), Some(&base));
        assert!(ok_read);
        assert!(read_out.contains("2: alpha"));

        let (ok_edit, _, _) = execute_tool(
            "edit_file",
            &json!({ "path": "sub/hello.txt", "target": "alpha", "replacement": "beta" }),
            Some(&base),
        );
        assert!(ok_edit);

        let (ok_search, search_out, _) = execute_tool(
            "search_text",
            &json!({ "root": ".", "query": "beta" }),
            Some(&base),
        );
        assert!(ok_search);
        assert!(search_out.contains("beta"));

        let (ok_mv, _, _) = execute_tool(
            "move_or_rename",
            &json!({ "source": "sub/hello.txt", "destination": "sub/renamed.txt" }),
            Some(&base),
        );
        assert!(ok_mv);

        let (ok_find, find_out, _) = execute_tool(
            "find_files",
            &json!({ "root": ".", "query": "*.txt" }),
            Some(&base),
        );
        assert!(ok_find);
        assert!(find_out.contains("renamed.txt"));

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn ssrf_blocks_local_private_and_metadata_urls() {
        assert!(validate_public_url("http://127.0.0.1:8080/admin", false).is_err());
        assert!(validate_public_url("http://localhost:1420/", false).is_err());
        assert!(validate_public_url("http://169.254.169.254/latest/meta-data", false).is_err());
        assert!(validate_public_url("http://192.168.1.1/", false).is_err());
        assert!(validate_public_url("http://[::1]:3000/", false).is_err());
        assert!(validate_public_url("file:///C:/Windows/win.ini", false).is_err());
        assert!(validate_public_url("https://example.com/docs", false).is_ok());
    }

    #[test]
    fn extracts_readable_html_and_ddg_results() {
        let html = r#"
          <html><head><style>.a{color:red}</style><script>alert(1)</script></head>
          <body><h1>Rust Docs</h1><p>Hello &amp; welcome!</p></body></html>
        "#;
        let text = html_to_readable_text(html, 500);
        assert!(!text.contains("alert(1)"));
        assert!(text.contains("Rust Docs"));
        assert!(text.contains("Hello & welcome!"));

        let ddg = r#"
          <div class="result">
            <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Frust-lang.org%2F&amp;rut=1">Rust Language</a>
            <a class="result__snippet">Systems programming language.</a>
          </div>
        "#;
        let hits = parse_ddg_html(ddg, 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "Rust Language");
        assert_eq!(hits[0].1, "https://rust-lang.org/");
        assert!(hits[0].2.contains("Systems programming"));
    }

    #[test]
    fn permission_modes_enforce_ask_default_and_auto_safely() {
        let base = std::env::temp_dir().join("buddy_perm_test");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let existing = base.join("existing.txt");
        fs::write(&existing, "hello").unwrap();

        // Read-only is auto across all three modes
        for mode in [AgentPermissionMode::Ask, AgentPermissionMode::Default, AgentPermissionMode::Auto] {
            assert!(!plan_tool_execution("read_file", &json!({"path": "existing.txt"}), Some(&base), mode).requires_approval);
            assert!(!plan_tool_execution("web_search", &json!({"query": "rust"}), Some(&base), mode).requires_approval);
        }

        // Reversible write: Ask requires step-by-step (no approve_all), Default allows approve_all, Auto skips prompt
        let ask_write = plan_tool_execution("write_file", &json!({"path": "new.txt", "content": "hi"}), Some(&base), AgentPermissionMode::Ask);
        assert!(ask_write.requires_approval && !ask_write.allow_approve_all && !ask_write.is_destructive);

        let def_write = plan_tool_execution("write_file", &json!({"path": "new.txt", "content": "hi"}), Some(&base), AgentPermissionMode::Default);
        assert!(def_write.requires_approval && def_write.allow_approve_all && !def_write.is_destructive);

        let auto_write = plan_tool_execution("write_file", &json!({"path": "new.txt", "content": "hi"}), Some(&base), AgentPermissionMode::Auto);
        assert!(!auto_write.requires_approval && !auto_write.is_destructive);

        // External side-effect (download_media / install_github_skill): Ask & Default ask, Auto executes cleanly
        let def_dl = plan_tool_execution("download_media", &json!({"url": "https://example.com/a"}), Some(&base), AgentPermissionMode::Default);
        assert!(def_dl.requires_approval && !def_dl.allow_approve_all);
        let auto_dl = plan_tool_execution("download_media", &json!({"url": "https://example.com/a"}), Some(&base), AgentPermissionMode::Auto);
        assert!(!auto_dl.requires_approval);

        // Deletion, overwrite, existing-target copy overwrite, and DANGEROUS run_shell ALWAYS ask even in Auto;
        // safe/non-destructive run_shell commands run automatically in Auto while still asking in Ask & Default.
        let safe_shell_ask = plan_tool_execution("run_shell", &json!({"command": "git status"}), Some(&base), AgentPermissionMode::Ask);
        assert!(safe_shell_ask.requires_approval && !safe_shell_ask.allow_approve_all);
        let safe_shell_def = plan_tool_execution("run_shell", &json!({"command": "python script.py --format json"}), Some(&base), AgentPermissionMode::Default);
        assert!(safe_shell_def.requires_approval && safe_shell_def.allow_approve_all && !safe_shell_def.is_destructive);
        let safe_shell_auto = plan_tool_execution("run_shell", &json!({"command": "Get-ChildItem | Select-Object Name"}), Some(&base), AgentPermissionMode::Auto);
        assert!(!safe_shell_auto.requires_approval && !safe_shell_auto.is_destructive);

        for (tool, args) in [
            ("delete_path", json!({"path": "existing.txt"})),
            ("run_shell", json!({"command": "Remove-Item existing.txt"})),
            ("run_shell", json!({"command": "Get-Date; rm existing.txt"})),
            ("run_shell", json!({"command": "cmd /c del existing.txt"})),
            ("run_shell", json!({"command": "Stop-Process -Name notepad"})),
            ("run_shell", json!({"command": "python -c \"import os; os.remove('a.txt')\""})),
            ("run_shell", json!({"command": "git reset --hard HEAD"})),
            ("write_file", json!({"path": "existing.txt", "content": "x", "overwrite": true})),
            ("copy_file", json!({"source": "existing.txt", "destination": "existing.txt"})),
        ] {
            let plan = plan_tool_execution(tool, &args, Some(&base), AgentPermissionMode::Auto);
            assert!(plan.requires_approval, "{tool} ({args}) must ask even in Auto");
            assert!(!plan.allow_approve_all, "{tool} ({args}) must not allow approve-all");
            assert!(plan.is_destructive, "{tool} ({args}) must be marked destructive");
        }

        let _ = fs::remove_dir_all(&base);
    }
}
