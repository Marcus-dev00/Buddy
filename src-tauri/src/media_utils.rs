//! Shared hashing, foreground app detection, thumbnail generation, and clipboard write helpers.

use crate::db::{now_ms, MemoryDb, MemoryItem};
use image::{imageops::FilterType, DynamicImage, ImageFormat};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::path::Path;
use std::sync::Mutex;

#[cfg(windows)]
use windows::core::PWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, HGLOBAL};
#[cfg(windows)]
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};

#[cfg(windows)]
const CF_DIB_FMT: u32 = 8;
#[cfg(windows)]
const CF_UNICODETEXT_FMT: u32 = 13;
#[cfg(windows)]
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
#[cfg(windows)]
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

/// Stores SHA-256 of content Buddy itself just wrote to the clipboard to prevent self-capture loops.
pub struct SelfCopyGuard {
    last_sha: Mutex<Option<(String, i64)>>,
}

impl Default for SelfCopyGuard {
    fn default() -> Self {
        Self {
            last_sha: Mutex::new(None),
        }
    }
}

impl SelfCopyGuard {
    pub fn mark(&self, sha: &str) {
        if let Ok(mut g) = self.last_sha.lock() {
            *g = Some((sha.to_string(), now_ms()));
        }
    }

    pub fn is_recent_self_copy(&self, sha: &str) -> bool {
        if let Ok(g) = self.last_sha.lock() {
            if let Some((s, ts)) = g.as_ref() {
                return s == sha && (now_ms() - ts) < 3500;
            }
        }
        false
    }
}

pub fn text_sha256(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"txt:");
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Computes a cross-channel SHA-256 from normalized RGBA pixels so clipboard DIB
/// and saved PNG screenshots deduplicate identically.
pub fn image_rgba_sha256(img: &DynamicImage) -> String {
    let rgba = img.to_rgba8();
    let mut hasher = Sha256::new();
    hasher.update(b"img:");
    hasher.update(rgba.width().to_le_bytes());
    hasher.update(rgba.height().to_le_bytes());
    hasher.update(rgba.as_raw());
    format!("{:x}", hasher.finalize())
}

#[cfg(windows)]
pub fn foreground_app_name() -> String {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return "Windows".to_string();
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(fg, Some(&mut pid));
        if pid == 0 || pid == std::process::id() {
            return "Buddy".to_string();
        }
        if let Ok(proc_h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            let mut buf = [0u16; 512];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(
                proc_h,
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut len,
            )
            .is_ok();
            let _ = CloseHandle(proc_h);
            if ok && len > 0 {
                let path = String::from_utf16_lossy(&buf[..len as usize]);
                if let Some(name) = Path::new(&path).file_name().and_then(|s| s.to_str()) {
                    return name.to_string();
                }
            }
        }
    }
    "Desktop".to_string()
}

#[cfg(not(windows))]
pub fn foreground_app_name() -> String {
    "Desktop".to_string()
}

/// Saves full PNG + 280px JPEG thumbnail and inserts/deduplicates into SQLite.
pub fn save_image_memory(
    db: &MemoryDb,
    img: &DynamicImage,
    kind: &str,
    app_name: &str,
) -> Result<(MemoryItem, bool), String> {
    let sha = image_rgba_sha256(img);

    // Fast path: if recently captured via the other channel (e.g. Win+Shift+S clipboard + folder), merge without re-encoding
    if db.has_recent_sha(&sha, 12_000) {
        return db.upsert_memory(kind, None, None, None, &sha, app_name);
    }

    let ts = now_ms();
    let short_sha = &sha[..12];
    let full_path = db.screenshots_dir.join(format!("{ts}_{short_sha}.png"));
    let thumb_path = db.thumbnails_dir.join(format!("{ts}_{short_sha}.jpg"));

    img.save_with_format(&full_path, ImageFormat::Png)
        .map_err(|e| format!("save png failed: {e}"))?;

    let thumb = if img.width() > 320 {
        let ratio = 320.0 / (img.width() as f32);
        let new_h = ((img.height() as f32) * ratio).round().max(1.0) as u32;
        img.resize(320, new_h, FilterType::Triangle)
    } else {
        img.clone()
    };
    let rgb = thumb.to_rgb8();
    let _ = rgb.save_with_format(&thumb_path, ImageFormat::Jpeg);

    let full_str = full_path.to_string_lossy().to_string();
    let thumb_str = thumb_path.to_string_lossy().to_string();
    let dims_label = format!("{}×{} px", img.width(), img.height());

    db.upsert_memory(
        kind,
        Some(&dims_label),
        Some(&full_str),
        Some(&thumb_str),
        &sha,
        app_name,
    )
}

