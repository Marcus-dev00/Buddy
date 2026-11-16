//! Windows native sensors, fixed-canvas top-center layout, cursor hit-testing,
//! Win+D desktop recovery, and accurate stealth detection.

use crate::settings::SettingsManager;
use serde::Serialize;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize};

#[cfg(windows)]
use windows::core::{PCWSTR, PWSTR};
#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, HWND, POINT, RECT};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    MONITOR_DEFAULTTOPRIMARY,
};
#[cfg(windows)]
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
#[cfg(windows)]
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_READ, REG_QWORD, REG_VALUE_TYPE,
};
#[cfg(windows)]
use windows::Win32::System::SystemInformation::GetTickCount;
#[cfg(windows)]
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
#[cfg(windows)]
use windows::Win32::UI::Shell::{
    SHQueryUserNotificationState, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetCursorPos, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, GWL_EXSTYLE, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SW_SHOWNOACTIVATE, WS_EX_NOACTIVATE,
};

pub const CANVAS_PAD: f64 = 8.0;
pub const DOCK_EDGE_GAP_LOGICAL: f64 = 9.0;
pub const EXPANDED_V_W: f64 = 420.0;
pub const EXPANDED_H_W: f64 = 456.0;
pub const WINDOW_CANVAS_W: f64 = EXPANDED_H_W + CANVAS_PAD * 2.0; // 472.0

#[derive(Clone, Copy, Serialize)]
pub struct LayoutDims {
    pub v_w: f64,
    pub v_h: f64,
    pub h_w: f64,
    pub h_h: f64,
    pub canvas_w: f64,
    pub canvas_h: f64,
    pub pad: f64,
}

#[derive(Clone, Serialize)]
pub struct DockStatePayload {
    pub dock_edge: String,
    pub snap_preview: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DragSession {
    pub start_cursor: (i32, i32),
    pub start_win_pos: (i32, i32),
    pub start_box_center: (f64, f64),
    pub initial_edge: String,
    pub detached: bool,
    pub expanded: bool,
}

#[derive(Clone, Serialize)]
pub struct PeekPayload {
    pub hovering: bool,
    pub dx: f64,
    pub dy: f64,
}

#[derive(Clone, Serialize)]
pub struct StealthPayload {
    pub active: bool,
    pub reason: String,
}

#[derive(Clone, Serialize)]
pub struct SleepPayload {
    pub sleeping: bool,
}

pub struct AppSensorState {
    pub is_expanded: AtomicBool,
    pub is_stealth: AtomicBool,
    pub auto_hide_enabled: AtomicBool,
    pub music_reactive_enabled: AtomicBool,
    pub is_music_playing: AtomicBool,
    pub is_sleeping: AtomicBool,
    pub manual_sleep: AtomicBool,
    pub is_dragging: AtomicBool,
    pub last_expand_ms: std::sync::atomic::AtomicU64,
    /// Active visual pill dimensions inside the fixed transparent canvas (logical px)
    pub pill_w: std::sync::atomic::AtomicU32,
    pub pill_h: std::sync::atomic::AtomicU32,
    pub dock_edge: Mutex<String>,
    pub anchor_cx: AtomicI32,
    pub anchor_cy: AtomicI32,
    pub has_anchor: AtomicBool,
    pub drag_session: Mutex<Option<DragSession>>,
}

impl Default for AppSensorState {
    fn default() -> Self {
        Self {
            is_expanded: AtomicBool::new(false),
            is_stealth: AtomicBool::new(false),
            auto_hide_enabled: AtomicBool::new(false),
            music_reactive_enabled: AtomicBool::new(true),
            is_music_playing: AtomicBool::new(false),
            is_sleeping: AtomicBool::new(false),
            manual_sleep: AtomicBool::new(false),
            is_dragging: AtomicBool::new(false),
            last_expand_ms: std::sync::atomic::AtomicU64::new(0),
            pill_w: std::sync::atomic::AtomicU32::new(152),
            pill_h: std::sync::atomic::AtomicU32::new(42),
            dock_edge: Mutex::new("top".to_string()),
            anchor_cx: AtomicI32::new(0),
            anchor_cy: AtomicI32::new(0),
            has_anchor: AtomicBool::new(false),
            drag_session: Mutex::new(None),
        }
    }
}

impl AppSensorState {
    pub fn get_dock_edge(&self) -> String {
        self.dock_edge
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|_| "top".to_string())
    }

    pub fn set_dock_edge(&self, edge: &str) {
        if let Ok(mut g) = self.dock_edge.lock() {
            *g = edge.to_string();
        }
    }
}

#[cfg(windows)]
fn to_hwnd(window: &tauri::WebviewWindow) -> Option<HWND> {
    window.hwnd().ok().map(|h| HWND(h.0 as *mut c_void))
}

/// Returns monitor work area containing `pt` (or primary) in physical pixels: (left, top, width, height)
#[cfg(windows)]
pub fn get_work_area_at(pt_x: i32, pt_y: i32) -> (i32, i32, i32, i32) {
    unsafe {
        let hmon = MonitorFromPoint(POINT { x: pt_x, y: pt_y }, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmon, &mut mi).as_bool() {
            let r = mi.rcWork;
            return (r.left, r.top, r.right - r.left, r.bottom - r.top);
        }
    }
    (0, 0, 1920, 1080)
}

