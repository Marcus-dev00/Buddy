use crate::mobile_bridge::{MobileBridgeManager, MobileBridgeStatus};
use std::sync::Arc;
use tauri::AppHandle;

#[tauri::command]
pub fn get_mobile_bridge_status(
    mgr: tauri::State<'_, Arc<MobileBridgeManager>>,
) -> MobileBridgeStatus {
    mgr.get_status()
}

#[tauri::command]
pub fn set_mobile_bridge_enabled(
    app: AppHandle,
    mgr: tauri::State<'_, Arc<MobileBridgeManager>>,
    enabled: bool,
) -> Result<MobileBridgeStatus, String> {
    mgr.inner().set_enabled(&app, enabled)
}

#[tauri::command]
pub async fn start_mobile_tunnel(
    app: AppHandle,
    mgr: tauri::State<'_, Arc<MobileBridgeManager>>,
) -> Result<MobileBridgeStatus, String> {
    let m = Arc::clone(&*mgr);
    m.start_tunnel(&app).await
}

#[tauri::command]
pub fn stop_mobile_tunnel(
    app: AppHandle,
    mgr: tauri::State<'_, Arc<MobileBridgeManager>>,
) -> MobileBridgeStatus {
    mgr.stop_tunnel(&app);
    mgr.get_status()
}

#[tauri::command]
pub fn update_mobile_remote_settings(
    app: AppHandle,
    mgr: tauri::State<'_, Arc<MobileBridgeManager>>,
    custom_remote_url: String,
    directory_url: String,
    auto_tunnel: bool,
) -> MobileBridgeStatus {
    mgr.inner()
        .update_remote_settings(&app, custom_remote_url, directory_url, auto_tunnel)
}

#[tauri::command]
pub fn reset_mobile_pairing_token(
    app: AppHandle,
    mgr: tauri::State<'_, Arc<MobileBridgeManager>>,
) -> MobileBridgeStatus {
    mgr.inner().reset_pairing_token(&app)
}
