use crate::agent_tools::{AgentApprovalManager, ApprovalDecision};
use crate::ai_client::{spawn_chat_stream, AiStreamController, AttachmentPayload};
use crate::browser::{BrowserManager, BrowserStatePayload};
use crate::db::{now_ms as db_now_ms, BrowserBookmark, ConversationSummary, MemoryDb, ReminderItem, StoredChatMessage};
use crate::mcp::McpManager;
use crate::plugins::PluginManager;
use crate::prompts::{PersonalityOption, PromptsManager};
use crate::reminders::{local_utc_offset_ms, parse_fire_at_ms};
use crate::screen_share::{
    enumerate_share_targets, ScreenShareManager, ShareStatePayload, ShareTargetInfo,
    StartShareRequest,
};
use crate::settings::SettingsManager;
use crate::skills::SkillsManager;
use crate::snip::{begin_snip_capture, SnipState};
use crate::win32_sensors::{self, AppSensorState};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

#[tauri::command]
pub fn ensure_active_conversation(db: tauri::State<'_, Arc<MemoryDb>>) -> Result<i64, String> {
    db.ensure_active_conversation()
}

#[tauri::command]
pub fn ensure_fresh_startup_conversation(db: tauri::State<'_, Arc<MemoryDb>>) -> Result<i64, String> {
    db.ensure_fresh_startup_conversation()
}

#[tauri::command]
pub fn list_conversations(db: tauri::State<'_, Arc<MemoryDb>>) -> Result<Vec<ConversationSummary>, String> {
    db.list_conversations(50)
}

#[tauri::command]
pub fn create_conversation(db: tauri::State<'_, Arc<MemoryDb>>) -> Result<ConversationSummary, String> {
    db.create_conversation(None)
}

#[tauri::command]
pub fn delete_conversation(
    db: tauri::State<'_, Arc<MemoryDb>>,
    conversation_id: i64,
) -> Result<(), String> {
    db.delete_conversation(conversation_id)
}

#[tauri::command]
pub fn rename_conversation(
    db: tauri::State<'_, Arc<MemoryDb>>,
    conversation_id: i64,
    title: String,
) -> Result<String, String> {
    db.rename_conversation(conversation_id, &title)
}

#[tauri::command]
pub fn search_conversations(
    db: tauri::State<'_, Arc<MemoryDb>>,
    query: String,
) -> Result<Vec<ConversationSummary>, String> {
    db.search_conversations(&query, 50)
}

#[tauri::command]
pub fn truncate_conversation_from_user_turn(
    db: tauri::State<'_, Arc<MemoryDb>>,
    conversation_id: i64,
    user_turn_index: usize,
) -> Result<usize, String> {
    db.truncate_conversation_by_user_index(conversation_id, user_turn_index)
}

#[tauri::command]
pub fn list_browser_bookmarks(
    db: tauri::State<'_, Arc<MemoryDb>>,
) -> Result<Vec<BrowserBookmark>, String> {
    db.list_bookmarks()
}

#[tauri::command]
pub fn add_browser_bookmark(
    db: tauri::State<'_, Arc<MemoryDb>>,
    title: String,
    url: String,
) -> Result<BrowserBookmark, String> {
    db.add_bookmark(&title, &url)
}

#[tauri::command]
pub fn delete_browser_bookmark(
    db: tauri::State<'_, Arc<MemoryDb>>,
    id: i64,
) -> Result<(), String> {
    db.delete_bookmark(id)
}

#[tauri::command]
pub fn list_reminders(
    db: tauri::State<'_, Arc<MemoryDb>>,
    include_completed: Option<bool>,
) -> Result<Vec<ReminderItem>, String> {
    db.list_reminders(include_completed.unwrap_or(true))
}

#[tauri::command]
pub fn create_reminder(
    db: tauri::State<'_, Arc<MemoryDb>>,
    title: String,
    delay_minutes: Option<f64>,
    at_time: Option<String>,
    repeat_rule: Option<String>,
) -> Result<ReminderItem, String> {
    let fire_ms = parse_fire_at_ms(
        delay_minutes,
        at_time.as_deref(),
        db_now_ms(),
        local_utc_offset_ms(),
    )?;
    db.create_reminder(&title, fire_ms, repeat_rule.as_deref())
}

#[tauri::command]
pub fn toggle_reminder(
    db: tauri::State<'_, Arc<MemoryDb>>,
    id: i64,
    enabled: bool,
) -> Result<ReminderItem, String> {
    db.toggle_reminder(id, enabled)
}