/// Returns primary monitor work area in physical pixels: (left, top, width, height)
#[cfg(windows)]
pub fn get_primary_work_area() -> (i32, i32, i32, i32) {
    unsafe {
        let hmon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmon, &mut mi).as_bool() {
            let r = mi.rcWork;
            return (r.left, r.top, r.right - r.left, r.bottom - r.top);
        }
    }
    (0, 0, 1920, 1080)
}

#[cfg(not(windows))]
pub fn get_work_area_at(_pt_x: i32, _pt_y: i32) -> (i32, i32, i32, i32) {
    (0, 0, 1920, 1080)
}

/// Computes Expanded + Canvas logical dimensions with strict <= 50% work-area height cap.
pub fn compute_layout_dims(scale: f64, anchor: Option<(i32, i32)>) -> LayoutDims {
    #[cfg(windows)]
    let (_, _, _, work_h_phys) = match anchor {
        Some((ax, ay)) => get_work_area_at(ax, ay),
        None => get_primary_work_area(),
    };
    #[cfg(not(windows))]
    let work_h_phys = 1080;

    let work_h_logical = (work_h_phys as f64) / scale.max(1.0);
    let max_half = (work_h_logical * 0.50).floor();
    let capped_h = max_half.clamp(360.0, 480.0);
    LayoutDims {
        v_w: EXPANDED_V_W,
        v_h: capped_h,
        h_w: EXPANDED_H_W,
        h_h: capped_h,
        canvas_w: WINDOW_CANVAS_W,
        canvas_h: capped_h + CANVAS_PAD * 2.0,
        pad: CANVAS_PAD,
    }
}

/// Returns active visual box (cx, cy, w, h) in logical px inside the transparent canvas.
pub fn active_box_metrics(
    dock_edge: &str,
    dims: LayoutDims,
    expanded: bool,
    pill_w: f64,
    pill_h: f64,
) -> (f64, f64, f64, f64) {
    let side_docked = matches!(dock_edge, "left" | "right");
    let bw = if expanded {
        if side_docked { dims.h_w } else { dims.v_w }
    } else {
        pill_w
    };
    let bh = if expanded {
        if side_docked { dims.h_h } else { dims.v_h }
    } else {
        pill_h
    };
    match dock_edge {
        "bottom" => (dims.canvas_w / 2.0, dims.canvas_h - dims.pad - bh / 2.0, bw, bh),
        "left" => (dims.pad + bw / 2.0, dims.canvas_h / 2.0, bw, bh),
        "right" => (dims.canvas_w - dims.pad - bw / 2.0, dims.canvas_h / 2.0, bw, bh),
        _ => (dims.canvas_w / 2.0, dims.pad + bh / 2.0, bw, bh),
    }
}

/// Toggle WS_EX_NOACTIVATE so collapsed states never steal focus from foreground apps.
#[cfg(windows)]
pub fn set_no_activate(window: &tauri::WebviewWindow, enable: bool) {
    if let Some(hwnd) = to_hwnd(window) {
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let flag = WS_EX_NOACTIVATE.0 as isize;
            let new_style = if enable { style | flag } else { style & !flag };
            if new_style != style {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_style);
                let _ = SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }
}

#[cfg(not(windows))]
pub fn set_no_activate(_window: &tauri::WebviewWindow, _enable: bool) {}

