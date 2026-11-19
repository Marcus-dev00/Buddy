//! Windows native sensors, cursor hit-testing, Win+D desktop recovery,
//! accurate game-only stealth hiding, and global toggle hotkey.

mod dock;
mod game_detector;

pub use dock::*;

#[cfg(windows)]
use dock::to_hwnd;
use serde::Serialize;
#[cfg(windows)]
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

#[cfg(windows)]
use windows::Win32::Foundation::{POINT, RECT};
#[cfg(windows)]
use windows::Win32::System::SystemInformation::GetTickCount;
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetLastInputInfo, RegisterHotKey, UnregisterHotKey, LASTINPUTINFO, MOD_ALT, MOD_CONTROL,
    MOD_NOREPEAT,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, GetMessageW, GetWindowLongPtrW, GetWindowRect, IsIconic,
    IsWindow, IsWindowVisible, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    ShowWindowAsync, GWL_EXSTYLE, HWND_BOTTOM, HWND_TOPMOST, MSG, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE, WM_HOTKEY, WS_EX_LAYERED, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT,
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
    pub is_window_mode: AtomicBool,
    pub is_window_maximized: AtomicBool,
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
            is_window_mode: AtomicBool::new(false),
            is_window_maximized: AtomicBool::new(false),
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
        let mut last_cursor_pos: Option<(i32, i32)> = None;
        #[cfg(windows)]
        let mut last_cursor_move_at = Instant::now();
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
                        let h = windows::Win32::Foundation::HWND(anchor.target_hwnd as *mut c_void);
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
                || state_cursor.is_window_mode.load(Ordering::Relaxed)
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
                            if last_cursor_pos != Some((pt.x, pt.y)) {
                                last_cursor_pos = Some((pt.x, pt.y));
                                last_cursor_move_at = Instant::now();
                            }
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

            #[cfg(windows)]
            let sleep_ms = if !was_hovering
                && (state_cursor.is_sleeping.load(Ordering::Relaxed)
                    || last_cursor_move_at.elapsed() >= Duration::from_millis(1500))
            {
                120
            } else {
                33
            };
            #[cfg(not(windows))]
            let sleep_ms = 33;
            std::thread::sleep(Duration::from_millis(sleep_ms));
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
                // Skip topmost enforcement when user is in normal desktop Window Mode so Alt+Tab works freely.
                let in_win_mode = state.is_window_mode.load(Ordering::Relaxed);
                if !in_win_mode && !state.is_stealth.load(Ordering::Acquire) {
                    if let Some(win) = app.get_webview_window("main") {
                        let fg = unsafe { GetForegroundWindow() };
                        let on_desktop = !fg.0.is_null() && {
                            let cls = game_detector::get_window_class(fg);
                            cls == "Progman" || cls == "WorkerW"
                        };
                        if on_desktop || !is_window_topmost_and_visible(&win) {
                            if let Ok(_vis_guard) = state.visibility_lock.lock() {
                                if !state.is_stealth.load(Ordering::Acquire)
                                    && !state.is_window_mode.load(Ordering::Relaxed)
                                {
                                    show_window_no_activate(&win);
                                }
                            }
                        }
                    }
                }

                // Stealth check: drop to bottom & hide ONLY for active games (never meetings, launchers, browsers, etc.)
                let auto_hide = state.auto_hide_enabled.load(Ordering::Relaxed);
                let mut is_expanded = state.is_expanded.load(Ordering::Relaxed);
                let (detected_detail, reason) = if auto_hide
                    && !in_win_mode
                    && !state.is_dragging.load(Ordering::Relaxed)
                {
                    if let Some(detail) = game_detector::detect_active_game(own_pid) {
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
