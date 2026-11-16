//! Native Win32 Layered Window utilities:
//! 1) Pin to Screen (`WS_EX_LAYERED` topmost draggable/zoomable/opacity-adjustable image pins)
//! 2) Native Multi-Monitor Screen Color Picker (magnified pixel loupe -> HEX/RGB/HSL)

use crate::media_utils::{write_text_to_clipboard, SelfCopyGuard};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use image::{imageops::FilterType, DynamicImage, RgbaImage};
use serde::Serialize;
use std::path::Path;
use std::sync::{Arc, Mutex, Once};
use tauri::{AppHandle, Emitter};

#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateDIBSection, CreatePen,
    CreateSolidBrush, DeleteDC, DeleteObject, FillRect, GetDC, GetDIBits, InvalidateRect,
    Rectangle, ReleaseDC, SelectObject, SetBkMode, SetDIBitsToDevice, SetTextColor, StretchBlt,
    TextOutW, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION,
    DIB_RGB_COLORS, HBRUSH, HPEN, PS_SOLID, SRCCOPY, TRANSPARENT,
};
#[cfg(windows)]
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, VK_ESCAPE};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetSystemMetrics, GetWindowLongPtrW, LoadCursorW, PostMessageW, PostQuitMessage,
    RegisterClassW, SendMessageW, SetWindowLongPtrW, SetWindowPos,
    TranslateMessage, UpdateLayeredWindow, CS_DBLCLKS, GWLP_USERDATA, HTCAPTION, HWND_TOPMOST,
    IDC_ARROW, IDC_CROSS, MSG, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, ULW_ALPHA,
    WM_CLOSE, WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_NCLBUTTONDOWN, WM_PAINT, WM_RBUTTONUP, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

#[cfg(windows)]
const MK_CTRL_FLAG: u32 = 0x0008;