/// Sizes and positions the transparent canvas cleanly for any dock edge (`top`, `bottom`, `left`, `right`, `free`).
pub fn apply_window_layout(
    window: &tauri::WebviewWindow,
    state: &AppSensorState,
    expanded: bool,
) -> LayoutDims {
    let scale = window.scale_factor().unwrap_or(1.0);
    let edge_gap = (DOCK_EDGE_GAP_LOGICAL * scale).round() as i32;

    if !state.has_anchor.load(Ordering::Relaxed) {
        let (wx, wy, ww, _) = get_primary_work_area();
        let default_cx = wx + ww / 2;
        let default_cy = wy + edge_gap + (21.0 * scale).round() as i32;
        state.anchor_cx.store(default_cx, Ordering::Relaxed);
        state.anchor_cy.store(default_cy, Ordering::Relaxed);
        state.has_anchor.store(true, Ordering::Relaxed);
    }

    let dock_edge = state.get_dock_edge();
    let mut cx = state.anchor_cx.load(Ordering::Relaxed);
    let mut cy = state.anchor_cy.load(Ordering::Relaxed);
    let dims = compute_layout_dims(scale, Some((cx, cy)));

    let phys_w = (dims.canvas_w * scale).round() as i32;
    let phys_h = (dims.canvas_h * scale).round() as i32;
    let pad_px = (dims.pad * scale).round() as i32;

    let cur_size = window.outer_size().ok();
    if cur_size.map(|s| s.width as i32 != phys_w || s.height as i32 != phys_h).unwrap_or(true) {
        let _ = window.set_size(PhysicalSize::new(phys_w as u32, phys_h as u32));
    }

    let (wx, wy, ww, wh) = get_work_area_at(cx, cy);
    let pill_w = state.pill_w.load(Ordering::Relaxed) as f64;
    let pill_h = state.pill_h.load(Ordering::Relaxed) as f64;
    let (_, _, box_w, box_h) = active_box_metrics(&dock_edge, dims, expanded, pill_w, pill_h);
    let bw_phys = (box_w * scale).round() as i32;
    let bh_phys = (box_h * scale).round() as i32;

    let (win_x, win_y) = match dock_edge.as_str() {
        "top" => {
            let min_cx = wx + bw_phys / 2 + edge_gap;
            let max_cx = (wx + ww - bw_phys / 2 - edge_gap).max(min_cx);
            cx = cx.clamp(min_cx, max_cx);
            state.anchor_cx.store(cx, Ordering::Relaxed);
            (cx - phys_w / 2, wy - pad_px + edge_gap)
        }
        "bottom" => {
            let min_cx = wx + bw_phys / 2 + edge_gap;
            let max_cx = (wx + ww - bw_phys / 2 - edge_gap).max(min_cx);
            cx = cx.clamp(min_cx, max_cx);
            state.anchor_cx.store(cx, Ordering::Relaxed);
            (cx - phys_w / 2, wy + wh - phys_h + pad_px - edge_gap)
        }
        "left" => {
            let min_cy = wy + bh_phys / 2 + edge_gap;
            let max_cy = (wy + wh - bh_phys / 2 - edge_gap).max(min_cy);
            cy = cy.clamp(min_cy, max_cy);
            state.anchor_cy.store(cy, Ordering::Relaxed);
            (wx - pad_px + edge_gap, cy - phys_h / 2)
        }
        "right" => {
            let min_cy = wy + bh_phys / 2 + edge_gap;
            let max_cy = (wy + wh - bh_phys / 2 - edge_gap).max(min_cy);
            cy = cy.clamp(min_cy, max_cy);
            state.anchor_cy.store(cy, Ordering::Relaxed);
            (wx + ww - phys_w + pad_px - edge_gap, cy - phys_h / 2)
        }
        _ => {
            // "free"
            let pill_half_h = (21.0 * scale).round() as i32;
            let min_cx = wx + bw_phys / 2 + edge_gap;
            let max_cx = (wx + ww - bw_phys / 2 - edge_gap).max(min_cx);
            let clamped_cx = cx.clamp(min_cx, max_cx);

            let desired_top = cy - pill_half_h;
            let min_top = wy + edge_gap;
            let max_top = (wy + wh - bh_phys - edge_gap).max(min_top);
            let clamped_top = desired_top.clamp(min_top, max_top);

            if !expanded {
                state.anchor_cx.store(clamped_cx, Ordering::Relaxed);
                state.anchor_cy.store(clamped_top + pill_half_h, Ordering::Relaxed);
            }
            (clamped_cx - phys_w / 2, clamped_top - pad_px)
        }
    };

    let _ = window.set_position(PhysicalPosition::new(win_x, win_y));
    dims
}

fn detect_snap_edge(
    pt_x: i32,
    pt_y: i32,
    box_cx: f64,
    box_cy: f64,
    box_w_phys: f64,
    box_h_phys: f64,
    wx: i32,
    wy: i32,
    ww: i32,
    wh: i32,
    scale: f64,
) -> Option<String> {
    let thresh = 30.0 * scale;
    let d_left = ((box_cx - box_w_phys / 2.0) - wx as f64).min((pt_x - wx) as f64);
    let d_right = ((wx + ww) as f64 - (box_cx + box_w_phys / 2.0)).min(((wx + ww) - pt_x) as f64);
    let d_top = ((box_cy - box_h_phys / 2.0) - wy as f64).min((pt_y - wy) as f64);
    let d_bottom = ((wy + wh) as f64 - (box_cy + box_h_phys / 2.0)).min(((wy + wh) - pt_y) as f64);

    let candidates = [
        ("top", d_top),
        ("bottom", d_bottom),
        ("left", d_left),
        ("right", d_right),
    ];

    candidates
        .into_iter()
        .filter(|(_, d)| *d <= thresh)
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(edge, _)| edge.to_string())
}

pub fn begin_drag(window: &tauri::WebviewWindow, state: &AppSensorState) {
    #[cfg(windows)]
    if let Some(hwnd) = to_hwnd(window) {
        unsafe {
            let mut pt = POINT::default();
            let mut rect = RECT::default();
            if GetCursorPos(&mut pt).is_ok() && GetWindowRect(hwnd, &mut rect).is_ok() {
                let scale = window.scale_factor().unwrap_or(1.0);
                let expanded = state.is_expanded.load(Ordering::Relaxed);
                let dock_edge = state.get_dock_edge();
                let dims = compute_layout_dims(scale, Some((pt.x, pt.y)));
                let pill_w = state.pill_w.load(Ordering::Relaxed) as f64;
                let pill_h = state.pill_h.load(Ordering::Relaxed) as f64;
                let (lx, ly, _, _) = active_box_metrics(&dock_edge, dims, expanded, pill_w, pill_h);
                let box_cx = rect.left as f64 + lx * scale;
                let box_cy = rect.top as f64 + ly * scale;

                if let Ok(mut g) = state.drag_session.lock() {
                    *g = Some(DragSession {
                        start_cursor: (pt.x, pt.y),
                        start_win_pos: (rect.left, rect.top),
                        start_box_center: (box_cx, box_cy),
                        detached: dock_edge == "free",
                        initial_edge: dock_edge,
                        expanded,
                    });
                }
                state.is_dragging.store(true, Ordering::Relaxed);
            }
        }
    }
}

