//! Local Desktop Agent toolset & interactive approval gate.
//! Provides read-only exploration (`list_dir`, `find_files`, `search_text`, `read_file`)
//! and approval-gated filesystem / shell mutations (`create_dir`, `write_file`, `edit_file`,
//! `move_or_rename`, `copy_file`, `delete_path`, `run_shell`).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
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

fn check_path_allowed(path: &Path, is_write: bool) -> Result<(), String> {
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
fn raw_tool_definitions() -> Vec<(&'static str, &'static str, Value)> {
    vec![
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
    ]
}

pub fn openai_tools_json() -> Value {
    let arr: Vec<Value> = raw_tool_definitions()
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

pub fn anthropic_tools_json() -> Value {
    let arr: Vec<Value> = raw_tool_definitions()
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

pub fn gemini_tools_json() -> Value {
    let decls: Vec<Value> = raw_tool_definitions()
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

/// Builds a user-readable plan and safety classification before executing a tool.
pub fn plan_tool_execution(
    name: &str,
    args: &Value,
    workspace_hint: Option<&Path>,
) -> ToolExecutionPlan {
    match name {
        "list_dir" => {
            let raw = args["path"].as_str().unwrap_or("");
            let label = resolve_user_path(raw, workspace_hint)
                .map(|p| short_path_label(&p))
                .unwrap_or_else(|_| raw.to_string());
            ToolExecutionPlan {
                summary: format!("List folder `{label}`"),
                preview: None,
                requires_approval: false,
                allow_approve_all: false,
                is_destructive: false,
            }
        }
        "find_files" => {
            let q = args["query"].as_str().unwrap_or("");
            let r = args["root"].as_str().unwrap_or("");
            let label = resolve_user_path(r, workspace_hint)
                .map(|p| short_path_label(&p))
                .unwrap_or_else(|_| r.to_string());
            ToolExecutionPlan {
                summary: format!("Find `{q}` in `{label}`"),
                preview: None,
                requires_approval: false,
                allow_approve_all: false,
                is_destructive: false,
            }
        }
        "search_text" => {
            let q = args["query"].as_str().unwrap_or("");
            let r = args["root"].as_str().unwrap_or("");
            let label = resolve_user_path(r, workspace_hint)
                .map(|p| short_path_label(&p))
                .unwrap_or_else(|_| r.to_string());
            ToolExecutionPlan {
                summary: format!("Search \"{q}\" in `{label}`"),
                preview: None,
                requires_approval: false,
                allow_approve_all: false,
                is_destructive: false,
            }
        }
        "read_file" => {
            let raw = args["path"].as_str().unwrap_or("");
            let label = resolve_user_path(raw, workspace_hint)
                .map(|p| short_path_label(&p))
                .unwrap_or_else(|_| raw.to_string());
            ToolExecutionPlan {
                summary: format!("Read `{label}`"),
                preview: None,
                requires_approval: false,
                allow_approve_all: false,
                is_destructive: false,
            }
        }
        "create_dir" => {
            let raw = args["path"].as_str().unwrap_or("");
            let resolved = resolve_user_path(raw, workspace_hint)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| raw.to_string());
            ToolExecutionPlan {
                summary: format!("Create folder `{}`", Path::new(&resolved).file_name().and_then(|s| s.to_str()).unwrap_or(&resolved)),
                preview: Some(resolved),
                requires_approval: true,
                allow_approve_all: true,
                is_destructive: false,
            }
        }
        "write_file" => {
            let raw = args["path"].as_str().unwrap_or("");
            let content = args["content"].as_str().unwrap_or("");
            let overwrite = args["overwrite"].as_bool().unwrap_or(false);
            let resolved = resolve_user_path(raw, workspace_hint)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| raw.to_string());
            let snippet = truncate_chars(content, 360);
            ToolExecutionPlan {
                summary: format!(
                    "{} `{}` ({} bytes)",
                    if overwrite { "Overwrite" } else { "Write" },
                    Path::new(&resolved)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or(&resolved),
                    content.len()
                ),
                preview: Some(format!("Path: {resolved}\n---\n{snippet}")),
                requires_approval: true,
                allow_approve_all: true,
                is_destructive: overwrite,
            }
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
            ToolExecutionPlan {
                summary: format!(
                    "Edit `{}`",
                    Path::new(&resolved)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or(&resolved)
                ),
                preview: Some(preview),
                requires_approval: true,
                allow_approve_all: true,
                is_destructive: false,
            }
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
                .unwrap_or(&src_res);
            let d_name = Path::new(&dst_res)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&dst_res);
            ToolExecutionPlan {
                summary: format!("Move `{s_name}` → `{d_name}`"),
                preview: Some(format!("From: {src_res}\nTo:   {dst_res}")),
                requires_approval: true,
                allow_approve_all: true,
                is_destructive: false,
            }
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
            let s_name = Path::new(&src_res)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&src_res);
            ToolExecutionPlan {
                summary: format!("Copy `{s_name}`"),
                preview: Some(format!("From: {src_res}\nTo:   {dst_res}")),
                requires_approval: true,
                allow_approve_all: true,
                is_destructive: false,
            }
        }
        "delete_path" => {
            let raw = args["path"].as_str().unwrap_or("");
            let resolved = resolve_user_path(raw, workspace_hint)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| raw.to_string());
            ToolExecutionPlan {
                summary: format!(
                    "Recycle `{}`",
                    Path::new(&resolved)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or(&resolved)
                ),
                preview: Some(format!("Send to Recycle Bin:\n{resolved}")),
                requires_approval: true,
                allow_approve_all: false,
                is_destructive: true,
            }
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
            ToolExecutionPlan {
                summary: "Run PowerShell command".to_string(),
                preview: Some(format!("CWD: {wd_label}\n$ {}", truncate_chars(cmd, 420))),
                requires_approval: true,
                allow_approve_all: false,
                is_destructive: true,
            }
        }
        "list_installed_skills" => ToolExecutionPlan {
            summary: "List installed Skills".to_string(),
            preview: None,
            requires_approval: false,
            allow_approve_all: false,
            is_destructive: false,
        },
        "read_skill" => {
            let id = args["skill_id"].as_str().unwrap_or("");
            ToolExecutionPlan {
                summary: format!("Load Skill `{id}`"),
                preview: None,
                requires_approval: false,
                allow_approve_all: false,
                is_destructive: false,
            }
        }
        "search_github_skills" => {
            let q = args["query"].as_str().unwrap_or("");
            ToolExecutionPlan {
                summary: format!("Search GitHub Skills `{q}`"),
                preview: None,
                requires_approval: false,
                allow_approve_all: false,
                is_destructive: false,
            }
        }
        "install_github_skill" => {
            let src = args["source"].as_str().unwrap_or("");
            ToolExecutionPlan {
                summary: format!("Install GitHub Skill `{}`", truncate_chars(src, 42)),
                preview: Some(format!("Source: {src}\nInstall into %LOCALAPPDATA%/Buddy/skills/")),
                requires_approval: true,
                allow_approve_all: false,
                is_destructive: false,
            }
        }
        "offline_ocr" => {
            let src = args["source"].as_str().unwrap_or("");
            let name = Path::new(src)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(src);
            ToolExecutionPlan {
                summary: format!("Offline OCR `{}`", truncate_chars(name, 36)),
                preview: None,
                requires_approval: false,
                allow_approve_all: false,
                is_destructive: false,
            }
        }
        other => ToolExecutionPlan {
            summary: format!("Call `{other}`"),
            preview: None,
            requires_approval: true,
            allow_approve_all: false,
            is_destructive: false,
        },
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

pub fn build_agent_system_appendix(workspace_cfg: &str) -> String {
    let home = user_home_dir();
    let desktop = home.join("Desktop");
    let documents = home.join("Documents");
    let downloads = home.join("Downloads");
    let ws_line = if workspace_cfg.trim().is_empty() {
        format!("- Default Workspace: {} (and any dragged paths)", home.display())
    } else {
        format!("- Configured Workspace: {}", workspace_cfg.trim())
    };
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    format!(
        "\n\n---\n\n# Desktop Agent Mode (Auto)\n\
         You have local Windows file & PowerShell tools (`list_dir`, `find_files`, `search_text`, `read_file`, `create_dir`, `write_file`, `edit_file`, `move_or_rename`, `copy_file`, `delete_path`, `run_shell`).\n\
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
}