#[tauri::command]
pub fn snooze_reminder(
    db: tauri::State<'_, Arc<MemoryDb>>,
    id: i64,
    minutes: i64,
) -> Result<ReminderItem, String> {
    db.snooze_reminder(id, minutes)
}

#[tauri::command]
pub fn delete_reminder(db: tauri::State<'_, Arc<MemoryDb>>, id: i64) -> Result<(), String> {
    db.delete_reminder(id)
}

#[tauri::command]
pub fn list_chat_history(
    db: tauri::State<'_, Arc<MemoryDb>>,
    conversation_id: i64,
) -> Result<Vec<StoredChatMessage>, String> {
    db.list_chat_messages(conversation_id, 80)
}

#[tauri::command]
pub fn list_personalities(prompts_mgr: tauri::State<'_, Arc<PromptsManager>>) -> Vec<PersonalityOption> {
    prompts_mgr.list_personalities()
}

#[tauri::command]
pub fn open_prompts_folder(prompts_mgr: tauri::State<'_, Arc<PromptsManager>>) -> Result<(), String> {
    prompts_mgr.open_folder()
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn send_chat_message(
    app: AppHandle,
    settings_mgr: tauri::State<'_, Arc<SettingsManager>>,
    prompts_mgr: tauri::State<'_, Arc<PromptsManager>>,
    skills_mgr: tauri::State<'_, Arc<SkillsManager>>,
    plugin_mgr: tauri::State<'_, Arc<PluginManager>>,
    browser_mgr: tauri::State<'_, Arc<BrowserManager>>,
    mcp_mgr: tauri::State<'_, Arc<McpManager>>,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
    share_mgr: tauri::State<'_, Arc<ScreenShareManager>>,
    db: tauri::State<'_, Arc<MemoryDb>>,
    controller: tauri::State<'_, Arc<AiStreamController>>,
    approval_mgr: tauri::State<'_, Arc<AgentApprovalManager>>,
    conversation_id: i64,
    stream_id: String,
    prompt: String,
    attachments: Vec<AttachmentPayload>,
    is_retry: Option<bool>,
) -> Result<(), String> {
    let retry_mode = is_retry.unwrap_or(false);
    if retry_mode {
        let _ = db.delete_last_assistant_message(conversation_id);
    }

    let (summary_opt, prior_msgs) = db
        .list_chat_context(conversation_id, 20)
        .unwrap_or_default();
    let mut prior = prior_msgs
        .into_iter()
        .map(|m| (m.role, m.content))
        .collect::<Vec<_>>();

    let att_json = if attachments.is_empty() {
        None
    } else {
        serde_json::to_string(&attachments).ok()
    };
    let stored_prompt = if prompt.trim().is_empty() {
        "Analyze attached item".to_string()
    } else {
        prompt.clone()
    };
    if retry_mode {
        if prior.last().map(|(r, _)| r == "user").unwrap_or(false) {
            prior.pop();
        }
    } else {
        let _ = db.append_chat_message(
            conversation_id,
            "user",
            &stored_prompt,
            att_json.as_deref(),
            None,
            None,
        );
    }

    let (live_frames, screen_goal, watch_override) = share_mgr.grab_frames_for_chat();

    spawn_chat_stream(
        app,
        settings_mgr.inner().clone(),
        prompts_mgr.inner().clone(),
        skills_mgr.inner().clone(),
        plugin_mgr.inner().clone(),
        browser_mgr.inner().clone(),
        mcp_mgr.inner().clone(),
        sensor_state.inner().clone(),
        db.inner().clone(),
        controller.inner().clone(),
        approval_mgr.inner().clone(),
        conversation_id,
        stream_id,
        stored_prompt,
        attachments,
        prior,
        summary_opt,
        live_frames,
        screen_goal,
        watch_override,
    );
    Ok(())
}

#[tauri::command]
pub fn steer_chat_stream(
    db: tauri::State<'_, Arc<MemoryDb>>,
    controller: tauri::State<'_, Arc<AiStreamController>>,
    conversation_id: i64,
    prompt: String,
) -> Result<(), String> {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return Err("Empty steering instruction".to_string());
    }
    let _ = db.append_chat_message(conversation_id, "user", trimmed, None, None, None);
    controller.push_steer(trimmed.to_string());
    Ok(())
}

#[tauri::command]
pub fn get_browser_state(browser_mgr: tauri::State<'_, Arc<BrowserManager>>) -> BrowserStatePayload {
    browser_mgr.get_state()
}