pub fn step_drag(window: &tauri::WebviewWindow, state: &AppSensorState) -> DockStatePayload {
    #[cfg(windows)]
    {
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt).is_ok() } {
            let scale = window.scale_factor().unwrap_or(1.0);
            let detach_thresh = 42.0 * scale;
            let (wx, wy, ww, wh) = get_work_area_at(pt.x, pt.y);
            let dims = compute_layout_dims(scale, Some((pt.x, pt.y)));
            let canvas_w_phys = dims.canvas_w * scale;
            let canvas_h_phys = dims.canvas_h * scale;
            let pad_phys = dims.pad * scale;

            let mut session_guard = match state.drag_session.lock() {
                Ok(g) => g,
                Err(_) => {
                    return DockStatePayload {
                        dock_edge: state.get_dock_edge(),
                        snap_preview: None,
                    }
                }
            };

            if let Some(sess) = session_guard.as_mut() {
                let dx = (pt.x - sess.start_cursor.0) as f64;
                let dy = (pt.y - sess.start_cursor.1) as f64;

                if !sess.detached {
                    let pulled_away = match sess.initial_edge.as_str() {
                        "left" => dx > detach_thresh,
                        "right" => dx < -detach_thresh,
                        "top" => dy > detach_thresh,
                        "bottom" => dy < -detach_thresh,
                        _ => true,
                    };
                    if pulled_away {
                        sess.detached = true;
                        state.set_dock_edge("free");
                    }
                }

                if !sess.detached {
                    // Slide smoothly along the currently docked edge rail
                    let (nx, ny) = match sess.initial_edge.as_str() {
                        "left" | "right" => {
                            let min_y = wy - (canvas_h_phys / 2.0) as i32 + (30.0 * scale) as i32;
                            let max_y = (wy + wh) - (canvas_h_phys / 2.0) as i32 - (30.0 * scale) as i32;
                            (
                                sess.start_win_pos.0,
                                (sess.start_win_pos.1 + dy.round() as i32).clamp(min_y, max_y.max(min_y)),
                            )
                        }
                        _ => {
                            let min_x = wx - (canvas_w_phys / 2.0) as i32 + (30.0 * scale) as i32;
                            let max_x = (wx + ww) - (canvas_w_phys / 2.0) as i32 - (30.0 * scale) as i32;
                            (
                                (sess.start_win_pos.0 + dx.round() as i32).clamp(min_x, max_x.max(min_x)),
                                sess.start_win_pos.1,
                            )
                        }
                    };
                    let _ = window.set_position(PhysicalPosition::new(nx, ny));
                    return DockStatePayload {
                        dock_edge: sess.initial_edge.clone(),
                        snap_preview: None,
                    };
                }

                // Detached ("free"): move visual box 1:1 with cursor across monitors & report snap preview
                let pill_w = state.pill_w.load(Ordering::Relaxed) as f64;
                let pill_h = state.pill_h.load(Ordering::Relaxed) as f64;
                let box_w_phys = (if sess.expanded { dims.v_w } else { pill_w.max(152.0) }) * scale;
                let box_h_phys = (if sess.expanded { dims.v_h } else { pill_h.min(78.0) }) * scale;

                let target_box_cx = sess.start_box_center.0 + dx;
                let target_box_cy = sess.start_box_center.1 + dy;

                let win_x = (target_box_cx - canvas_w_phys / 2.0).round() as i32;
                let win_y = (target_box_cy - box_h_phys / 2.0 - pad_phys).round() as i32;
                let _ = window.set_position(PhysicalPosition::new(win_x, win_y));

                let preview = detect_snap_edge(
                    pt.x,
                    pt.y,
                    target_box_cx,
                    target_box_cy,
                    box_w_phys,
                    box_h_phys,
                    wx,
                    wy,
                    ww,
                    wh,
                    scale,
                );
                return DockStatePayload {
                    dock_edge: "free".to_string(),
                    snap_preview: preview,
                };
            }
        }
    }
    DockStatePayload {
        dock_edge: state.get_dock_edge(),
        snap_preview: None,
    }
}

