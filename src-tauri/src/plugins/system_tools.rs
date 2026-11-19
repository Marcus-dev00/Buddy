//! Native Windows utility tools: offline OCR (`Windows.Media.Ocr`), smart clipboard parser
//! & clean paste, Restart Manager file locksmith, and Explorer output reveal.

use crate::media_utils::{self, SelfCopyGuard};
use crate::pin_window::load_dynamic_image_from_source;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use image::ImageFormat;
use serde::Serialize;
use std::io::Cursor;
use std::path::Path;
use std::process::Command;

#[derive(Clone, Serialize)]
pub struct OcrResult {
    pub text: String,
    pub line_count: usize,
    pub language: String,
}

#[derive(Clone, Serialize)]
pub struct ClipboardParseItem {
    pub kind: String, // "timestamp" | "json" | "base64" | "color" | "url" | "clean_text"
    pub label: String,
    pub value: String,
}

#[derive(Clone, Serialize)]
pub struct ClipboardParseResult {
    pub raw_length: usize,
    pub items: Vec<ClipboardParseItem>,
    pub cleaned_text: String,
}

#[derive(Clone, Serialize)]
pub struct LockProcessInfo {
    pub pid: u32,
    pub app_name: String,
    pub service_name: String,
    pub app_type: String,
    pub restartable: bool,
}

// ============================================================================
// 1. Offline Windows OCR (Windows.Media.Ocr)
// ============================================================================

pub async fn run_offline_ocr(source: String) -> Result<OcrResult, String> {
    tokio::task::spawn_blocking(move || run_offline_ocr_sync(&source))
        .await
        .map_err(|e| e.to_string())?
}

fn run_offline_ocr_sync(source: &str) -> Result<OcrResult, String> {
    let img = load_dynamic_image_from_source(source)?;
    let mut png_bytes = Vec::new();
    img.write_to(&mut Cursor::new(&mut png_bytes), ImageFormat::Png)
        .map_err(|e| format!("PNG encode failed: {e}"))?;

    #[cfg(windows)]
    unsafe {
        use windows::Graphics::Imaging::BitmapDecoder;
        use windows::Media::Ocr::OcrEngine;
        use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        let stream = InMemoryRandomAccessStream::new().map_err(|e| e.to_string())?;
        let writer = DataWriter::CreateDataWriter(&stream).map_err(|e| e.to_string())?;
        writer.WriteBytes(&png_bytes).map_err(|e| e.to_string())?;
        let store_op = writer.StoreAsync().map_err(|e| e.to_string())?;
        let _ = store_op.get().map_err(|e| e.to_string())?;
        let flush_op = writer.FlushAsync().map_err(|e| e.to_string())?;
        let _ = flush_op.get().map_err(|e| e.to_string())?;
        stream.Seek(0).map_err(|e| e.to_string())?;

        let dec_op = BitmapDecoder::CreateAsync(&stream).map_err(|e| e.to_string())?;
        let decoder = dec_op.get().map_err(|e| e.to_string())?;
        let bmp_op = decoder.GetSoftwareBitmapAsync().map_err(|e| e.to_string())?;
        let software_bitmap = bmp_op.get().map_err(|e| e.to_string())?;

        let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(|_| {
            "No Windows OCR language pack installed in User Profile Languages".to_string()
        })?;
        let lang = engine
            .RecognizerLanguage()
            .and_then(|l| l.DisplayName())
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "System OCR".into());

        let rec_op = engine
            .RecognizeAsync(&software_bitmap)
            .map_err(|e| e.to_string())?;
        let result = rec_op.get().map_err(|e| e.to_string())?;

        let lines = result.Lines().map_err(|e| e.to_string())?;
        let line_count = lines.Size().unwrap_or(0) as usize;
        let mut out_lines = Vec::with_capacity(line_count);
        for i in 0..line_count {
            if let Ok(line) = lines.GetAt(i as u32) {
                if let Ok(t) = line.Text() {
                    let s = t.to_string();
                    if !s.trim().is_empty() {
                        out_lines.push(s);
                    }
                }
            }
        }
        let text = if out_lines.is_empty() {
            result.Text().map(|t| t.to_string()).unwrap_or_default()
        } else {
            out_lines.join("\n")
        };

        Ok(OcrResult {
            text,
            line_count,
            language: lang,
        })
    }

    #[cfg(not(windows))]
    {
        let _ = png_bytes;
        Err("Windows OCR is only supported on Windows".into())
    }
}

// ============================================================================
// 2. Smart Clipboard Parser & Clean Paste
// ============================================================================