#[derive(Clone, Serialize)]
pub struct ColorPickedPayload {
    pub hex: String,
    pub rgb: String,
    pub hsl: String,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

static ACTIVE_PINS: Mutex<Vec<isize>> = Mutex::new(Vec::new());
static PIN_CLASS_ONCE: Once = Once::new();
static PICKER_CLASS_ONCE: Once = Once::new();

pub fn load_dynamic_image_from_source(source: &str) -> Result<DynamicImage, String> {
    let trimmed = source.trim();
    if let Some(idx) = trimmed.find(";base64,") {
        let b64_part = &trimmed[idx + 8..];
        let bytes = B64.decode(b64_part).map_err(|e| e.to_string())?;
        return image::load_from_memory(&bytes).map_err(|e| e.to_string());
    }
    let path = Path::new(trimmed);
    if !path.exists() {
        return Err(format!("Image file not found: {trimmed}"));
    }
    image::open(path).map_err(|e| e.to_string())
}

pub fn format_color_values(r: u8, g: u8, b: u8) -> ColorPickedPayload {
    let hex = format!("#{:02X}{:02X}{:02X}", r, g, b);
    let rgb = format!("rgb({}, {}, {})", r, g, b);

    let rf = r as f64 / 255.0;
    let gf = g as f64 / 255.0;
    let bf = b as f64 / 255.0;
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let l = (max + min) / 2.0;
    let (h, s) = if (max - min).abs() < 1e-6 {
        (0.0, 0.0)
    } else {
        let d = max - min;
        let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
        let mut h = if (max - rf).abs() < 1e-6 {
            (gf - bf) / d + (if gf < bf { 6.0 } else { 0.0 })
        } else if (max - gf).abs() < 1e-6 {
            (bf - rf) / d + 2.0
        } else {
            (rf - gf) / d + 4.0
        };
        h /= 6.0;
        (h * 360.0, s)
    };
    let hsl = format!(
        "hsl({}, {}%, {}%)",
        h.round() as i32,
        (s * 100.0).round() as i32,
        (l * 100.0).round() as i32
    );

    ColorPickedPayload {
        hex,
        rgb,
        hsl,
        r,
        g,
        b,
    }
}

#[cfg(windows)]
struct PinWindowState {
    img: RgbaImage,
    base_w: i32,
    base_h: i32,
    scale: f64,
    opacity: u8,
}

#[cfg(windows)]
fn render_pin_layered(hwnd: HWND, st: &PinWindowState) {
    unsafe {
        let target_w = ((st.base_w as f64) * st.scale).round().clamp(80.0, 1800.0) as i32;
        let target_h = ((st.base_h as f64) * st.scale).round().clamp(60.0, 1400.0) as i32;

        let resized = if target_w as u32 == st.img.width() && target_h as u32 == st.img.height() {
            st.img.clone()
        } else {
            image::imageops::resize(&st.img, target_w as u32, target_h as u32, FilterType::Triangle)
        };

        let hdc_screen = GetDC(None);
        let hdc_mem = CreateCompatibleDC(hdc_screen);

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: target_w,
                biHeight: -target_h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits_ptr: *mut c_void = std::ptr::null_mut();
        let Ok(hbm) = CreateDIBSection(hdc_screen, &bmi, DIB_RGB_COLORS, &mut bits_ptr, None, 0) else {
            let _ = DeleteDC(hdc_mem);
            ReleaseDC(None, hdc_screen);
            return;
        };

        if !bits_ptr.is_null() {
            let len = (target_w as usize) * (target_h as usize) * 4;
            let dst = std::slice::from_raw_parts_mut(bits_ptr as *mut u8, len);
            let raw = resized.as_raw();
            for y in 0..target_h {
                for x in 0..target_w {
                    let i = ((y as usize) * (target_w as usize) + (x as usize)) * 4;
                    let is_border = x == 0 || y == 0 || x == target_w - 1 || y == target_h - 1;
                    if is_border {
                        // Crisp translucent 1px white frame around pinned image
                        dst[i] = 255;
                        dst[i + 1] = 255;
                        dst[i + 2] = 255;
                        dst[i + 3] = 175;
                    } else {
                        let a = raw[i + 3] as u16;
                        dst[i] = ((raw[i + 2] as u16 * a) / 255) as u8;
                        dst[i + 1] = ((raw[i + 1] as u16 * a) / 255) as u8;
                        dst[i + 2] = ((raw[i] as u16 * a) / 255) as u8;
                        dst[i + 3] = a as u8;
                    }
                }
            }
        }

        let old_bm = SelectObject(hdc_mem, hbm);
        let pt_src = POINT { x: 0, y: 0 };
        let sz = SIZE {
            cx: target_w,
            cy: target_h,
        };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: st.opacity,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };

        let _ = SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            target_w,
            target_h,
            SWP_NOMOVE | SWP_NOACTIVATE,
        );
        let _ = UpdateLayeredWindow(
            hwnd,
            hdc_screen,
            None,
            Some(&sz),
            hdc_mem,
            Some(&pt_src),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );

        SelectObject(hdc_mem, old_bm);
        let _ = DeleteObject(hbm);
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(None, hdc_screen);
    }
}