pub fn end_drag(
    app: &AppHandle,
    window: &tauri::WebviewWindow,
    state: &AppSensorState,
    settings_mgr: &SettingsManager,
) -> DockStatePayload {
    state.is_dragging.store(false, Ordering::Relaxed);
    let sess = state.drag_session.lock().ok().and_then(|mut g| g.take());

    #[cfg(windows)]
    if let Some(sess) = sess {
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt).is_ok() } {
            let scale = window.scale_factor().unwrap_or(1.0);
            let dx = (pt.x - sess.start_cursor.0) as f64;
            let dy = (pt.y - sess.start_cursor.1) as f64;
            let (wx, wy, ww, wh) = get_work_area_at(pt.x, pt.y);
            let dims = compute_layout_dims(scale, Some((pt.x, pt.y)));

            let target_box_cx = sess.start_box_center.0 + dx;
            let target_box_cy = sess.start_box_center.1 + dy;
            let pill_w = state.pill_w.load(Ordering::Relaxed) as f64;
            let pill_h = state.pill_h.load(Ordering::Relaxed) as f64;

            let final_edge = if !sess.detached {
                sess.initial_edge
            } else {
                let box_w_phys = (if sess.expanded { dims.v_w } else { pill_w.max(152.0) }) * scale;
                let box_h_phys = (if sess.expanded { dims.v_h } else { pill_h.min(78.0) }) * scale;
                detect_snap_edge(
                    pt.x,
                    pt.y,
                    target_box_cx,
                    target_box_cy,
                    box_w_phys,
                    box_h_phys,
                    wx,
                    wy,
                    ww,
                    wh,
                    scale,
                )
                .unwrap_or_else(|| "free".to_string())
            };

            // Store collapsed pill center consistently regardless of expanded/collapsed state
            let pill_half_h = (21.0 * scale).round();
            let box_h_phys = (if sess.expanded { dims.v_h } else { pill_h }) * scale;
            let anchor_cx = target_box_cx.round() as i32;
            let anchor_cy = if final_edge == "free" && sess.expanded {
                (target_box_cy - box_h_phys / 2.0 + pill_half_h).round() as i32
            } else {
                target_box_cy.round() as i32
            };

            state.set_dock_edge(&final_edge);
            state.anchor_cx.store(anchor_cx, Ordering::Relaxed);
            state.anchor_cy.store(anchor_cy, Ordering::Relaxed);
            state.has_anchor.store(true, Ordering::Relaxed);

            apply_window_layout(window, state, sess.expanded);
            let saved_cx = state.anchor_cx.load(Ordering::Relaxed);
            let saved_cy = state.anchor_cy.load(Ordering::Relaxed);
            settings_mgr.save_dock_position(&final_edge, saved_cx, saved_cy);

            let payload = DockStatePayload {
                dock_edge: final_edge,
                snap_preview: None,
            };
            let _ = app.emit("buddy://dock-changed", payload.clone());
            return payload;
        }
    }

    let payload = DockStatePayload {
        dock_edge: state.get_dock_edge(),
        snap_preview: None,
    };
    let _ = app.emit("buddy://dock-changed", payload.clone());
    payload
}

pub fn reset_dock_to_top(
    app: &AppHandle,
    window: &tauri::WebviewWindow,
    state: &AppSensorState,
    settings_mgr: &SettingsManager,
) -> DockStatePayload {
    let scale = window.scale_factor().unwrap_or(1.0);
    let (wx, wy, ww, _) = get_primary_work_area();
    let cx = wx + ww / 2;
    let cy = wy + ((DOCK_EDGE_GAP_LOGICAL + 21.0) * scale).round() as i32;
    state.set_dock_edge("top");
    state.anchor_cx.store(cx, Ordering::Relaxed);
    state.anchor_cy.store(cy, Ordering::Relaxed);
    state.has_anchor.store(true, Ordering::Relaxed);
    apply_window_layout(window, state, state.is_expanded.load(Ordering::Relaxed));
    settings_mgr.save_dock_position("top", cx, cy);
    let payload = DockStatePayload {
        dock_edge: "top".to_string(),
        snap_preview: None,
    };
    let _ = app.emit("buddy://dock-changed", payload.clone());
    payload
}

/// Updates focus/cursor pass-through state instantaneously without resizing the OS window.
pub fn apply_interaction_mode(window: &tauri::WebviewWindow, expanded: bool) {
    if expanded {
        set_no_activate(window, false);
        let _ = window.set_ignore_cursor_events(false);
        #[cfg(windows)]
        if let Some(hwnd) = to_hwnd(window) {
            unsafe {
                let _ = SetForegroundWindow(hwnd);
            }
        }
        let _ = window.set_focus();
    } else {
        set_no_activate(window, true);
        let _ = window.set_ignore_cursor_events(true);
    }
}

/// Show window without activating foreground focus and assert HWND_TOPMOST.
#[cfg(windows)]
pub fn show_window_no_activate(window: &tauri::WebviewWindow) {
    if let Some(hwnd) = to_hwnd(window) {
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    } else {
        let _ = window.show();
    }
}

#[cfg(not(windows))]
pub fn show_window_no_activate(window: &tauri::WebviewWindow) {
    let _ = window.show();
}

#[cfg(windows)]
fn get_window_class(hwnd: HWND) -> String {
    unsafe {
        let mut class_buf = [0u16; 128];
        let len = GetClassNameW(hwnd, &mut class_buf);
        if len > 0 {
            return String::from_utf16_lossy(&class_buf[..len as usize]);
        }
    }
    String::new()
}

