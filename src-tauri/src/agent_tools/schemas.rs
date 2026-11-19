//! Canonical tool schema definitions for OpenAI, Anthropic, and Gemini protocols
//! plus dynamic desktop workspace appendix generation.

use super::security::user_home_dir;
use crate::mcp::McpToolInfo;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

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
            "Fetch a public http:// or https:// web page as plain HTTP text (fast for static pages/docs, but cannot execute JavaScript). Use `browser_open` for JS-rendered SPAs, video/music sites, search pages, or interactive sites.",
            json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Public http:// or https:// URL." },
                    "max_chars": { "type": "integer", "description": "Maximum text characters to return (default 8000, max 16000)." }
                },
                "required": ["url"]
            }),
        ),
        (
            "browser_open",
            "Open a web URL (public http(s), localhost/127.0.0.1/LAN dev servers, or local `file://` HTML files) in Buddy's built-in WebView2 browser (persists user cookies, executes full JS/SPAs silently in background unless user opens Watch view). Returns rendered text and indexed interactive elements `[e1]`, `[e2]` with links.",
            json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL or local HTML file path to open (e.g. `https://...`, `http://localhost:3000`, `file:///C:/.../index.html`)." },
                    "wait_ms": { "type": "integer", "description": "Optional max milliseconds to wait for dynamic page hydration (default 5500)." }
                },
                "required": ["url"]
            }),
        ),
        (
            "browser_read",
            "Read the currently loaded page in Buddy's built-in browser (title, URL, scroll position, readable text, and refreshed `[e1]`, `[e2]` interactive elements).",
            json!({
                "type": "object",
                "properties": {
                    "max_chars": { "type": "integer", "description": "Max text chars to extract (default 10000, max 16000)." }
                }
            }),
        ),
        (
            "browser_click",
            "Click an element in Buddy's built-in browser by element ID (`e1`, `e2`...), CSS selector, or visible button/link text, highlighting it visually for the user.",
            json!({
                "type": "object",
                "properties": {
                    "target": { "type": "string", "description": "Element ID like `e3`, CSS selector, or visible text." }
                },
                "required": ["target"]
            }),
        ),
        (
            "browser_type",
            "Type text into an input/search field in Buddy's built-in browser (refuses password inputs automatically). Optionally submit with Enter.",
            json!({
                "type": "object",
                "properties": {
                    "target": { "type": "string", "description": "Element ID (`e1`...), CSS selector, or placeholder hint." },
                    "text": { "type": "string", "description": "Text to enter into the input field." },
                    "submit": { "type": "boolean", "description": "Press Enter / submit form after typing (default true)." }
                },
                "required": ["target", "text"]
            }),
        ),
        (
            "browser_scroll",
            "Scroll Buddy's built-in browser viewport to inspect more content or search results.",
            json!({
                "type": "object",
                "properties": {
                    "direction": { "type": "string", "enum": ["down", "up", "top", "bottom"], "description": "Scroll direction (default 'down')." },
                    "amount": { "type": "integer", "description": "Pixels to scroll for up/down (default 650)." }
                }
            }),
        ),
        (
            "browser_screenshot",
            "Capture a visual snapshot of the current built-in browser page when layout, video cards, or visual confirmation is needed.",
            json!({
                "type": "object",
                "properties": {}
            }),
        ),
        (
            "browser_console",
            "Read captured JavaScript console errors, warnings, uncaught exceptions, and failed fetch/HTTP (>=400) requests from the current built-in browser page. Ideal for debugging localhost web apps.",
            json!({
                "type": "object",
                "properties": {
                    "clear": { "type": "boolean", "description": "Whether to clear the captured log buffer after reading (default false)." }
                }
            }),
        ),
        (
            "browser_close",
            "Close Buddy's built-in browser window immediately when done to release memory.",
            json!({
                "type": "object",
                "properties": {}
            }),
        ),
        (
            "create_reminder",
            "Schedule a persistent desktop reminder that pops up in Buddy's top Dynamic Island capsule when due (survives app restarts). Use `delay_minutes` for relative delays (e.g. 20) OR `at_time` for local clock time (`HH:MM` or `YYYY-MM-DD HH:MM`).",
            json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Short reminder text to show the user." },
                    "delay_minutes": { "type": "number", "description": "Optional minutes from now (e.g. 15, 30, 120)." },
                    "at_time": { "type": "string", "description": "Optional local clock time: `HH:MM` (next occurrence) or `YYYY-MM-DD HH:MM`." },
                    "repeat": { "type": "string", "enum": ["none", "daily", "weekdays", "weekly"], "description": "Optional repeat schedule (default 'none')." }
                },
                "required": ["title"]
            }),
        ),
        (
            "list_reminders",
            "List scheduled persistent reminders (active upcoming reminders by default).",
            json!({
                "type": "object",
                "properties": {
                    "include_completed": { "type": "boolean", "description": "Include already-fired/disabled reminders (default false)." }
                }
            }),
        ),
        (
            "delete_reminder",
            "Delete a scheduled reminder by its numeric ID (use `list_reminders` first if unsure of ID).",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "Reminder ID to delete." }
                },
                "required": ["id"]
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

