use super::{now_ms, sync_sensor_from_settings};
use crate::ai_client::verify_model_connection;
use crate::db::{MemoryDb, MemoryItem};
use crate::media_utils::{self, SelfCopyGuard};
use crate::settings::{AppSettings, ModelProfile, SettingsManager};
use crate::win32_sensors::{
    apply_interaction_mode, apply_window_layout, apply_window_mode_bounds, begin_drag,
    compute_layout_dims, end_drag, reset_dock_to_top, set_window_topmost, step_drag,
    AppSensorState, DockStatePayload, LayoutDims,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

#[derive(Clone, serde::Serialize)]
pub(crate) struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    pub body: String,
    pub date: Option<String>,
}

#[derive(Clone, serde::Serialize)]
struct UpdateProgressPayload {
    downloaded: u64,
    total: Option<u64>,
}

pub(crate) fn build_updater(
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

fn resolve_key_for_verify(
    settings_mgr: &SettingsManager,
    profile_id: &str,
    new_api_key: &Option<String>,
) -> String {
    match new_api_key {
        Some(k) if !k.trim().is_empty() => k.trim().to_string(),
        Some(_) => String::new(),
        None => settings_mgr.load_key_for_profile(profile_id).unwrap_or_default(),
    }
}

#[tauri::command]
pub fn set_expanded(
    app: AppHandle,
    state: tauri::State<'_, Arc<AppSensorState>>,
    expanded: bool,
) -> LayoutDims {
    state.is_expanded.store(expanded, Ordering::Relaxed);
    if expanded {
        state.manual_sleep.store(false, Ordering::Relaxed);
        state.is_sleeping.store(false, Ordering::Relaxed);
        state.last_expand_ms.store(now_ms(), Ordering::Relaxed);
    } else if state.is_window_mode.swap(false, Ordering::Relaxed) {
        state.is_window_maximized.store(false, Ordering::Relaxed);
        if let Some(win) = app.get_webview_window("main") {
            let _ = win.set_min_size::<tauri::LogicalSize<f64>>(None);
            let _ = win.set_resizable(false);
            let _ = win.set_skip_taskbar(true);
            set_window_topmost(&win, true);
        }
    }
    if let Some(win) = app.get_webview_window("main") {
        let dims = apply_window_layout(&win, &state, expanded);
        apply_interaction_mode(&win, expanded);
        return dims;
    }
    compute_layout_dims(1.0, None)
}

#[tauri::command]
pub fn set_window_mode(
    app: AppHandle,
    state: tauri::State<'_, Arc<AppSensorState>>,
    enabled: bool,
    maximized: Option<bool>,
) -> bool {
    if let Some(win) = app.get_webview_window("main") {
        if enabled {
            let max = maximized.unwrap_or(false);
            state.is_window_mode.store(true, Ordering::Relaxed);
            state.is_window_maximized.store(max, Ordering::Relaxed);
            state.is_expanded.store(true, Ordering::Relaxed);
            state.manual_sleep.store(false, Ordering::Relaxed);
            state.is_sleeping.store(false, Ordering::Relaxed);
            state.last_expand_ms.store(now_ms(), Ordering::Relaxed);
            apply_interaction_mode(&win, true);
            set_window_topmost(&win, false);
            let _ = win.set_skip_taskbar(false);
            let _ = win.set_min_size(Some(tauri::LogicalSize::new(640.0, 480.0)));
            let _ = win.set_resizable(!max);
            apply_window_mode_bounds(&win, &state, max);
            let _ = win.show();
            let _ = win.set_focus();
            return max;
        } else {
            state.is_window_mode.store(false, Ordering::Relaxed);
            state.is_window_maximized.store(false, Ordering::Relaxed);
            let _ = win.set_min_size::<tauri::LogicalSize<f64>>(None);
            let _ = win.set_resizable(false);
            let _ = win.set_skip_taskbar(true);
            set_window_topmost(&win, true);
            apply_window_layout(&win, &state, state.is_expanded.load(Ordering::Relaxed));
            apply_interaction_mode(&win, state.is_expanded.load(Ordering::Relaxed));
            return false;
        }
    }
    false
}

#[tauri::command]
pub fn toggle_window_maximize(
    app: AppHandle,
    state: tauri::State<'_, Arc<AppSensorState>>,
) -> bool {
    if !state.is_window_mode.load(Ordering::Relaxed) {
        return false;
    }
    let next = !state.is_window_maximized.load(Ordering::Relaxed);
    state.is_window_maximized.store(next, Ordering::Relaxed);
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_resizable(!next);
        apply_window_mode_bounds(&win, &state, next);
        let _ = win.set_focus();
    }
    next
}