#[cfg(windows)]
pub fn write_text_to_clipboard(text: &str, guard: &SelfCopyGuard) -> Result<(), String> {
    guard.mark(&text_sha256(text));
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = wide.len() * 2;
    unsafe {
        if OpenClipboard(None).is_err() {
            return Err("OpenClipboard failed".into());
        }
        let _ = EmptyClipboard();
        let hmem: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, bytes).map_err(|e| {
            let _ = CloseClipboard();
            e.to_string()
        })?;
        let ptr = GlobalLock(hmem) as *mut u16;
        if !ptr.is_null() {
            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
            let _ = GlobalUnlock(hmem);
        }
        let res = SetClipboardData(
            CF_UNICODETEXT_FMT,
            windows::Win32::Foundation::HANDLE(hmem.0),
        );
        let _ = CloseClipboard();
        if res.is_err() {
            return Err("SetClipboardData text failed".into());
        }
    }
    Ok(())
}

#[cfg(windows)]
pub fn write_image_to_clipboard(img: &DynamicImage, guard: &SelfCopyGuard) -> Result<(), String> {
    guard.mark(&image_rgba_sha256(img));
    let mut bmp_bytes = Vec::new();
    img.write_to(&mut Cursor::new(&mut bmp_bytes), ImageFormat::Bmp)
        .map_err(|e| format!("encode bmp failed: {e}"))?;
    if bmp_bytes.len() <= 14 {
        return Err("invalid bmp output".into());
    }
    // CF_DIB skips the 14-byte BITMAPFILEHEADER
    let dib = &bmp_bytes[14..];
    unsafe {
        if OpenClipboard(None).is_err() {
            return Err("OpenClipboard failed".into());
        }
        let _ = EmptyClipboard();
        let hmem: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, dib.len()).map_err(|e| {
            let _ = CloseClipboard();
            e.to_string()
        })?;
        let ptr = GlobalLock(hmem) as *mut u8;
        if !ptr.is_null() {
            std::ptr::copy_nonoverlapping(dib.as_ptr(), ptr, dib.len());
            let _ = GlobalUnlock(hmem);
        }
        let res = SetClipboardData(
            CF_DIB_FMT,
            windows::Win32::Foundation::HANDLE(hmem.0),
        );
        let _ = CloseClipboard();
        if res.is_err() {
            return Err("SetClipboardData CF_DIB failed".into());
        }
    }
    Ok(())
}

#[cfg(windows)]
pub fn read_clipboard_text_only() -> Option<String> {
    unsafe {
        if OpenClipboard(None).is_err() {
            return None;
        }
        let mut out = None;
        if let Ok(handle) = GetClipboardData(CF_UNICODETEXT_FMT) {
            let hmem = HGLOBAL(handle.0);
            let ptr = GlobalLock(hmem) as *const u16;
            if !ptr.is_null() {
                let mut len = 0usize;
                while *ptr.add(len) != 0 && len < 262_144 {
                    len += 1;
                }
                let slice = std::slice::from_raw_parts(ptr, len);
                out = Some(String::from_utf16_lossy(slice));
                let _ = GlobalUnlock(hmem);
            }
        }
        let _ = CloseClipboard();
        out
    }
}
