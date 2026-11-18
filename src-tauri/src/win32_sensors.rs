//! Windows native sensors, fixed-canvas top-center layout, cursor hit-testing,
//! Win+D desktop recovery, and accurate stealth detection.

use crate::settings::SettingsManager;
use serde::Serialize;
use std::ffi::c_void;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize};

#[cfg(windows)]
use windows::core::PWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, HWND, POINT, RECT};
#[cfg(windows)]
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    MONITOR_DEFAULTTOPRIMARY,
};
#[cfg(windows)]
use windows::Win32::System::SystemInformation::GetTickCount;
#[cfg(windows)]
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetLastInputInfo, RegisterHotKey, UnregisterHotKey, LASTINPUTINFO, MOD_ALT, MOD_CONTROL,
    MOD_NOREPEAT,
};
#[cfg(windows)]
use windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_RUNNING_D3D_FULL_SCREEN};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetCursorPos, GetForegroundWindow, GetMessageW, GetWindowLongPtrW,
    GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
    IsWindowVisible, SetForegroundWindow, SetWindowDisplayAffinity, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, ShowWindowAsync, GWL_EXSTYLE, GWL_STYLE, HWND_BOTTOM, HWND_TOPMOST,
    MSG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE,
    WINDOW_DISPLAY_AFFINITY, WM_HOTKEY, WS_CAPTION, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT,
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

#[derive(Clone, Debug, Serialize)]
pub struct WindowDockAnchor {
    pub target_hwnd: isize,
    pub edge: String, // "top" | "bottom" | "left" | "right"
    pub ratio: f64,   // 0.0..=1.0 along edge (0.5 = center)
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
    /// Prevents auto-collapse when opening native modal dialogs (e.g. file/folder pickers).
    pub suspend_auto_collapse: AtomicBool,
    /// Only collapse on window blur when the user explicitly toggles Pin off (default: false = stay open until X/Esc/Ctrl+Alt+B).
    pub auto_collapse_on_blur: AtomicBool,
    pub last_expand_ms: std::sync::atomic::AtomicU64,
    /// Active visual pill dimensions inside the fixed transparent canvas (logical px)
    pub pill_w: std::sync::atomic::AtomicU32,
    pub pill_h: std::sync::atomic::AtomicU32,
    pub dock_edge: Mutex<String>,
    pub anchor_cx: AtomicI32,
    pub anchor_cy: AtomicI32,
    pub has_anchor: AtomicBool,
    pub drag_session: Mutex<Option<DragSession>>,
    pub window_dock: Mutex<Option<WindowDockAnchor>>,
    pub saved_desktop_dock: Mutex<Option<(String, i32, i32)>>,
    pub whisper_bounds: Mutex<Option<(f64, f64, f64, f64)>>,
    pub visibility_lock: Mutex<()>,
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
            suspend_auto_collapse: AtomicBool::new(false),
            auto_collapse_on_blur: AtomicBool::new(false),
            last_expand_ms: std::sync::atomic::AtomicU64::new(0),
            pill_w: std::sync::atomic::AtomicU32::new(152),
            pill_h: std::sync::atomic::AtomicU32::new(42),
            dock_edge: Mutex::new("top".to_string()),
            anchor_cx: AtomicI32::new(0),
            anchor_cy: AtomicI32::new(0),
            has_anchor: AtomicBool::new(false),
            drag_session: Mutex::new(None),
            window_dock: Mutex::new(None),
            saved_desktop_dock: Mutex::new(None),
            whisper_bounds: Mutex::new(None),
            visibility_lock: Mutex::new(()),
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

/// Excludes/includes a window from OS screen captures (Windows 10 2004+ WDA_EXCLUDEFROMCAPTURE).
#[cfg(windows)]
pub fn set_hwnd_capture_excluded(hwnd: HWND, exclude: bool) {
    unsafe {
        let affinity = WINDOW_DISPLAY_AFFINITY(if exclude { 0x11 } else { 0x00 });
        let _ = SetWindowDisplayAffinity(hwnd, affinity);
    }
}

pub fn set_main_capture_excluded(window: &tauri::WebviewWindow, exclude: bool) {
    #[cfg(windows)]
    if let Some(hwnd) = to_hwnd(window) {
        set_hwnd_capture_excluded(hwnd, exclude);
    }
}

/// Returns visible physical rectangle `(left, top, width, height)` of `hwnd` using DWM bounds.
#[cfg(windows)]
pub fn get_window_visible_rect(hwnd: HWND) -> Option<(i32, i32, i32, i32)> {
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            return None;
        }
        let mut rect = RECT::default();
        let dwm_ok = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut RECT as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        )
        .is_ok();
        if !dwm_ok && GetWindowRect(hwnd, &mut rect).is_err() {
            return None;
        }
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        if w <= 0 || h <= 0 {
            return None;
        }
        Some((rect.left, rect.top, w, h))
    }
}

pub fn attach_dock_to_window(
    app: &AppHandle,
    window: &tauri::WebviewWindow,
    state: &AppSensorState,
    target_hwnd: isize,
    edge: &str,
    ratio: f64,
) {
    if let Ok(mut saved) = state.saved_desktop_dock.lock() {
        if saved.is_none() {
            *saved = Some((
                state.get_dock_edge(),
                state.anchor_cx.load(Ordering::Relaxed),
                state.anchor_cy.load(Ordering::Relaxed),
            ));
        }
    }
    let clean_edge = match edge {
        "bottom" | "left" | "right" => edge.to_string(),
        _ => "top".to_string(),
    };
    if let Ok(mut wd) = state.window_dock.lock() {
        *wd = Some(WindowDockAnchor {
            target_hwnd,
            edge: clean_edge.clone(),
            ratio: ratio.clamp(0.05, 0.95),
        });
    }
    state.set_dock_edge(&clean_edge);
    apply_window_layout(window, state, state.is_expanded.load(Ordering::Relaxed));
    let _ = app.emit(
        "buddy://dock-changed",
        DockStatePayload {
            dock_edge: clean_edge,
            snap_preview: None,
        },
    );
}

