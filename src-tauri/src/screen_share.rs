//! Live Screen & Target-Window Sharing engine:
//! - Enumerates cursor-following monitor, physical displays, and visible application windows.
//! - Captures monitors via BitBlt and GPU/browser windows via PrintWindow(PW_RENDERFULLCONTENT) with BitBlt fallback.
//! - Maintains an in-memory sliding frame buffer (latest + recent comparison frame) with zero disk clutter.
//! - Drives a quiet, settle-debounced proactive sentinel using the configured Watch Vision model (`[SILENT]` when calm).
//! - Renders a click-through, capture-excluded Win32 soft-border overlay around the shared monitor or window.

use crate::ai_client::{
    encode_dynamic_image_jpeg_base64, run_screen_watch_check, AiStreamController, InlineImageFrame,
};
use crate::settings::SettingsManager;
use crate::win32_sensors::{
    attach_dock_to_window, detach_dock_from_window, get_window_visible_rect,
    set_hwnd_capture_excluded, set_main_capture_excluded, AppSensorState,
};
use image::{DynamicImage, RgbaImage};
use serde::{Deserialize, Serialize};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
#[cfg(windows)]
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
    EnumDisplayMonitors, GetDC, GetDIBits, GetMonitorInfoW, MonitorFromPoint, ReleaseDC,
    SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION,
    DIB_RGB_COLORS, HDC, HMONITOR, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST, SRCCOPY,
};
#[cfg(windows)]
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
#[cfg(windows)]
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
#[cfg(windows)]
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_NOREPEAT, MOD_SHIFT,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EnumWindows, GetCursorPos,
    GetForegroundWindow, GetMessageW, GetWindow, GetWindowLongW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
    IsWindowVisible, PeekMessageW, RegisterClassW, SetWindowPos, ShowWindow, TranslateMessage,
    UpdateLayeredWindow, GWL_EXSTYLE, GW_OWNER, HWND_TOPMOST, MSG, PM_REMOVE, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA, WM_HOTKEY, WNDCLASSW,
    WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

const LUMA_W: u32 = 48;
const LUMA_H: u32 = 27;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Clone, Debug, Serialize)]
pub struct ShareTargetInfo {
    pub id: String,
    pub kind: String, // "auto_cursor" | "monitor" | "window"
    pub title: String,
    pub app_name: String,
    pub width: i32,
    pub height: i32,
    pub is_primary: bool,
    pub hwnd: isize,
}

#[derive(Clone, Debug, Serialize)]
pub struct ShareStatePayload {
    pub active: bool,
    pub paused: bool,
    pub minimized: bool,
    pub analyzing: bool,
    pub target_id: String,
    pub target_kind: String, // "monitor" | "window"
    pub target_label: String,
    pub target_hwnd: isize,
    pub dock_edge: String,
    pub goal: String,
    pub cadence: String, // "quiet" | "balanced" | "active"
    pub show_border: bool,
    pub watch_profile_id: String,
    pub frame_count: u64,
    pub reason: String,
    pub last_error: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ShareWhisperPayload {
    pub id: String,
    pub text: String,
    pub timestamp_ms: u64,
    pub target_label: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StartShareRequest {
    pub target_id: String,
    #[serde(default)]
    pub goal: String,
    #[serde(default = "default_cadence")]
    pub cadence: String,
    #[serde(default = "default_true")]
    pub show_border: bool,
    #[serde(default)]
    pub watch_profile_id: String,
    #[serde(default)]
    pub window_edge: Option<String>,
    #[serde(default)]
    pub window_ratio: Option<f64>,
}

fn default_cadence() -> String {
    "balanced".to_string()
}

fn default_true() -> bool {
    true
}

#[derive(Clone)]
struct CapturedFrame {
    label: String,
    jpeg_b64: String,
    luma: Vec<u8>,
    captured_at_ms: u64,
}

#[derive(Clone, PartialEq)]
struct BorderSpec {
    visible: bool,
    is_window_mode: bool,
    target_hwnd: isize,
    rect: (i32, i32, i32, i32), // (left, top, width, height) of target monitor or target window
    dimmed: bool,               // true when target window is not foreground
}

impl Default for BorderSpec {
    fn default() -> Self {
        Self {
            visible: false,
            is_window_mode: false,
            target_hwnd: 0,
            rect: (0, 0, 0, 0),
            dimmed: false,
        }
    }
}

struct ShareSessionRuntime {
    active: bool,
    paused: bool,
    minimized: bool,
    analyzing: bool,
    generation: u64,
    target_id: String,
    target_kind: String, // "monitor" | "window"
    target_label: String,
    target_hwnd: isize,
    monitor_index: Option<usize>,
    goal: String,
    cadence: String,
    show_border: bool,
    watch_profile_id: String,
    frame_count: u64,
    latest_frame: Option<CapturedFrame>,
    prev_frame: Option<CapturedFrame>,
    prev_live_luma: Option<Vec<u8>>,
    last_watched_luma: Option<Vec<u8>>,
    last_watched_frame: Option<InlineImageFrame>,
    last_change_ms: u64,
    pending_settle: bool,
    last_ai_check_ms: u64,
    recent_whispers: Vec<String>,
    last_error: String,
}

impl Default for ShareSessionRuntime {
    fn default() -> Self {
        Self {
            active: false,
            paused: false,
            minimized: false,
            analyzing: false,
            generation: 0,
            target_id: String::new(),
            target_kind: "monitor".to_string(),
            target_label: String::new(),
            target_hwnd: 0,
            monitor_index: None,
            goal: String::new(),
            cadence: "balanced".to_string(),
            show_border: true,
            watch_profile_id: String::new(),
            frame_count: 0,
            latest_frame: None,
            prev_frame: None,
            prev_live_luma: None,
            last_watched_luma: None,
            last_watched_frame: None,
            last_change_ms: 0,
            pending_settle: false,
            last_ai_check_ms: 0,
            recent_whispers: Vec::new(),
            last_error: String::new(),
        }
    }
}

pub struct ScreenShareManager {
    runtime: Mutex<ShareSessionRuntime>,
    border_desired: Arc<Mutex<BorderSpec>>,
    stop_requested: AtomicBool,
    session_counter: AtomicU64,
}

impl Default for ScreenShareManager {
    fn default() -> Self {
        let desired = Arc::new(Mutex::new(BorderSpec::default()));
        #[cfg(windows)]
        spawn_border_overlay_thread(desired.clone());
        Self {
            runtime: Mutex::new(ShareSessionRuntime::default()),
            border_desired: desired,
            stop_requested: AtomicBool::new(false),
            session_counter: AtomicU64::new(0),
        }
    }
}

#[cfg(windows)]
#[derive(Clone)]
struct MonitorRectEntry {
    device_name: String,
    rect: (i32, i32, i32, i32), // left, top, width, height
    is_primary: bool,
}

#[cfg(windows)]
unsafe extern "system" fn enum_monitors_proc(
    hmon: HMONITOR,
    _hdc: HDC,
    _lprc: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let list = &mut *(lparam.0 as *mut Vec<MonitorRectEntry>);
    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(hmon, &mut mi.monitorInfo as *mut _).as_bool() {
        let r = mi.monitorInfo.rcMonitor;
        let is_primary = (mi.monitorInfo.dwFlags & 1) != 0; // MONITORINFOF_PRIMARY
        let nul = mi
            .szDevice
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(mi.szDevice.len());
        let dev = String::from_utf16_lossy(&mi.szDevice[..nul]);
        list.push(MonitorRectEntry {
            device_name: dev,
            rect: (r.left, r.top, r.right - r.left, r.bottom - r.top),
            is_primary,
        });
    }
    BOOL(1)
}

#[cfg(windows)]
fn enumerate_monitors() -> Vec<MonitorRectEntry> {
    let mut list: Vec<MonitorRectEntry> = Vec::new();
    unsafe {
        let ptr = &mut list as *mut Vec<MonitorRectEntry> as isize;
        let _ = EnumDisplayMonitors(None, None, Some(enum_monitors_proc), LPARAM(ptr));
    }
    list
}

#[cfg(windows)]
fn get_monitor_at_cursor() -> (i32, i32, i32, i32) {
    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFOEXW::default();
        mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(hmon, &mut mi.monitorInfo as *mut _).as_bool() {
            let r = mi.monitorInfo.rcMonitor;
            return (r.left, r.top, r.right - r.left, r.bottom - r.top);
        }
    }
    (0, 0, 1920, 1080)
}

#[cfg(windows)]
fn process_exe_name_for_pid(pid: u32) -> String {
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buf = [0u16; 260];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = windows::Win32::Foundation::CloseHandle(handle);
        if ok && len > 0 {
            let full = String::from_utf16_lossy(&buf[..len as usize]);
            return std::path::Path::new(&full)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(&full)
                .to_string();
        }
    }
    String::new()
}

