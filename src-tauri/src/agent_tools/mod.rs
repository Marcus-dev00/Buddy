//! Local Desktop Agent toolset & interactive approval gate.
//! Provides read-only exploration (`list_dir`, `find_files`, `search_text`, `read_file`)
//! and approval-gated filesystem / shell mutations (`create_dir`, `write_file`, `edit_file`,
//! `move_or_rename`, `copy_file`, `delete_path`, `run_shell`).

mod schemas;
mod security;
mod web;

pub use schemas::{
    anthropic_tools_json, build_agent_system_appendix, gemini_tools_json, openai_tools_json,
};
#[allow(unused_imports)]
pub use security::{
    is_blocked_ip, resolve_user_path, user_home_dir, validate_public_url, AgentPermissionMode,
};
pub(crate) use security::check_path_allowed;
use security::{approval_flags_for, is_dangerous_shell_command, ToolRiskKind};
#[allow(unused_imports)]
pub use web::{
    execute_fetch_web_page, execute_web_search, html_to_readable_text, parse_ddg_html,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
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

pub(super) fn truncate_chars(s: &str, max_chars: usize) -> String {
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

fn short_path_label(path: &Path) -> String {
    let s = path.to_string_lossy();
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 44 {
        return s.into_owned();
    }
    let tail: String = chars[chars.len() - 36..].iter().collect();
    format!("...{tail}")
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
        "browser_open" => {
            let url = args["url"].as_str().unwrap_or("");
            make_plan(
                format!("Browser open `{}`", truncate_chars(url, 38)),
                Some(url.to_string()),
                permission_mode,
                ToolRiskKind::ReadOnly,
            )
        }
        "browser_read" => make_plan(
            "Browser read page".to_string(),
            None,
            permission_mode,
            ToolRiskKind::ReadOnly,
        ),
        "browser_click" => {
            let t = args["target"].as_str().unwrap_or("");
            make_plan(
                format!("Browser click `{}`", truncate_chars(t, 34)),
                Some(format!("Target element: {t}")),
                permission_mode,
                ToolRiskKind::ReversibleWrite,
            )
        }
        "browser_type" => {
            let t = args["target"].as_str().unwrap_or("");
            let txt = args["text"].as_str().unwrap_or("");
            make_plan(
                format!("Browser type into `{}`", truncate_chars(t, 28)),
                Some(format!("Target: {t}\nText: {}", truncate_chars(txt, 140))),
                permission_mode,
                ToolRiskKind::ReversibleWrite,
            )
        }
        "browser_scroll" => {
            let dir = args["direction"].as_str().unwrap_or("down");
            make_plan(
                format!("Browser scroll {dir}"),
                None,
                permission_mode,
                ToolRiskKind::ReadOnly,
            )
        }
        "browser_screenshot" => make_plan(
            "Browser capture view".to_string(),
            None,
            permission_mode,
            ToolRiskKind::ReadOnly,
        ),
        "browser_console" => make_plan(
            "Read browser console logs".to_string(),
            None,
            permission_mode,
            ToolRiskKind::ReadOnly,
        ),
        "browser_close" => make_plan(
            "Close built-in browser".to_string(),
            None,
            permission_mode,
            ToolRiskKind::ReadOnly,
        ),
        "create_reminder" => {
            let title = args["title"].as_str().unwrap_or("Reminder");
            make_plan(
                format!("Set reminder `{}`", truncate_chars(title, 34)),
                None,
                permission_mode,
                ToolRiskKind::ReadOnly,
            )
        }
        "list_reminders" => make_plan(
            "List reminders".to_string(),
            None,
            permission_mode,
            ToolRiskKind::ReadOnly,
        ),
        "delete_reminder" => {
            let id = args["id"].as_i64().unwrap_or(0);
            make_plan(
                format!("Delete reminder #{id}"),
                None,
                permission_mode,
                ToolRiskKind::ReversibleWrite,
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
        other if other.starts_with("mcp__") => {
            let parts: Vec<&str> = other.splitn(3, "__").collect();
            let (srv, tool) = if parts.len() == 3 {
                (parts[1], parts[2])
            } else {
                ("mcp", other)
            };
            let args_preview = serde_json::to_string_pretty(args)
                .ok()
                .map(|s| truncate_chars(&s, 280));
            make_plan(
                format!("MCP [{srv}]: `{tool}`"),
                args_preview,
                permission_mode,
                ToolRiskKind::ReversibleWrite,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
