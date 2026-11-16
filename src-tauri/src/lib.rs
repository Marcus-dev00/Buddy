mod agent_tools;
mod ai_client;
mod db;
mod media_utils;
mod music_sensor;
mod prompts;
mod settings;
mod snip;
mod watchers;
mod win32_sensors;

use agent_tools::{AgentApprovalManager, ApprovalDecision};
use ai_client::{
    spawn_chat_stream, verify_model_connection, AiStreamController, AttachmentPayload,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use db::{ConversationSummary, MemoryDb, MemoryItem, StoredChatMessage};
use media_utils::SelfCopyGuard;
use prompts::{PersonalityOption, PromptsManager};
use settings::{AppSettings, SettingsManager};
use snip::{
    begin_snip_capture, cancel_snip, finish_snip, get_snip_preview, start_snip_hotkey_listener,
    SnipState,
};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, DragDropEvent, Emitter, Manager, WindowEvent};
use tauri_plugin_updater::UpdaterExt;
use win32_sensors::{
    apply_interaction_mode, apply_window_layout, begin_drag, compute_layout_dims, end_drag,
    reset_dock_to_top, show_window_no_activate, start_sensor_loops, step_drag, AppSensorState,
    DockStatePayload, LayoutDims,
};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[tauri::command]
fn set_expanded(
    app: AppHandle,
    state: tauri::State<'_, Arc<AppSensorState>>,
    expanded: bool,
) -> LayoutDims {
    state.is_expanded.store(expanded, Ordering::Relaxed);
    if expanded {
        state.manual_sleep.store(false, Ordering::Relaxed);
        state.is_sleeping.store(false, Ordering::Relaxed);
        state.last_expand_ms.store(now_ms(), Ordering::Relaxed);
    }
    if let Some(win) = app.get_webview_window("main") {
        let dims = apply_window_layout(&win, &state, expanded);
        apply_interaction_mode(&win, expanded);
        return dims;
    }
    compute_layout_dims(1.0, None)
}

#[tauri::command]
fn update_pill_bounds(state: tauri::State<'_, Arc<AppSensorState>>, width: u32, height: u32) {
    state.pill_w.store(width.max(36), Ordering::Relaxed);
    state.pill_h.store(height.max(36), Ordering::Relaxed);
}

#[tauri::command]
fn get_dock_state(state: tauri::State<'_, Arc<AppSensorState>>) -> DockStatePayload {
    DockStatePayload {
        dock_edge: state.get_dock_edge(),
        snap_preview: None,
    }
}

#[tauri::command]
fn begin_buddy_drag(app: AppHandle, state: tauri::State<'_, Arc<AppSensorState>>) {
    if let Some(win) = app.get_webview_window("main") {
        begin_drag(&win, &state);
    }
}

#[tauri::command]
fn step_buddy_drag(app: AppHandle, state: tauri::State<'_, Arc<AppSensorState>>) -> DockStatePayload {
    if let Some(win) = app.get_webview_window("main") {
        return step_drag(&win, &state);
    }
    DockStatePayload {
        dock_edge: state.get_dock_edge(),
        snap_preview: None,
    }
}

#[tauri::command]
fn end_buddy_drag(
    app: AppHandle,
    state: tauri::State<'_, Arc<AppSensorState>>,
    settings_mgr: tauri::State<'_, Arc<SettingsManager>>,
) -> DockStatePayload {
    if let Some(win) = app.get_webview_window("main") {
        return end_drag(&app, &win, &state, &settings_mgr);
    }
    DockStatePayload {
        dock_edge: state.get_dock_edge(),
        snap_preview: None,
    }
}

#[tauri::command]
fn reset_dock_position(
    app: AppHandle,
    state: tauri::State<'_, Arc<AppSensorState>>,
    settings_mgr: tauri::State<'_, Arc<SettingsManager>>,
) -> DockStatePayload {
    if let Some(win) = app.get_webview_window("main") {
        return reset_dock_to_top(&app, &win, &state, &settings_mgr);
    }
    DockStatePayload {
        dock_edge: "top".to_string(),
        snap_preview: None,
    }
}

#[derive(Clone, serde::Serialize)]
struct UpdateInfo {
    version: String,
    current_version: String,
    body: String,
    date: Option<String>,
}

