use crate::file_dialog;
use crate::logger;
use crate::media_utils::{self, SelfCopyGuard};
use crate::pin_window;
use crate::plugins::{
    apply_clean_paste_text, inspect_file_locks, open_output_in_explorer, optimize_images,
    parse_clipboard_text, run_media_download, run_media_tool, run_offline_ocr, run_pdf_tool,
    terminate_locking_process, ClipboardParseResult, LockProcessInfo, OcrResult,
    PluginListResponse, PluginManager, PluginToolOutput,
};
use crate::skills::{
    search_github_skills, GithubSkillCandidate, SkillDetail, SkillInfo, SkillsManager,
};
use crate::win32_sensors::AppSensorState;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

#[tauri::command]
pub fn list_plugins(pm: tauri::State<'_, Arc<PluginManager>>) -> PluginListResponse {
    pm.list()
}

#[tauri::command]
pub fn toggle_plugin(
    pm: tauri::State<'_, Arc<PluginManager>>,
    id: String,
    enabled: bool,
) -> PluginListResponse {
    pm.toggle(&id, enabled)
}

#[tauri::command]
pub fn set_plugin_mirror(
    pm: tauri::State<'_, Arc<PluginManager>>,
    prefix: String,
) -> PluginListResponse {
    pm.set_mirror_prefix(&prefix)
}

#[tauri::command]
pub async fn install_plugin_engine(
    app: AppHandle,
    pm: tauri::State<'_, Arc<PluginManager>>,
    id: String,
) -> Result<PluginListResponse, String> {
    pm.install_engine(&app, &id).await
}

#[tauri::command]
pub fn cancel_plugin_download(pm: tauri::State<'_, Arc<PluginManager>>) {
    pm.cancel_active_download();
}

#[tauri::command]
pub fn uninstall_plugin_engine(
    pm: tauri::State<'_, Arc<PluginManager>>,
    id: String,
) -> Result<PluginListResponse, String> {
    pm.uninstall_engine(&id)
}

#[tauri::command]
pub async fn plugin_offline_ocr(source: String) -> Result<OcrResult, String> {
    run_offline_ocr(source).await
}

#[tauri::command]
pub fn plugin_pin_image(source: String) -> Result<(), String> {
    pin_window::pin_image_to_screen(&source)
}

#[tauri::command]
pub fn plugin_close_all_pins() {
    pin_window::close_all_pins();
}

#[tauri::command]
pub fn plugin_parse_clipboard(text: Option<String>) -> Result<ClipboardParseResult, String> {
    let raw = match text {
        Some(t) => t,
        None => {
            #[cfg(windows)]
            {
                media_utils::read_clipboard_text_only().unwrap_or_default()
            }
            #[cfg(not(windows))]
            {
                String::new()
            }
        }
    };
    Ok(parse_clipboard_text(&raw))
}

#[tauri::command]
pub fn plugin_clean_paste(
    guard: tauri::State<'_, Arc<SelfCopyGuard>>,
    text: String,
) -> Result<String, String> {
    apply_clean_paste_text(&text, &guard)
}

#[tauri::command]
pub fn plugin_inspect_locks(path: String) -> Result<Vec<LockProcessInfo>, String> {
    inspect_file_locks(&path)
}

#[tauri::command]
pub fn plugin_unlock_pid(pid: u32) -> Result<(), String> {
    terminate_locking_process(pid)
}

#[tauri::command]
pub fn plugin_start_color_picker(
    app: AppHandle,
    guard: tauri::State<'_, Arc<SelfCopyGuard>>,
) -> Result<(), String> {
    pin_window::start_screen_color_picker(app, guard.inner().clone())
}

#[tauri::command]
pub fn plugin_set_keep_awake(
    pm: tauri::State<'_, Arc<PluginManager>>,
    enable: bool,
) -> Result<bool, String> {
    pm.set_keep_awake(enable)
}

#[tauri::command]
pub fn plugin_get_keep_awake(pm: tauri::State<'_, Arc<PluginManager>>) -> bool {
    pm.is_keep_awake_active()
}

#[tauri::command]
pub fn plugin_start_timer(
    app: AppHandle,
    pm: tauri::State<'_, Arc<PluginManager>>,
    seconds: u64,
    label: Option<String>,
) {
    pm.inner()
        .start_capsule_timer(app, seconds, label.unwrap_or_else(|| "Focus".into()));
}

#[tauri::command]
pub fn plugin_cancel_timer(app: AppHandle, pm: tauri::State<'_, Arc<PluginManager>>) {
    pm.cancel_capsule_timer(&app);
}