#[tauri::command]
pub fn set_browser_watching(
    app: AppHandle,
    browser_mgr: tauri::State<'_, Arc<BrowserManager>>,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
    watching: bool,
) -> BrowserStatePayload {
    browser_mgr.inner().clone().set_watching(&app, &sensor_state, watching)
}

#[tauri::command]
pub fn set_browser_takeover(
    app: AppHandle,
    browser_mgr: tauri::State<'_, Arc<BrowserManager>>,
    takeover: bool,
) -> BrowserStatePayload {
    browser_mgr.set_user_takeover(&app, takeover)
}

#[tauri::command]
pub fn browser_navigate(
    app: AppHandle,
    browser_mgr: tauri::State<'_, Arc<BrowserManager>>,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
    url: String,
) -> Result<BrowserStatePayload, String> {
    browser_mgr.inner().clone().navigate_user_url(&app, &sensor_state, &url)
}

#[tauri::command]
pub fn open_url_in_builtin_browser(
    app: AppHandle,
    browser_mgr: tauri::State<'_, Arc<BrowserManager>>,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
    url: String,
    watch: Option<bool>,
) -> Result<BrowserStatePayload, String> {
    browser_mgr
        .inner()
        .clone()
        .open_user_url(&app, &sensor_state, &url, watch.unwrap_or(true))
}

