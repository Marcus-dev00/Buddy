//! Zero-polling Windows clipboard listener (AddClipboardFormatListener)
//! and system screenshot folder watcher (FOLDERID_Screenshots + notify).

use crate::db::MemoryDb;
use crate::media_utils::{
    foreground_app_name, save_image_memory, text_sha256, SelfCopyGuard,
};
use image::ImageFormat;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
#[cfg(windows)]
use windows::Win32::System::Com::CoTaskMemFree;
#[cfg(windows)]
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, GetClipboardData, IsClipboardFormatAvailable,
    OpenClipboard, RegisterClipboardFormatW, RemoveClipboardFormatListener,
};

#[cfg(windows)]
const CF_DIB_FMT: u32 = 8;
#[cfg(windows)]
const CF_UNICODETEXT_FMT: u32 = 13;
#[cfg(windows)]
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
#[cfg(windows)]
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
#[cfg(windows)]
use windows::Win32::UI::Shell::{FOLDERID_Screenshots, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    RegisterClassW, TranslateMessage, HWND_MESSAGE, MSG, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_CLIPBOARDUPDATE, WNDCLASSW,
};

const MAX_TEXT_BYTES: usize = 512 * 1024;
const MAX_IMAGE_BYTES: usize = 24 * 1024 * 1024;

#[cfg(windows)]
fn read_clipboard_bytes(format: u32) -> Option<Vec<u8>> {
    unsafe {
        let handle = GetClipboardData(format).ok()?;
        let hglobal = windows::Win32::Foundation::HGLOBAL(handle.0);
        let size = GlobalSize(hglobal);
        if size == 0 || size > MAX_IMAGE_BYTES {
            return None;
        }
        let ptr = GlobalLock(hglobal) as *const u8;
        if ptr.is_null() {
            return None;
        }
        let slice = std::slice::from_raw_parts(ptr, size);
        let out = slice.to_vec();
        let _ = GlobalUnlock(hglobal);
        Some(out)
    }
}

#[cfg(windows)]
fn read_clipboard_unicode_text() -> Option<String> {
    unsafe {
        let handle = GetClipboardData(CF_UNICODETEXT_FMT).ok()?;
        let hglobal = windows::Win32::Foundation::HGLOBAL(handle.0);
        let size = GlobalSize(hglobal);
        if size < 2 {
            return None;
        }
        let ptr = GlobalLock(hglobal) as *const u16;
        if ptr.is_null() {
            return None;
        }
        let max_u16 = (size / 2).min(MAX_TEXT_BYTES / 2);
        let slice = std::slice::from_raw_parts(ptr, max_u16);
        let nul_pos = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
        let text = String::from_utf16_lossy(&slice[..nul_pos]);
        let _ = GlobalUnlock(hglobal);
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    }
}

fn decode_cf_dib(dib: &[u8]) -> Option<image::DynamicImage> {
    if dib.len() < 40 {
        return None;
    }
    let bi_size = u32::from_le_bytes(dib[0..4].try_into().ok()?);
    if bi_size < 12 || (bi_size as usize) > dib.len() {
        return None;
    }
    let (bi_bit_count, bi_compression, bi_clr_used) = if bi_size >= 40 {
        let bc = u16::from_le_bytes(dib[14..16].try_into().ok()?);
        let comp = u32::from_le_bytes(dib[16..20].try_into().ok()?);
        let clr = u32::from_le_bytes(dib[32..36].try_into().ok()?);
        (bc, comp, clr)
    } else {
        (24, 0, 0)
    };

    let palette_entries = if bi_clr_used > 0 {
        bi_clr_used
    } else if bi_bit_count <= 8 {
        1u32 << bi_bit_count
    } else if bi_compression == 3 && bi_size == 40 {
        3
    } else {
        0
    };

    let bf_off_bits = 14u32.saturating_add(bi_size).saturating_add(palette_entries * 4);
    let file_size = 14u32.saturating_add(dib.len() as u32);

    let mut bmp = Vec::with_capacity(14 + dib.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&file_size.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&bf_off_bits.to_le_bytes());
    bmp.extend_from_slice(dib);

    image::load_from_memory_with_format(&bmp, ImageFormat::Bmp).ok()
}