#[cfg(windows)]
unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut (u32, Vec<ShareTargetInfo>));
    let own_pid = ctx.0;

    if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
        return BOOL(1);
    }

    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == 0 || pid == own_pid {
        return BOOL(1);
    }

    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    if (ex_style & WS_EX_TOOLWINDOW.0) != 0 && (ex_style & WS_EX_APPWINDOW.0) == 0 {
        return BOOL(1);
    }
    if let Ok(owner) = GetWindow(hwnd, GW_OWNER) {
        if !owner.0.is_null() && (ex_style & WS_EX_APPWINDOW.0) == 0 {
            return BOOL(1);
        }
    }

    // Skip cloaked UWP background frames
    let mut cloaked: u32 = 0;
    if DwmGetWindowAttribute(
        hwnd,
        DWMWA_CLOAKED,
        &mut cloaked as *mut u32 as *mut c_void,
        std::mem::size_of::<u32>() as u32,
    )
    .is_ok()
        && cloaked != 0
    {
        return BOOL(1);
    }

    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return BOOL(1);
    }
    let mut buf = vec![0u16; (len as usize) + 1];
    let read_len = GetWindowTextW(hwnd, &mut buf);
    if read_len <= 0 {
        return BOOL(1);
    }
    let title = String::from_utf16_lossy(&buf[..read_len as usize])
        .trim()
        .to_string();
    if title.is_empty()
        || matches!(
            title.as_str(),
            "Program Manager"
                | "Windows Input Experience"
                | "Default IME"
                | "MSCTFIME UI"
                | "Settings"
        )
    {
        return BOOL(1);
    }

    let Some((_, _, w, h)) = get_window_visible_rect(hwnd) else {
        return BOOL(1);
    };
    if w < 140 || h < 100 {
        return BOOL(1);
    }

    let app_name = process_exe_name_for_pid(pid);
    ctx.1.push(ShareTargetInfo {
        id: format!("window:{}", hwnd.0 as isize),
        kind: "window".to_string(),
        title,
        app_name,
        width: w,
        height: h,
        is_primary: false,
        hwnd: hwnd.0 as isize,
    });

    BOOL(1)
}

pub fn enumerate_share_targets() -> Vec<ShareTargetInfo> {
    let mut out = Vec::new();
    #[cfg(windows)]
    {
        let (cw, ch) = {
            let r = get_monitor_at_cursor();
            (r.2, r.3)
        };
        out.push(ShareTargetInfo {
            id: "auto_cursor".to_string(),
            kind: "auto_cursor".to_string(),
            title: "Mouse Screen (Auto)".to_string(),
            app_name: "Monitor".to_string(),
            width: cw,
            height: ch,
            is_primary: true,
            hwnd: 0,
        });

        let mons = enumerate_monitors();
        for (idx, m) in mons.into_iter().enumerate() {
            let short_dev = m
                .device_name
                .trim_start_matches("\\\\.\\")
                .to_string();
            out.push(ShareTargetInfo {
                id: format!("monitor:{idx}"),
                kind: "monitor".to_string(),
                title: format!(
                    "Display {} ({})",
                    idx + 1,
                    if short_dev.is_empty() {
                        "Screen"
                    } else {
                        &short_dev
                    }
                ),
                app_name: "Monitor".to_string(),
                width: m.rect.2,
                height: m.rect.3,
                is_primary: m.is_primary,
                hwnd: 0,
            });
        }

        let mut win_ctx = (std::process::id(), Vec::<ShareTargetInfo>::new());
        unsafe {
            let ptr = &mut win_ctx as *mut (u32, Vec<ShareTargetInfo>) as isize;
            let _ = EnumWindows(Some(enum_windows_proc), LPARAM(ptr));
        }
        out.extend(win_ctx.1);
    }
    out
}

#[cfg(windows)]
fn bitblt_rect_image(vx: i32, vy: i32, vw: i32, vh: i32) -> Option<DynamicImage> {
    if vw <= 0 || vh <= 0 {
        return None;
    }
    unsafe {
        let hdc_screen = GetDC(None);
        if hdc_screen.is_invalid() {
            return None;
        }
        let hdc_mem = CreateCompatibleDC(hdc_screen);
        let hbm = CreateCompatibleBitmap(hdc_screen, vw, vh);
        let old = SelectObject(hdc_mem, hbm);

        let blt_ok = BitBlt(hdc_mem, 0, 0, vw, vh, hdc_screen, vx, vy, SRCCOPY).is_ok();
        let img = if blt_ok {
            read_hbitmap_to_dynamic(hdc_mem, hbm, vw, vh)
        } else {
            None
        };

        SelectObject(hdc_mem, old);
        let _ = DeleteObject(hbm);
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(None, hdc_screen);
        img
    }
}

