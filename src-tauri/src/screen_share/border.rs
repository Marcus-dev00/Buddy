//! Capture-excluded click-through Win32 hollow-border overlay around the shared monitor or window.

#[cfg(windows)]
use crate::win32_sensors::{get_window_visible_rect, set_hwnd_capture_excluded};
#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use std::sync::{Arc, Mutex};
#[cfg(windows)]
use std::time::Duration;

#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
};
#[cfg(windows)]
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
    IsIconic, IsWindow, PeekMessageW, RegisterClassW, SetWindowPos, ShowWindow, TranslateMessage,
    UpdateLayeredWindow, HWND_TOPMOST, MSG, PM_REMOVE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

#[derive(Clone, PartialEq)]
pub(super) struct BorderSpec {
    pub visible: bool,
    pub is_window_mode: bool,
    pub target_hwnd: isize,
    pub rect: (i32, i32, i32, i32), // (left, top, width, height) of target monitor or target window
    pub dimmed: bool,               // true when target window is not foreground
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

/// Renders one unified hollow-center feathered ARGB perimeter overlay (`WS_EX_LAYERED | WS_EX_TRANSPARENT`)
/// excluded from screen capture (`WDA_EXCLUDEFROMCAPTURE`) with synchronous 140Hz target-window tracking.
#[cfg(windows)]
pub(super) fn spawn_border_overlay_thread(desired: Arc<Mutex<BorderSpec>>) {
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
    hinst: HINSTANCE,
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
    hinst: HINSTANCE,
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
#[cfg(any(windows, test))]
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
    let Ok(hbm) = CreateDIBSection(hdc_mem, &bmi, DIB_RGB_COLORS, &mut bits_ptr, None, 0) else {
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
        COLORREF(0),
        Some(&blend),
        ULW_ALPHA,
    );

    SelectObject(hdc_mem, old);
    let _ = DeleteObject(hbm);
    let _ = DeleteDC(hdc_mem);
    ReleaseDC(None, hdc_screen);
}

#[cfg(test)]
mod tests {
    use super::*;

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