#[tauri::command]
pub fn start_window_mode_drag(app: AppHandle, state: tauri::State<'_, Arc<AppSensorState>>) {
    if state.is_window_mode.load(Ordering::Relaxed)
        && !state.is_window_maximized.load(Ordering::Relaxed)
    {
        if let Some(win) = app.get_webview_window("main") {
            let _ = win.start_dragging();
        }
    }
}

#[tauri::command]
pub fn update_pill_bounds(state: tauri::State<'_, Arc<AppSensorState>>, width: u32, height: u32) {
    state.pill_w.store(width.max(36), Ordering::Relaxed);
    state.pill_h.store(height.max(36), Ordering::Relaxed);
}

#[tauri::command]
pub fn get_dock_state(state: tauri::State<'_, Arc<AppSensorState>>) -> DockStatePayload {
    DockStatePayload {
        dock_edge: state.get_dock_edge(),
        snap_preview: None,
    }
}

#[tauri::command]
pub fn begin_buddy_drag(app: AppHandle, state: tauri::State<'_, Arc<AppSensorState>>) {
    if let Some(win) = app.get_webview_window("main") {
        begin_drag(&win, &state);
    }
}

#[tauri::command]
pub fn step_buddy_drag(app: AppHandle, state: tauri::State<'_, Arc<AppSensorState>>) -> DockStatePayload {
    if let Some(win) = app.get_webview_window("main") {
        return step_drag(&win, &state);
    }
    DockStatePayload {
        dock_edge: state.get_dock_edge(),
        snap_preview: None,
    }
}