#[cfg(windows)]
unsafe fn read_hbitmap_to_dynamic(hdc: HDC, hbm: windows::Win32::Graphics::Gdi::HBITMAP, w: i32, h: i32) -> Option<DynamicImage> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bgra = vec![0u8; (w as usize) * (h as usize) * 4];
    let lines = GetDIBits(
        hdc,
        hbm,
        0,
        h as u32,
        Some(bgra.as_mut_ptr() as *mut _),
        &mut bmi,
        DIB_RGB_COLORS,
    );
    if lines == 0 {
        return None;
    }
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
        px[3] = 255;
    }
    let rgba = RgbaImage::from_raw(w as u32, h as u32, bgra)?;
    Some(DynamicImage::ImageRgba8(rgba))
}

#[cfg(windows)]
fn is_image_mostly_blank(img: &DynamicImage) -> bool {
    let rgb = img.to_rgb8();
    let (w, h) = (rgb.width(), rgb.height());
    if w < 8 || h < 8 {
        return true;
    }
    let mut non_black = 0usize;
    for sy in 1..9 {
        for sx in 1..9 {
            let x = (sx * w) / 10;
            let y = (sy * h) / 10;
            let p = rgb.get_pixel(x, y);
            if p[0] > 3 || p[1] > 3 || p[2] > 3 {
                non_black += 1;
            }
        }
    }
    non_black < 2
}

#[cfg(windows)]
fn capture_hwnd_image(hwnd: HWND) -> Option<DynamicImage> {
    unsafe {
        if !IsWindow(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return None;
        }
        let mut full_r = RECT::default();
        if GetWindowRect(hwnd, &mut full_r).is_err() {
            return None;
        }
        let fw = full_r.right - full_r.left;
        let fh = full_r.bottom - full_r.top;
        if fw <= 0 || fh <= 0 {
            return None;
        }

        let vis_r = get_window_visible_rect(hwnd).unwrap_or((full_r.left, full_r.top, fw, fh));

        // 1. Try offscreen PrintWindow(PW_RENDERFULLCONTENT = 2) so even occluded Chrome/Canva captures cleanly
        let hdc_screen = GetDC(None);
        let mut captured: Option<DynamicImage> = None;
        if !hdc_screen.is_invalid() {
            let hdc_mem = CreateCompatibleDC(hdc_screen);
            let hbm = CreateCompatibleBitmap(hdc_screen, fw, fh);
            let old = SelectObject(hdc_mem, hbm);

            // PW_RENDERFULLCONTENT = 2
            let pw_ok = PrintWindow(hwnd, hdc_mem, PRINT_WINDOW_FLAGS(2)).as_bool();
            if pw_ok {
                if let Some(full_img) = read_hbitmap_to_dynamic(hdc_mem, hbm, fw, fh) {
                    let crop_x = (vis_r.0 - full_r.left).max(0) as u32;
                    let crop_y = (vis_r.1 - full_r.top).max(0) as u32;
                    let crop_w = (vis_r.2.max(1) as u32).min(full_img.width().saturating_sub(crop_x));
                    let crop_h = (vis_r.3.max(1) as u32).min(full_img.height().saturating_sub(crop_y));
                    let cropped = if crop_w > 16 && crop_h > 16 {
                        full_img.crop_imm(crop_x, crop_y, crop_w, crop_h)
                    } else {
                        full_img
                    };
                    if !is_image_mostly_blank(&cropped) {
                        captured = Some(cropped);
                    }
                }
            }

            SelectObject(hdc_mem, old);
            let _ = DeleteObject(hbm);
            let _ = DeleteDC(hdc_mem);
            ReleaseDC(None, hdc_screen);
        }

        // 2. Fallback to desktop region BitBlt over the visible window rectangle
        if captured.is_none() {
            captured = bitblt_rect_image(vis_r.0, vis_r.1, vis_r.2, vis_r.3);
        }
        captured
    }
}

fn compute_luma_thumb(img: &DynamicImage) -> Vec<u8> {
    let tiny = img
        .resize_exact(LUMA_W, LUMA_H, image::imageops::FilterType::Triangle)
        .to_luma8();
    tiny.into_raw()
}

/// Returns fraction of pixels changed significantly (0.0..=1.0).
fn luma_change_ratio(a: &[u8], b: &[u8]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 1.0;
    }
    let mut changed = 0usize;
    let mut sum_diff = 0u64;
    for (&x, &y) in a.iter().zip(b.iter()) {
        let d = x.abs_diff(y);
        sum_diff += d as u64;
        if d > 11 {
            changed += 1;
        }
    }
    let pct = (changed as f32) / (a.len() as f32);
    let mean = (sum_diff as f32) / (a.len() as f32) / 255.0;
    pct.max(mean * 2.5)
}

fn resolve_cadence_timings(cadence: &str) -> (u64, u64) {
    match cadence {
        "active" => (1500, 10_000),
        "quiet" => (u64::MAX, u64::MAX),
        _ => (2800, 35_000), // balanced default
    }
}

/// Computes whether live screen motion has settled and whether an AI watch check should trigger.
fn update_settle_and_should_trigger(
    cadence: &str,
    prev_live_luma: &mut Option<Vec<u8>>,
    last_watched_luma: Option<&[u8]>,
    new_luma: &[u8],
    last_change_ms: &mut u64,
    pending_settle: &mut bool,
    last_ai_check_ms: u64,
    analyzing: bool,
    busy_chatting: bool,
    now_ms: u64,
) -> bool {
    if let Some(prev) = prev_live_luma.as_deref() {
        if luma_change_ratio(prev, new_luma) >= 0.015 {
            *last_change_ms = now_ms;
            *pending_settle = true;
        }
    } else {
        // First captured frame initiates the settle clock once
        *last_change_ms = now_ms;
        *pending_settle = true;
    }
    *prev_live_luma = Some(new_luma.to_vec());

    let unseen_vs_watched = last_watched_luma
        .map(|wl| luma_change_ratio(wl, new_luma) >= 0.020)
        .unwrap_or(true);

    let (settle_ms, cooldown_ms) = resolve_cadence_timings(cadence);
    let settled = *pending_settle && now_ms.saturating_sub(*last_change_ms) >= settle_ms;
    let cooled_down = last_ai_check_ms == 0 || now_ms.saturating_sub(last_ai_check_ms) >= cooldown_ms;

    cadence != "quiet" && !analyzing && !busy_chatting && unseen_vs_watched && settled && cooled_down
}