#[cfg(windows)]
fn process_clipboard_update(
    app: &AppHandle,
    db: &MemoryDb,
    guard: &SelfCopyGuard,
    fmt_exclude: u32,
    fmt_can_history: u32,
    fmt_png: u32,
) {
    let app_name = foreground_app_name();

    // Retry OpenClipboard briefly if source app still holds clipboard lock
    let mut opened = false;
    for attempt in 0..5 {
        if unsafe { OpenClipboard(None).is_ok() } {
            opened = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(18 + attempt * 14));
    }
    if !opened {
        return;
    }

    // Respect Windows privacy markers from password managers / password inputs
    let excluded = unsafe {
        (fmt_exclude != 0 && IsClipboardFormatAvailable(fmt_exclude).is_ok())
            || (fmt_can_history != 0
                && IsClipboardFormatAvailable(fmt_can_history).is_ok()
                && read_clipboard_bytes(fmt_can_history)
                    .map(|b| b.len() >= 4 && u32::from_le_bytes([b[0], b[1], b[2], b[3]]) == 0)
                    .unwrap_or(false))
    };

    if excluded {
        unsafe {
            let _ = CloseClipboard();
        }
        return;
    }

    let text_opt = unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT_FMT).is_ok() {
            read_clipboard_unicode_text()
        } else {
            None
        }
    };

    let png_bytes = if text_opt.is_none() && fmt_png != 0 {
        unsafe {
            if IsClipboardFormatAvailable(fmt_png).is_ok() {
                read_clipboard_bytes(fmt_png)
            } else {
                None
            }
        }
    } else {
        None
    };

    let dib_bytes = if text_opt.is_none() && png_bytes.is_none() {
        unsafe {
            if IsClipboardFormatAvailable(CF_DIB_FMT).is_ok() {
                read_clipboard_bytes(CF_DIB_FMT)
            } else {
                None
            }
        }
    } else {
        None
    };

    unsafe {
        let _ = CloseClipboard();
    }

    if let Some(text) = text_opt {
        let sha = text_sha256(&text);
        if guard.is_recent_self_copy(&sha) {
            return;
        }
        if let Ok((item, was_merged)) =
            db.upsert_memory("clipboard_text", Some(&text), None, None, &sha, &app_name)
        {
            let _ = app.emit("buddy://memory-updated", item);
            if !was_merged {
                let _ = app.emit("buddy://capture", ());
            }
        }
        return;
    }

    let decoded_img = if let Some(png) = png_bytes {
        image::load_from_memory_with_format(&png, ImageFormat::Png).ok()
    } else if let Some(dib) = dib_bytes {
        decode_cf_dib(&dib)
    } else {
        None
    };

    if let Some(img) = decoded_img {
        let sha = crate::media_utils::image_rgba_sha256(&img);
        if guard.is_recent_self_copy(&sha) {
            return;
        }
        if let Ok((item, was_merged)) = save_image_memory(db, &img, "clipboard_image", &app_name) {
            let _ = app.emit("buddy://memory-updated", item);
            if !was_merged {
                let _ = app.emit("buddy://capture", ());
            }
        }
    }
}