#[tauri::command]
pub fn end_buddy_drag(
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
pub fn reset_dock_position(
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

#[tauri::command]
pub fn get_app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
pub async fn check_app_update(
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
pub async fn install_app_update(
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
pub fn set_auto_hide_stealth(state: tauri::State<'_, Arc<AppSensorState>>, enabled: bool) {
    state.auto_hide_enabled.store(enabled, Ordering::Relaxed);
}

#[tauri::command]
pub fn set_auto_collapse_on_blur(
    settings_mgr: tauri::State<'_, Arc<SettingsManager>>,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
    enabled: bool,
) -> Result<(), String> {
    sensor_state
        .auto_collapse_on_blur
        .store(enabled, Ordering::Relaxed);
    settings_mgr.set_auto_collapse_on_blur(enabled)
}

#[tauri::command]
pub fn set_agent_permission_mode(
    settings_mgr: tauri::State<'_, Arc<SettingsManager>>,
    mode: String,
) -> Result<String, String> {
    settings_mgr.set_agent_permission_mode(&mode)
}

#[tauri::command]
pub fn set_music_reactive(state: tauri::State<'_, Arc<AppSensorState>>, enabled: bool) {
    state.music_reactive_enabled.store(enabled, Ordering::Relaxed);
    if !enabled {
        state.is_music_playing.store(false, Ordering::Relaxed);
    }
}

#[tauri::command]
pub fn set_music_playing_flag(state: tauri::State<'_, Arc<AppSensorState>>, playing: bool) {
    state.is_music_playing.store(playing, Ordering::Relaxed);
}

#[tauri::command]
pub fn get_expanded_bounds(
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
pub fn list_memories(
    db: tauri::State<'_, Arc<MemoryDb>>,
    query: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<MemoryItem>, String> {
    db.list_memories(query.as_deref(), limit.unwrap_or(80))
}

#[tauri::command]
pub fn list_memory_facts(
    db: tauri::State<'_, Arc<MemoryDb>>,
    limit: Option<usize>,
) -> Result<Vec<MemoryItem>, String> {
    db.list_active_facts(limit.unwrap_or(100))
}

#[tauri::command]
pub fn add_memory_fact(
    db: tauri::State<'_, Arc<MemoryDb>>,
    content: String,
    category: Option<String>,
) -> Result<MemoryItem, String> {
    db.save_fact(&content, category.as_deref())
}

#[tauri::command]
pub fn toggle_memory_star(db: tauri::State<'_, Arc<MemoryDb>>, id: i64) -> Result<bool, String> {
    db.toggle_star(id)
}

#[tauri::command]
pub fn delete_memory(db: tauri::State<'_, Arc<MemoryDb>>, id: i64) -> Result<(), String> {
    db.delete_memory(id)
}

#[tauri::command]
pub fn copy_memory_to_clipboard(
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
pub fn read_memory_image(
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
pub fn get_settings(settings_mgr: tauri::State<'_, Arc<SettingsManager>>) -> AppSettings {
    settings_mgr.get()
}

#[derive(serde::Serialize)]
pub struct VerifySaveResult {
    settings: AppSettings,
    latency_ms: u64,
}

#[tauri::command]
pub async fn verify_and_save_model_config(
    settings_mgr: tauri::State<'_, Arc<SettingsManager>>,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
    settings: AppSettings,
    new_api_key: Option<String>,
) -> Result<VerifySaveResult, String> {
    let key_to_test =
        resolve_key_for_verify(&settings_mgr, &settings.chat_profile_id, &new_api_key);

    let latency_ms = verify_model_connection(
        &settings.api_provider,
        &settings.base_url,
        &settings.model_id,
        &key_to_test,
    )
    .await?;

    let saved = settings_mgr.update(settings, new_api_key)?;
    sync_sensor_from_settings(&sensor_state, &saved);

    Ok(VerifySaveResult {
        settings: saved,
        latency_ms,
    })
}

#[tauri::command]
pub fn save_model_profile(
    settings_mgr: tauri::State<'_, Arc<SettingsManager>>,
    profile: ModelProfile,
    new_api_key: Option<String>,
    set_as_chat: bool,
    set_as_watch: bool,
) -> Result<AppSettings, String> {
    settings_mgr.save_profile(profile, new_api_key, set_as_chat, set_as_watch)
}

#[tauri::command]
pub async fn verify_and_save_model_profile(
    settings_mgr: tauri::State<'_, Arc<SettingsManager>>,
    profile: ModelProfile,
    new_api_key: Option<String>,
    set_as_chat: bool,
    set_as_watch: bool,
) -> Result<VerifySaveResult, String> {
    let key_to_test = resolve_key_for_verify(&settings_mgr, &profile.id, &new_api_key);

    let latency_ms = verify_model_connection(
        &profile.api_provider,
        &profile.base_url,
        &profile.model_id,
        &key_to_test,
    )
    .await?;

    let saved = settings_mgr.save_profile(profile, new_api_key, set_as_chat, set_as_watch)?;
    Ok(VerifySaveResult {
        settings: saved,
        latency_ms,
    })
}

#[tauri::command]
pub fn delete_model_profile(
    settings_mgr: tauri::State<'_, Arc<SettingsManager>>,
    profile_id: String,
) -> Result<AppSettings, String> {
    settings_mgr.delete_profile(&profile_id)
}

#[tauri::command]
pub fn select_role_models(
    settings_mgr: tauri::State<'_, Arc<SettingsManager>>,
    chat_profile_id: Option<String>,
    watch_profile_id: Option<String>,
) -> Result<AppSettings, String> {
    settings_mgr.select_role_models(chat_profile_id, watch_profile_id)
}

#[tauri::command]
pub fn save_settings(
    settings_mgr: tauri::State<'_, Arc<SettingsManager>>,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
    settings: AppSettings,
    new_api_key: Option<String>,
) -> Result<AppSettings, String> {
    let saved = settings_mgr.update(settings, new_api_key)?;
    sync_sensor_from_settings(&sensor_state, &saved);
    Ok(saved)
}