#[tauri::command]
pub async fn plugin_optimize_images(
    paths: Vec<String>,
    target_format: String,
    max_dim: u32,
    quality: u8,
) -> Result<PluginToolOutput, String> {
    tokio::task::spawn_blocking(move || optimize_images(paths, target_format, max_dim, quality))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn plugin_run_pdf_tool(
    pm: tauri::State<'_, Arc<PluginManager>>,
    action: String,
    paths: Vec<String>,
    page_range: Option<String>,
) -> Result<PluginToolOutput, String> {
    let pm_arc = pm.inner().clone();
    tokio::task::spawn_blocking(move || run_pdf_tool(&pm_arc, &action, paths, page_range))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn plugin_run_media_tool(
    app: AppHandle,
    pm: tauri::State<'_, Arc<PluginManager>>,
    action: String,
    input_path: String,
    max_width: Option<u32>,
    fps: Option<u32>,
) -> Result<PluginToolOutput, String> {
    run_media_tool(&app, &pm, &action, &input_path, max_width, fps).await
}

#[tauri::command]
pub async fn plugin_download_media(
    app: AppHandle,
    pm: tauri::State<'_, Arc<PluginManager>>,
    url: String,
    format: String,
    output_dir: Option<String>,
) -> Result<PluginToolOutput, String> {
    run_media_download(&app, &pm, &url, &format, output_dir.as_deref()).await
}

#[tauri::command]
pub fn cancel_plugin_task(pm: tauri::State<'_, Arc<PluginManager>>) {
    pm.cancel_active_task();
}

#[tauri::command]
pub fn plugin_open_folder(path: String) {
    open_output_in_explorer(&path);
}

#[tauri::command]
pub fn open_logs_folder() -> String {
    let dir = logger::logs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path_str = dir.to_string_lossy().to_string();
    open_output_in_explorer(&path_str);
    path_str
}

#[tauri::command]
pub async fn pick_folder(
    app: AppHandle,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
    title: Option<String>,
    initial_dir: Option<String>,
) -> Result<Option<String>, String> {
    file_dialog::pick_folder(&app, &sensor_state, title, initial_dir).await
}

#[tauri::command]
pub async fn pick_files(
    app: AppHandle,
    sensor_state: tauri::State<'_, Arc<AppSensorState>>,
    title: Option<String>,
    multiple: Option<bool>,
    filter_name: Option<String>,
    filter_spec: Option<String>,
    initial_dir: Option<String>,
) -> Result<Vec<String>, String> {
    file_dialog::pick_files(
        &app,
        &sensor_state,
        title,
        multiple.unwrap_or(false),
        filter_name,
        filter_spec,
        initial_dir,
    )
    .await
}

#[tauri::command]
pub fn list_skills(sm: tauri::State<'_, Arc<SkillsManager>>) -> Vec<SkillInfo> {
    sm.list()
}

#[tauri::command]
pub fn get_skill_detail(
    sm: tauri::State<'_, Arc<SkillsManager>>,
    id: String,
) -> Result<SkillDetail, String> {
    sm.get_detail(&id)
}

#[tauri::command]
pub fn toggle_skill(
    sm: tauri::State<'_, Arc<SkillsManager>>,
    id: String,
    enabled: bool,
) -> Vec<SkillInfo> {
    sm.toggle(&id, enabled)
}

#[tauri::command]
pub fn uninstall_skill(
    sm: tauri::State<'_, Arc<SkillsManager>>,
    id: String,
) -> Result<Vec<SkillInfo>, String> {
    sm.uninstall(&id)
}

#[tauri::command]
pub fn import_local_skill(
    sm: tauri::State<'_, Arc<SkillsManager>>,
    path: String,
) -> Result<Vec<SkillInfo>, String> {
    let _ = sm.import_local(&path)?;
    Ok(sm.list())
}

#[tauri::command]
pub async fn search_github_skills_cmd(query: String) -> Result<Vec<GithubSkillCandidate>, String> {
    search_github_skills(&query).await
}

#[tauri::command]
pub async fn install_github_skill_cmd(
    app: AppHandle,
    sm: tauri::State<'_, Arc<SkillsManager>>,
    pm: tauri::State<'_, Arc<PluginManager>>,
    spec: String,
) -> Result<Vec<SkillInfo>, String> {
    let mirror = pm.list().mirror_prefix;
    let _ = sm.install_from_github(&spec, &mirror).await?;
    let _ = app.emit("buddy://skills-updated", ());
    Ok(sm.list())
}

#[tauri::command]
pub fn open_skills_folder(sm: tauri::State<'_, Arc<SkillsManager>>, id: Option<String>) {
    sm.open_folder(id.as_deref());
}