impl ScreenShareManager {
    pub fn get_state(&self, sensor_state: &AppSensorState) -> ShareStatePayload {
        let g = self.runtime.lock().ok();
        let dock_edge = sensor_state.get_dock_edge();
        if let Some(rt) = g {
            ShareStatePayload {
                active: rt.active,
                paused: rt.paused,
                minimized: rt.minimized,
                analyzing: rt.analyzing,
                target_id: rt.target_id.clone(),
                target_kind: rt.target_kind.clone(),
                target_label: rt.target_label.clone(),
                target_hwnd: rt.target_hwnd,
                dock_edge,
                goal: rt.goal.clone(),
                cadence: rt.cadence.clone(),
                show_border: rt.show_border,
                watch_profile_id: rt.watch_profile_id.clone(),
                frame_count: rt.frame_count,
                reason: String::new(),
                last_error: rt.last_error.clone(),
            }
        } else {
            ShareStatePayload {
                active: false,
                paused: false,
                minimized: false,
                analyzing: false,
                target_id: String::new(),
                target_kind: "monitor".to_string(),
                target_label: String::new(),
                target_hwnd: 0,
                dock_edge,
                goal: String::new(),
                cadence: "balanced".to_string(),
                show_border: true,
                watch_profile_id: String::new(),
                frame_count: 0,
                reason: String::new(),
                last_error: String::new(),
            }
        }
    }

    fn emit_state(&self, app: &AppHandle, sensor_state: &AppSensorState, reason: &str) {
        let mut payload = self.get_state(sensor_state);
        payload.reason = reason.to_string();
        let _ = app.emit("buddy://share-state", payload);
    }

    fn set_border(&self, spec: BorderSpec) {
        if let Ok(mut b) = self.border_desired.lock() {
            *b = spec;
        }
    }

    pub fn update_live_options(
        &self,
        app: &AppHandle,
        sensor_state: &AppSensorState,
        goal: Option<String>,
        cadence: Option<String>,
        show_border: Option<bool>,
        watch_profile_id: Option<String>,
    ) -> ShareStatePayload {
        if let Ok(mut rt) = self.runtime.lock() {
            if let Some(g) = goal {
                rt.goal = g;
            }
            if let Some(c) = cadence {
                rt.cadence = c;
            }
            if let Some(sb) = show_border {
                rt.show_border = sb;
                if !sb || !rt.active || rt.paused || rt.minimized {
                    self.set_border(BorderSpec::default());
                }
            }
            if let Some(wp) = watch_profile_id {
                rt.watch_profile_id = wp;
            }
        }
        self.emit_state(app, sensor_state, "options_updated");
        self.get_state(sensor_state)
    }

    pub fn toggle_pause(&self, app: &AppHandle, sensor_state: &AppSensorState) -> ShareStatePayload {
        if let Ok(mut rt) = self.runtime.lock() {
            if !rt.active {
                return self.get_state(sensor_state);
            }
            rt.paused = !rt.paused;
            if rt.paused {
                rt.pending_settle = false;
                rt.analyzing = false;
                self.set_border(BorderSpec::default());
            }
        }
        self.emit_state(app, sensor_state, "pause_toggled");
        self.get_state(sensor_state)
    }

    pub fn stop(&self, app: &AppHandle, sensor_state: &AppSensorState, reason: &str) -> ShareStatePayload {
        self.stop_requested.store(true, Ordering::Relaxed);
        if let Ok(mut rt) = self.runtime.lock() {
            *rt = ShareSessionRuntime {
                generation: rt.generation.wrapping_add(1),
                ..Default::default()
            };
        }
        self.set_border(BorderSpec::default());

        if let Some(win) = app.get_webview_window("main") {
            set_main_capture_excluded(&win, false);
            detach_dock_from_window(app, &win, sensor_state);
        }

        self.emit_state(app, sensor_state, reason);
        self.get_state(sensor_state)
    }

    pub fn start(
        self: &Arc<Self>,
        app: AppHandle,
        sensor_state: Arc<AppSensorState>,
        settings_mgr: Arc<SettingsManager>,
        ai_ctrl: Arc<AiStreamController>,
        req: StartShareRequest,
    ) -> Result<ShareStatePayload, String> {
        // Resolve requested target
        let targets = enumerate_share_targets();
        let (kind, hwnd, mon_idx, label) = if let Some(rest) = req.target_id.strip_prefix("window:") {
            let parsed_hwnd: isize = rest
                .trim()
                .parse()
                .map_err(|_| "Invalid window handle".to_string())?;
            #[cfg(windows)]
            {
                if unsafe { !IsWindow(HWND(parsed_hwnd as *mut c_void)).as_bool() } {
                    return Err("Selected window is no longer open.".to_string());
                }
            }
            let lbl = targets
                .iter()
                .find(|t| t.hwnd == parsed_hwnd)
                .map(|t| {
                    if t.app_name.is_empty() {
                        t.title.clone()
                    } else {
                        format!("{} — {}", t.app_name, t.title)
                    }
                })
                .unwrap_or_else(|| "Selected Window".to_string());
            ("window".to_string(), parsed_hwnd, None, lbl)
        } else if let Some(idx_str) = req.target_id.strip_prefix("monitor:") {
            let idx: usize = idx_str.trim().parse().unwrap_or(0);
            let lbl = targets
                .iter()
                .find(|t| t.id == req.target_id)
                .map(|t| t.title.clone())
                .unwrap_or_else(|| format!("Display {}", idx + 1));
            ("monitor".to_string(), 0, Some(idx), lbl)
        } else {
            (
                "monitor".to_string(),
                0,
                None,
                "Mouse Screen (Auto)".to_string(),
            )
        };

        let gen = self.session_counter.fetch_add(1, Ordering::Relaxed) + 1;
        self.stop_requested.store(false, Ordering::Relaxed);

        if let Ok(mut rt) = self.runtime.lock() {
            *rt = ShareSessionRuntime {
                active: true,
                paused: false,
                minimized: false,
                analyzing: false,
                generation: gen,
                target_id: req.target_id.clone(),
                target_kind: kind.clone(),
                target_label: label.clone(),
                target_hwnd: hwnd,
                monitor_index: mon_idx,
                goal: req.goal.trim().to_string(),
                cadence: match req.cadence.as_str() {
                    "quiet" | "active" => req.cadence.clone(),
                    _ => "balanced".to_string(),
                },
                show_border: req.show_border,
                watch_profile_id: req.watch_profile_id.clone(),
                frame_count: 0,
                latest_frame: None,
                prev_frame: None,
                prev_live_luma: None,
                last_watched_luma: None,
                last_watched_frame: None,
                last_change_ms: 0,
                pending_settle: false,
                last_ai_check_ms: 0,
                recent_whispers: Vec::new(),
                last_error: String::new(),
            };
        }

        if let Some(win) = app.get_webview_window("main") {
            set_main_capture_excluded(&win, true);
            if kind == "window" && hwnd != 0 {
                let edge = req.window_edge.as_deref().unwrap_or("top");
                let ratio = req.window_ratio.unwrap_or(0.5);
                attach_dock_to_window(&app, &win, &sensor_state, hwnd, edge, ratio);
            } else {
                detach_dock_from_window(&app, &win, &sensor_state);
            }
        }

        self.emit_state(&app, &sensor_state, "started");
        let snapshot = self.get_state(&sensor_state);

        // Spawn dedicated session worker thread
        let mgr = Arc::clone(self);
        std::thread::spawn(move || {
            mgr.run_session_loop(gen, app, sensor_state, settings_mgr, ai_ctrl);
        });

        Ok(snapshot)
    }