#[cfg(windows)]
fn is_shell_or_desktop_class(cls: &str) -> bool {
    matches!(
        cls,
        "Progman"
            | "WorkerW"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "NotifyIconOverflowWindow"
            | "TopLevelWindowForOverflowXamlIsland"
            | "Windows.UI.Core.CoreWindow"
            | "MultitaskingViewFrame"
            | "ForegroundStaging"
    )
}

#[cfg(windows)]
fn is_fullscreen_or_game(own_pid: u32) -> bool {
    unsafe {
        if let Ok(state) = SHQueryUserNotificationState() {
            if state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE {
                return true;
            }
        }

        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return false;
        }
        let mut fg_pid: u32 = 0;
        GetWindowThreadProcessId(fg, Some(&mut fg_pid));
        if fg_pid == own_pid || fg_pid == 0 {
            return false;
        }

        let cls = get_window_class(fg);
        if is_shell_or_desktop_class(&cls) {
            return false;
        }

        let mut fg_rect = RECT::default();
        if GetWindowRect(fg, &mut fg_rect).is_err() {
            return false;
        }
        let hmon = MonitorFromPoint(
            POINT {
                x: fg_rect.left + 10,
                y: fg_rect.top + 10,
            },
            MONITOR_DEFAULTTOPRIMARY,
        );
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmon, &mut mi).as_bool() {
            let m = mi.rcMonitor;
            let covers_screen = fg_rect.left <= m.left
                && fg_rect.top <= m.top
                && fg_rect.right >= m.right
                && fg_rect.bottom >= m.bottom;
            if covers_screen {
                return true;
            }
        }
    }
    false
}

#[cfg(windows)]
fn running_meeting_processes() -> (Vec<String>, Vec<String>) {
    const MEETING_PROCS: &[&str] = &[
        "zoom.exe",
        "teams.exe",
        "ms-teams.exe",
        "wemeetapp.exe",
        "lark.exe",
        "feishu.exe",
        "dingtalk.exe",
        "ciscocollabhost.exe",
        "obs64.exe",
        "discord.exe",
    ];
    let mut exe_names = Vec::new();
    let mut hash_paths = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return (exe_names, hash_paths);
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let end = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..end]).to_ascii_lowercase();
                if MEETING_PROCS.contains(&name.as_str()) {
                    exe_names.push(name);
                    if let Ok(proc_handle) =
                        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, entry.th32ProcessID)
                    {
                        let mut path_buf = [0u16; 512];
                        let mut len = path_buf.len() as u32;
                        if QueryFullProcessImageNameW(
                            proc_handle,
                            PROCESS_NAME_WIN32,
                            PWSTR(path_buf.as_mut_ptr()),
                            &mut len,
                        )
                        .is_ok()
                        {
                            let full_path = String::from_utf16_lossy(&path_buf[..len as usize])
                                .to_ascii_lowercase()
                                .replace('\\', "#");
                            hash_paths.push(full_path);
                        }
                        let _ = CloseHandle(proc_handle);
                    }
                }
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    (exe_names, hash_paths)
}

#[cfg(windows)]
fn check_consent_store_for_running(
    subpath: &str,
    exe_names: &[String],
    hash_paths: &[String],
) -> bool {
    fn key_matches_running(
        hkey: HKEY,
        is_non_packaged: bool,
        exe_names: &[String],
        hash_paths: &[String],
    ) -> bool {
        unsafe {
            let mut idx = 0u32;
            let mut name_buf = [0u16; 260];
            loop {
                let mut name_len = name_buf.len() as u32;
                let status = RegEnumKeyExW(
                    hkey,
                    idx,
                    PWSTR(name_buf.as_mut_ptr()),
                    &mut name_len,
                    None,
                    PWSTR::null(),
                    None,
                    None,
                );
                if status.is_err() {
                    break;
                }
                idx += 1;
                let sub_name: Vec<u16> = name_buf[..name_len as usize]
                    .iter()
                    .copied()
                    .chain(std::iter::once(0))
                    .collect();
                let mut child = HKEY::default();
                if RegOpenKeyExW(hkey, PCWSTR(sub_name.as_ptr()), 0, KEY_READ, &mut child).is_ok() {
                    let sub_str =
                        String::from_utf16_lossy(&name_buf[..name_len as usize]).to_ascii_lowercase();
                    let active = if sub_str == "nonpackaged" {
                        key_matches_running(child, true, exe_names, hash_paths)
                    } else {
                        let matches_proc = if is_non_packaged {
                            hash_paths.iter().any(|p| p == &sub_str)
                        } else {
                            (sub_str.contains("msteams")
                                && exe_names.iter().any(|e| e == "ms-teams.exe" || e == "teams.exe"))
                                || (sub_str.contains("zoom")
                                    && exe_names.iter().any(|e| e == "zoom.exe"))
                        };

                        if matches_proc {
                            let val_name: Vec<u16> = "LastUsedTimeStop\0".encode_utf16().collect();
                            let mut val_type = REG_VALUE_TYPE::default();
                            let mut data = 0u64;
                            let mut data_size = std::mem::size_of::<u64>() as u32;
                            let q = RegQueryValueExW(
                                child,
                                PCWSTR(val_name.as_ptr()),
                                None,
                                Some(&mut val_type),
                                Some(&mut data as *mut u64 as *mut u8),
                                Some(&mut data_size),
                            );
                            q.is_ok() && val_type == REG_QWORD && data == 0
                        } else {
                            false
                        }
                    };
                    let _ = RegCloseKey(child);
                    if active {
                        return true;
                    }
                }
            }
        }
        false
    }

    let wide: Vec<u16> = subpath.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(wide.as_ptr()), 0, KEY_READ, &mut hkey).is_ok() {
            let active = key_matches_running(hkey, false, exe_names, hash_paths);
            let _ = RegCloseKey(hkey);
            return active;
        }
    }
    false
}

