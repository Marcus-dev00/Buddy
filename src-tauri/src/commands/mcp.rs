use crate::mcp::{McpManager, McpServerConfig, McpServerStatus};
use std::sync::Arc;

#[tauri::command]
pub fn list_mcp_servers(mcp_mgr: tauri::State<'_, Arc<McpManager>>) -> Vec<McpServerStatus> {
    mcp_mgr.list_statuses()
}

#[tauri::command]
pub async fn save_mcp_server(
    mcp_mgr: tauri::State<'_, Arc<McpManager>>,
    config: McpServerConfig,
) -> Result<Vec<McpServerStatus>, String> {
    mcp_mgr.save_server(config).await
}

#[tauri::command]
pub async fn import_mcp_servers_json(
    mcp_mgr: tauri::State<'_, Arc<McpManager>>,
    raw_json: String,
) -> Result<Vec<McpServerStatus>, String> {
    mcp_mgr.import_json(&raw_json).await
}

#[tauri::command]
pub async fn toggle_mcp_server(
    mcp_mgr: tauri::State<'_, Arc<McpManager>>,
    id: String,
    enabled: bool,
) -> Result<Vec<McpServerStatus>, String> {
    mcp_mgr.toggle_server(&id, enabled).await
}

#[tauri::command]
pub fn set_mcp_server_trusted(
    mcp_mgr: tauri::State<'_, Arc<McpManager>>,
    id: String,
    trusted: bool,
) -> Result<Vec<McpServerStatus>, String> {
    mcp_mgr.set_server_trusted(&id, trusted)
}

#[tauri::command]
pub async fn reconnect_mcp_server(
    mcp_mgr: tauri::State<'_, Arc<McpManager>>,
    id: String,
) -> Result<Vec<McpServerStatus>, String> {
    let _ = mcp_mgr.connect_server(&id).await?;
    Ok(mcp_mgr.list_statuses())
}

#[tauri::command]
pub async fn reconnect_all_mcp_servers(
    mcp_mgr: tauri::State<'_, Arc<McpManager>>,
) -> Result<Vec<McpServerStatus>, String> {
    let mgr = Arc::clone(&*mcp_mgr);
    mgr.connect_all_enabled().await;
    Ok(mgr.list_statuses())
}

#[tauri::command]
pub fn delete_mcp_server(
    mcp_mgr: tauri::State<'_, Arc<McpManager>>,
    id: String,
) -> Result<Vec<McpServerStatus>, String> {
    mcp_mgr.delete_server(&id)
}