pub fn clean_pdf_line_wraps(raw: &str) -> String {
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::new();
    for para in normalized.split("\n\n") {
        let joined = para
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if !joined.is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(&joined);
        }
    }
    out
}

pub fn parse_clipboard_text(raw: &str) -> ClipboardParseResult {
    let trimmed = raw.trim();
    let mut items = Vec::new();

    // 1. Unix timestamp (10-digit seconds or 13-digit ms)
    if (trimmed.len() == 10 || trimmed.len() == 13) && trimmed.chars().all(|c| c.is_ascii_digit()) {
        if let Ok(num) = trimmed.parse::<i64>() {
            let secs = if trimmed.len() == 13 { num / 1000 } else { num };
            let ms = if trimmed.len() == 13 { num % 1000 } else { 0 };
            if (946_684_800..=4_102_444_800).contains(&secs) {
                items.push(ClipboardParseItem {
                    kind: "timestamp".into(),
                    label: "Unix Timestamp (UTC)".into(),
                    value: format_unix_utc(secs, ms),
                });
            }
        }
    }

    // 2. JSON pretty-print
    if (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
    {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Ok(pretty) = serde_json::to_string_pretty(&val) {
                items.push(ClipboardParseItem {
                    kind: "json".into(),
                    label: "Formatted JSON".into(),
                    value: pretty,
                });
            }
        }
    }

    // 3. HEX Color
    if let Some(hex) = parse_hex_color(trimmed) {
        let c = crate::pin_window::format_color_values(hex.0, hex.1, hex.2);
        items.push(ClipboardParseItem {
            kind: "color".into(),
            label: "Color Value".into(),
            value: format!("{} · {} · {}", c.hex, c.rgb, c.hsl),
        });
    }

    // 4. URL query params
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        if let Ok(u) = reqwest::Url::parse(trimmed) {
            let mut lines = vec![format!(
                "Host: {}{}",
                u.host_str().unwrap_or(""),
                u.path()
            )];
            for (k, v) in u.query_pairs() {
                lines.push(format!("{k} = {v}"));
            }
            items.push(ClipboardParseItem {
                kind: "url".into(),
                label: "URL Breakdown".into(),
                value: lines.join("\n"),
            });
        }
    }

    // 5. Base64 UTF-8 decode
    if trimmed.len() >= 8
        && trimmed.len() <= 8192
        && !trimmed.contains(' ')
        && trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
    {
        if let Ok(bytes) = B64.decode(trimmed) {
            if let Ok(s) = std::str::from_utf8(&bytes) {
                if s.chars().all(|c| !c.is_control() || c == '\n' || c == '\r' || c == '\t') {
                    items.push(ClipboardParseItem {
                        kind: "base64".into(),
                        label: "Base64 Decoded".into(),
                        value: s.to_string(),
                    });
                }
            }
        }
    }

    let cleaned_text = clean_pdf_line_wraps(raw);
    if !cleaned_text.is_empty() && cleaned_text != trimmed {
        items.push(ClipboardParseItem {
            kind: "clean_text".into(),
            label: "Cleaned (PDF Wraps Removed)".into(),
            value: cleaned_text.clone(),
        });
    }

    ClipboardParseResult {
        raw_length: raw.chars().count(),
        items,
        cleaned_text,
    }
}

fn parse_hex_color(s: &str) -> Option<(u8, u8, u8)> {
    let raw = s.strip_prefix('#').unwrap_or(s);
    if !raw.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    if s.starts_with('#') && raw.len() == 6 {
        let r = u8::from_str_radix(&raw[0..2], 16).ok()?;
        let g = u8::from_str_radix(&raw[2..4], 16).ok()?;
        let b = u8::from_str_radix(&raw[4..6], 16).ok()?;
        return Some((r, g, b));
    }
    if s.starts_with('#') && raw.len() == 3 {
        let r = u8::from_str_radix(&raw[0..1], 16).ok()? * 17;
        let g = u8::from_str_radix(&raw[1..2], 16).ok()? * 17;
        let b = u8::from_str_radix(&raw[2..3], 16).ok()? * 17;
        return Some((r, g, b));
    }
    None
}

fn format_unix_utc(secs: i64, ms: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;
    // Civil date from Unix epoch days (Howard Hinnant algorithm)
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    if ms > 0 {
        format!("{year:04}-{m:02}-{d:02} {hour:02}:{min:02}:{sec:02}.{ms:03} UTC")
    } else {
        format!("{year:04}-{m:02}-{d:02} {hour:02}:{min:02}:{sec:02} UTC")
    }
}

