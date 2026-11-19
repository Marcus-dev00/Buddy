//! Display/window enumeration, Win32 BitBlt + PrintWindow offscreen capture,
//! and luma thumbnail settle-debounce gating for proactive Screen Share watch.

use super::ShareTargetInfo;
#[cfg(windows)]
use crate::win32_sensors::get_window_visible_rect;
#[cfg(windows)]
use image::{DynamicImage, RgbaImage};
#[cfg(windows)]
use std::ffi::c_void;

#[cfg(windows)]
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT};
#[cfg(windows)]
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
    EnumDisplayMonitors, GetDC, GetDIBits, GetMonitorInfoW, MonitorFromPoint, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC, HMONITOR,
    MONITORINFOEXW, MONITOR_DEFAULTTONEAREST, SRCCOPY,
};
#[cfg(windows)]
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
#[cfg(windows)]
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetCursorPos, GetWindow, GetWindowLongW, GetWindowRect, GetWindowTextLengthW,
    GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, GWL_EXSTYLE,
    GW_OWNER, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

#[cfg(any(windows, test))]
pub(super) const LUMA_W: u32 = 48;
#[cfg(any(windows, test))]
pub(super) const LUMA_H: u32 = 27;

#[cfg(windows)]
#[derive(Clone)]
pub(super) struct MonitorRectEntry {
    pub device_name: String,
    pub rect: (i32, i32, i32, i32), // left, top, width, height
    pub is_primary: bool,
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
pub(super) fn enumerate_monitors() -> Vec<MonitorRectEntry> {
    let mut list: Vec<MonitorRectEntry> = Vec::new();
    unsafe {
        let ptr = &mut list as *mut Vec<MonitorRectEntry> as isize;
        let _ = EnumDisplayMonitors(None, None, Some(enum_monitors_proc), LPARAM(ptr));
    }
    list
}

#[cfg(windows)]
pub(super) fn get_monitor_at_cursor() -> (i32, i32, i32, i32) {
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
pub(super) fn resolve_monitor_rect(mon_idx: Option<usize>) -> (i32, i32, i32, i32) {
    if let Some(idx) = mon_idx {
        let mons = enumerate_monitors();
        mons.get(idx)
            .map(|m| m.rect)
            .unwrap_or_else(get_monitor_at_cursor)
    } else {
        get_monitor_at_cursor()
    }
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
pub(super) fn bitblt_rect_image(vx: i32, vy: i32, vw: i32, vh: i32) -> Option<DynamicImage> {
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

pub fn capture_primary_desktop_jpeg_b64(max_dim: u32, quality: u8) -> Result<String, String> {
    #[cfg(windows)]
    {
        let monitors = enumerate_monitors();
        let target = monitors
            .iter()
            .find(|m| m.is_primary)
            .or_else(|| monitors.first())
            .ok_or_else(|| "No active display monitor found.".to_string())?;
        let (x, y, w, h) = target.rect;
        let img = bitblt_rect_image(x, y, w, h)
            .ok_or_else(|| "Failed to capture desktop frame.".to_string())?;
        crate::ai_client::encode_dynamic_image_jpeg_base64(&img, max_dim, quality)
    }
    #[cfg(not(windows))]
    {
        let _ = (max_dim, quality);
        Err("Desktop screenshot capture is supported on Windows only.".to_string())
    }
}

#[cfg(windows)]
unsafe fn read_hbitmap_to_dynamic(hdc: HDC, hbm: HBITMAP, w: i32, h: i32) -> Option<DynamicImage> {
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
pub(super) fn capture_hwnd_image(hwnd: HWND) -> Option<DynamicImage> {
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

#[cfg(windows)]
pub(super) fn compute_luma_thumb(img: &DynamicImage) -> Vec<u8> {
    let tiny = img
        .resize_exact(LUMA_W, LUMA_H, image::imageops::FilterType::Triangle)
        .to_luma8();
    tiny.into_raw()
}

/// Returns fraction of pixels changed significantly (0.0..=1.0).
pub(super) fn luma_change_ratio(a: &[u8], b: &[u8]) -> f32 {
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

pub(super) fn resolve_cadence_timings(cadence: &str) -> (u64, u64) {
    match cadence {
        "active" => (1500, 10_000),
        "quiet" => (u64::MAX, u64::MAX),
        _ => (2800, 35_000), // balanced default
    }
}

/// Computes whether live screen motion has settled and whether an AI watch check should trigger.
#[allow(clippy::too_many_arguments)]
pub(super) fn update_settle_and_should_trigger(
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
}