    fn capture_frame_once(
        &self,
        kind: &str,
        hwnd: isize,
        mon_idx: Option<usize>,
        label: &str,
        max_dim: u32,
        quality: u8,
    ) -> Result<Option<(CapturedFrame, (i32, i32, i32, i32), bool)>, String> {
        #[cfg(windows)]
        {
            if kind == "window" && hwnd != 0 {
                let h = HWND(hwnd as *mut c_void);
                if unsafe { !IsWindow(h).as_bool() } {
                    return Err("window_closed".to_string());
                }
                if unsafe { IsIconic(h).as_bool() } {
                    return Ok(None);
                }
                let rect = get_window_visible_rect(h).unwrap_or((0, 0, 800, 600));
                let fg = unsafe { GetForegroundWindow() };
                let dimmed = fg.0 != h.0;
                if let Some(img) = capture_hwnd_image(h) {
                    let b64 = encode_dynamic_image_jpeg_base64(&img, max_dim, quality)?;
                    let luma = compute_luma_thumb(&img);
                    return Ok(Some((
                        CapturedFrame {
                            label: label.to_string(),
                            jpeg_b64: b64,
                            luma,
                            captured_at_ms: now_ms(),
                        },
                        rect,
                        dimmed,
                    )));
                }
                return Ok(None);
            } else {
                let rect = if let Some(idx) = mon_idx {
                    let mons = enumerate_monitors();
                    mons.get(idx)
                        .map(|m| m.rect)
                        .unwrap_or_else(get_monitor_at_cursor)
                } else {
                    get_monitor_at_cursor()
                };
                if let Some(img) = bitblt_rect_image(rect.0, rect.1, rect.2, rect.3) {
                    let b64 = encode_dynamic_image_jpeg_base64(&img, max_dim, quality)?;
                    let luma = compute_luma_thumb(&img);
                    return Ok(Some((
                        CapturedFrame {
                            label: label.to_string(),
                            jpeg_b64: b64,
                            luma,
                            captured_at_ms: now_ms(),
                        },
                        rect,
                        false,
                    )));
                }
                return Ok(None);
            }
        }
        #[cfg(not(windows))]
        Ok(None)
    }

    pub fn grab_frames_for_chat(&self) -> (Vec<InlineImageFrame>, Option<String>, Option<String>) {
        let snapshot = self.runtime.lock().ok().and_then(|rt| {
            if !rt.active || rt.paused || rt.minimized {
                return None;
            }
            Some((
                rt.target_kind.clone(),
                rt.target_hwnd,
                rt.monitor_index,
                rt.target_label.clone(),
                rt.goal.clone(),
                rt.watch_profile_id.clone(),
                rt.latest_frame.clone(),
                rt.prev_frame.clone(),
            ))
        });

        let Some((kind, hwnd, mon_idx, label, goal, watch_prof, mut latest, mut prev)) = snapshot
        else {
            return (Vec::new(), None, None);
        };

        let stale = latest
            .as_ref()
            .map(|f| now_ms().saturating_sub(f.captured_at_ms) > 650)
            .unwrap_or(true);

        if stale {
            if let Ok(Some((fresh, _, _))) =
                self.capture_frame_once(&kind, hwnd, mon_idx, &label, 2304, 88)
            {
                if let Some(old) = &latest {
                    if luma_change_ratio(&old.luma, &fresh.luma) >= 0.025 {
                        prev = Some(old.clone());
                    }
                }
                latest = Some(fresh.clone());
                if let Ok(mut rt) = self.runtime.lock() {
                    if rt.active {
                        rt.prev_frame = prev.clone();
                        rt.latest_frame = Some(fresh);
                        rt.frame_count += 1;
                    }
                }
            }
        }

        let mut frames = Vec::new();
        if let (Some(p), Some(l)) = (&prev, &latest) {
            if luma_change_ratio(&p.luma, &l.luma) >= 0.025 {
                frames.push(InlineImageFrame {
                    label: format!("{} (just before)", p.label),
                    jpeg_b64: p.jpeg_b64.clone(),
                });
            }
        }
        if let Some(l) = &latest {
            frames.push(InlineImageFrame {
                label: format!("{} (live now)", l.label),
                jpeg_b64: l.jpeg_b64.clone(),
            });
        }

        let wp = if watch_prof.trim().is_empty() {
            None
        } else {
            Some(watch_prof)
        };
        (frames, Some(goal), wp)
    }