#[cfg(windows)]
fn is_active_meeting() -> bool {
    let (exe_names, hash_paths) = running_meeting_processes();
    if exe_names.is_empty() {
        return false;
    }
    let mic_active = check_consent_store_for_running(
        "Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\microphone",
        &exe_names,
        &hash_paths,
    );
    let cam_active = check_consent_store_for_running(
        "Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\webcam",
        &exe_names,
        &hash_paths,
    );
    mic_active || cam_active
}

#[cfg(windows)]
fn get_system_idle_ms() -> u32 {
    unsafe {
        let mut lii = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if GetLastInputInfo(&mut lii).as_bool() {
            let now = GetTickCount();
            return now.wrapping_sub(lii.dwTime);
        }
    }
    0
}

pub fn start_sensor_loops(app: AppHandle, state: Arc<AppSensorState>) {
    // Loop 1: Cursor Hit-Testing (~30Hz when collapsed) + Win+D Show Desktop protection
    let app_cursor = app.clone();
    let state_cursor = state.clone();
    std::thread::spawn(move || {
        let mut was_hovering = false;
        let mut was_ignoring = false;
        let mut last_dx = 0.0f64;
        let mut last_dy = 0.0f64;

        loop {
            if state_cursor.is_expanded.load(Ordering::Relaxed)
                || state_cursor.is_stealth.load(Ordering::Relaxed)
                || state_cursor.is_dragging.load(Ordering::Relaxed)
            {
                if state_cursor.is_dragging.load(Ordering::Relaxed) && was_ignoring {
                    if let Some(window) = app_cursor.get_webview_window("main") {
                        let _ = window.set_ignore_cursor_events(false);
                    }
                    was_ignoring = false;
                }
                if was_hovering {
                    was_hovering = false;
                    let _ = app_cursor.emit(
                        "buddy://peek",
                        PeekPayload {
                            hovering: false,
                            dx: 0.0,
                            dy: 0.0,
                        },
                    );
                }
                if !state_cursor.is_dragging.load(Ordering::Relaxed) {
                    was_ignoring = false;
                }
                std::thread::sleep(Duration::from_millis(60));
                continue;
            }

            #[cfg(windows)]
            if let Some(window) = app_cursor.get_webview_window("main") {
                if let Some(hwnd) = to_hwnd(&window) {
                    unsafe {
                        // Guard against Win+D / Show Desktop minimizing or lowering Buddy
                        if IsIconic(hwnd).as_bool() || !IsWindowVisible(hwnd).as_bool() {
                            show_window_no_activate(&window);
                        }

                        let mut pt = POINT::default();
                        let mut rect = RECT::default();
                        if GetCursorPos(&mut pt).is_ok() && GetWindowRect(hwnd, &mut rect).is_ok() {
                            let scale = window.scale_factor().unwrap_or(1.0);
                            let dock_edge = state_cursor.get_dock_edge();
                            let pill_w_log = state_cursor.pill_w.load(Ordering::Relaxed) as f64;
                            let pill_h_log = state_cursor.pill_h.load(Ordering::Relaxed) as f64;
                            let dims = compute_layout_dims(
                                scale,
                                Some((state_cursor.anchor_cx.load(Ordering::Relaxed), state_cursor.anchor_cy.load(Ordering::Relaxed))),
                            );
                            let (lx, ly, bw, bh) =
                                active_box_metrics(&dock_edge, dims, false, pill_w_log, pill_h_log);
                            let cx = rect.left as f64 + lx * scale;
                            let cy = rect.top as f64 + ly * scale;

                            let half_w = (bw * scale) / 2.0 + 6.0 * scale;
                            let half_h = (bh * scale) / 2.0 + 6.0 * scale;

                            let inside = (pt.x as f64) >= (cx - half_w)
                                && (pt.x as f64) <= (cx + half_w)
                                && (pt.y as f64) >= (cy - half_h)
                                && (pt.y as f64) <= (cy + half_h);

                            if inside != !was_ignoring {
                                let _ = window.set_ignore_cursor_events(!inside);
                                was_ignoring = !inside;
                            }

                            if inside {
                                let dx = (((pt.x as f64) - cx) / (18.0 * scale)).clamp(-8.0, 8.0);
                                let dy = (((pt.y as f64) - cy) / (18.0 * scale)).clamp(-8.0, 8.0);
                                if !was_hovering
                                    || (dx - last_dx).abs() > 0.15
                                    || (dy - last_dy).abs() > 0.15
                                {
                                    if !was_hovering {
                                        state_cursor.manual_sleep.store(false, Ordering::Relaxed);
                                        state_cursor.is_sleeping.store(false, Ordering::Relaxed);
                                    }
                                    was_hovering = true;
                                    last_dx = dx;
                                    last_dy = dy;
                                    let _ = app_cursor.emit(
                                        "buddy://peek",
                                        PeekPayload {
                                            hovering: true,
                                            dx,
                                            dy,
                                        },
                                    );
                                }
                            } else if was_hovering {
                                was_hovering = false;
                                let _ = app_cursor.emit(
                                    "buddy://peek",
                                    PeekPayload {
                                        hovering: false,
                                        dx: 0.0,
                                        dy: 0.0,
                                    },
                                );
                            }
                        }
                    }
                }
            }

            std::thread::sleep(Duration::from_millis(33));
        }
    });

    // Loop 2: Stealth (Games & Meetings), Win+D Topmost Guard, Sleep (GetLastInputInfo), Display Sync
    std::thread::spawn(move || {
        let own_pid = std::process::id();
        let mut clear_since: Option<Instant> = None;
        #[cfg(windows)]
        let mut last_work_area = get_primary_work_area();

        loop {
            #[cfg(windows)]
            {
                let ax = state.anchor_cx.load(Ordering::Relaxed);
                let ay = state.anchor_cy.load(Ordering::Relaxed);
                let cur_work_area = if state.has_anchor.load(Ordering::Relaxed) {
                    get_work_area_at(ax, ay)
                } else {
                    get_primary_work_area()
                };
                if cur_work_area != last_work_area && !state.is_dragging.load(Ordering::Relaxed) {
                    last_work_area = cur_work_area;
                    if let Some(win) = app.get_webview_window("main") {
                        apply_window_layout(&win, &state, state.is_expanded.load(Ordering::Relaxed));
                    }
                }

                // Re-assert topmost when user goes back to Windows Desktop (Win+D / Show Desktop)
                if !state.is_stealth.load(Ordering::Relaxed) {
                    let fg = unsafe { GetForegroundWindow() };
                    if !fg.0.is_null() {
                        let cls = get_window_class(fg);
                        if cls == "Progman" || cls == "WorkerW" {
                            if let Some(win) = app.get_webview_window("main") {
                                show_window_no_activate(&win);
                            }
                        }
                    }
                }

                // Stealth check (never hide mid-interaction if user has panel expanded)
                let auto_hide = state.auto_hide_enabled.load(Ordering::Relaxed);
                let is_expanded = state.is_expanded.load(Ordering::Relaxed);
                let (detected, reason) = if auto_hide && !is_expanded {
                    if is_fullscreen_or_game(own_pid) {
                        (true, "Fullscreen / Game".to_string())
                    } else if is_active_meeting() {
                        (true, "Video Meeting".to_string())
                    } else {
                        (false, String::new())
                    }
                } else {
                    (false, String::new())
                };

                let currently_stealth = state.is_stealth.load(Ordering::Relaxed);
                if detected {
                    clear_since = None;
                    if !currently_stealth {
                        state.is_stealth.store(true, Ordering::Relaxed);
                        if let Some(win) = app.get_webview_window("main") {
                            let _ = win.hide();
                        }
                        let _ = app.emit(
                            "buddy://stealth",
                            StealthPayload {
                                active: true,
                                reason,
                            },
                        );
                    }
                } else if currently_stealth {
                    let start = clear_since.get_or_insert_with(Instant::now);
                    if start.elapsed() >= Duration::from_millis(2500) {
                        clear_since = None;
                        state.is_stealth.store(false, Ordering::Relaxed);
                        if let Some(win) = app.get_webview_window("main") {
                            apply_window_layout(&win, &state, state.is_expanded.load(Ordering::Relaxed));
                            show_window_no_activate(&win);
                        }
                        let _ = app.emit(
                            "buddy://stealth",
                            StealthPayload {
                                active: false,
                                reason: String::new(),
                            },
                        );
                    }
                }

                // Sleep check (manual tray toggle OR 10 minutes idle via GetLastInputInfo; paused when Music is playing)
                let idle_ms = get_system_idle_ms();
                let manual = state.manual_sleep.load(Ordering::Relaxed);
                let should_sleep = (manual || idle_ms >= 10 * 60 * 1000)
                    && !is_expanded
                    && !state.is_stealth.load(Ordering::Relaxed)
                    && !state.is_music_playing.load(Ordering::Relaxed);
                let was_sleeping = state.is_sleeping.load(Ordering::Relaxed);
                if should_sleep != was_sleeping {
                    state.is_sleeping.store(should_sleep, Ordering::Relaxed);
                    let _ = app.emit(
                        "buddy://idle-sleep",
                        SleepPayload {
                            sleeping: should_sleep,
                        },
                    );
                }
            }

            std::thread::sleep(Duration::from_millis(450));
        }
    });
}
