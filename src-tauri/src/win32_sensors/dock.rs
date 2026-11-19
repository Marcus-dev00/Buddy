//! Multi-edge screen & target-window magnetic docking, drag session math,
//! monitor work-area clamping, and DWM window layout sizing.

use super::{
    AppSensorState, DockStatePayload, DragSession, LayoutDims, WindowDockAnchor, CANVAS_PAD,
    DOCK_EDGE_GAP_LOGICAL, EXPANDED_H_W, EXPANDED_V_W, WINDOW_CANVAS_W,
};
use crate::settings::SettingsManager;
use std::ffi::c_void;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, PhysicalPosition, PhysicalSize};

#[cfg(windows)]
use windows::Win32::Foundation::{HWND, POINT, RECT};
#[cfg(windows)]
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    MONITOR_DEFAULTTOPRIMARY,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowLongPtrW, GetWindowRect, IsIconic, IsWindow, SetWindowDisplayAffinity,
    SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WINDOW_DISPLAY_AFFINITY, WS_EX_NOACTIVATE,
};

#[cfg(windows)]
pub(super) fn to_hwnd(window: &tauri::WebviewWindow) -> Option<HWND> {
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

#[cfg(not(windows))]
pub fn get_primary_work_area() -> (i32, i32, i32, i32) {
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

#[cfg(windows)]
fn apply_ratio_detents(ratio: f64, span_px: f64, scale: f64) -> f64 {
    let detent_tol = ((20.0 * scale) / span_px.max(1.0)).min(0.05);
    for anchor_r in [0.15, 0.50, 0.85] {
        if (ratio - anchor_r).abs() <= detent_tol {
            return anchor_r;
        }
    }
    ratio
}

#[cfg(windows)]
pub fn set_window_topmost(window: &tauri::WebviewWindow, topmost: bool) {
    if let Some(hwnd) = to_hwnd(window) {
        unsafe {
            let z = if topmost { HWND_TOPMOST } else { HWND_NOTOPMOST };
            let _ = SetWindowPos(hwnd, z, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    } else {
        let _ = window.set_always_on_top(topmost);
    }
}

#[cfg(not(windows))]
pub fn set_window_topmost(window: &tauri::WebviewWindow, topmost: bool) {
    let _ = window.set_always_on_top(topmost);
}

/// Positions and sizes the main window for Large Window Mode (centered spacious window or full work-area maximized).
pub fn apply_window_mode_bounds(
    window: &tauri::WebviewWindow,
    state: &AppSensorState,
    maximized: bool,
) {
    let scale = window.scale_factor().unwrap_or(1.0);
    let (wx, wy, ww, wh) = if state.has_anchor.load(Ordering::Relaxed) {
        get_work_area_at(
            state.anchor_cx.load(Ordering::Relaxed),
            state.anchor_cy.load(Ordering::Relaxed),
        )
    } else {
        get_primary_work_area()
    };

    if maximized {
        let _ = window.set_size(PhysicalSize::new(ww.max(480) as u32, wh.max(400) as u32));
        let _ = window.set_position(PhysicalPosition::new(wx, wy));
    } else {
        let margin_x = (80.0 * scale).round() as i32;
        let margin_y = (72.0 * scale).round() as i32;
        let target_w = ((1120.0 * scale).round() as i32)
            .min((ww - margin_x).max(760))
            .max(680);
        let target_h = ((760.0 * scale).round() as i32)
            .min((wh - margin_y).max(540))
            .max(500);
        let x = wx + (ww - target_w) / 2;
        let y = wy + (wh - target_h) / 2;
        let _ = window.set_size(PhysicalSize::new(target_w as u32, target_h as u32));
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
}

/// Sizes and positions the transparent canvas cleanly for any dock edge (`top`, `bottom`, `left`, `right`, `free`),
/// or magnetically docked to a shared target window edge when `window_dock` is active.
pub fn apply_window_layout(
    window: &tauri::WebviewWindow,
    state: &AppSensorState,
    expanded: bool,
) -> LayoutDims {
    let scale = window.scale_factor().unwrap_or(1.0);
    if state.is_window_mode.load(Ordering::Relaxed) {
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

    [
        ("top", d_top),
        ("bottom", d_bottom),
        ("left", d_left),
        ("right", d_right),
    ]
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
        ("top", d_top, (pt_x - tx) as f64 / (tw.max(1) as f64), tw as f64),
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

    Some((edge, apply_ratio_detents(raw_ratio, span_px, scale)))
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
                            let max_y =
                                (wy + wh) - (canvas_h_phys / 2.0) as i32 - (30.0 * scale) as i32;
                            (
                                sess.start_win_pos.0,
                                (sess.start_win_pos.1 + dy.round() as i32)
                                    .clamp(min_y, max_y.max(min_y)),
                            )
                        }
                        _ => {
                            let min_x = wx - (canvas_w_phys / 2.0) as i32 + (30.0 * scale) as i32;
                            let max_x =
                                (wx + ww) - (canvas_w_phys / 2.0) as i32 - (30.0 * scale) as i32;
                            (
                                (sess.start_win_pos.0 + dx.round() as i32)
                                    .clamp(min_x, max_x.max(min_x)),
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
                let box_w_phys =
                    (if sess.expanded { dims.v_w } else { pill_w.max(152.0) }) * scale;
                let box_h_phys =
                    (if sess.expanded { dims.v_h } else { pill_h.min(78.0) }) * scale;

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
                            let span = match sess.initial_edge.as_str() {
                                "left" | "right" => th as f64,
                                _ => tw as f64,
                            };
                            let r = apply_ratio_detents(raw_r, span, scale);
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
                let box_w_phys =
                    (if sess.expanded { dims.v_w } else { pill_w.max(152.0) }) * scale;
                let box_h_phys =
                    (if sess.expanded { dims.v_h } else { pill_h.min(78.0) }) * scale;
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