#[cfg(windows)]
unsafe extern "system" fn pin_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_LBUTTONDOWN => {
            let _ = ReleaseCapture();
            SendMessageW(hwnd, WM_NCLBUTTONDOWN, WPARAM(HTCAPTION as usize), LPARAM(0));
            LRESULT(0)
        }
        WM_LBUTTONDBLCLK | WM_RBUTTONUP => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 == VK_ESCAPE.0 as usize {
                let _ = DestroyWindow(hwnd);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_MOUSEWHEEL => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinWindowState;
            if !ptr.is_null() {
                let st = &mut *ptr;
                let delta = ((wparam.0 >> 16) as u16) as i16;
                let keys = (wparam.0 & 0xffff) as u32;
                let steps = (delta as f64) / 120.0;
                if (keys & MK_CTRL_FLAG) != 0 {
                    let next_op = (st.opacity as f64 + steps * 20.0).clamp(64.0, 255.0);
                    st.opacity = next_op.round() as u8;
                } else {
                    let next_scale = (st.scale + steps * 0.12).clamp(0.30, 3.0);
                    st.scale = next_scale;
                }
                render_pin_layered(hwnd, st);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinWindowState;
            if !ptr.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(ptr));
            }
            if let Ok(mut list) = ACTIVE_PINS.lock() {
                list.retain(|&h| h != hwnd.0 as isize);
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub fn pin_image_to_screen(source: &str) -> Result<(), String> {
    let dyn_img = load_dynamic_image_from_source(source)?;
    let rgba = dyn_img.to_rgba8();
    let orig_w = rgba.width() as i32;
    let orig_h = rgba.height() as i32;
    if orig_w <= 0 || orig_h <= 0 {
        return Err("Empty image dimensions".into());
    }

    let max_dim = orig_w.max(orig_h) as f64;
    let init_scale = if max_dim > 460.0 { 460.0 / max_dim } else { 1.0 };

    #[cfg(windows)]
    {
        std::thread::spawn(move || unsafe {
            let class_name: Vec<u16> = "BuddyPinWindowClass\0".encode_utf16().collect();
            let hinst = GetModuleHandleW(None).unwrap_or_default();

            PIN_CLASS_ONCE.call_once(|| {
                let wc = WNDCLASSW {
                    style: CS_DBLCLKS,
                    lpfnWndProc: Some(pin_wnd_proc),
                    hInstance: hinst.into(),
                    hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                    lpszClassName: PCWSTR(class_name.as_ptr()),
                    ..Default::default()
                };
                RegisterClassW(&wc);
            });

            let state = Box::new(PinWindowState {
                img: rgba,
                base_w: orig_w,
                base_h: orig_h,
                scale: init_scale,
                opacity: 250,
            });
            let w = ((orig_w as f64) * init_scale).round() as i32;
            let h = ((orig_h as f64) * init_scale).round() as i32;
            let sx = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let sy = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let sw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let sh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            let px = sx + (sw - w) / 2;
            let py = sy + (sh - h) / 2;

            let title: Vec<u16> = "Buddy Pin\0".encode_utf16().collect();
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
                PCWSTR(class_name.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_POPUP | WS_VISIBLE,
                px,
                py,
                w,
                h,
                None,
                None,
                hinst,
                None,
            )
            .unwrap_or_default();

            if hwnd.0.is_null() {
                drop(state);
                return;
            }

            let raw_ptr = Box::into_raw(state);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw_ptr as isize);
            if let Ok(mut list) = ACTIVE_PINS.lock() {
                list.push(hwnd.0 as isize);
            }
            render_pin_layered(hwnd, &*raw_ptr);

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
    }

    Ok(())
}

pub fn close_all_pins() {
    #[cfg(windows)]
    if let Ok(list) = ACTIVE_PINS.lock() {
        for &raw in list.iter() {
            unsafe {
                let _ = PostMessageW(HWND(raw as *mut c_void), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }
}

#[cfg(windows)]
struct ColorPickerState {
    bgra: Vec<u8>,
    vw: i32,
    vh: i32,
    cursor_x: i32,
    cursor_y: i32,
    app: AppHandle,
    guard: Arc<SelfCopyGuard>,
}

#[cfg(windows)]
unsafe extern "system" fn picker_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEMOVE => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ColorPickerState;
            if !ptr.is_null() {
                let st = &mut *ptr;
                st.cursor_x = (lparam.0 & 0xffff) as i16 as i32;
                st.cursor_y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ColorPickerState;
            let mut ps = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
            let hdc = windows::Win32::Graphics::Gdi::BeginPaint(hwnd, &mut ps);
            if !ptr.is_null() {
                let st = &*ptr;
                let hdc_buf = CreateCompatibleDC(hdc);
                let hbm_buf = CreateCompatibleBitmap(hdc, st.vw, st.vh);
                let old_buf = SelectObject(hdc_buf, hbm_buf);

                let bmi = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: st.vw,
                        biHeight: -st.vh,
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                SetDIBitsToDevice(
                    hdc_buf,
                    0,
                    0,
                    st.vw as u32,
                    st.vh as u32,
                    0,
                    0,
                    0,
                    st.vh as u32,
                    st.bgra.as_ptr() as *const _,
                    &bmi,
                    DIB_RGB_COLORS,
                );

                // Draw crisp 9x9 pixel loupe HUD offset near cursor
                let cx = st.cursor_x.clamp(0, st.vw - 1);
                let cy = st.cursor_y.clamp(0, st.vh - 1);
                let hud_w = 132;
                let hud_h = 152;
                let hx = if cx + 20 + hud_w > st.vw { cx - 20 - hud_w } else { cx + 20 };
                let hy = if cy + 20 + hud_h > st.vh { cy - 20 - hud_h } else { cy + 20 };

                let dark_brush: HBRUSH = CreateSolidBrush(COLORREF(0x00120F0E));
                let border_pen: HPEN = CreatePen(PS_SOLID, 1, COLORREF(0x0055504E));
                let old_br = SelectObject(hdc_buf, dark_brush);
                let old_pen = SelectObject(hdc_buf, border_pen);
                let _ = Rectangle(hdc_buf, hx, hy, hx + hud_w, hy + hud_h);
                SelectObject(hdc_buf, old_br);
                SelectObject(hdc_buf, old_pen);
                let _ = DeleteObject(dark_brush);
                let _ = DeleteObject(border_pen);

                let zoom_box = 114;
                let _ = StretchBlt(
                    hdc_buf,
                    hx + 9,
                    hy + 9,
                    zoom_box,
                    zoom_box,
                    hdc_buf,
                    (cx - 5).max(0),
                    (cy - 5).max(0),
                    11,
                    11,
                    SRCCOPY,
                );

                // Crosshair center pixel indicator inside zoom box
                let mid_pen = CreatePen(PS_SOLID, 2, COLORREF(0x00FFFFFF));
                let op2 = SelectObject(hdc_buf, mid_pen);
                let px_box = hx + 9 + (zoom_box / 2) - 5;
                let py_box = hy + 9 + (zoom_box / 2) - 5;
                let null_br = windows::Win32::Graphics::Gdi::GetStockObject(
                    windows::Win32::Graphics::Gdi::NULL_BRUSH,
                );
                let ob2 = SelectObject(hdc_buf, null_br);
                let _ = Rectangle(hdc_buf, px_box, py_box, px_box + 10, py_box + 10);
                SelectObject(hdc_buf, ob2);
                SelectObject(hdc_buf, op2);
                let _ = DeleteObject(mid_pen);

                // Sample RGB at exact cursor pixel
                let idx = ((cy as usize) * (st.vw as usize) + (cx as usize)) * 4;
                let b = st.bgra.get(idx).copied().unwrap_or(0);
                let g = st.bgra.get(idx + 1).copied().unwrap_or(0);
                let r = st.bgra.get(idx + 2).copied().unwrap_or(0);

                // Swatch dot + HEX readout
                let sw_brush = CreateSolidBrush(COLORREF(((b as u32) << 16) | ((g as u32) << 8) | (r as u32)));
                let _ = FillRect(
                    hdc_buf,
                    &RECT {
                        left: hx + 10,
                        top: hy + 130,
                        right: hx + 26,
                        bottom: hy + 144,
                    },
                    sw_brush,
                );
                let _ = DeleteObject(sw_brush);

                SetBkMode(hdc_buf, TRANSPARENT);
                SetTextColor(hdc_buf, COLORREF(0x00FFFFFF));
                let label: Vec<u16> = format!("#{:02X}{:02X}{:02X}", r, g, b)
                    .encode_utf16()
                    .collect();
                let _ = TextOutW(hdc_buf, hx + 32, hy + 129, &label);

                let _ = BitBlt(hdc, 0, 0, st.vw, st.vh, hdc_buf, 0, 0, SRCCOPY);
                SelectObject(hdc_buf, old_buf);
                let _ = DeleteObject(hbm_buf);
                let _ = DeleteDC(hdc_buf);
            }
            let _ = windows::Win32::Graphics::Gdi::EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ColorPickerState;
            if !ptr.is_null() {
                let st = &*ptr;
                let cx = ((lparam.0 & 0xffff) as i16 as i32).clamp(0, st.vw - 1);
                let cy = (((lparam.0 >> 16) & 0xffff) as i16 as i32).clamp(0, st.vh - 1);
                let idx = ((cy as usize) * (st.vw as usize) + (cx as usize)) * 4;
                let b = st.bgra.get(idx).copied().unwrap_or(0);
                let g = st.bgra.get(idx + 1).copied().unwrap_or(0);
                let r = st.bgra.get(idx + 2).copied().unwrap_or(0);
                let payload = format_color_values(r, g, b);
                let _ = write_text_to_clipboard(&payload.hex, &st.guard);
                let _ = st.app.emit("buddy://color-picked", payload);
            }
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 == VK_ESCAPE.0 as usize {
                let _ = DestroyWindow(hwnd);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_DESTROY => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ColorPickerState;
            if !ptr.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(ptr));
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub fn start_screen_color_picker(app: AppHandle, guard: Arc<SelfCopyGuard>) -> Result<(), String> {
    #[cfg(windows)]
    {
        std::thread::spawn(move || unsafe {
            let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            if vw <= 0 || vh <= 0 {
                return;
            }

            let hdc_screen = GetDC(None);
            let hdc_mem = CreateCompatibleDC(hdc_screen);
            let hbm = CreateCompatibleBitmap(hdc_screen, vw, vh);
            let old = SelectObject(hdc_mem, hbm);
            let blt_ok = BitBlt(hdc_mem, 0, 0, vw, vh, hdc_screen, vx, vy, SRCCOPY).is_ok();

            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: vw,
                    biHeight: -vh,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bgra = vec![0u8; (vw as usize) * (vh as usize) * 4];
            if blt_ok {
                GetDIBits(
                    hdc_mem,
                    hbm,
                    0,
                    vh as u32,
                    Some(bgra.as_mut_ptr() as *mut _),
                    &mut bmi,
                    DIB_RGB_COLORS,
                );
            }
            SelectObject(hdc_mem, old);
            let _ = DeleteObject(hbm);
            let _ = DeleteDC(hdc_mem);
            ReleaseDC(None, hdc_screen);
            if !blt_ok {
                return;
            }

            let class_name: Vec<u16> = "BuddyColorPickerClass\0".encode_utf16().collect();
            let hinst = GetModuleHandleW(None).unwrap_or_default();
            PICKER_CLASS_ONCE.call_once(|| {
                let wc = WNDCLASSW {
                    lpfnWndProc: Some(picker_wnd_proc),
                    hInstance: hinst.into(),
                    hCursor: LoadCursorW(None, IDC_CROSS).unwrap_or_default(),
                    lpszClassName: PCWSTR(class_name.as_ptr()),
                    ..Default::default()
                };
                RegisterClassW(&wc);
            });

            let mut pt = POINT::default();
            let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt);
            let state = Box::new(ColorPickerState {
                bgra,
                vw,
                vh,
                cursor_x: pt.x - vx,
                cursor_y: pt.y - vy,
                app,
                guard,
            });
            let title: Vec<u16> = "Buddy Color Picker\0".encode_utf16().collect();
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                PCWSTR(class_name.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_POPUP | WS_VISIBLE,
                vx,
                vy,
                vw,
                vh,
                None,
                None,
                hinst,
                None,
            )
            .unwrap_or_default();

            if hwnd.0.is_null() {
                drop(state);
                return;
            }
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
            let _ = SetWindowPos(hwnd, HWND_TOPMOST, vx, vy, vw, vh, SWP_NOZORDER);

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
    }
    #[cfg(not(windows))]
    {
        let _ = (app, guard);
    }
    Ok(())
}
