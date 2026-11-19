//! Tauri IPC command handlers grouped by domain:
//! - `system`: window/dock geometry, auto-updater, memory vault, settings & model profiles.
//! - `chat`: conversations, chat streaming, approvals, screen share, drag-drop, and snip.
//! - `plugins`: local utility plugins, tools, file dialogs, and agent skills.

pub mod chat;
pub mod mcp;
pub mod mobile;
pub mod plugins;
pub mod system;

use crate::settings::AppSettings;
use crate::win32_sensors::AppSensorState;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub(crate) fn sync_sensor_from_settings(sensor_state: &AppSensorState, saved: &AppSettings) {
    sensor_state
        .auto_hide_enabled
        .store(saved.auto_hide_stealth, Ordering::Relaxed);
    sensor_state
        .auto_collapse_on_blur
        .store(saved.auto_collapse_on_blur, Ordering::Relaxed);
    sensor_state
        .music_reactive_enabled
        .store(saved.music_reactive, Ordering::Relaxed);
    if !saved.music_reactive {
        sensor_state.is_music_playing.store(false, Ordering::Relaxed);
    }
}