#[tauri::command]
pub fn open_external_url(url: String) -> Result<(), String> {
    let parsed = crate::agent_tools::validate_public_url(&url, false)?;
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg(parsed.as_str())
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn save_pasted_image_attachment(
    db: tauri::State<'_, Arc<MemoryDb>>,
    data_base64: String,
    mime_type: Option<String>,
) -> Result<AttachmentPayload, String> {
    let raw_b64 = data_base64
        .split_once(',')
        .map(|(_, b)| b)
        .unwrap_or(&data_base64)
        .trim();
    if raw_b64.is_empty() {
        return Err("Empty image data".to_string());
    }
    let bytes = B64
        .decode(raw_b64)
        .map_err(|e| format!("Invalid pasted image data: {e}"))?;
    if bytes.len() > 15 * 1024 * 1024 {
        return Err("Pasted image exceeds 15 MB limit".to_string());
    }
    let ext = match mime_type
        .as_deref()
        .unwrap_or("image/png")
        .to_ascii_lowercase()
        .as_str()
    {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        _ => "png",
    };
    let dir = db.data_dir.join("pasted");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ts = super::now_ms();
    let filename = format!("paste_{ts}.{ext}");
    let path = dir.join(&filename);
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(AttachmentPayload {
        kind: "image".to_string(),
        label: filename,
        text_content: None,
        file_path: Some(path.to_string_lossy().to_string()),
    })
}

#[tauri::command]
pub fn browser_nav_action(
    app: AppHandle,
    browser_mgr: tauri::State<'_, Arc<BrowserManager>>,
    action: String,
) -> Result<(), String> {
    browser_mgr.browser_history_action(&app, &action)
}

#[tauri::command]
pub fn close_builtin_browser(
    app: AppHandle,
    browser_mgr: tauri::State<'_, Arc<BrowserManager>>,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
) -> BrowserStatePayload {
    browser_mgr.close_browser(&app, &sensor_state)
}

#[tauri::command]
pub fn clear_browser_data(
    app: AppHandle,
    browser_mgr: tauri::State<'_, Arc<BrowserManager>>,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
) -> Result<(), String> {
    browser_mgr.clear_profile_data(&app, &sensor_state)
}

#[tauri::command]
pub fn open_local_media_file(path: String) -> Result<(), String> {
    let target = Path::new(&path);
    let canon = target.canonicalize().map_err(|e| format!("File not found: {e}"))?;
    if !canon.is_file() {
        return Err("Target path is not a file".to_string());
    }
    let ext = canon
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    if !matches!(
        ext.as_str(),
        "mp3" | "flac" | "m4a" | "wav" | "aac" | "ogg" | "opus" | "mp4" | "mkv" | "webm" | "mov" | "avi"
    ) {
        return Err(format!("Unsupported media extension `.{ext}`"));
    }
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg(&canon)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn read_local_audio_data_url(path: String) -> Result<String, String> {
    let target = Path::new(&path);
    let canon = target.canonicalize().map_err(|e| format!("File not found: {e}"))?;
    if !canon.is_file() {
        return Err("Target path is not a file".to_string());
    }
    let meta = std::fs::metadata(&canon).map_err(|e| e.to_string())?;
    if meta.len() > 28 * 1024 * 1024 {
        return Err("Audio file is larger than 28 MB for inline preview; use external player.".to_string());
    }
    let ext = canon
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    let mime = match ext.as_str() {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "ogg" | "opus" => "audio/ogg",
        "webm" => "audio/webm",
        _ => return Err(format!("Inline player not supported for `.{ext}`")),
    };
    let bytes = std::fs::read(&canon).map_err(|e| e.to_string())?;
    Ok(format!("data:{mime};base64,{}", B64.encode(&bytes)))
}

#[tauri::command]
pub fn list_share_targets() -> Vec<ShareTargetInfo> {
    enumerate_share_targets()
}

#[tauri::command]
pub fn get_share_state(
    share_mgr: tauri::State<'_, Arc<ScreenShareManager>>,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
) -> ShareStatePayload {
    share_mgr.get_state(&sensor_state)
}

#[tauri::command]
pub fn start_screen_share(
    app: AppHandle,
    share_mgr: tauri::State<'_, Arc<ScreenShareManager>>,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
    settings_mgr: tauri::State<'_, Arc<SettingsManager>>,
    ai_ctrl: tauri::State<'_, Arc<AiStreamController>>,
    request: StartShareRequest,
) -> Result<ShareStatePayload, String> {
    share_mgr.inner().clone().start(
        app,
        sensor_state.inner().clone(),
        settings_mgr.inner().clone(),
        ai_ctrl.inner().clone(),
        request,
    )
}

#[tauri::command]
pub fn stop_screen_share(
    app: AppHandle,
    share_mgr: tauri::State<'_, Arc<ScreenShareManager>>,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
) -> ShareStatePayload {
    share_mgr.stop(&app, &sensor_state, "user_stopped")
}

#[tauri::command]
pub fn toggle_screen_share_pause(
    app: AppHandle,
    share_mgr: tauri::State<'_, Arc<ScreenShareManager>>,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
) -> ShareStatePayload {
    share_mgr.toggle_pause(&app, &sensor_state)
}

#[tauri::command]
pub fn update_screen_share_options(
    app: AppHandle,
    share_mgr: tauri::State<'_, Arc<ScreenShareManager>>,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
    goal: Option<String>,
    cadence: Option<String>,
    show_border: Option<bool>,
    watch_profile_id: Option<String>,
) -> ShareStatePayload {
    share_mgr.update_live_options(
        &app,
        &sensor_state,
        goal,
        cadence,
        show_border,
        watch_profile_id,
    )
}

#[tauri::command]
pub fn set_window_dock_preset(
    app: AppHandle,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
    share_mgr: tauri::State<'_, Arc<ScreenShareManager>>,
    edge: String,
    ratio: f64,
) -> ShareStatePayload {
    let st = share_mgr.get_state(&sensor_state);
    if st.active && st.target_kind == "window" && st.target_hwnd != 0 {
        if let Some(win) = app.get_webview_window("main") {
            win32_sensors::attach_dock_to_window(
                &app,
                &win,
                &sensor_state,
                st.target_hwnd,
                &edge,
                ratio,
            );
        }
    }
    share_mgr.get_state(&sensor_state)
}

#[tauri::command]
pub fn set_whisper_bounds(
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
    bounds: Option<(f64, f64, f64, f64)>,
) {
    if let Ok(mut g) = sensor_state.whisper_bounds.lock() {
        *g = bounds;
    }
}

#[tauri::command]
pub fn accept_share_whisper(
    db: tauri::State<'_, Arc<MemoryDb>>,
    conversation_id: i64,
    text: String,
) -> Result<i64, String> {
    db.append_chat_message(conversation_id, "assistant", &text, None, None, None)
}

#[tauri::command]
pub fn resolve_agent_approval(
    approval_mgr: tauri::State<'_, Arc<AgentApprovalManager>>,
    step_id: String,
    decision: ApprovalDecision,
) -> Result<(), String> {
    approval_mgr.resolve(&step_id, decision)
}

#[tauri::command]
pub fn cancel_chat_stream(
    app: AppHandle,
    controller: tauri::State<'_, Arc<AiStreamController>>,
    approval_mgr: tauri::State<'_, Arc<AgentApprovalManager>>,
    browser_mgr: tauri::State<'_, Arc<BrowserManager>>,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
) {
    controller.cancel_flag.store(true, Ordering::Relaxed);
    controller.clear_steers();
    approval_mgr.reject_all();
    if browser_mgr.is_user_takeover() {
        browser_mgr.set_user_takeover(&app, false);
    }
    if !browser_mgr.is_watching() {
        browser_mgr.close_browser(&app, &sensor_state);
    }
}

#[tauri::command]
pub fn trigger_snip(app: AppHandle, snip_state: tauri::State<'_, Arc<SnipState>>) {
    begin_snip_capture(&app, &snip_state);
}

#[tauri::command]
pub async fn pick_chat_attachments(
    app: AppHandle,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
) -> Result<Vec<AttachmentPayload>, String> {
    let picked = crate::file_dialog::pick_files(
        &app,
        &sensor_state,
        Some("Attach files or images".to_string()),
        true,
        None,
        None,
        None,
    )
    .await?;
    if picked.is_empty() {
        return Ok(Vec::new());
    }
    let paths: Vec<std::path::PathBuf> = picked.into_iter().map(std::path::PathBuf::from).collect();
    Ok(classify_dropped_paths(&paths))
}

#[tauri::command]
pub async fn export_conversation_markdown(
    app: AppHandle,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
    db: tauri::State<'_, Arc<MemoryDb>>,
    conversation_id: i64,
) -> Result<Option<String>, String> {
    let title = db
        .get_conversation_title(conversation_id)
        .unwrap_or_else(|_| "Buddy Chat".to_string());
    let messages = db.list_chat_messages(conversation_id, 2000)?;

    let safe_name: String = title
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect::<String>()
        .trim()
        .chars()
        .take(48)
        .collect();
    let default_name = if safe_name.is_empty() {
        "buddy-chat.md".to_string()
    } else {
        format!("{safe_name}.md")
    };

    let Some(target_path) = crate::file_dialog::save_file(
        &app,
        &sensor_state,
        Some("Export Conversation as Markdown".to_string()),
        Some(default_name),
        Some("md".to_string()),
        Some("Markdown (*.md)".to_string()),
        Some("*.md".to_string()),
    )
    .await?
    else {
        return Ok(None);
    };

    let mut md = String::new();
    md.push_str(&format!("# {}\n\n", title.trim()));
    md.push_str("_Exported from Buddy_\n\n---\n\n");
    for msg in messages {
        let speaker = if msg.role == "user" { "You" } else { "Buddy" };
        md.push_str(&format!("### {speaker}\n\n"));
        if let Some(thought) = msg.thought_text.as_deref().filter(|t| !t.trim().is_empty()) {
            md.push_str("<details>\n<summary>Thought process</summary>\n\n");
            md.push_str(thought.trim());
            md.push_str("\n\n</details>\n\n");
        }
        md.push_str(msg.content.trim());
        md.push_str("\n\n---\n\n");
    }

    std::fs::write(&target_path, md.as_bytes())
        .map_err(|e| format!("Failed to save Markdown file: {e}"))?;
    Ok(Some(target_path))
}

pub(crate) fn classify_dropped_paths(paths: &[std::path::PathBuf]) -> Vec<AttachmentPayload> {
    let mut out = Vec::new();
    for path in paths.iter().take(6) {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("item")
            .to_string();
        let full_path = path.to_string_lossy().to_string();

        if path.is_dir() {
            out.push(AttachmentPayload {
                kind: "folder".to_string(),
                label: name,
                text_content: None,
                file_path: Some(full_path),
            });
            continue;
        }

        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();

        if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "bmp") {
            out.push(AttachmentPayload {
                kind: "image".to_string(),
                label: name,
                text_content: None,
                file_path: Some(full_path),
            });
        } else if let Ok(bytes) = std::fs::read(path) {
            let is_likely_binary = bytes.iter().take(4096).filter(|&&b| b == 0).count() > 8;
            if !is_likely_binary {
                if let Ok(text) = std::str::from_utf8(&bytes) {
                    let capped: String = text.chars().take(16_000).collect();
                    out.push(AttachmentPayload {
                        kind: "text".to_string(),
                        label: name,
                        text_content: Some(capped),
                        file_path: Some(full_path),
                    });
                    continue;
                }
            }
            out.push(AttachmentPayload {
                kind: "file".to_string(),
                label: name,
                text_content: Some(format!("{} file, {} bytes", ext.to_uppercase(), bytes.len())),
                file_path: Some(full_path),
            });
        } else {
            out.push(AttachmentPayload {
                kind: "file".to_string(),
                label: name,
                text_content: Some("Unreadable or locked file".to_string()),
                file_path: Some(full_path),
            });
        }
    }
    out
}
