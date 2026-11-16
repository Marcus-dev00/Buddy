//! Multi-DPI built-in region screenshot capture (Alt + Shift + S) via Win32 GDI BitBlt.

use crate::db::MemoryDb;
use crate::media_utils::{foreground_app_name, save_image_memory, write_image_to_clipboard, SelfCopyGuard};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use image::{DynamicImage, ImageFormat, RgbaImage};
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
};
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_NOREPEAT, MOD_SHIFT,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, GetSystemMetrics, MSG, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, WM_HOTKEY,
};

pub struct SnipSession {
    pub img: DynamicImage,
    pub preview_data_url: String,
    pub app_name: String,
}

#[derive(Default)]
pub struct SnipState {
    pub current: Mutex<Option<SnipSession>>,
}

#[cfg(windows)]
fn capture_virtual_screen() -> Option<(i32, i32, i32, i32, DynamicImage)> {
    unsafe {
        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        if vw <= 0 || vh <= 0 {
            return None;
        }

        let hdc_screen = GetDC(None);
        if hdc_screen.is_invalid() {
            return None;
        }
        let hdc_mem = CreateCompatibleDC(hdc_screen);
        let hbm = CreateCompatibleBitmap(hdc_screen, vw, vh);
        let old = SelectObject(hdc_mem, hbm);

        let blt_ok = BitBlt(hdc_mem, 0, 0, vw, vh, hdc_screen, vx, vy, SRCCOPY).is_ok();

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: vw,
                biHeight: -vh, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bgra = vec![0u8; (vw as usize) * (vh as usize) * 4];
        let lines = if blt_ok {
            GetDIBits(
                hdc_mem,
                hbm,
                0,
                vh as u32,
                Some(bgra.as_mut_ptr() as *mut _),
                &mut bmi,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };

        SelectObject(hdc_mem, old);
        let _ = DeleteObject(hbm);
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(None, hdc_screen);

        if lines == 0 {
            return None;
        }

        // BGRA -> RGBA
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }

        let rgba = RgbaImage::from_raw(vw as u32, vh as u32, bgra)?;
        Some((vx, vy, vw, vh, DynamicImage::ImageRgba8(rgba)))
    }
}

pub fn begin_snip_capture(app: &AppHandle, snip_state: &Arc<SnipState>) {
    #[cfg(windows)]
    {
        let app_name = foreground_app_name();
        let Some((vx, vy, vw, vh, img)) = capture_virtual_screen() else {
            return;
        };

        // Encode fast JPEG preview for the frozen screen overlay
        let mut jpeg_buf = Vec::new();
        if img
            .to_rgb8()
            .write_to(&mut Cursor::new(&mut jpeg_buf), ImageFormat::Jpeg)
            .is_err()
        {
            return;
        }
        let preview_data_url = format!("data:image/jpeg;base64,{}", B64.encode(&jpeg_buf));

        if let Ok(mut lock) = snip_state.current.lock() {
            *lock = Some(SnipSession {
                img,
                preview_data_url,
                app_name,
            });
        }

        if let Some(existing) = app.get_webview_window("snip") {
            let _ = existing.destroy();
        }

        if let Ok(win) = WebviewWindowBuilder::new(
            app,
            "snip",
            WebviewUrl::App("index.html#snip".into()),
        )
        .title("Buddy Snip")
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .build()
        {
            let _ = win.set_position(PhysicalPosition::new(vx, vy));
            let _ = win.set_size(PhysicalSize::new(vw as u32, vh as u32));
            let _ = win.set_focus();
        }
    }
}

#[tauri::command]
pub fn get_snip_preview(state: tauri::State<'_, Arc<SnipState>>) -> Option<(String, u32, u32)> {
    let lock = state.current.lock().ok()?;
    let sess = lock.as_ref()?;
    Some((
        sess.preview_data_url.clone(),
        sess.img.width(),
        sess.img.height(),
    ))
}

#[tauri::command]
pub fn finish_snip(
    app: AppHandle,
    snip_state: tauri::State<'_, Arc<SnipState>>,
    db: tauri::State<'_, Arc<MemoryDb>>,
    guard: tauri::State<'_, Arc<SelfCopyGuard>>,
    rx: f64,
    ry: f64,
    rw: f64,
    rh: f64,
) {
    let session = snip_state.current.lock().ok().and_then(|mut l| l.take());
    if let Some(win) = app.get_webview_window("snip") {
        let _ = win.close();
    }

    let Some(sess) = session else {
        return;
    };

    let iw = sess.img.width() as f64;
    let ih = sess.img.height() as f64;
    let x = (rx.clamp(0.0, 1.0) * iw).round() as u32;
    let y = (ry.clamp(0.0, 1.0) * ih).round() as u32;
    let w = ((rw.clamp(0.0, 1.0) * iw).round() as u32).max(2).min(sess.img.width().saturating_sub(x));
    let h = ((rh.clamp(0.0, 1.0) * ih).round() as u32).max(2).min(sess.img.height().saturating_sub(y));

    if w < 4 || h < 4 {
        return;
    }

    let cropped = sess.img.crop_imm(x, y, w, h);
    #[cfg(windows)]
    let _ = write_image_to_clipboard(&cropped, &guard);

    if let Ok((item, _)) = save_image_memory(&db, &cropped, "screenshot", &sess.app_name) {
        let _ = app.emit("buddy://memory-updated", item);
        let _ = app.emit("buddy://capture", ());
    }
}

#[tauri::command]
pub fn cancel_snip(app: AppHandle, snip_state: tauri::State<'_, Arc<SnipState>>) {
    if let Ok(mut lock) = snip_state.current.lock() {
        *lock = None;
    }
    if let Some(win) = app.get_webview_window("snip") {
        let _ = win.close();
    }
}

/// Registers Alt + Shift + S global shortcut via Win32 RegisterHotKey.
pub fn start_snip_hotkey_listener(app: AppHandle, snip_state: Arc<SnipState>) {
    #[cfg(windows)]
    std::thread::spawn(move || unsafe {
        const HOTKEY_ID: i32 = 101;
        // VK_S = 0x53
        if RegisterHotKey(None, HOTKEY_ID, MOD_ALT | MOD_SHIFT | MOD_NOREPEAT, 0x53).is_err() {
            return;
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_HOTKEY && msg.wParam.0 as i32 == HOTKEY_ID {
                begin_snip_capture(&app, &snip_state);
            }
        }
        let _ = UnregisterHotKey(None, HOTKEY_ID);
    });
}