pub fn apply_clean_paste_text(text: &str, guard: &SelfCopyGuard) -> Result<String, String> {
    let cleaned = clean_pdf_line_wraps(text);
    media_utils::write_text_to_clipboard(&cleaned, guard)?;
    Ok(cleaned)
}

// ============================================================================
// 3. File Locksmith (Windows Restart Manager API)
// ============================================================================

pub fn inspect_file_locks(path: &str) -> Result<Vec<LockProcessInfo>, String> {
    let target = Path::new(path.trim());
    if !target.exists() {
        return Err(format!("Path does not exist: {path}"));
    }
    let abs = target
        .canonicalize()
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .trim_start_matches("\\\\?\\")
        .to_string();

    #[cfg(windows)]
    unsafe {
        use windows::core::{PCWSTR, PWSTR};
        use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
        use windows::Win32::System::RestartManager::{
            RmEndSession, RmGetList, RmRegisterResources, RmStartSession, CCH_RM_SESSION_KEY,
            RM_PROCESS_INFO,
        };

        let mut session_handle = 0u32;
        let mut session_key = [0u16; (CCH_RM_SESSION_KEY as usize) + 1];
        let start_err = RmStartSession(&mut session_handle, 0, PWSTR(session_key.as_mut_ptr()));
        if start_err != ERROR_SUCCESS {
            return Err(format!("RmStartSession failed (code {})", start_err.0));
        }

        let wide_path: Vec<u16> = abs.encode_utf16().chain(std::iter::once(0)).collect();
        let files = [PCWSTR(wide_path.as_ptr())];
        let reg_err = RmRegisterResources(session_handle, Some(&files), None, None);
        if reg_err != ERROR_SUCCESS {
            let _ = RmEndSession(session_handle);
            return Err(format!("RmRegisterResources failed (code {})", reg_err.0));
        }

        let mut needed = 0u32;
        let mut count = 0u32;
        let mut reasons = 0u32;
        let mut list: Vec<RM_PROCESS_INFO> = Vec::new();
        let mut res_err = RmGetList(
            session_handle,
            &mut needed,
            &mut count,
            None,
            &mut reasons,
        );
        if res_err == ERROR_MORE_DATA && needed > 0 {
            list.resize(needed as usize, RM_PROCESS_INFO::default());
            count = needed;
            res_err = RmGetList(
                session_handle,
                &mut needed,
                &mut count,
                Some(list.as_mut_ptr()),
                &mut reasons,
            );
        }
        let _ = RmEndSession(session_handle);

        if res_err != ERROR_SUCCESS && res_err != ERROR_MORE_DATA {
            return Err(format!("RmGetList failed (code {})", res_err.0));
        }

        let mut out = Vec::new();
        for info in list.into_iter().take(count as usize) {
            let app_len = info
                .strAppName
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(info.strAppName.len());
            let srv_len = info
                .strServiceShortName
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(info.strServiceShortName.len());
            let app_name = String::from_utf16_lossy(&info.strAppName[..app_len]);
            let service_name = String::from_utf16_lossy(&info.strServiceShortName[..srv_len]);
            let app_type = format!("{:?}", info.ApplicationType);
            out.push(LockProcessInfo {
                pid: info.Process.dwProcessId,
                app_name,
                service_name,
                app_type,
                restartable: info.bRestartable.as_bool(),
            });
        }
        Ok(out)
    }

    #[cfg(not(windows))]
    Ok(Vec::new())
}

pub fn terminate_locking_process(pid: u32) -> Result<(), String> {
    if pid == std::process::id() {
        return Err("Refusing to terminate Buddy itself".into());
    }
    #[cfg(windows)]
    unsafe {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

        let handle = OpenProcess(PROCESS_TERMINATE, false, pid).map_err(|_| {
            "Access denied: this process may be running as Administrator or a protected system service."
                .to_string()
        })?;
        let ok = TerminateProcess(handle, 1).is_ok();
        let _ = CloseHandle(handle);
        if !ok {
            return Err("TerminateProcess failed (requires elevated privileges).".into());
        }
    }
    Ok(())
}

pub fn open_output_in_explorer(path: &str) {
    let p = Path::new(path);
    if !p.exists() {
        return;
    }
    #[cfg(windows)]
    {
        if p.is_dir() {
            let _ = Command::new("explorer").arg(p).spawn();
        } else {
            let _ = Command::new("explorer")
                .arg(format!("/select,{}", p.display()))
                .spawn();
        }
    }
    #[cfg(not(windows))]
    {}
}