#[derive(Clone, serde::Serialize)]
struct UpdateProgressPayload {
    downloaded: u64,
    total: Option<u64>,
}

fn build_updater(
    app: &AppHandle,
    settings_mgr: &SettingsManager,
) -> Result<tauri_plugin_updater::Updater, String> {
    let custom = settings_mgr.get().update_endpoint;
    let trimmed = custom.trim();
    if trimmed.is_empty() {
        return app.updater().map_err(|e| e.to_string());
    }
    let url = reqwest::Url::parse(trimmed).map_err(|e| format!("Invalid update endpoint URL: {e}"))?;
    app.updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
async fn check_app_update(
    app: AppHandle,
    settings_mgr: tauri::State<'_, Arc<SettingsManager>>,
) -> Result<Option<UpdateInfo>, String> {
    let custom = settings_mgr.get().update_endpoint;
    if custom.trim().is_empty() {
        // Guard against unconfigured placeholder URL in tauri.conf.json
        if let Some(cfg) = app.config().plugins.0.get("updater") {
            let s = cfg.to_string();
            if s.contains("YOUR_OWNER") {
                return Err("Update feed endpoint not configured yet".to_string());
            }
        }
    }
    let current = app.package_info().version.to_string();
    let updater = build_updater(&app, &settings_mgr)?;
    match updater.check().await {
        Ok(Some(upd)) => {
            let info = UpdateInfo {
                version: upd.version.clone(),
                current_version: current,
                body: upd.body.clone().unwrap_or_default(),
                date: upd.date.map(|d| d.to_string()),
            };
            let _ = app.emit("buddy://update-available", info.clone());
            Ok(Some(info))
        }
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
async fn install_app_update(
    app: AppHandle,
    settings_mgr: tauri::State<'_, Arc<SettingsManager>>,
    db: tauri::State<'_, Arc<MemoryDb>>,
) -> Result<(), String> {
    let updater = build_updater(&app, &settings_mgr)?;
    let Some(upd) = updater.check().await.map_err(|e| e.to_string())? else {
        return Err("No update available".to_string());
    };
    let app_prog = app.clone();
    let mut downloaded: u64 = 0;
    upd.download_and_install(
        move |chunk_len, content_len| {
            downloaded += chunk_len as u64;
            let _ = app_prog.emit(
                "buddy://update-progress",
                UpdateProgressPayload {
                    downloaded,
                    total: content_len,
                },
            );
        },
        || {},
    )
    .await
    .map_err(|e| e.to_string())?;

    db.checkpoint_wal();
    app.restart();
}

#[tauri::command]
fn set_auto_hide_stealth(state: tauri::State<'_, Arc<AppSensorState>>, enabled: bool) {
    state.auto_hide_enabled.store(enabled, Ordering::Relaxed);
}

#[tauri::command]
fn set_music_reactive(state: tauri::State<'_, Arc<AppSensorState>>, enabled: bool) {
    state.music_reactive_enabled.store(enabled, Ordering::Relaxed);
    if !enabled {
        state.is_music_playing.store(false, Ordering::Relaxed);
    }
}

#[tauri::command]
fn set_music_playing_flag(state: tauri::State<'_, Arc<AppSensorState>>, playing: bool) {
    state.is_music_playing.store(playing, Ordering::Relaxed);
}

#[tauri::command]
fn get_expanded_bounds(
    app: AppHandle,
    state: tauri::State<'_, Arc<AppSensorState>>,
) -> LayoutDims {
    if let Some(win) = app.get_webview_window("main") {
        let scale = win.scale_factor().unwrap_or(1.0);
        let anchor = if state.has_anchor.load(Ordering::Relaxed) {
            Some((
                state.anchor_cx.load(Ordering::Relaxed),
                state.anchor_cy.load(Ordering::Relaxed),
            ))
        } else {
            None
        };
        return compute_layout_dims(scale, anchor);
    }
    compute_layout_dims(1.0, None)
}

#[tauri::command]
fn list_memories(
    db: tauri::State<'_, Arc<MemoryDb>>,
    query: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<MemoryItem>, String> {
    db.list_memories(query.as_deref(), limit.unwrap_or(80))
}

#[tauri::command]
fn toggle_memory_star(db: tauri::State<'_, Arc<MemoryDb>>, id: i64) -> Result<bool, String> {
    db.toggle_star(id)
}

#[tauri::command]
fn delete_memory(db: tauri::State<'_, Arc<MemoryDb>>, id: i64) -> Result<(), String> {
    db.delete_memory(id)
}

#[tauri::command]
fn copy_memory_to_clipboard(
    db: tauri::State<'_, Arc<MemoryDb>>,
    guard: tauri::State<'_, Arc<SelfCopyGuard>>,
    id: i64,
) -> Result<(), String> {
    let items = db.list_memories(None, 200)?;
    let Some(item) = items.into_iter().find(|m| m.id == id) else {
        return Err("Memory item not found".into());
    };
    #[cfg(windows)]
    {
        if item.kind == "clipboard_text" {
            if let Some(txt) = item.content_text {
                return media_utils::write_text_to_clipboard(&txt, &guard);
            }
        } else if let Some(fp) = item.file_path {
            let img = image::open(&fp).map_err(|e| format!("open image failed: {e}"))?;
            return media_utils::write_image_to_clipboard(&img, &guard);
        }
    }
    #[cfg(not(windows))]
    let _ = guard;
    Ok(())
}

#[tauri::command]
fn read_memory_image(
    db: tauri::State<'_, Arc<MemoryDb>>,
    path: String,
) -> Result<String, String> {
    let target = Path::new(&path);
    let canon_target = target.canonicalize().map_err(|e| e.to_string())?;
    let canon_root = db.data_dir.canonicalize().map_err(|e| e.to_string())?;
    if !canon_target.starts_with(&canon_root) {
        return Err("Access denied outside Buddy data directory".into());
    }
    let bytes = std::fs::read(&canon_target).map_err(|e| e.to_string())?;
    let mime = match canon_target
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        _ => "image/png",
    };
    Ok(format!("data:{mime};base64,{}", B64.encode(&bytes)))
}

#[tauri::command]
fn get_settings(settings_mgr: tauri::State<'_, Arc<SettingsManager>>) -> AppSettings {
    settings_mgr.get()
}

#[derive(serde::Serialize)]
struct VerifySaveResult {
    settings: AppSettings,
    latency_ms: u64,
}

#[tauri::command]
async fn verify_and_save_model_config(
    settings_mgr: tauri::State<'_, Arc<SettingsManager>>,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
    settings: AppSettings,
    new_api_key: Option<String>,
) -> Result<VerifySaveResult, String> {
    let key_to_test = match &new_api_key {
        Some(k) if !k.trim().is_empty() => k.trim().to_string(),
        Some(_) => String::new(),
        None => settings_mgr.load_api_key().unwrap_or_default(),
    };

    let latency_ms = verify_model_connection(
        &settings.api_provider,
        &settings.base_url,
        &settings.model_id,
        &key_to_test,
    )
    .await?;

    let saved = settings_mgr.update(settings, new_api_key)?;
    sensor_state
        .auto_hide_enabled
        .store(saved.auto_hide_stealth, Ordering::Relaxed);
    sensor_state
        .music_reactive_enabled
        .store(saved.music_reactive, Ordering::Relaxed);
    if !saved.music_reactive {
        sensor_state.is_music_playing.store(false, Ordering::Relaxed);
    }

    Ok(VerifySaveResult {
        settings: saved,
        latency_ms,
    })
}

#[tauri::command]
fn save_settings(
    settings_mgr: tauri::State<'_, Arc<SettingsManager>>,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
    settings: AppSettings,
    new_api_key: Option<String>,
) -> Result<AppSettings, String> {
    let saved = settings_mgr.update(settings, new_api_key)?;
    sensor_state
        .auto_hide_enabled
        .store(saved.auto_hide_stealth, Ordering::Relaxed);
    sensor_state
        .music_reactive_enabled
        .store(saved.music_reactive, Ordering::Relaxed);
    if !saved.music_reactive {
        sensor_state.is_music_playing.store(false, Ordering::Relaxed);
    }
    Ok(saved)
}

#[tauri::command]
fn ensure_active_conversation(db: tauri::State<'_, Arc<MemoryDb>>) -> Result<i64, String> {
    db.ensure_active_conversation()
}

#[tauri::command]
fn list_conversations(db: tauri::State<'_, Arc<MemoryDb>>) -> Result<Vec<ConversationSummary>, String> {
    db.list_conversations(50)
}

#[tauri::command]
fn create_conversation(db: tauri::State<'_, Arc<MemoryDb>>) -> Result<ConversationSummary, String> {
    db.create_conversation(None)
}

#[tauri::command]
fn delete_conversation(
    db: tauri::State<'_, Arc<MemoryDb>>,
    conversation_id: i64,
) -> Result<(), String> {
    db.delete_conversation(conversation_id)
}

#[tauri::command]
fn list_chat_history(
    db: tauri::State<'_, Arc<MemoryDb>>,
    conversation_id: i64,
) -> Result<Vec<StoredChatMessage>, String> {
    db.list_chat_messages(conversation_id, 80)
}

#[tauri::command]
fn list_personalities(prompts_mgr: tauri::State<'_, Arc<PromptsManager>>) -> Vec<PersonalityOption> {
    prompts_mgr.list_personalities()
}

#[tauri::command]
fn open_prompts_folder(prompts_mgr: tauri::State<'_, Arc<PromptsManager>>) -> Result<(), String> {
    prompts_mgr.open_folder()
}

#[tauri::command]
fn send_chat_message(
    app: AppHandle,
    settings_mgr: tauri::State<'_, Arc<SettingsManager>>,
    prompts_mgr: tauri::State<'_, Arc<PromptsManager>>,
    db: tauri::State<'_, Arc<MemoryDb>>,
    controller: tauri::State<'_, Arc<AiStreamController>>,
    approval_mgr: tauri::State<'_, Arc<AgentApprovalManager>>,
    conversation_id: i64,
    stream_id: String,
    prompt: String,
    attachments: Vec<AttachmentPayload>,
) -> Result<(), String> {
    let prior = db
        .list_chat_messages(conversation_id, 16)
        .unwrap_or_default()
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
    let _ = db.append_chat_message(
        conversation_id,
        "user",
        &stored_prompt,
        att_json.as_deref(),
        None,
        None,
    );

    spawn_chat_stream(
        app,
        settings_mgr.inner().clone(),
        prompts_mgr.inner().clone(),
        db.inner().clone(),
        controller.inner().clone(),
        approval_mgr.inner().clone(),
        conversation_id,
        stream_id,
        stored_prompt,
        attachments,
        prior,
    );
    Ok(())
}

#[tauri::command]
fn resolve_agent_approval(
    approval_mgr: tauri::State<'_, Arc<AgentApprovalManager>>,
    step_id: String,
    decision: ApprovalDecision,
) -> Result<(), String> {
    approval_mgr.resolve(&step_id, decision)
}

#[tauri::command]
fn cancel_chat_stream(
    controller: tauri::State<'_, Arc<AiStreamController>>,
    approval_mgr: tauri::State<'_, Arc<AgentApprovalManager>>,
) {
    controller.cancel_flag.store(true, Ordering::Relaxed);
    approval_mgr.reject_all();
}

#[tauri::command]
fn trigger_snip(app: AppHandle, snip_state: tauri::State<'_, Arc<SnipState>>) {
    begin_snip_capture(&app, &snip_state);
}

fn classify_dropped_paths(paths: &[std::path::PathBuf]) -> Vec<AttachmentPayload> {
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let db = Arc::new(MemoryDb::open().expect("failed to initialize buddy.db"));
    let settings_mgr = Arc::new(SettingsManager::load(&db.data_dir));
    let prompts_mgr = Arc::new(PromptsManager::init(&db.data_dir));
    let initial_cfg = settings_mgr.get();

    let sensor_state = Arc::new(AppSensorState::default());
    sensor_state
        .auto_hide_enabled
        .store(initial_cfg.auto_hide_stealth, Ordering::Relaxed);
    sensor_state
        .music_reactive_enabled
        .store(initial_cfg.music_reactive, Ordering::Relaxed);
    if matches!(
        initial_cfg.dock_edge.as_str(),
        "top" | "bottom" | "left" | "right" | "free"
    ) {
        sensor_state.set_dock_edge(&initial_cfg.dock_edge);
    }
    if let (Some(cx), Some(cy)) = (initial_cfg.dock_cx, initial_cfg.dock_cy) {
        sensor_state.anchor_cx.store(cx, Ordering::Relaxed);
        sensor_state.anchor_cy.store(cy, Ordering::Relaxed);
        sensor_state.has_anchor.store(true, Ordering::Relaxed);
    }

    let copy_guard = Arc::new(SelfCopyGuard::default());
    let snip_state = Arc::new(SnipState::default());
    let ai_controller = Arc::new(AiStreamController::default());
    let approval_mgr = Arc::new(AgentApprovalManager::default());

    let _ = db.cleanup_expired();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                show_window_no_activate(&win);
                let _ = app.emit("buddy://expand", ());
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(sensor_state.clone())
        .manage(db.clone())
        .manage(settings_mgr.clone())
        .manage(prompts_mgr.clone())
        .manage(copy_guard.clone())
        .manage(snip_state.clone())
        .manage(ai_controller.clone())
        .manage(approval_mgr.clone())
        .invoke_handler(tauri::generate_handler![
            set_expanded,
            update_pill_bounds,
            get_dock_state,
            begin_buddy_drag,
            step_buddy_drag,
            end_buddy_drag,
            reset_dock_position,
            get_app_version,
            check_app_update,
            install_app_update,
            set_auto_hide_stealth,
            set_music_reactive,
            set_music_playing_flag,
            get_expanded_bounds,
            list_memories,
            toggle_memory_star,
            delete_memory,
            copy_memory_to_clipboard,
            read_memory_image,
            get_settings,
            save_settings,
            verify_and_save_model_config,
            list_personalities,
            open_prompts_folder,
            ensure_active_conversation,
            list_conversations,
            create_conversation,
            delete_conversation,
            list_chat_history,
            send_chat_message,
            resolve_agent_approval,
            cancel_chat_stream,
            trigger_snip,
            get_snip_preview,
            finish_snip,
            cancel_snip
        ])
        .setup(move |app| {
            let main_win = app
                .get_webview_window("main")
                .expect("main window should exist");

            apply_window_layout(&main_win, &sensor_state, false);
            apply_interaction_mode(&main_win, false);

            // Silent background update check ~4 seconds after launch
            let app_upd = app.handle().clone();
            let mgr_upd = settings_mgr.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                let cfg = mgr_upd.get();
                let custom = cfg.update_endpoint.trim().to_string();
                let mut skip = false;
                if custom.is_empty() {
                    if let Some(u_cfg) = app_upd.config().plugins.0.get("updater") {
                        if u_cfg.to_string().contains("YOUR_OWNER") {
                            skip = true;
                        }
                    }
                }
                if !skip {
                    if let Ok(updater) = build_updater(&app_upd, &mgr_upd) {
                        if let Ok(Some(upd)) = updater.check().await {
                            let current = app_upd.package_info().version.to_string();
                            let _ = app_upd.emit(
                                "buddy://update-available",
                                UpdateInfo {
                                    version: upd.version,
                                    current_version: current,
                                    body: upd.body.unwrap_or_default(),
                                    date: upd.date.map(|d| d.to_string()),
                                },
                            );
                        }
                    }
                }
            });

            let expand_item = MenuItem::with_id(app, "expand", "Open Buddy", true, None::<&str>)?;
            let snip_item = MenuItem::with_id(
                app,
                "snip",
                "Quick Screenshot (Alt+Shift+S)",
                true,
                None::<&str>,
            )?;
            let music_toggle = CheckMenuItem::with_id(
                app,
                "music_reactive",
                "Music Reactive",
                true,
                initial_cfg.music_reactive,
                None::<&str>,
            )?;
            let stealth_toggle = CheckMenuItem::with_id(
                app,
                "auto_hide",
                "Auto-Hide in Games & Meetings",
                true,
                initial_cfg.auto_hide_stealth,
                None::<&str>,
            )?;
            let capture_item = MenuItem::with_id(
                app,
                "sim_capture",
                "Test Capture Animation",
                true,
                None::<&str>,
            )?;
            let sleep_item =
                MenuItem::with_id(app, "sim_sleep", "Toggle Sleep Mode", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit Buddy", true, None::<&str>)?;

            let tray_menu = Menu::with_items(
                app,
                &[
                    &expand_item,
                    &snip_item,
                    &music_toggle,
                    &stealth_toggle,
                    &capture_item,
                    &sleep_item,
                    &quit_item,
                ],
            )?;

            let tray_state = sensor_state.clone();
            let tray_db = db.clone();
            let tray_snip = snip_state.clone();
            let tray_settings = settings_mgr.clone();
            let stealth_item_ref = stealth_toggle.clone();
            let music_item_ref = music_toggle.clone();
            let mut tray_builder = TrayIconBuilder::new()
                .menu(&tray_menu)
                .tooltip("Buddy — Desktop AI Companion");

            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            }

            tray_builder
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "expand" => {
                        if let Some(win) = app.get_webview_window("main") {
                            tray_state.is_stealth.store(false, Ordering::Relaxed);
                            let _ = win.show();
                            let _ = app.emit("buddy://expand", ());
                        }
                    }
                    "snip" => {
                        begin_snip_capture(app, &tray_snip);
                    }
                    "music_reactive" => {
                        let checked = music_item_ref.is_checked().unwrap_or(true);
                        tray_state
                            .music_reactive_enabled
                            .store(checked, Ordering::Relaxed);
                        if !checked {
                            tray_state.is_music_playing.store(false, Ordering::Relaxed);
                        }
                        let mut cfg = tray_settings.get();
                        cfg.music_reactive = checked;
                        let _ = tray_settings.update(cfg, None);
                        let _ = app.emit("buddy://music-reactive-changed", checked);
                    }
                    "auto_hide" => {
                        let checked = stealth_item_ref.is_checked().unwrap_or(false);
                        tray_state
                            .auto_hide_enabled
                            .store(checked, Ordering::Relaxed);
                        let mut cfg = tray_settings.get();
                        cfg.auto_hide_stealth = checked;
                        let _ = tray_settings.update(cfg, None);
                        let _ = app.emit("buddy://auto-hide-changed", checked);
                    }
                    "sim_capture" => {
                        let _ = app.emit("buddy://simulate-capture", ());
                    }
                    "sim_sleep" => {
                        let next = !tray_state.is_sleeping.load(Ordering::Relaxed);
                        tray_state.manual_sleep.store(next, Ordering::Relaxed);
                        tray_state.is_sleeping.store(next, Ordering::Relaxed);
                        let _ = app.emit(
                            "buddy://idle-sleep",
                            win32_sensors::SleepPayload { sleeping: next },
                        );
                    }
                    "quit" => {
                        tray_db.checkpoint_wal();
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            let win_handle = app.handle().clone();
            let focus_state = sensor_state.clone();
            main_win.on_window_event(move |event| match event {
                WindowEvent::Focused(false) => {
                    if focus_state.is_expanded.load(Ordering::Relaxed)
                        && !focus_state.is_dragging.load(Ordering::Relaxed)
                    {
                        let elapsed =
                            now_ms().saturating_sub(focus_state.last_expand_ms.load(Ordering::Relaxed));
                        if elapsed > 280 {
                            let _ = win_handle.emit("buddy://collapse", ());
                        }
                    }
                }
                WindowEvent::ScaleFactorChanged { .. } => {
                    if let Some(win) = win_handle.get_webview_window("main") {
                        apply_window_layout(
                            &win,
                            &focus_state,
                            focus_state.is_expanded.load(Ordering::Relaxed),
                        );
                    }
                }
                WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) => {
                    let attachments = classify_dropped_paths(paths);
                    if !attachments.is_empty() {
                        let _ = win_handle.emit("buddy://files-dropped", attachments);
                    }
                }
                _ => {}
            });

            start_sensor_loops(app.handle().clone(), sensor_state.clone());
            watchers::start_clipboard_watcher(
                app.handle().clone(),
                db.clone(),
                copy_guard.clone(),
            );
            watchers::start_screenshot_folder_watcher(app.handle().clone(), db.clone());
            start_snip_hotkey_listener(app.handle().clone(), snip_state.clone());
            music_sensor::start_music_sensor(app.handle().clone(), sensor_state.clone());

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Buddy");
}