fn sanitize_gemini_schema(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, val) in map {
                if matches!(
                    k.as_str(),
                    "$schema"
                        | "additionalProperties"
                        | "$defs"
                        | "definitions"
                        | "$id"
                        | "default"
                        | "title"
                        | "examples"
                        | "const"
                ) {
                    continue;
                }
                if k == "type" && val.is_array() {
                    if let Some(first_str) = val
                        .as_array()
                        .and_then(|arr| arr.iter().find_map(|x| x.as_str().filter(|s| *s != "null")))
                    {
                        out.insert(k.clone(), Value::String(first_str.to_string()));
                        continue;
                    }
                }
                out.insert(k.clone(), sanitize_gemini_schema(val));
            }
            if !out.contains_key("type") && out.contains_key("properties") {
                out.insert("type".to_string(), Value::String("object".to_string()));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(sanitize_gemini_schema).collect()),
        other => other.clone(),
    }
}

pub fn openai_tools_json(include_media_downloader: bool, mcp_tools: &[McpToolInfo]) -> Value {
    let mut arr: Vec<Value> = raw_tool_definitions(include_media_downloader)
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
    for mt in mcp_tools {
        let desc = if mt.description.is_empty() {
            format!("[MCP: {}] {}", mt.server_name, mt.tool_name)
        } else {
            format!("[MCP: {}] {}", mt.server_name, mt.description)
        };
        arr.push(json!({
            "type": "function",
            "function": {
                "name": mt.qualified_name,
                "description": desc,
                "parameters": mt.input_schema
            }
        }));
    }
    Value::Array(arr)
}

pub fn anthropic_tools_json(include_media_downloader: bool, mcp_tools: &[McpToolInfo]) -> Value {
    let mut arr: Vec<Value> = raw_tool_definitions(include_media_downloader)
        .into_iter()
        .map(|(name, desc, params)| {
            json!({
                "name": name,
                "description": desc,
                "input_schema": params
            })
        })
        .collect();
    for mt in mcp_tools {
        let desc = if mt.description.is_empty() {
            format!("[MCP: {}] {}", mt.server_name, mt.tool_name)
        } else {
            format!("[MCP: {}] {}", mt.server_name, mt.description)
        };
        arr.push(json!({
            "name": mt.qualified_name,
            "description": desc,
            "input_schema": mt.input_schema
        }));
    }
    Value::Array(arr)
}

pub fn gemini_tools_json(include_media_downloader: bool, mcp_tools: &[McpToolInfo]) -> Value {
    let mut decls: Vec<Value> = raw_tool_definitions(include_media_downloader)
        .into_iter()
        .map(|(name, desc, params)| {
            json!({
                "name": name,
                "description": desc,
                "parameters": params
            })
        })
        .collect();
    for mt in mcp_tools {
        let desc = if mt.description.is_empty() {
            format!("[MCP: {}] {}", mt.server_name, mt.tool_name)
        } else {
            format!("[MCP: {}] {}", mt.server_name, mt.description)
        };
        decls.push(json!({
            "name": mt.qualified_name,
            "description": desc,
            "parameters": sanitize_gemini_schema(&mt.input_schema)
        }));
    }
    json!([{ "functionDeclarations": decls }])
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
        "\n- Enabled Plugin Tool: `download_media` (yt-dlp + FFmpeg) is active! When user asks to find a song/video and send/convert to MP3/FLAC/MP4, you can use `browser_open`/`browser_type`/`browser_click` on a public music/video site (e.g. YouTube/Bilibili/SoundCloud) to verify the exact playable URL, then call `download_media` (`audio_mp3` when MP3 requested, `mp4_lossless` for MP4, `audio_best` for highest fidelity) and include the saved file path clearly."
    } else {
        ""
    };
    let local_now = crate::reminders::format_local_now();
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    format!(
        "\n\n---\n\n# Desktop Agent Environment\n\
         User Profile: `{}` | Desktop: `{}` | Documents: `{}` | Downloads: `{}`\n\
         {}\n\
         Local Date/Time: {local_now} (Epoch: {ts}){media_line}\n\
         - Built-in Browser: Supports public sites, `localhost`/LAN dev servers, and local `file://` HTML files. Use `browser_open` + `browser_read` + `browser_click`/`browser_type`, and call `browser_console` when debugging JS/fetch errors. If `[BROWSER ATTENTION REQUIRED]` appears, stop automating and ask the user to complete login/CAPTCHA manually in Watch view.\n\
         - Reminders: Use `create_reminder` / `list_reminders` / `delete_reminder` when the user asks to be reminded later or on a recurring schedule.\n\
         - Reply directly to casual Q&A without calling unnecessary tools; for file tasks, inspect with read tools first before mutating (writes/deletions/commands obey the active permission mode).",
        home.display(),
        desktop.display(),
        documents.display(),
        downloads.display(),
        ws_line
    )
}