pub fn detach_dock_from_window(
    app: &AppHandle,
    window: &tauri::WebviewWindow,
    state: &AppSensorState,
) {
    if let Ok(mut wd) = state.window_dock.lock() {
        *wd = None;
    }
    let restored = state.saved_desktop_dock.lock().ok().and_then(|mut g| g.take());
    if let Some((edge, cx, cy)) = restored {
        state.set_dock_edge(&edge);
        state.anchor_cx.store(cx, Ordering::Relaxed);
        state.anchor_cy.store(cy, Ordering::Relaxed);
        state.has_anchor.store(true, Ordering::Relaxed);
    }
    apply_window_layout(window, state, state.is_expanded.load(Ordering::Relaxed));
    let _ = app.emit(
        "buddy://dock-changed",
        DockStatePayload {
            dock_edge: state.get_dock_edge(),
            snap_preview: None,
        },
    );
}

#[cfg(windows)]
fn move_window_fast(window: &tauri::WebviewWindow, x: i32, y: i32) {
    if let Some(hwnd) = to_hwnd(window) {
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
            );
        }
    } else {
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
}

#[cfg(not(windows))]
fn move_window_fast(window: &tauri::WebviewWindow, x: i32, y: i32) {
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

/// Sizes and positions the transparent canvas cleanly for any dock edge (`top`, `bottom`, `left`, `right`, `free`),
/// or magnetically docked to a shared target window edge when `window_dock` is active.
pub fn apply_window_layout(
    window: &tauri::WebviewWindow,
    state: &AppSensorState,
    expanded: bool,
) -> LayoutDims {
    let scale = window.scale_factor().unwrap_or(1.0);
    let edge_gap = (DOCK_EDGE_GAP_LOGICAL * scale).round() as i32;

    #[cfg(windows)]
    {
        let wd_opt = state.window_dock.lock().ok().and_then(|g| g.clone());
        if let Some(anchor) = wd_opt {
            let target_hwnd = HWND(anchor.target_hwnd as *mut c_void);
            if unsafe { IsWindow(target_hwnd).as_bool() } {
                if unsafe { IsIconic(target_hwnd).as_bool() } {
                    // Target window is minimized: park Buddy cleanly at Top Center of the screen to wait!
                    let prev_edge = state.get_dock_edge();
                    if prev_edge != "top" {
                        state.set_dock_edge("top");
                    }
                    let ax = state.anchor_cx.load(Ordering::Relaxed);
                    let ay = state.anchor_cy.load(Ordering::Relaxed);
                    let (wx, wy, ww, _) = if state.has_anchor.load(Ordering::Relaxed) {
                        get_work_area_at(ax, ay)
                    } else {
                        get_primary_work_area()
                    };
                    let cx = wx + ww / 2;
                    let cy = wy + edge_gap + (21.0 * scale).round() as i32;
                    let dims = compute_layout_dims(scale, Some((cx, cy)));
                    let phys_w = (dims.canvas_w * scale).round() as i32;
                    let phys_h = (dims.canvas_h * scale).round() as i32;
                    let pad_px = (dims.pad * scale).round() as i32;

                    let cur_size = window.outer_size().ok();
                    if cur_size
                        .map(|s| s.width as i32 != phys_w || s.height as i32 != phys_h)
                        .unwrap_or(true)
                    {
                        let _ = window.set_size(PhysicalSize::new(phys_w as u32, phys_h as u32));
                    }
                    state.anchor_cx.store(cx, Ordering::Relaxed);
                    state.anchor_cy.store(cy, Ordering::Relaxed);
                    state.has_anchor.store(true, Ordering::Relaxed);
                    move_window_fast(window, cx - phys_w / 2, wy - pad_px + edge_gap);
                    return dims;
                }

                if let Some((tx, ty, tw, th)) = get_window_visible_rect(target_hwnd) {
                    let prev_edge = state.get_dock_edge();
                    let dock_edge = anchor.edge.clone();
                    if prev_edge != dock_edge {
                        state.set_dock_edge(&dock_edge);
                    }
                    let dims = compute_layout_dims(scale, Some((tx + tw / 2, ty + th / 2)));
                    let phys_w = (dims.canvas_w * scale).round() as i32;
                    let phys_h = (dims.canvas_h * scale).round() as i32;
                    let pad_px = (dims.pad * scale).round() as i32;

                    let cur_size = window.outer_size().ok();
                    if cur_size
                        .map(|s| s.width as i32 != phys_w || s.height as i32 != phys_h)
                        .unwrap_or(true)
                    {
                        let _ = window.set_size(PhysicalSize::new(phys_w as u32, phys_h as u32));
                    }

                    let (wx, wy, ww, wh) = get_work_area_at(tx + tw / 2, ty + th / 2);
                    let pill_w_log = state.pill_w.load(Ordering::Relaxed) as f64;
                    let pill_h_log = state.pill_h.load(Ordering::Relaxed) as f64;
                    let side_docked = matches!(dock_edge.as_str(), "left" | "right");
                    let col_w_log = if side_docked { 42.0 } else { pill_w_log.max(152.0) };
                    let col_h_log = if side_docked { pill_h_log.max(152.0) } else { 42.0 };
                    let col_w_px = (col_w_log * scale).round() as i32;
                    let col_h_px = (col_h_log * scale).round() as i32;

                    let (_, _, box_w, box_h) =
                        active_box_metrics(&dock_edge, dims, expanded, col_w_log, col_h_log);
                    let bw_phys = (box_w * scale).round() as i32;
                    let bh_phys = (box_h * scale).round() as i32;
                    let outside_gap = (4.0 * scale).round() as i32;

                    let (win_x, win_y, pill_cx, pill_cy) = match dock_edge.as_str() {
                        "bottom" => {
                            let margin = col_w_px / 2 + (10.0 * scale).round() as i32;
                            let span_min = (tx + margin).min(tx + tw / 2);
                            let span_max = (tx + tw - margin).max(span_min);
                            let raw_cx = span_min
                                + ((span_max - span_min) as f64 * anchor.ratio).round() as i32;
                            let min_cx = wx + bw_phys / 2 + edge_gap;
                            let max_cx = (wx + ww - bw_phys / 2 - edge_gap).max(min_cx);
                            let cx = raw_cx.clamp(min_cx, max_cx);

                            let desired_bottom = if (wy + wh) - (ty + th) >= col_h_px + edge_gap + 4 {
                                ty + th + col_h_px + outside_gap
                            } else {
                                (ty + th).min(wy + wh) - edge_gap
                            };
                            let min_bottom = wy + bh_phys + edge_gap;
                            let max_bottom = (wy + wh - edge_gap).max(min_bottom);
                            let pill_bottom = desired_bottom.clamp(min_bottom, max_bottom);
                            (
                                cx - phys_w / 2,
                                pill_bottom + pad_px - phys_h,
                                cx,
                                pill_bottom - col_h_px / 2,
                            )
                        }
                        "left" => {
                            let margin = col_h_px / 2 + (10.0 * scale).round() as i32;
                            let span_min = (ty + margin).min(ty + th / 2);
                            let span_max = (ty + th - margin).max(span_min);
                            let raw_cy = span_min
                                + ((span_max - span_min) as f64 * anchor.ratio).round() as i32;
                            let min_cy = wy + bh_phys / 2 + edge_gap;
                            let max_cy = (wy + wh - bh_phys / 2 - edge_gap).max(min_cy);
                            let cy = raw_cy.clamp(min_cy, max_cy);

                            let desired_left = if tx - wx >= col_w_px + edge_gap + 4 {
                                tx - col_w_px - outside_gap
                            } else {
                                tx.max(wx) + edge_gap
                            };
                            let min_left = wx + edge_gap;
                            let max_left = (wx + ww - bw_phys - edge_gap).max(min_left);
                            let pill_left = desired_left.clamp(min_left, max_left);
                            (
                                pill_left - pad_px,
                                cy - phys_h / 2,
                                pill_left + col_w_px / 2,
                                cy,
                            )
                        }
                        "right" => {
                            let margin = col_h_px / 2 + (10.0 * scale).round() as i32;
                            let span_min = (ty + margin).min(ty + th / 2);
                            let span_max = (ty + th - margin).max(span_min);
                            let raw_cy = span_min
                                + ((span_max - span_min) as f64 * anchor.ratio).round() as i32;
                            let min_cy = wy + bh_phys / 2 + edge_gap;
                            let max_cy = (wy + wh - bh_phys / 2 - edge_gap).max(min_cy);
                            let cy = raw_cy.clamp(min_cy, max_cy);

                            let desired_right = if (wx + ww) - (tx + tw) >= col_w_px + edge_gap + 4 {
                                tx + tw + col_w_px + outside_gap
                            } else {
                                (tx + tw).min(wx + ww) - edge_gap
                            };
                            let min_right = wx + bw_phys + edge_gap;
                            let max_right = (wx + ww - edge_gap).max(min_right);
                            let pill_right = desired_right.clamp(min_right, max_right);
                            (
                                pill_right + pad_px - phys_w,
                                cy - phys_h / 2,
                                pill_right - col_w_px / 2,
                                cy,
                            )
                        }
                        _ => {
                            // "top" (default center, or top-left / top-right / custom ratio)
                            let margin = col_w_px / 2 + (10.0 * scale).round() as i32;
                            let span_min = (tx + margin).min(tx + tw / 2);
                            let span_max = (tx + tw - margin).max(span_min);
                            let raw_cx = span_min
                                + ((span_max - span_min) as f64 * anchor.ratio).round() as i32;
                            let min_cx = wx + bw_phys / 2 + edge_gap;
                            let max_cx = (wx + ww - bw_phys / 2 - edge_gap).max(min_cx);
                            let cx = raw_cx.clamp(min_cx, max_cx);

                            let desired_top = if ty - wy >= col_h_px + edge_gap + 4 {
                                ty - col_h_px - outside_gap
                            } else {
                                ty.max(wy) + edge_gap
                            };
                            let min_top = wy + edge_gap;
                            let max_top = (wy + wh - bh_phys - edge_gap).max(min_top);
                            let pill_top = desired_top.clamp(min_top, max_top);
                            (
                                cx - phys_w / 2,
                                pill_top - pad_px,
                                cx,
                                pill_top + col_h_px / 2,
                            )
                        }
                    };

                    state.anchor_cx.store(pill_cx, Ordering::Relaxed);
                    state.anchor_cy.store(pill_cy, Ordering::Relaxed);
                    state.has_anchor.store(true, Ordering::Relaxed);
                    move_window_fast(window, win_x, win_y);
                    return dims;
                }
            }
        }
    }

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

#[cfg(windows)]
fn snap_point_to_target_window(
    pt_x: i32,
    pt_y: i32,
    target_hwnd: isize,
    scale: f64,
) -> Option<(String, f64)> {
    let hwnd = HWND(target_hwnd as *mut c_void);
    let (tx, ty, tw, th) = get_window_visible_rect(hwnd)?;
    let d_top = (pt_y - ty).abs();
    let d_bottom = (pt_y - (ty + th)).abs();
    let d_left = (pt_x - tx).abs();
    let d_right = (pt_x - (tx + tw)).abs();

    let (edge, raw_ratio, span_px) = [
        (
            "top",
            d_top,
            (pt_x - tx) as f64 / (tw.max(1) as f64),
            tw as f64,
        ),
        (
            "bottom",
            d_bottom,
            (pt_x - tx) as f64 / (tw.max(1) as f64),
            tw as f64,
        ),
        (
            "left",
            d_left,
            (pt_y - ty) as f64 / (th.max(1) as f64),
            th as f64,
        ),
        (
            "right",
            d_right,
            (pt_y - ty) as f64 / (th.max(1) as f64),
            th as f64,
        ),
    ]
    .into_iter()
    .min_by_key(|item| item.1)
    .map(|(e, _, r, s)| (e.to_string(), r.clamp(0.05, 0.95), s))?;

    let mut ratio = raw_ratio;
    let detent_tol = ((20.0 * scale) / span_px.max(1.0)).min(0.05);
    for anchor_r in [0.15, 0.50, 0.85] {
        if (ratio - anchor_r).abs() <= detent_tol {
            ratio = anchor_r;
            break;
        }
    }
    Some((edge, ratio))
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

                let wd_target = state
                    .window_dock
                    .lock()
                    .ok()
                    .and_then(|g| g.as_ref().map(|w| w.target_hwnd));
                let preview = if let Some(thwnd) = wd_target {
                    snap_point_to_target_window(pt.x, pt.y, thwnd, scale).map(|(e, _)| e)
                } else {
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
                };
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

            // If docked magnetically to a shared target window, snap cleanly onto any of its 4 edges with exact ratio
            let wd_target = state
                .window_dock
                .lock()
                .ok()
                .and_then(|g| g.as_ref().map(|w| w.target_hwnd));
            if let Some(thwnd) = wd_target {
                if let Some((edge, ratio)) = snap_point_to_target_window(pt.x, pt.y, thwnd, scale) {
                    let chosen_edge = if !sess.detached {
                        // Keep sliding edge and compute ratio along that edge
                        if let Some((tx, ty, tw, th)) =
                            get_window_visible_rect(HWND(thwnd as *mut c_void))
                        {
                            let raw_r = match sess.initial_edge.as_str() {
                                "left" | "right" => (pt.y - ty) as f64 / (th.max(1) as f64),
                                _ => (pt.x - tx) as f64 / (tw.max(1) as f64),
                            }
                            .clamp(0.05, 0.95);
                            let mut r = raw_r;
                            let span = match sess.initial_edge.as_str() {
                                "left" | "right" => th as f64,
                                _ => tw as f64,
                            };
                            let tol = ((20.0 * scale) / span.max(1.0)).min(0.05);
                            for anchor_r in [0.15, 0.50, 0.85] {
                                if (r - anchor_r).abs() <= tol {
                                    r = anchor_r;
                                    break;
                                }
                            }
                            if let Ok(mut g) = state.window_dock.lock() {
                                *g = Some(WindowDockAnchor {
                                    target_hwnd: thwnd,
                                    edge: sess.initial_edge.clone(),
                                    ratio: r,
                                });
                            }
                        }
                        sess.initial_edge
                    } else {
                        if let Ok(mut g) = state.window_dock.lock() {
                            *g = Some(WindowDockAnchor {
                                target_hwnd: thwnd,
                                edge: edge.clone(),
                                ratio,
                            });
                        }
                        edge
                    };
                    state.set_dock_edge(&chosen_edge);
                    apply_window_layout(window, state, sess.expanded);
                    let payload = DockStatePayload {
                        dock_edge: chosen_edge,
                        snap_preview: None,
                    };
                    let _ = app.emit("buddy://dock-changed", payload.clone());
                    return payload;
                }
            }

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

/// Toggle `WS_EX_TRANSPARENT | WS_EX_LAYERED` directly on the HWND so OS click-through works
/// outside the capsule without `tao` running `SetWindowLongW(GWL_STYLE)` + `SWP_FRAMECHANGED` + `ShowWindow(SW_SHOW)`.
#[cfg(windows)]
fn set_click_through_fast(window: &tauri::WebviewWindow, ignore: bool) {
    if let Some(hwnd) = to_hwnd(window) {
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let flag = (WS_EX_TRANSPARENT.0 | WS_EX_LAYERED.0) as isize;
            let new_style = if ignore { style | flag } else { style & !flag };
            if new_style != style {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_style);
            }
        }
    } else {
        let _ = window.set_ignore_cursor_events(ignore);
    }
}

#[cfg(not(windows))]
fn set_click_through_fast(window: &tauri::WebviewWindow, ignore: bool) {
    let _ = window.set_ignore_cursor_events(ignore);
}

/// Updates focus/cursor pass-through state instantaneously without resizing or re-framing the OS window.
pub fn apply_interaction_mode(window: &tauri::WebviewWindow, expanded: bool) {
    if expanded {
        set_no_activate(window, false);
        set_click_through_fast(window, false);
        #[cfg(windows)]
        if let Some(hwnd) = to_hwnd(window) {
            unsafe {
                let _ = SetForegroundWindow(hwnd);
            }
        }
        let _ = window.set_focus();
    } else {
        set_no_activate(window, true);
        set_click_through_fast(window, true);
    }
}

/// Show window without activating foreground focus and assert HWND_TOPMOST.
#[cfg(windows)]
pub fn show_window_no_activate(window: &tauri::WebviewWindow) {
    if let Some(hwnd) = to_hwnd(window) {
        unsafe {
            if !IsWindowVisible(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
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

#[cfg(windows)]
fn is_window_topmost_and_visible(window: &tauri::WebviewWindow) -> bool {
    if let Some(hwnd) = to_hwnd(window) {
        unsafe {
            let visible = IsWindowVisible(hwnd).as_bool();
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let topmost = (ex & (WS_EX_TOPMOST.0 as isize)) != 0;
            return visible && topmost;
        }
    }
    true
}

#[cfg(not(windows))]
pub fn show_window_no_activate(window: &tauri::WebviewWindow) {
    let _ = window.show();
}

/// Immediately drop Z-order below all windows (`HWND_BOTTOM`) and hide via direct Win32 without triggering `tao` re-frame.
#[cfg(windows)]
fn demote_and_hide_window(window: &tauri::WebviewWindow) {
    if let Some(hwnd) = to_hwnd(window) {
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                HWND_BOTTOM,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            let _ = ShowWindowAsync(hwnd, SW_HIDE);
        }
    } else {
        let _ = window.hide();
    }
}

#[cfg(windows)]
fn restore_topmost_and_show(window: &tauri::WebviewWindow, state: &AppSensorState) {
    apply_window_layout(window, state, state.is_expanded.load(Ordering::Relaxed));
    show_window_no_activate(window);
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
fn get_window_title_lower(hwnd: HWND) -> String {
    unsafe {
        let mut buf = [0u16; 128];
        let len = GetWindowTextW(hwnd, &mut buf);
        if len > 0 {
            return String::from_utf16_lossy(&buf[..len as usize]).to_ascii_lowercase();
        }
    }
    String::new()
}

#[cfg(windows)]
fn query_process_image_path_lower(pid: u32) -> Option<String> {
    unsafe {
        let proc_handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut path_buf = [0u16; 512];
        let mut len = path_buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            proc_handle,
            PROCESS_NAME_WIN32,
            PWSTR(path_buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(proc_handle);
        if ok && len > 0 {
            return Some(String::from_utf16_lossy(&path_buf[..len as usize]).to_ascii_lowercase());
        }
    }
    None
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

fn is_non_game_window_class(win_class: &str) -> bool {
    matches!(
        win_class,
        "Chrome_WidgetWin_0"
            | "Chrome_WidgetWin_1"
            | "Chrome_RenderWidgetHostHWND"
            | "MozillaWindowClass"
            | "MozillaDialogClass"
            | "ApplicationFrameWindow"
            | "CASCADIA_HOSTING_WINDOW_CLASS"
            | "ConsoleWindowClass"
            | "VirtualConsoleClass"
            | "CabinetWClass"
            | "ExploreWClass"
            | "Electron"
    ) || (win_class.starts_with("Qt") && win_class.ends_with("QWindowIcon"))
        || win_class.starts_with("HwndWrapper[")
}

fn is_known_game_class(cls: &str) -> bool {
    matches!(
        cls,
        "UnityWndClass"
            | "UnrealWindow"
            | "RiotWindowClass"
            | "Valve001"
            | "TslGame"
            | "Respawn001"
            | "CryENGINE"
            | "Frostbite"
            | "LaunchUnrealUWindowsClient"
            | "grcWindow"
            | "sgaWinClass"
            | "LWJGL"
    )
}

fn is_non_game_exe(exe_name: &str) -> bool {
    const NON_GAME_EXES: &[&str] = &[
        "chrome.exe",
        "chrome_proxy.exe",
        "msedge.exe",
        "msedge_proxy.exe",
        "msedgewebview2.exe",
        "firefox.exe",
        "zen.exe",
        "floorp.exe",
        "librewolf.exe",
        "waterfox.exe",
        "thorium.exe",
        "brave.exe",
        "opera.exe",
        "opera_gx.exe",
        "vivaldi.exe",
        "arc.exe",
        "comet.exe",
        "yandex.exe",
        "quark.exe",
        "360se.exe",
        "360chrome.exe",
        "qqbrowser.exe",
        "sogouexplorer.exe",
        "2345explorer.exe",
        "liebao.exe",
        "centbrowser.exe",
        "code.exe",
        "devenv.exe",
        "idea64.exe",
        "clion64.exe",
        "pycharm64.exe",
        "goland64.exe",
        "webstorm64.exe",
        "rider64.exe",
        "cursor.exe",
        "windsurf.exe",
        "antigravity.exe",
        "trae.exe",
        "kiro.exe",
        "zed.exe",
        "sublime_text.exe",
        "subl.exe",
        "explorer.exe",
        "applicationframehost.exe",
        "notepad.exe",
        "notepad++.exe",
        "obsidian.exe",
        "notion.exe",
        "typora.exe",
        "logseq.exe",
        "siyuan.exe",
        "wps.exe",
        "et.exe",
        "wpp.exe",
        "winword.exe",
        "excel.exe",
        "powerpnt.exe",
        "acrord32.exe",
        "acrobat.exe",
        "sumatrapdf.exe",
        "foxitpdfreader.exe",
        "foxitreader.exe",
        "vlc.exe",
        "potplayer.exe",
        "potplayermini64.exe",
        "potplayermini.exe",
        "mpv.exe",
        "mpv.net.exe",
        "iina.exe",
        "mpc-hc64.exe",
        "mpc-be64.exe",
        "kodi.exe",
        "plex.exe",
        "plexhtpc.exe",
        "jellyfinmediaplayer.exe",
        "bilibili.exe",
        "douyin.exe",
        "qqlive.exe",
        "tencentvideo.exe",
        "iqiyi.exe",
        "qiyi.exe",
        "youku.exe",
        "qq.exe",
        "wechat.exe",
        "weixin.exe",
        "tim.exe",
        "feishu.exe",
        "lark.exe",
        "dingtalk.exe",
        "wemeetapp.exe",
        "zoom.exe",
        "teams.exe",
        "ms-teams.exe",
        "discord.exe",
        "spotify.exe",
        "cloudmusic.exe",
        "qqmusic.exe",
        "kugou.exe",
        "kwmusic.exe",
        "windowsterminal.exe",
        "powershell.exe",
        "pwsh.exe",
        "cmd.exe",
        "conhost.exe",
        "taskmgr.exe",
        "buddy.exe",
        "mstsc.exe",
        "parsec.exe",
        "moonlight.exe",
        "sunshine.exe",
        "todesk.exe",
        "sunloginclient.exe",
        "anydesk.exe",
        "teamviewer.exe",
        "rustdesk.exe",
        "snipaste.exe",
        "pixpin.exe",
        "sharex.exe",
        // Launchers, live wallpapers, overlays, and streaming tools (not games themselves)
        "steam.exe",
        "steamwebhelper.exe",
        "epicgameslauncher.exe",
        "epicwebhelper.exe",
        "riotclientservices.exe",
        "riotclientux.exe",
        "wegame.exe",
        "hoyoplay.exe",
        "launcher.exe",
        "battlenet.exe",
        "uu.exe",
        "leigod.exe",
        "wallpaper32.exe",
        "wallpaper64.exe",
        "wallpaperui.exe",
        "ui32.exe",
        "webwallpaper32.exe",
        "losslessscaling.exe",
        "obs64.exe",
        "obs32.exe",
        "ciscocollabhost.exe",
        "gamebar.exe",
        "nvidia overlay.exe",
        "nvidiashare.exe",
    ];
    NON_GAME_EXES.contains(&exe_name)
}

fn is_known_game_signature(exe_path_lower: &str, exe_name: &str, cls: &str, title_lower: &str) -> bool {
    if is_known_game_class(cls) {
        return true;
    }

    // Ignore updaters/uninstallers/crash reporters sitting inside game folders.
    if exe_name.starts_with("unins")
        || exe_name.contains("crashhandler")
        || exe_name.contains("crashpad")
        || exe_name.contains("crashreport")
    {
        return false;
    }

    const PATH_FRAGMENTS: &[&str] = &[
        "\\steamapps\\common\\",
        "\\steamlibrary\\",
        "\\epic games\\",
        "\\riot games\\",
        "\\wegameapps\\",
        "\\xboxgames\\",
        "\\ubisoft game launcher\\games\\",
        "\\ea games\\",
        "\\origin games\\",
        "\\battle.net\\",
        "\\hoyoverse\\",
        "\\hoyoplay\\games\\",
        "\\mihoyo\\",
        "\\genshin impact\\",
        "\\kuro games\\",
        "\\rockstar games\\",
        "\\gog galaxy\\games\\",
    ];
    if PATH_FRAGMENTS.iter().any(|f| exe_path_lower.contains(f)) {
        return true;
    }

    if exe_name.ends_with("-win64-shipping.exe") || exe_name.ends_with("-win32-shipping.exe") {
        return true;
    }
    if (exe_name == "javaw.exe" || exe_name == "java.exe") && title_lower.contains("minecraft") {
        return true;
    }

    const GAME_EXES: &[&str] = &[
        "valorant-win64-shipping.exe",
        "league of legends.exe",
        "yuanshen.exe",
        "genshinimpact.exe",
        "starrail.exe",
        "zenlesszonezero.exe",
        "bh3.exe",
        "tslgame.exe",
        "cs2.exe",
        "csgo.exe",
        "dota2.exe",
        "overwatch.exe",
        "r5apex.exe",
        "eldenring.exe",
        "sekiro.exe",
        "cyberpunk2077.exe",
        "gtav.exe",
        "gta5.exe",
        "rdr2.exe",
        "helldivers2.exe",
        "fc24.exe",
        "fc25.exe",
        "minecraft.windows.exe",
        "robloxplayerbeta.exe",
        "deltaforceclient-win64-shipping.exe",
        "narakabladepoint.exe",
        "naraka.exe",
        "b1-win64-shipping.exe",
        "client-win64-shipping.exe",
        "palworld-win64-shipping.exe",
        "marvel-win64-shipping.exe",
        "wow.exe",
        "wowclassic.exe",
        "ffxiv_dx11.exe",
        "diablo iv.exe",
        "hearthstone.exe",
        "starcraft2.exe",
        "starcraft2_x64.exe",
        "crossfire.exe",
        "dnf.exe",
        "pathofexile.exe",
        "pathofexile_x64.exe",
        "pathofexile2.exe",
        "bg3.exe",
        "bg3_dx11.exe",
        "witcher3.exe",
        "mhworld.exe",
        "monsterhunterwilds.exe",
    ];
    GAME_EXES.contains(&exe_name)
}

/// Distinguishes real games (known engines/paths/exes, anti-cheat protected game classes, or
/// genuine D3D exclusive fullscreen non-app windows) from normal fullscreen browsers, editors, or players.
fn classify_foreground_game(
    exe_path_opt: Option<&str>,
    win_class: &str,
    title_lower: &str,
    covers_screen: bool,
    is_d3d_exclusive: bool,
    has_caption: bool,
    large_window: bool,
) -> bool {
    if is_shell_or_desktop_class(win_class) || is_non_game_window_class(win_class) {
        return false;
    }

    let Some(raw_path) = exe_path_opt else {
        // Protected kernel anti-cheat games (ACE/Vanguard/EAC) can deny OpenProcess:
        // match known game classes or true D3D exclusive fullscreen (never plain borderless desktop apps).
        if is_known_game_class(win_class) {
            return large_window || covers_screen || is_d3d_exclusive;
        }
        return is_d3d_exclusive && covers_screen && !has_caption;
    };

    let path_lower = raw_path.to_ascii_lowercase();
    let exe_name = Path::new(&path_lower)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    if is_non_game_exe(exe_name) {
        return false;
    }

    if is_known_game_signature(&path_lower, exe_name, win_class, title_lower) {
        return large_window || covers_screen || is_d3d_exclusive;
    }

    // Unknown app: hide ONLY for genuine Direct3D exclusive fullscreen + captionless monitor coverage,
    // NEVER for ordinary F11/borderless desktop apps, video players, browsers, or UWP apps.
    is_d3d_exclusive && covers_screen && !has_caption
}

#[cfg(windows)]
fn detect_active_game(own_pid: u32) -> Option<String> {
    unsafe {
        let d3d_exclusive = SHQueryUserNotificationState()
            .map(|s| s == QUNS_RUNNING_D3D_FULL_SCREEN)
            .unwrap_or(false);

        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return None;
        }
        let mut fg_pid: u32 = 0;
        GetWindowThreadProcessId(fg, Some(&mut fg_pid));
        if fg_pid == own_pid || fg_pid == 0 {
            return None;
        }

        let cls = get_window_class(fg);
        if is_shell_or_desktop_class(&cls) || is_non_game_window_class(&cls) {
            return None;
        }

        let mut fg_rect = RECT::default();
        if GetWindowRect(fg, &mut fg_rect).is_err() {
            return None;
        }
        let w = fg_rect.right - fg_rect.left;
        let h = fg_rect.bottom - fg_rect.top;
        let large_window = w >= 640 && h >= 480;

        let mut covers_screen = false;
        let hmon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmon, &mut mi).as_bool() {
            let m = mi.rcMonitor;
            covers_screen = fg_rect.left <= m.left + 3
                && fg_rect.top <= m.top + 3
                && fg_rect.right >= m.right - 3
                && fg_rect.bottom >= m.bottom - 3;
        }

        let style = GetWindowLongPtrW(fg, GWL_STYLE) as u32;
        let has_caption = (style & WS_CAPTION.0) == WS_CAPTION.0;
        let exe_path = query_process_image_path_lower(fg_pid);
        let title_lower = get_window_title_lower(fg);

        if classify_foreground_game(
            exe_path.as_deref(),
            &cls,
            &title_lower,
            covers_screen,
            d3d_exclusive,
            has_caption,
            large_window,
        ) {
            let exe_label = exe_path
                .as_deref()
                .and_then(|p| Path::new(p).file_name().and_then(|n| n.to_str()))
                .unwrap_or("<denied>");
            return Some(format!(
                "exe={}, class={}, covers={}, d3d={}, caption={}",
                exe_label, cls, covers_screen, d3d_exclusive, has_caption
            ));
        }
        None
    }
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
        #[cfg(windows)]
        let mut last_tracked_rect: Option<(i32, i32, i32, i32)> = None;
        #[cfg(windows)]
        let mut last_was_iconic = false;
        #[cfg(windows)]
        let mut last_motion_at: Option<Instant> = None;

        // Dedicated 144Hz adaptive tracking helper thread when magnetically attached to a shared window
        let app_track = app_cursor.clone();
        let state_track = state_cursor.clone();
        std::thread::spawn(move || loop {
            #[cfg(windows)]
            {
                let mut moving_recently = false;
                if !state_track.is_dragging.load(Ordering::Relaxed)
                    && !state_track.is_stealth.load(Ordering::Relaxed)
                {
                    let wd_anchor = state_track.window_dock.lock().ok().and_then(|g| g.clone());
                    if let Some(anchor) = wd_anchor {
                        let h = HWND(anchor.target_hwnd as *mut c_void);
                        if unsafe { IsWindow(h).as_bool() } {
                            let is_min = unsafe { IsIconic(h).as_bool() };
                            if is_min != last_was_iconic {
                                last_was_iconic = is_min;
                                last_tracked_rect = None;
                                last_motion_at = Some(Instant::now());
                                if let Some(window) = app_track.get_webview_window("main") {
                                    apply_window_layout(
                                        &window,
                                        &state_track,
                                        state_track.is_expanded.load(Ordering::Relaxed),
                                    );
                                    let _ = app_track.emit(
                                        "buddy://dock-changed",
                                        DockStatePayload {
                                            dock_edge: state_track.get_dock_edge(),
                                            snap_preview: None,
                                        },
                                    );
                                }
                            } else if !is_min {
                                if let Some(cur_r) = get_window_visible_rect(h) {
                                    if last_tracked_rect != Some(cur_r) {
                                        last_tracked_rect = Some(cur_r);
                                        last_motion_at = Some(Instant::now());
                                        if let Some(window) = app_track.get_webview_window("main") {
                                            apply_window_layout(
                                                &window,
                                                &state_track,
                                                state_track.is_expanded.load(Ordering::Relaxed),
                                            );
                                        }
                                    }
                                }
                            }
                            if let Some(m) = last_motion_at {
                                moving_recently = m.elapsed() < Duration::from_millis(260);
                            }
                            std::thread::sleep(Duration::from_millis(if moving_recently { 7 } else { 25 }));
                            continue;
                        }
                    } else {
                        last_tracked_rect = None;
                        last_was_iconic = false;
                    }
                }
                let _ = moving_recently;
            }
            std::thread::sleep(Duration::from_millis(45));
        });

        loop {
            if state_cursor.is_expanded.load(Ordering::Relaxed)
                || state_cursor.is_stealth.load(Ordering::Relaxed)
                || state_cursor.is_dragging.load(Ordering::Relaxed)
            {
                if state_cursor.is_dragging.load(Ordering::Relaxed) && was_ignoring {
                    if let Some(window) = app_cursor.get_webview_window("main") {
                        set_click_through_fast(&window, false);
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
                        // Guard against Win+D / Show Desktop minimizing or lowering Buddy (serialized against stealth hide)
                        let stealth_now = if let Ok(_vis_guard) = state_cursor.visibility_lock.lock() {
                            if state_cursor.is_stealth.load(Ordering::Acquire) {
                                true
                            } else {
                                if IsIconic(hwnd).as_bool() || !IsWindowVisible(hwnd).as_bool() {
                                    show_window_no_activate(&window);
                                }
                                false
                            }
                        } else {
                            state_cursor.is_stealth.load(Ordering::Acquire)
                        };

                        let mut pt = POINT::default();
                        let mut rect = RECT::default();
                        if !stealth_now && GetCursorPos(&mut pt).is_ok() && GetWindowRect(hwnd, &mut rect).is_ok() {
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

                            let half_w = (bw * scale) / 2.0 + 2.0 * scale;
                            let half_h = (bh * scale) / 2.0 + 2.0 * scale;

                            let in_pill = (pt.x as f64) >= (cx - half_w)
                                && (pt.x as f64) <= (cx + half_w)
                                && (pt.y as f64) >= (cy - half_h)
                                && (pt.y as f64) <= (cy + half_h);

                            let in_whisper = state_cursor
                                .whisper_bounds
                                .lock()
                                .ok()
                                .and_then(|g| *g)
                                .map(|(wx_log, wy_log, ww_log, wh_log)| {
                                    let rx0 = rect.left as f64 + wx_log * scale;
                                    let ry0 = rect.top as f64 + wy_log * scale;
                                    let rx1 = rx0 + ww_log * scale;
                                    let ry1 = ry0 + wh_log * scale;
                                    (pt.x as f64) >= rx0
                                        && (pt.x as f64) <= rx1
                                        && (pt.y as f64) >= ry0
                                        && (pt.y as f64) <= ry1
                                })
                                .unwrap_or(false);

                            let inside = in_pill || in_whisper;

                            if inside != !was_ignoring {
                                set_click_through_fast(&window, !inside);
                                was_ignoring = !inside;
                            }

                            if in_pill {
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

    // Loop 2: Stealth (Games Only), Non-Game Always-On-Top Guard, Sleep (GetLastInputInfo), Display Sync
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

                // Always keep Buddy visible & HWND_TOPMOST in all non-game situations (including Win+D Desktop)
                if !state.is_stealth.load(Ordering::Acquire) {
                    if let Some(win) = app.get_webview_window("main") {
                        let fg = unsafe { GetForegroundWindow() };
                        let on_desktop = !fg.0.is_null() && {
                            let cls = get_window_class(fg);
                            cls == "Progman" || cls == "WorkerW"
                        };
                        if on_desktop || !is_window_topmost_and_visible(&win) {
                            if let Ok(_vis_guard) = state.visibility_lock.lock() {
                                if !state.is_stealth.load(Ordering::Acquire) {
                                    show_window_no_activate(&win);
                                }
                            }
                        }
                    }
                }

                // Stealth check: drop to bottom & hide ONLY for active games (never meetings, launchers, browsers, etc.)
                let auto_hide = state.auto_hide_enabled.load(Ordering::Relaxed);
                let mut is_expanded = state.is_expanded.load(Ordering::Relaxed);
                let (detected_detail, reason) = if auto_hide && !state.is_dragging.load(Ordering::Relaxed) {
                    if let Some(detail) = detect_active_game(own_pid) {
                        if is_expanded {
                            state.is_expanded.store(false, Ordering::Relaxed);
                            is_expanded = false;
                            let _ = app.emit("buddy://collapse", ());
                        }
                        (Some(detail), "Game Active".to_string())
                    } else {
                        (None, String::new())
                    }
                } else {
                    (None, String::new())
                };

                if let Some(detail) = detected_detail {
                    if let Ok(_vis_guard) = state.visibility_lock.lock() {
                        let currently_stealth = state.is_stealth.load(Ordering::Acquire);
                        clear_since = None;
                        if !currently_stealth {
                            state.is_stealth.store(true, Ordering::Release);
                            crate::logger::log_info(
                                "stealth",
                                &format!(
                                    "game detected ({}) -> dropping Buddy to bottom and hiding",
                                    detail
                                ),
                            );
                            if let Some(win) = app.get_webview_window("main") {
                                demote_and_hide_window(&win);
                            }
                            let _ = app.emit(
                                "buddy://stealth",
                                StealthPayload {
                                    active: true,
                                    reason,
                                },
                            );
                        } else if let Some(win) = app.get_webview_window("main") {
                            // Self-heal if external OS events or races made the window visible during stealth
                            if let Some(hwnd) = to_hwnd(&win) {
                                if unsafe { IsWindowVisible(hwnd).as_bool() } {
                                    demote_and_hide_window(&win);
                                }
                            }
                        }
                    }
                } else if state.is_stealth.load(Ordering::Acquire) {
                    let start = clear_since.get_or_insert_with(Instant::now);
                    if !auto_hide || start.elapsed() >= Duration::from_millis(1500) {
                        if let Ok(_vis_guard) = state.visibility_lock.lock() {
                            clear_since = None;
                            state.is_stealth.store(false, Ordering::Release);
                            crate::logger::log_info("stealth", "game exited -> restoring Buddy HWND_TOPMOST");
                            if let Some(win) = app.get_webview_window("main") {
                                restore_topmost_and_show(&win, &state);
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

/// Registers `Ctrl + Alt + B` global shortcut to toggle Buddy's expanded chat panel.
pub fn start_toggle_hotkey_listener(app: AppHandle, state: Arc<AppSensorState>) {
    #[cfg(windows)]
    std::thread::spawn(move || unsafe {
        const HOTKEY_ID: i32 = 103;
        // VK_B = 0x42
        if RegisterHotKey(None, HOTKEY_ID, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, 0x42).is_err() {
            crate::logger::log_warn("hotkey", "failed to register Ctrl+Alt+B global hotkey");
            return;
        }
        crate::logger::log_info("hotkey", "registered Ctrl+Alt+B global toggle hotkey");
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_HOTKEY && msg.wParam.0 as i32 == HOTKEY_ID {
                if state.is_stealth.load(Ordering::Acquire) {
                    continue;
                }
                if let Ok(_vis_guard) = state.visibility_lock.lock() {
                    if state.is_stealth.load(Ordering::Acquire) {
                        continue;
                    }
                    state.manual_sleep.store(false, Ordering::Relaxed);
                    state.is_sleeping.store(false, Ordering::Relaxed);
                    let next_expanded = !state.is_expanded.load(Ordering::Relaxed);
                    if next_expanded {
                        let now_ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0);
                        state.last_expand_ms.store(now_ms, Ordering::Relaxed);
                        if let Some(win) = app.get_webview_window("main") {
                            apply_window_layout(&win, &state, true);
                            apply_interaction_mode(&win, true);
                            show_window_no_activate(&win);
                            if let Some(hwnd) = to_hwnd(&win) {
                                let _ = SetForegroundWindow(hwnd);
                            }
                        }
                    }
                    let _ = app.emit("buddy://toggle-panel", next_expanded);
                }
            }
        }
        let _ = UnregisterHotKey(None, HOTKEY_ID);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_games_without_hiding_fullscreen_browsers_or_editors() {
        // Non-game fullscreen apps MUST NOT trigger stealth hide
        assert!(!classify_foreground_game(
            Some(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
            "Chrome_WidgetWin_1",
            "youtube - google chrome",
            true,
            false,
            false,
            true
        ));
        assert!(!classify_foreground_game(
            Some(r"C:\Users\Marcus\AppData\Local\Programs\Microsoft VS Code\Code.exe"),
            "Chrome_WidgetWin_1",
            "app.tsx",
            true,
            true,
            false,
            true
        ));
        assert!(!classify_foreground_game(
            Some(r"C:\Program Files\VideoLAN\VLC\vlc.exe"),
            "Qt5QWindowIcon",
            "movie",
            true,
            false,
            false,
            true
        ));
        assert!(!classify_foreground_game(
            Some(r"C:\Program Files (x86)\Steam\steam.exe"),
            "SDL_app",
            "steam",
            true,
            true,
            false,
            true
        ));
        assert!(!classify_foreground_game(
            Some(r"C:\Program Files (x86)\Steam\steamapps\common\wallpaper_engine\wallpaper64.exe"),
            "WallpaperClass",
            "wallpaper engine",
            true,
            true,
            false,
            true
        ));
        assert!(!classify_foreground_game(
            Some(r"C:\Program Files\obs-studio\bin\64bit\obs64.exe"),
            "Qt662QWindowIcon",
            "obs studio",
            true,
            false,
            false,
            true
        ));
        assert!(!classify_foreground_game(
            Some(r"C:\Users\Marcus\AppData\Local\Discord\app-1.0\Discord.exe"),
            "Chrome_WidgetWin_1",
            "discord",
            true,
            false,
            false,
            true
        ));

        // Unlisted fullscreen desktop apps, UWP apps, elevated apps, and non-fullscreen D3D false positives MUST NOT hide Buddy
        assert!(!classify_foreground_game(
            Some(r"C:\Program Files\SomeUnlistedApp\SomeViewer.exe"),
            "SomeViewerWindowClass",
            "full screen presentation",
            true,
            false,
            false,
            true
        ));
        assert!(!classify_foreground_game(
            Some(r"C:\Program Files\SomeUnlistedApp\SomeViewer.exe"),
            "SomeViewerWindowClass",
            "small window with lingering d3d state",
            false,
            true,
            true,
            true
        ));
        assert!(!classify_foreground_game(
            None,
            "CASCADIA_HOSTING_WINDOW_CLASS",
            "administrator: powershell",
            true,
            false,
            false,
            true
        ));
        assert!(!classify_foreground_game(
            None,
            "CustomElevatedApp",
            "fullscreen admin app",
            true,
            false,
            false,
            true
        ));
        assert!(!classify_foreground_game(
            Some(r"D:\Games\Genshin Impact\launcher.exe"),
            "Qt5152QWindowIcon",
            "genshin launcher",
            false,
            false,
            false,
            true
        ));

        // Actual games MUST trigger stealth hide (both fullscreen and windowed/borderless)
        assert!(classify_foreground_game(
            Some(r"D:\SteamLibrary\steamapps\common\EldenRing\Game\eldenring.exe"),
            "ELDEN RING™",
            "elden ring",
            false,
            false,
            true,
            true
        ));
        assert!(classify_foreground_game(
            Some(r"C:\Games\Custom\Game-Win64-Shipping.exe"),
            "UnrealWindow",
            "game",
            true,
            true,
            false,
            true
        ));
        // Kernel anti-cheat protected game blocking OpenProcess in fullscreen borderless or exclusive D3D
        assert!(classify_foreground_game(
            None,
            "RiotWindowClass",
            "valorant",
            true,
            false,
            false,
            true
        ));
        assert!(classify_foreground_game(
            None,
            "CustomProtectedGameWnd",
            "protected d3d game",
            true,
            true,
            false,
            true
        ));
    }
}