#[cfg(windows)]
pub fn start_clipboard_watcher(app: AppHandle, db: Arc<MemoryDb>, guard: Arc<SelfCopyGuard>) {
    struct ClipCtx {
        app: AppHandle,
        db: Arc<MemoryDb>,
        guard: Arc<SelfCopyGuard>,
        fmt_exclude: u32,
        fmt_can_history: u32,
        fmt_png: u32,
    }

    static CLIP_CTX: std::sync::OnceLock<ClipCtx> = std::sync::OnceLock::new();

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_CLIPBOARDUPDATE {
            if let Some(ctx) = CLIP_CTX.get() {
                process_clipboard_update(
                    &ctx.app,
                    &ctx.db,
                    &ctx.guard,
                    ctx.fmt_exclude,
                    ctx.fmt_can_history,
                    ctx.fmt_png,
                );
            }
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    std::thread::spawn(move || unsafe {
        let fmt_exclude_w: Vec<u16> = "ExcludeClipboardContentFromMonitorProcessing\0"
            .encode_utf16()
            .collect();
        let fmt_can_w: Vec<u16> = "CanIncludeInClipboardHistory\0".encode_utf16().collect();
        let fmt_png_w: Vec<u16> = "PNG\0".encode_utf16().collect();

        let _ = CLIP_CTX.set(ClipCtx {
            app,
            db,
            guard,
            fmt_exclude: RegisterClipboardFormatW(PCWSTR(fmt_exclude_w.as_ptr())),
            fmt_can_history: RegisterClipboardFormatW(PCWSTR(fmt_can_w.as_ptr())),
            fmt_png: RegisterClipboardFormatW(PCWSTR(fmt_png_w.as_ptr())),
        });

        let class_name: Vec<u16> = "BuddyClipMsgClass\0".encode_utf16().collect();
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: HINSTANCE(hinst.0),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        RegisterClassW(&wc);

        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(class_name.as_ptr()),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            None,
            HINSTANCE(hinst.0),
            None,
        ) else {
            return;
        };

        if AddClipboardFormatListener(hwnd).is_err() {
            let _ = DestroyWindow(hwnd);
            return;
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let _ = RemoveClipboardFormatListener(hwnd);
        let _ = DestroyWindow(hwnd);
    });
}

#[cfg(not(windows))]
pub fn start_clipboard_watcher(_app: AppHandle, _db: Arc<MemoryDb>, _guard: Arc<SelfCopyGuard>) {}

#[cfg(windows)]
fn resolve_screenshots_folder() -> Option<PathBuf> {
    unsafe {
        if let Ok(pwstr) = SHGetKnownFolderPath(&FOLDERID_Screenshots, KF_FLAG_DEFAULT, None) {
            let len = (0..).take_while(|&i| *pwstr.0.add(i) != 0).count();
            let slice = std::slice::from_raw_parts(pwstr.0, len);
            let path = PathBuf::from(String::from_utf16_lossy(slice));
            CoTaskMemFree(Some(pwstr.0 as *const _));
            if path.exists() {
                return Some(path);
            }
        }
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        let p = PathBuf::from(home).join("Pictures").join("Screenshots");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

pub fn start_screenshot_folder_watcher(app: AppHandle, db: Arc<MemoryDb>) {
    std::thread::spawn(move || {
        #[cfg(windows)]
        let Some(watch_dir) = resolve_screenshots_folder() else {
            return;
        };
        #[cfg(not(windows))]
        return;

        #[cfg(windows)]
        {
            let (tx, rx) = std::sync::mpsc::channel();
            let Ok(mut watcher) = RecommendedWatcher::new(
                move |res| {
                    let _ = tx.send(res);
                },
                notify::Config::default(),
            ) else {
                return;
            };

            if watcher.watch(&watch_dir, RecursiveMode::NonRecursive).is_err() {
                return;
            }

            let seen_files: Arc<Mutex<HashSet<PathBuf>>> = Arc::new(Mutex::new(HashSet::new()));
            let mut last_cleanup = Instant::now();

            loop {
                match rx.recv_timeout(Duration::from_secs(60)) {
                    Ok(Ok(event)) => {
                        if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                            for path in event.paths {
                                let ext = path
                                    .extension()
                                    .and_then(|s| s.to_str())
                                    .map(|s| s.to_ascii_lowercase())
                                    .unwrap_or_default();
                                if ext != "png" && ext != "jpg" && ext != "jpeg" {
                                    continue;
                                }

                                {
                                    let mut guard = seen_files.lock().unwrap();
                                    if !guard.insert(path.clone()) {
                                        continue;
                                    }
                                }

                                let app_c = app.clone();
                                let db_c = db.clone();
                                let seen_c = seen_files.clone();
                                std::thread::spawn(move || {
                                    // Allow Snipping Tool / OneDrive to finish writing file
                                    std::thread::sleep(Duration::from_millis(240));
                                    if let Ok(img) = image::open(&path) {
                                        let app_name = foreground_app_name();
                                        if let Ok((item, was_merged)) =
                                            save_image_memory(&db_c, &img, "screenshot", &app_name)
                                        {
                                            let _ = app_c.emit("buddy://memory-updated", item);
                                            if !was_merged {
                                                let _ = app_c.emit("buddy://capture", ());
                                            }
                                        }
                                    }
                                    std::thread::sleep(Duration::from_secs(3));
                                    if let Ok(mut guard) = seen_c.lock() {
                                        guard.remove(&path);
                                    }
                                });
                            }
                        }
                    }
                    Ok(Err(_)) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }

                // Periodic 7-day expiration check every 4 hours
                if last_cleanup.elapsed() >= Duration::from_secs(4 * 3600) {
                    last_cleanup = Instant::now();
                    let removed = db.cleanup_expired();
                    if removed > 0 {
                        let _ = app.emit("buddy://memory-cleaned", removed);
                    }
                }
            }
        }
    });
}