    fn run_session_loop(
        self: Arc<Self>,
        gen: u64,
        app: AppHandle,
        sensor_state: Arc<AppSensorState>,
        settings_mgr: Arc<SettingsManager>,
        ai_ctrl: Arc<AiStreamController>,
    ) {
        let mut last_capture_ms = 0u64;

        loop {
            if self.stop_requested.load(Ordering::Relaxed) {
                break;
            }

            let state_snap = match self.runtime.lock() {
                Ok(rt) => {
                    if !rt.active || rt.generation != gen {
                        break;
                    }
                    (
                        rt.paused,
                        rt.minimized,
                        rt.analyzing,
                        rt.target_kind.clone(),
                        rt.target_hwnd,
                        rt.monitor_index,
                        rt.target_label.clone(),
                        rt.show_border,
                        rt.cadence.clone(),
                        rt.goal.clone(),
                        rt.watch_profile_id.clone(),
                        rt.last_change_ms,
                        rt.pending_settle,
                        rt.last_ai_check_ms,
                        rt.recent_whispers.clone(),
                    )
                }
                Err(_) => break,
            };

            let (
                paused,
                was_minimized,
                analyzing,
                kind,
                hwnd,
                mon_idx,
                label,
                show_border,
                cadence,
                goal,
                watch_profile_id,
                last_change_ms,
                pending_settle,
                last_ai_check_ms,
                recent_whispers,
            ) = state_snap;

            #[cfg(windows)]
            if kind == "window" && hwnd != 0 {
                let h = HWND(hwnd as *mut c_void);
                if unsafe { !IsWindow(h).as_bool() } {
                    self.stop(&app, &sensor_state, "window_closed");
                    break;
                }
                let is_min = unsafe { IsIconic(h).as_bool() };
                if is_min != was_minimized {
                    if let Ok(mut rt) = self.runtime.lock() {
                        if rt.generation == gen {
                            rt.minimized = is_min;
                        }
                    }
                    self.emit_state(
                        &app,
                        &sensor_state,
                        if is_min { "minimized" } else { "restored" },
                    );
                }

                // Border overlay self-tracks target_hwnd at 140Hz directly; update high-level state here
                if show_border && !paused && !is_min {
                    if let Some(vr) = get_window_visible_rect(h) {
                        let fg = unsafe { GetForegroundWindow() };
                        let dimmed = fg.0 != h.0;
                        self.set_border(BorderSpec {
                            visible: true,
                            is_window_mode: true,
                            target_hwnd: hwnd,
                            rect: vr,
                            dimmed,
                        });
                    }
                } else {
                    self.set_border(BorderSpec::default());
                }
            } else if show_border && !paused {
                let rect = if let Some(idx) = mon_idx {
                    let mons = enumerate_monitors();
                    mons.get(idx)
                        .map(|m| m.rect)
                        .unwrap_or_else(get_monitor_at_cursor)
                } else {
                    get_monitor_at_cursor()
                };
                self.set_border(BorderSpec {
                    visible: true,
                    is_window_mode: false,
                    target_hwnd: 0,
                    rect,
                    dimmed: false,
                });
            } else {
                self.set_border(BorderSpec::default());
            }

            let t_now = now_ms();
            if !paused && !was_minimized && t_now.saturating_sub(last_capture_ms) >= 1050 {
                last_capture_ms = t_now;
                match self.capture_frame_once(&kind, hwnd, mon_idx, &label, 2048, 86) {
                    Err(e) if e == "window_closed" => {
                        self.stop(&app, &sensor_state, "window_closed");
                        break;
                    }
                    Ok(Some((frame, _rect, _dimmed))) => {
                        let mut should_trigger_watch = false;
                        let mut check_prev_frame: Option<InlineImageFrame> = None;
                        let mut check_curr_frame: Option<InlineImageFrame> = None;

                        if let Ok(mut rt) = self.runtime.lock() {
                            if !rt.active || rt.generation != gen {
                                break;
                            }
                            if let Some(prev_live) = &rt.latest_frame {
                                if luma_change_ratio(&prev_live.luma, &frame.luma) >= 0.014 {
                                    rt.prev_frame = Some(prev_live.clone());
                                }
                            }
                            rt.latest_frame = Some(frame.clone());
                            rt.frame_count += 1;

                            let busy_chatting = ai_ctrl.cancel_flag.load(Ordering::Relaxed)
                                || sensor_state.is_dragging.load(Ordering::Relaxed);
                            let cadence_now = rt.cadence.clone();
                            let last_ai_ms = rt.last_ai_check_ms;
                            let analyzing_now = rt.analyzing;
                            let last_watched = rt.last_watched_luma.clone();
                            let mut prev_live = rt.prev_live_luma.take();
                            let mut chg_ms = rt.last_change_ms;
                            let mut pend = rt.pending_settle;

                            let fire = update_settle_and_should_trigger(
                                &cadence_now,
                                &mut prev_live,
                                last_watched.as_deref(),
                                &frame.luma,
                                &mut chg_ms,
                                &mut pend,
                                last_ai_ms,
                                analyzing_now,
                                busy_chatting,
                                t_now,
                            );
                            rt.prev_live_luma = prev_live;
                            rt.last_change_ms = chg_ms;
                            rt.pending_settle = pend;

                            if fire {
                                rt.analyzing = true;
                                rt.pending_settle = false;
                                rt.last_ai_check_ms = t_now;
                                rt.last_watched_luma = Some(frame.luma.clone());
                                check_prev_frame = rt.last_watched_frame.clone();
                                let inline = InlineImageFrame {
                                    label: label.clone(),
                                    jpeg_b64: frame.jpeg_b64.clone(),
                                };
                                rt.last_watched_frame = Some(inline.clone());
                                check_curr_frame = Some(inline);
                                should_trigger_watch = true;
                            }
                        }

                        if should_trigger_watch {
                            self.emit_state(&app, &sensor_state, "analyzing");
                            if let Some(curr) = check_curr_frame {
                                let mgr_async = Arc::clone(&self);
                                let app_c = app.clone();
                                let ss_c = sensor_state.clone();
                                let sm_c = settings_mgr.clone();
                                let wp = watch_profile_id.clone();
                                let lbl_c = label.clone();
                                let goal_c = goal.clone();
                                let cad_c = cadence.clone();
                                let recent_c = recent_whispers.clone();
                                tauri::async_runtime::spawn(async move {
                                    let cfg = sm_c.get();
                                    let (model, key) = sm_c.resolve_watch_target(if wp.trim().is_empty() {
                                        None
                                    } else {
                                        Some(&wp)
                                    });
                                    let res = if !model.supports_vision {
                                        Err("Watch model has Vision disabled in Settings.".to_string())
                                    } else if key.trim().is_empty()
                                        && !model.base_url.contains("localhost")
                                        && !model.base_url.contains("127.0.0.1")
                                    {
                                        Err("No API key configured for Watch model.".to_string())
                                    } else if let Ok(client) = reqwest::Client::builder()
                                        .timeout(Duration::from_secs(45))
                                        .build()
                                    {
                                        run_screen_watch_check(
                                            &client,
                                            &model,
                                            &key,
                                            &lbl_c,
                                            &goal_c,
                                            &cad_c,
                                            &cfg.ui_language,
                                            &recent_c,
                                            check_prev_frame.as_ref(),
                                            &curr,
                                        )
                                        .await
                                    } else {
                                        Err("Failed to build HTTP client.".to_string())
                                    };

                                    let mut emit_whisper: Option<ShareWhisperPayload> = None;
                                    if let Ok(mut rt) = mgr_async.runtime.lock() {
                                        if rt.generation == gen {
                                            rt.analyzing = false;
                                            match res {
                                                Ok(Some(text)) => {
                                                    rt.last_error.clear();
                                                    let clean = text.trim().to_string();
                                                    if !clean.is_empty() {
                                                        rt.recent_whispers.push(clean.clone());
                                                        if rt.recent_whispers.len() > 4 {
                                                            rt.recent_whispers.remove(0);
                                                        }
                                                        emit_whisper = Some(ShareWhisperPayload {
                                                            id: format!("w_{}", now_ms()),
                                                            text: clean,
                                                            timestamp_ms: now_ms(),
                                                            target_label: lbl_c.clone(),
                                                        });
                                                    }
                                                }
                                                Ok(None) => {
                                                    rt.last_error.clear();
                                                }
                                                Err(err) => {
                                                    rt.last_error = err.chars().take(180).collect();
                                                }
                                            }
                                        }
                                    }
                                    mgr_async.emit_state(&app_c, &ss_c, "analysis_done");
                                    if let Some(w) = emit_whisper {
                                        let _ = app_c.emit("buddy://share-whisper", w);
                                    }
                                });
                            }
                        } else {
                            let _ = (cadence, goal, last_change_ms, pending_settle, last_ai_check_ms, analyzing);
                        }
                    }
                    _ => {}
                }
            }

            std::thread::sleep(Duration::from_millis(260));
        }
    }
}

/// Renders one unified hollow-center feathered ARGB perimeter overlay (`WS_EX_LAYERED | WS_EX_TRANSPARENT`)
/// excluded from screen capture (`WDA_EXCLUDEFROMCAPTURE`) with synchronous 140Hz target-window tracking.
#[cfg(windows)]
fn spawn_border_overlay_thread(desired: Arc<Mutex<BorderSpec>>) {
    std::thread::spawn(move || unsafe {
        let class_name: Vec<u16> = "BuddyShareBorderOverlay\0".encode_utf16().collect();
        let hinst = GetModuleHandleW(None).unwrap_or_default();

        unsafe extern "system" fn border_wndproc(
            hwnd: HWND,
            msg: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        let wc = WNDCLASSW {
            lpfnWndProc: Some(border_wndproc),
            hInstance: hinst.into(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);

        let mut overlay_hwnd: Option<HWND> = None;
        let mut last_spec = BorderSpec::default();
        let mut last_motion: Option<std::time::Instant> = None;
        let mut msg = MSG::default();

        loop {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            let mut cur = desired
                .lock()
                .ok()
                .map(|g| g.clone())
                .unwrap_or_default();

            // Sample live target_hwnd rect directly on this GUI thread at up to 140Hz so the border stays locked to drags.
            if cur.visible && cur.is_window_mode && cur.target_hwnd != 0 {
                let th = HWND(cur.target_hwnd as *mut c_void);
                if !IsWindow(th).as_bool() || IsIconic(th).as_bool() {
                    cur.visible = false;
                } else if let Some(vr) = get_window_visible_rect(th) {
                    cur.rect = vr;
                    cur.dimmed = GetForegroundWindow().0 != th.0;
                }
            }

            if cur != last_spec {
                if cur.rect != last_spec.rect {
                    last_motion = Some(std::time::Instant::now());
                }
                apply_border_overlay(
                    &mut overlay_hwnd,
                    hinst.into(),
                    &class_name,
                    &last_spec,
                    &cur,
                );
                last_spec = cur;
            }

            let moving = last_motion
                .map(|m| m.elapsed() < Duration::from_millis(260))
                .unwrap_or(false);
            let sleep_ms = if moving {
                7
            } else if last_spec.visible {
                24
            } else {
                45
            };
            std::thread::sleep(Duration::from_millis(sleep_ms));
        }

        #[allow(unreachable_code)]
        if let Some(hwnd) = overlay_hwnd {
            if !hwnd.0.is_null() {
                let _ = DestroyWindow(hwnd);
            }
        }
    });
}

#[cfg(windows)]
unsafe fn ensure_border_overlay_hwnd(
    slot: &mut Option<HWND>,
    hinst: windows::Win32::Foundation::HINSTANCE,
    class_name: &[u16],
) -> Option<HWND> {
    if let Some(h) = *slot {
        if !h.0.is_null() && IsWindow(h).as_bool() {
            return Some(h);
        }
    }
    let ex = WS_EX_LAYERED
        | WS_EX_TRANSPARENT
        | WS_EX_TOOLWINDOW
        | WS_EX_NOACTIVATE
        | WS_EX_TOPMOST;
    let Ok(hwnd) = CreateWindowExW(
        ex,
        PCWSTR(class_name.as_ptr()),
        PCWSTR::null(),
        WS_POPUP,
        0,
        0,
        10,
        10,
        None,
        None,
        hinst,
        None,
    ) else {
        return None;
    };
    set_hwnd_capture_excluded(hwnd, true);
    *slot = Some(hwnd);
    Some(hwnd)
}

#[cfg(windows)]
unsafe fn apply_border_overlay(
    hwnd_slot: &mut Option<HWND>,
    hinst: windows::Win32::Foundation::HINSTANCE,
    class_name: &[u16],
    prev: &BorderSpec,
    cur: &BorderSpec,
) {
    if !cur.visible || cur.rect.2 <= 0 || cur.rect.3 <= 0 {
        if prev.visible {
            if let Some(hwnd) = *hwnd_slot {
                if !hwnd.0.is_null() {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
            }
        }
        return;
    }

    let Some(hwnd) = ensure_border_overlay_hwnd(hwnd_slot, hinst, class_name) else {
        return;
    };

    let (x, y, w, h) = cur.rect;
    let alpha_scale = if cur.dimmed { 0.42f32 } else { 1.0f32 };

    let needs_redraw = !prev.visible
        || prev.is_window_mode != cur.is_window_mode
        || prev.dimmed != cur.dimmed
        || prev.rect.2 != w
        || prev.rect.3 != h;

    if !needs_redraw && prev.visible {
        // Synchronous translation on this overlay's owning GUI thread keeps drags locked with zero async drops.
        let _ = SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE,
        );
        return;
    }

    paint_hollow_border(hwnd, x, y, w.max(1), h.max(1), cur.is_window_mode, alpha_scale);
    if !prev.visible {
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

#[inline]
fn compute_border_pixel(
    px: i32,
    py: i32,
    w: i32,
    h: i32,
    thick: i32,
    window_mode: bool,
    alpha_scale: f32,
) -> u32 {
    let edge_px = px.min(w - 1 - px).min(py.min(h - 1 - py));
    if edge_px < 0 || edge_px >= thick {
        return 0;
    }
    let d_from_edge = edge_px as f32 / (thick.max(1) as f32);
    let base_a = if window_mode {
        if d_from_edge < 0.14 {
            0.82
        } else {
            let t = (1.0 - (d_from_edge - 0.14) / 0.86).clamp(0.0, 1.0);
            t.powf(1.35) * 0.62
        }
    } else if d_from_edge < 0.10 {
        0.74
    } else {
        let t = (1.0 - (d_from_edge - 0.10) / 0.90).clamp(0.0, 1.0);
        t.powf(1.30) * 0.60
    };

    let alpha = ((base_a * alpha_scale).clamp(0.0, 1.0) * 255.0).round() as u8;
    let (r, g, b): (u8, u8, u8) = (6, 8, 12);
    let pr = ((r as u16 * alpha as u16) / 255) as u32;
    let pg = ((g as u16 * alpha as u16) / 255) as u32;
    let pb = ((b as u16 * alpha as u16) / 255) as u32;
    ((alpha as u32) << 24) | (pr << 16) | (pg << 8) | pb
}

#[cfg(windows)]
unsafe fn paint_hollow_border(
    hwnd: HWND,
    wx: i32,
    wy: i32,
    w: i32,
    h: i32,
    window_mode: bool,
    alpha_scale: f32,
) {
    let hdc_screen = GetDC(None);
    let hdc_mem = CreateCompatibleDC(hdc_screen);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits_ptr: *mut c_void = std::ptr::null_mut();
    let Ok(hbm) = CreateDIBSection(hdc_mem, &bmi, DIB_RGB_COLORS, &mut bits_ptr, None, 0)
    else {
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(None, hdc_screen);
        return;
    };

    if !bits_ptr.is_null() {
        let px_count = (w as usize) * (h as usize);
        let slice = std::slice::from_raw_parts_mut(bits_ptr as *mut u32, px_count);
        slice.fill(0);

        let thick = if window_mode { 22 } else { 28 };
        let stride = w as usize;
        let top_end = thick.min(h);
        let bot_start = (h - thick).max(top_end);
        let left_end = thick.min(w);
        let right_start = (w - thick).max(left_end);

        for py in 0..top_end {
            let row = (py as usize) * stride;
            for px in 0..w {
                slice[row + (px as usize)] =
                    compute_border_pixel(px, py, w, h, thick, window_mode, alpha_scale);
            }
        }
        for py in bot_start..h {
            let row = (py as usize) * stride;
            for px in 0..w {
                slice[row + (px as usize)] =
                    compute_border_pixel(px, py, w, h, thick, window_mode, alpha_scale);
            }
        }
        for py in top_end..bot_start {
            let row = (py as usize) * stride;
            for px in 0..left_end {
                slice[row + (px as usize)] =
                    compute_border_pixel(px, py, w, h, thick, window_mode, alpha_scale);
            }
            for px in right_start..w {
                slice[row + (px as usize)] =
                    compute_border_pixel(px, py, w, h, thick, window_mode, alpha_scale);
            }
        }
    }

    let old = SelectObject(hdc_mem, hbm);
    let pt_src = POINT { x: 0, y: 0 };
    let pt_dst = POINT { x: wx, y: wy };
    let size = SIZE { cx: w, cy: h };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };

    let _ = UpdateLayeredWindow(
        hwnd,
        hdc_screen,
        Some(&pt_dst),
        Some(&size),
        hdc_mem,
        Some(&pt_src),
        windows::Win32::Foundation::COLORREF(0),
        Some(&blend),
        ULW_ALPHA,
    );

    SelectObject(hdc_mem, old);
    let _ = DeleteObject(hbm);
    let _ = DeleteDC(hdc_mem);
    ReleaseDC(None, hdc_screen);
}

/// Registers global Alt + Shift + P hotkey to toggle Screen Share privacy pause/resume.
pub fn start_share_hotkey_listener(
    app: AppHandle,
    sensor_state: Arc<AppSensorState>,
    share_mgr: Arc<ScreenShareManager>,
) {
    #[cfg(windows)]
    std::thread::spawn(move || unsafe {
        const HOTKEY_PAUSE_ID: i32 = 102;
        // Alt + Shift + P (VK_P = 0x50)
        if RegisterHotKey(None, HOTKEY_PAUSE_ID, MOD_ALT | MOD_SHIFT | MOD_NOREPEAT, 0x50).is_err() {
            return;
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_HOTKEY && msg.wParam.0 as i32 == HOTKEY_PAUSE_ID {
                share_mgr.toggle_pause(&app, &sensor_state);
            }
        }
        let _ = UnregisterHotKey(None, HOTKEY_PAUSE_ID);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luma_diff_detects_static_vs_changed_frames() {
        let a = vec![100u8; (LUMA_W * LUMA_H) as usize];
        let b = vec![100u8; (LUMA_W * LUMA_H) as usize];
        assert!(luma_change_ratio(&a, &b) < 0.001);

        let mut c = a.clone();
        for px in c.iter_mut().take(300) {
            *px = 200;
        }
        assert!(luma_change_ratio(&a, &c) > 0.20);
    }

    #[test]
    fn cadence_timings_match_modes() {
        assert_eq!(resolve_cadence_timings("balanced"), (2800, 35_000));
        assert_eq!(resolve_cadence_timings("active"), (1500, 10_000));
        assert_eq!(resolve_cadence_timings("quiet").0, u64::MAX);
    }

    #[test]
    fn settle_state_machine_fires_without_deadlocking() {
        let frame_a = vec![90u8; (LUMA_W * LUMA_H) as usize];
        let mut frame_b = frame_a.clone();
        for px in frame_b.iter_mut().take(300) {
            *px = 210;
        }

        let mut prev_live: Option<Vec<u8>> = None;
        let mut last_chg = 0u64;
        let mut pend = false;

        // First captured frame initiates settle clock cleanly
        assert!(!update_settle_and_should_trigger(
            "active",
            &mut prev_live,
            None,
            &frame_a,
            &mut last_chg,
            &mut pend,
            0,
            false,
            false,
            10_000,
        ));
        assert!(pend);
        assert_eq!(last_chg, 10_000);

        // Static frames advance past 1500ms settle and trigger!
        assert!(update_settle_and_should_trigger(
            "active",
            &mut prev_live,
            None,
            &frame_a,
            &mut last_chg,
            &mut pend,
            0,
            false,
            false,
            12_000,
        ));

        // After first AI check watched frame_a, moving to frame_b starts new settle
        assert!(!update_settle_and_should_trigger(
            "active",
            &mut prev_live,
            Some(&frame_a),
            &frame_b,
            &mut last_chg,
            &mut pend,
            12_000,
            false,
            false,
            23_000,
        ));
        assert_eq!(last_chg, 23_000);

        // Holding steady on frame_b DOES NOT reset last_chg and triggers cleanly once settled!
        assert!(update_settle_and_should_trigger(
            "active",
            &mut prev_live,
            Some(&frame_a),
            &frame_b,
            &mut last_chg,
            &mut pend,
            12_000,
            false,
            false,
            25_000,
        ));
    }

    #[test]
    fn hollow_border_covers_full_perimeter_and_corners_seamlessly() {
        let (w, h, thick) = (400, 300, 22);
        let alpha = |px: i32, py: i32| -> u8 {
            (compute_border_pixel(px, py, w, h, thick, true, 1.0) >> 24) as u8
        };

        // Every single outer edge pixel (including corners and near-corner transitions) has equal full alpha
        let corner_a = alpha(0, 0);
        assert!(corner_a > 190);
        for px in 0..w {
            assert_eq!(alpha(px, 0), corner_a);
            assert_eq!(alpha(px, h - 1), corner_a);
        }
        for py in 0..h {
            assert_eq!(alpha(0, py), corner_a);
            assert_eq!(alpha(w - 1, py), corner_a);
        }

        // Interior beyond `thick` stays completely transparent
        assert_eq!(alpha(w / 2, h / 2), 0);
        assert_eq!(alpha(thick, thick), 0);
    }
}
