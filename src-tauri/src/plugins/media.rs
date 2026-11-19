//! Image optimizer, Document & PDF suite, FFmpeg media converter, and yt-dlp lossless downloader.

use super::{PluginManager, PluginProgressPayload};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::ImageFormat;
use serde::Serialize;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Clone, Serialize)]
pub struct PluginToolOutput {
    pub outputs: Vec<String>,
    pub summary: String,
}

// ============================================================================
// 4. Image Optimizer (Resize, Convert Format, Strip EXIF Metadata)
// ============================================================================

pub fn unique_output_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let clean_ext = ext.trim_start_matches('.');
    let first = dir.join(format!("{stem}.{clean_ext}"));
    if !first.exists() {
        return first;
    }
    for idx in 1..10_000 {
        let cand = dir.join(format!("{stem}_{idx}.{clean_ext}"));
        if !cand.exists() {
            return cand;
        }
    }
    dir.join(format!("{stem}_out.{clean_ext}"))
}

pub fn optimize_images(
    paths: Vec<String>,
    target_format: String,
    max_dim: u32,
    quality: u8,
) -> Result<PluginToolOutput, String> {
    if paths.is_empty() {
        return Err("No image files provided".into());
    }
    let fmt_lower = target_format.trim().to_ascii_lowercase();
    let (fmt, ext) = match fmt_lower.as_str() {
        "png" => (ImageFormat::Png, "png"),
        "webp" => (ImageFormat::WebP, "webp"),
        "bmp" => (ImageFormat::Bmp, "bmp"),
        _ => (ImageFormat::Jpeg, "jpg"),
    };

    let mut outputs = Vec::new();
    let mut saved_bytes: i64 = 0;

    for raw_p in paths {
        let src = Path::new(raw_p.trim());
        if !src.exists() {
            continue;
        }
        let orig_size = src.metadata().map(|m| m.len() as i64).unwrap_or(0);
        let mut img = image::open(src).map_err(|e| format!("Failed opening {}: {e}", src.display()))?;
        if max_dim >= 64 && (img.width() > max_dim || img.height() > max_dim) {
            img = img.resize(max_dim, max_dim, FilterType::Lanczos3);
        }

        let dir = src.parent().unwrap_or_else(|| Path::new("."));
        let stem = src
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("image");
        let out_path = unique_output_path(dir, &format!("{stem}_opt"), ext);

        if fmt == ImageFormat::Jpeg {
            let rgb = img.to_rgb8();
            let f = File::create(&out_path).map_err(|e| e.to_string())?;
            let mut enc = JpegEncoder::new_with_quality(f, quality.clamp(30, 100));
            enc.encode_image(&rgb).map_err(|e| e.to_string())?;
        } else {
            // Writing cleanly via `image` crate strips all EXIF/XMP/IPTC chunks
            img.save_with_format(&out_path, fmt).map_err(|e| e.to_string())?;
        }

        let new_size = out_path.metadata().map(|m| m.len() as i64).unwrap_or(0);
        saved_bytes += orig_size - new_size;
        outputs.push(out_path.to_string_lossy().to_string());
    }

    if outputs.is_empty() {
        return Err("No valid image files processed".into());
    }
    let kb_saved = saved_bytes as f64 / 1024.0;
    let summary = format!(
        "Processed {} image(s), EXIF stripped, net size change: {:+.1} KB",
        outputs.len(),
        -kb_saved
    );
    Ok(PluginToolOutput { outputs, summary })
}

// ============================================================================
// 5. Document & PDF Suite (Images->PDF, MD/Office->PDF, QPDF Merge/Split/Compress)
// ============================================================================

pub fn run_pdf_tool(
    mgr: &PluginManager,
    action: &str,
    paths: Vec<String>,
    page_range: Option<String>,
) -> Result<PluginToolOutput, String> {
    if paths.is_empty() {
        return Err("No input files provided".into());
    }
    match action {
        "images_to_pdf" => {
            let first = Path::new(&paths[0]);
            let dir = first.parent().unwrap_or_else(|| Path::new("."));
            let stem = first.file_stem().and_then(|s| s.to_str()).unwrap_or("images");
            let out = unique_output_path(dir, &format!("{stem}_bundle"), "pdf");
            write_images_to_pdf(&paths, &out)?;
            Ok(PluginToolOutput {
                outputs: vec![out.to_string_lossy().to_string()],
                summary: format!("Created PDF from {} image(s)", paths.len()),
            })
        }
        "doc_to_pdf" => {
            let mut outputs = Vec::new();
            for p in &paths {
                outputs.push(convert_doc_or_md_to_pdf(Path::new(p))?);
            }
            Ok(PluginToolOutput {
                summary: format!("Converted {} file(s) to PDF", outputs.len()),
                outputs,
            })
        }
        "merge" | "split" | "compress" => {
            let qpdf = mgr.engine_binary_path("pdf_suite");
            if !qpdf.exists() {
                return Err("QPDF engine is not installed yet. Click Install on Document & PDF Suite first.".into());
            }
            run_qpdf_action(&qpdf, action, &paths, page_range.as_deref())
        }
        _ => Err(format!("Unknown PDF action: {action}")),
    }
}

/// Pure-Rust valid PDF 1.4 generator embedding baseline JPEG XObjects (zero dependencies)
fn write_images_to_pdf(paths: &[String], out_pdf: &Path) -> Result<(), String> {
    let mut page_images: Vec<(u32, u32, Vec<u8>)> = Vec::new();
    for p in paths {
        let img = image::open(Path::new(p)).map_err(|e| format!("Open image failed: {e}"))?;
        let rgb = img.to_rgb8();
        let mut jpeg_buf = Vec::new();
        JpegEncoder::new_with_quality(&mut jpeg_buf, 88)
            .encode_image(&rgb)
            .map_err(|e| e.to_string())?;
        page_images.push((rgb.width(), rgb.height(), jpeg_buf));
    }
    if page_images.is_empty() {
        return Err("No valid images for PDF".into());
    }

    let n = page_images.len();
    let total_objs = 2 + n * 3;
    let mut offsets = vec![0usize; total_objs + 1];
    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

    offsets[1] = pdf.len();
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    offsets[2] = pdf.len();
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + i * 3)).collect();
    pdf.extend_from_slice(
        format!("2 0 obj\n<< /Type /Pages /Kids [{}] /Count {} >>\nendobj\n", kids.join(" "), n)
            .as_bytes(),
    );

    for (i, (w, h, jpeg)) in page_images.iter().enumerate() {
        let page_id = 3 + i * 3;
        let cont_id = 4 + i * 3;
        let img_id = 5 + i * 3;
        let scale = (595.0 / *w as f64).min(842.0 / *h as f64).min(1.0);
        let pw = ((*w as f64) * scale).max(72.0);
        let ph = ((*h as f64) * scale).max(72.0);

        offsets[page_id] = pdf.len();
        pdf.extend_from_slice(
            format!(
                "{page_id} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << /Im0 {img_id} 0 R >> >> /Contents {cont_id} 0 R >>\nendobj\n"
            )
            .as_bytes(),
        );

        let stream_str = format!("q\n{pw:.2} 0 0 {ph:.2} 0 0 cm\n/Im0 Do\nQ\n");
        offsets[cont_id] = pdf.len();
        pdf.extend_from_slice(
            format!(
                "{cont_id} 0 obj\n<< /Length {} >>\nstream\n{stream_str}endstream\nendobj\n",
                stream_str.len()
            )
            .as_bytes(),
        );

        offsets[img_id] = pdf.len();
        pdf.extend_from_slice(
            format!(
                "{img_id} 0 obj\n<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
                jpeg.len()
            )
            .as_bytes(),
        );
        pdf.extend_from_slice(jpeg);
        pdf.extend_from_slice(b"\nendstream\nendobj\n");
    }

    let xref_pos = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", total_objs + 1).as_bytes());
    for off in offsets.iter().skip(1) {
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n", total_objs + 1)
            .as_bytes(),
    );

    fs::write(out_pdf, pdf).map_err(|e| e.to_string())
}

fn find_edge_exe() -> Option<PathBuf> {
    let candidates = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    ];
    candidates.iter().map(PathBuf::from).find(|p| p.exists())
}

fn convert_doc_or_md_to_pdf(src: &Path) -> Result<String, String> {
    if !src.exists() {
        return Err(format!("File not found: {}", src.display()));
    }
    let ext = src
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let dir = src.parent().unwrap_or_else(|| Path::new("."));
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("doc");
    let out_pdf = unique_output_path(dir, stem, "pdf");

    if matches!(ext.as_str(), "doc" | "docx" | "ppt" | "pptx" | "xls" | "xlsx") {
        let src_esc = src.to_string_lossy().replace('\'', "''");
        let dst_esc = out_pdf.to_string_lossy().replace('\'', "''");
        let script = match ext.as_str() {
            "doc" | "docx" => format!(
                "$w = New-Object -ComObject Word.Application; $w.Visible = $false; try {{ $d = $w.Documents.Open('{src_esc}', $false, $true); $d.SaveAs([ref]'{dst_esc}', [ref]17); $d.Close($false) }} finally {{ $w.Quit() }}"
            ),
            "ppt" | "pptx" => format!(
                "$p = New-Object -ComObject PowerPoint.Application; try {{ $pres = $p.Presentations.Open('{src_esc}', $true, $false, $false); $pres.SaveAs('{dst_esc}', 32); $pres.Close() }} finally {{ $p.Quit() }}"
            ),
            _ => format!(
                "$x = New-Object -ComObject Excel.Application; $x.Visible = $false; try {{ $wb = $x.Workbooks.Open('{src_esc}'); $wb.ExportAsFixedFormat(0, '{dst_esc}'); $wb.Close($false) }} finally {{ $x.Quit() }}"
            ),
        };
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-Command", &script]);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        let status = cmd.status().map_err(|e| e.to_string())?;
        if !status.success() || !out_pdf.exists() {
            return Err("Office COM export failed (Microsoft Office may not be installed on this PC).".into());
        }
        return Ok(out_pdf.to_string_lossy().to_string());
    }

    // Markdown / Text / HTML -> PDF via Microsoft Edge headless
    let edge = find_edge_exe().ok_or("Microsoft Edge not found for HTML/Markdown PDF export")?;
    let raw_text = fs::read_to_string(src).map_err(|e| e.to_string())?;
    let html_content = if ext == "html" || ext == "htm" {
        raw_text
    } else {
        let escaped = raw_text
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><style>body{{font-family:'Segoe UI','Microsoft YaHei',sans-serif;max-width:760px;margin:32px auto;line-height:1.6;color:#111;white-space:pre-wrap;font-size:13px;}}</style></head><body>{escaped}</body></html>"
        )
    };

    let tmp_html = std::env::temp_dir().join(format!("buddy_pdf_{}.html", std::process::id()));
    fs::write(&tmp_html, html_content).map_err(|e| e.to_string())?;

    let mut cmd = Command::new(edge);
    cmd.args([
        "--headless",
        "--disable-gpu",
        "--no-pdf-header-footer",
        &format!("--print-to-pdf={}", out_pdf.display()),
        &format!("file:///{}", tmp_html.to_string_lossy().replace('\\', "/")),
    ]);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let _ = cmd.status();
    let _ = fs::remove_file(&tmp_html);

    if !out_pdf.exists() {
        return Err("Failed generating PDF via headless Edge".into());
    }
    Ok(out_pdf.to_string_lossy().to_string())
}

fn run_qpdf_action(
    qpdf: &Path,
    action: &str,
    paths: &[String],
    page_range: Option<&str>,
) -> Result<PluginToolOutput, String> {
    let first = Path::new(&paths[0]);
    let dir = first.parent().unwrap_or_else(|| Path::new("."));
    let stem = first.file_stem().and_then(|s| s.to_str()).unwrap_or("output");

    let mut cmd = Command::new(qpdf);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let out_path = match action {
        "merge" => {
            if paths.len() < 2 {
                return Err("Select at least 2 PDFs to merge".into());
            }
            let out = unique_output_path(dir, &format!("{stem}_merged"), "pdf");
            cmd.arg("--empty").arg("--pages");
            for p in paths {
                cmd.arg(p);
            }
            cmd.arg("--").arg(&out);
            out
        }
        "split" => {
            let pages = page_range.map(|s| s.trim()).filter(|s| !s.is_empty()).unwrap_or("1");
            let out = unique_output_path(dir, &format!("{stem}_p{}", pages.replace(',', "_")), "pdf");
            cmd.args(["--empty", "--pages", &paths[0], pages, "--"])
                .arg(&out);
            out
        }
        _ => {
            let out = unique_output_path(dir, &format!("{stem}_compressed"), "pdf");
            cmd.args([
                "--stream-data=compress",
                "--recompress-flate",
                "--compression-level=9",
                "--object-streams=generate",
                &paths[0],
            ])
            .arg(&out);
            out
        }
    };

    let output = cmd.output().map_err(|e| format!("Failed launching qpdf: {e}"))?;
    if !output.status.success() && output.status.code() != Some(3) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("QPDF error: {}", stderr.trim()));
    }
    Ok(PluginToolOutput {
        outputs: vec![out_path.to_string_lossy().to_string()],
        summary: format!("PDF {action} completed cleanly"),
    })
}

// ============================================================================
// 6. Media / GIF Converter (On-demand FFmpeg Engine)
// ============================================================================

fn emit_plugin_progress(app: &AppHandle, plugin_id: &str, stage: &str, percent: u32, detail: &str) {
    let _ = app.emit(
        "buddy://plugin-progress",
        PluginProgressPayload {
            plugin_id: plugin_id.to_string(),
            stage: stage.to_string(),
            percent,
            detail: detail.chars().take(80).collect(),
        },
    );
}

fn spawn_cancel_watchdog(child_pid: u32, done_flag: Arc<AtomicBool>, cancel_mirror: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let _ = child_pid;
        while !done_flag.load(Ordering::Relaxed) {
            if cancel_mirror.load(Ordering::Relaxed) {
                #[cfg(windows)]
                {
                    let mut tk = Command::new("taskkill");
                    tk.args(["/PID", &child_pid.to_string(), "/T", "/F"]);
                    tk.creation_flags(CREATE_NO_WINDOW);
                    let _ = tk.output();
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(120));
        }
    });
}

/// Lossy line reader that splits on `\n` or `\r` without failing on non-UTF-8 Windows console bytes.
fn for_each_lossy_line<R: Read, F: FnMut(&str)>(reader: R, mut on_line: F) {
    let mut br = BufReader::new(reader);
    let mut buf = Vec::with_capacity(256);
    loop {
        buf.clear();
        let Ok(n) = br.read_until(b'\n', &mut buf) else {
            break;
        };
        if n == 0 {
            break;
        }
        let chunk = String::from_utf8_lossy(&buf);
        for part in chunk.split('\r') {
            let trimmed = part.trim();
            if !trimmed.is_empty() {
                on_line(trimmed);
            }
        }
    }
}

fn parse_ffmpeg_secs(tag: &str) -> Option<f64> {
    let mut parts = tag.trim().split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let s: f64 = parts.next()?.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

fn parse_ffmpeg_progress_line(line: &str, total_secs: &mut Option<f64>) -> Option<u32> {
    if let Some(pos) = line.find("Duration:") {
        let rest = line[pos + 9..].trim();
        let dur_str = rest.split(',').next().unwrap_or("");
        if let Some(secs) = parse_ffmpeg_secs(dur_str) {
            if secs > 0.1 {
                *total_secs = Some(secs);
            }
        }
    }
    if let Some(pos) = line.find("time=") {
        let rest = line[pos + 5..].trim();
        let time_str = rest.split_whitespace().next().unwrap_or("");
        if let Some(cur) = parse_ffmpeg_secs(time_str) {
            if let Some(tot) = *total_secs {
                let pct = ((cur / tot) * 85.0 + 10.0).clamp(10.0, 95.0) as u32;
                return Some(pct);
            }
        }
    }
    None
}

pub async fn run_media_tool(
    app: &AppHandle,
    mgr: &PluginManager,
    action: &str,
    input_path: &str,
    max_width: Option<u32>,
    fps: Option<u32>,
) -> Result<PluginToolOutput, String> {
    match run_media_tool_inner(app, mgr, action, input_path, max_width, fps).await {
        Ok(out) => Ok(out),
        Err(err) => {
            crate::logger::log_error("media_gif", &format!("ffmpeg `{action}` failed: {err}"));
            emit_plugin_progress(app, "media_gif", "error", 0, &err);
            Err(err)
        }
    }
}

async fn run_media_tool_inner(
    app: &AppHandle,
    mgr: &PluginManager,
    action: &str,
    input_path: &str,
    max_width: Option<u32>,
    fps: Option<u32>,
) -> Result<PluginToolOutput, String> {
    let ffmpeg = mgr.resolve_ffmpeg_path().unwrap_or_else(|| mgr.engine_binary_path("media_gif"));
    if !ffmpeg.exists() {
        return Err("FFmpeg engine not installed yet. Install Media / GIF Converter first.".into());
    }
    let src = Path::new(input_path.trim());
    if !src.exists() {
        return Err(format!("Media file not found: {input_path}"));
    }

    mgr.cancel_task.store(false, Ordering::Relaxed);

    let dir = src.parent().unwrap_or_else(|| Path::new("."));
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("clip");
    let width = max_width.unwrap_or(540).clamp(240, 1920);
    let fps_val = fps.unwrap_or(12).clamp(5, 30);

    let (out_path, args): (PathBuf, Vec<String>) = match action {
        "gif" => {
            let out = unique_output_path(dir, stem, "gif");
            let vf = format!(
                "fps={fps_val},scale='min({width},iw)':-1:flags=lanczos,split[s0][s1];[s0]palettegen=stats_mode=diff[p];[s1][p]paletteuse=dither=bayer:bayer_scale=5"
            );
            (
                out.clone(),
                vec![
                    "-y".into(),
                    "-i".into(),
                    src.to_string_lossy().into(),
                    "-vf".into(),
                    vf,
                    out.to_string_lossy().into(),
                ],
            )
        }
        "audio" => {
            let out = unique_output_path(dir, stem, "mp3");
            (
                out.clone(),
                vec![
                    "-y".into(),
                    "-i".into(),
                    src.to_string_lossy().into(),
                    "-vn".into(),
                    "-acodec".into(),
                    "libmp3lame".into(),
                    "-q:a".into(),
                    "2".into(),
                    out.to_string_lossy().into(),
                ],
            )
        }
        _ => {
            let out = unique_output_path(dir, &format!("{stem}_compressed"), "mp4");
            (
                out.clone(),
                vec![
                    "-y".into(),
                    "-i".into(),
                    src.to_string_lossy().into(),
                    "-vf".into(),
                    "scale=trunc(iw/2)*2:trunc(ih/2)*2".into(),
                    "-vcodec".into(),
                    "libx264".into(),
                    "-pix_fmt".into(),
                    "yuv420p".into(),
                    "-crf".into(),
                    "26".into(),
                    "-preset".into(),
                    "veryfast".into(),
                    "-c:a".into(),
                    "aac".into(),
                    "-b:a".into(),
                    "160k".into(),
                    "-movflags".into(),
                    "+faststart".into(),
                    out.to_string_lossy().into(),
                ],
            )
        }
    };

    emit_plugin_progress(app, "media_gif", "running", 8, &format!("Running FFmpeg ({action})..."));

    let mut cmd = Command::new(&ffmpeg);
    cmd.args(&args).stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed starting ffmpeg: {e}"))?;
    let child_pid = child.id();
    let stderr = child.stderr.take();

    let cancel_done = Arc::new(AtomicBool::new(false));
    let cancel_mirror = Arc::new(AtomicBool::new(false));
    spawn_cancel_watchdog(child_pid, Arc::clone(&cancel_done), Arc::clone(&cancel_mirror));

    let app_c = app.clone();
    let action_owned = action.to_string();
    let blocking_fut = tokio::task::spawn_blocking(move || {
        let mut tail = Vec::<String>::new();
        let mut total_secs: Option<f64> = None;
        let mut last_pct = 8u32;
        if let Some(se) = stderr {
            for_each_lossy_line(se, |line| {
                if let Some(pct) = parse_ffmpeg_progress_line(line, &mut total_secs) {
                    if pct > last_pct {
                        last_pct = pct;
                        emit_plugin_progress(
                            &app_c,
                            "media_gif",
                            "running",
                            pct,
                            &format!("Converting ({action_owned}) {pct}%"),
                        );
                    }
                }
                tail.push(line.to_string());
                if tail.len() > 10 {
                    tail.remove(0);
                }
            });
        }
        let status = child.wait().map_err(|e| format!("Wait error: {e}"))?;
        Ok::<(std::process::ExitStatus, String), String>((status, tail.join("\n")))
    });

    while !blocking_fut.is_finished() {
        if mgr.cancel_task.load(Ordering::Relaxed) {
            cancel_mirror.store(true, Ordering::Relaxed);
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
    let (status, err_tail) = blocking_fut.await.map_err(|e| e.to_string())??;
    cancel_done.store(true, Ordering::Relaxed);

    if mgr.cancel_task.load(Ordering::Relaxed) {
        let _ = fs::remove_file(&out_path);
        return Err("Conversion cancelled.".into());
    }

    if !status.success() || !out_path.exists() {
        let tail: String = err_tail.chars().rev().take(280).collect::<String>().chars().rev().collect();
        return Err(format!("FFmpeg failed: {}", tail.trim()));
    }

    emit_plugin_progress(app, "media_gif", "done", 100, "Completed");

    Ok(PluginToolOutput {
        outputs: vec![out_path.to_string_lossy().to_string()],
        summary: format!("Saved {}", out_path.file_name().unwrap_or_default().to_string_lossy()),
    })
}

// ============================================================================
// 7. Lossless Link Media & Audio Downloader (On-demand yt-dlp + FFmpeg)
// ============================================================================

pub fn default_downloads_dir() -> PathBuf {
    let dl = crate::agent_tools::user_home_dir().join("Downloads");
    if dl.exists() {
        dl
    } else {
        crate::agent_tools::user_home_dir()
    }
}

fn validate_media_url(raw: &str) -> Result<String, String> {
    let clean = raw.trim().trim_matches('"').to_string();
    if !(clean.starts_with("https://") || clean.starts_with("http://")) {
        return Err("URL must begin with http:// or https://".into());
    }
    if clean.len() < 12 || clean.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("Invalid URL format (no spaces or control characters allowed).".into());
    }
    Ok(clean)
}

fn build_ytdlp_command_args(
    format_key: &str,
    url: &str,
    out_dir: &Path,
    ffmpeg_path: Option<&Path>,
) -> (&'static str, Vec<String>) {
    let mut args = vec![
        "--no-playlist".to_string(),
        "--encoding".to_string(),
        "utf-8".to_string(),
        "--windows-filenames".to_string(),
        "--newline".to_string(),
        "--no-simulate".to_string(),
        "--print".to_string(),
        "after_move:filepath".to_string(),
        "-P".to_string(),
        out_dir.to_string_lossy().into_owned(),
        "-o".to_string(),
        "%(title).120B [%(id)s].%(ext)s".to_string(),
    ];

    if let Some(ff) = ffmpeg_path {
        let loc = ff.parent().unwrap_or(ff);
        args.push("--ffmpeg-location".to_string());
        args.push(loc.to_string_lossy().into_owned());
    }

    let label = match format_key.trim().to_ascii_lowercase().as_str() {
        "mp4" | "mp4_lossless" | "video" => {
            args.extend([
                "-f".to_string(),
                "bv*[vcodec^=avc1]+ba[acodec^=mp4a]/bv*[ext=mp4]+ba[ext=m4a]/b[ext=mp4]/bv*+ba/b".to_string(),
                "--merge-output-format".to_string(),
                "mp4".to_string(),
            ]);
            "MP4 (Zero Re-encode)"
        }
        "flac" | "audio_flac" => {
            args.extend([
                "-f".to_string(),
                "bestaudio/best".to_string(),
                "-x".to_string(),
                "--audio-format".to_string(),
                "flac".to_string(),
                "--add-metadata".to_string(),
            ]);
            "FLAC (Lossless)"
        }
        "mp3" | "audio_mp3" => {
            args.extend([
                "-f".to_string(),
                "bestaudio/best".to_string(),
                "-x".to_string(),
                "--audio-format".to_string(),
                "mp3".to_string(),
                "--audio-quality".to_string(),
                "320K".to_string(),
                "--add-metadata".to_string(),
            ]);
            "MP3 320k"
        }
        _ => {
            args.extend([
                "-f".to_string(),
                "bestaudio/best".to_string(),
                "-x".to_string(),
                "--audio-format".to_string(),
                "best".to_string(),
                "--add-metadata".to_string(),
            ]);
            "Original Audio (Zero Re-encode)"
        }
    };

    args.push("--".to_string());
    args.push(url.to_string());
    (label, args)
}

fn parse_ytdlp_progress_line(line: &str) -> Option<(u32, String)> {
    let trimmed = line.trim();
    if trimmed.contains("[ExtractAudio]") || trimmed.contains("[Merger]") {
        return Some((94, "Remuxing / extracting lossless stream via FFmpeg...".into()));
    }
    if !trimmed.starts_with("[download]") {
        return None;
    }
    if let Some(pct_idx) = trimmed.find('%') {
        let prefix = &trimmed[..pct_idx];
        let num_str: String = prefix
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        if let Ok(val) = num_str.parse::<f32>() {
            let pct = val.round().clamp(1.0, 92.0) as u32;
            let detail = trimmed
                .strip_prefix("[download]")
                .unwrap_or(trimmed)
                .trim()
                .chars()
                .take(56)
                .collect();
            return Some((pct, detail));
        }
    }
    None
}

fn find_newest_media_in_dir(dir: &Path, since: SystemTime) -> Option<String> {
    let cutoff = since.checked_sub(Duration::from_secs(2)).unwrap_or(since);
    let mut best: Option<(SystemTime, PathBuf)> = None;
    let rd = fs::read_dir(dir).ok()?;
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let name = p
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if name.ends_with(".part") || name.ends_with(".ytdl") || name.ends_with(".tmp") {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(mtime) = meta.modified() {
                if mtime >= cutoff && best.as_ref().map(|b| mtime > b.0).unwrap_or(true) {
                    best = Some((mtime, p));
                }
            }
        }
    }
    best.map(|(_, p)| p.to_string_lossy().to_string())
}

pub async fn run_media_download(
    app: &AppHandle,
    mgr: &PluginManager,
    url: &str,
    format: &str,
    output_dir: Option<&str>,
) -> Result<PluginToolOutput, String> {
    match run_media_download_inner(app, mgr, url, format, output_dir).await {
        Ok(out) => Ok(out),
        Err(err) => {
            crate::logger::log_error("media_downloader", &format!("download failed: {err}"));
            emit_plugin_progress(app, "media_downloader", "error", 0, &err);
            Err(err)
        }
    }
}

async fn run_media_download_inner(
    app: &AppHandle,
    mgr: &PluginManager,
    url: &str,
    format: &str,
    output_dir: Option<&str>,
) -> Result<PluginToolOutput, String> {
    let yt_dlp = mgr.engine_binary_path("media_downloader");
    if !yt_dlp.exists() {
        return Err("Media Downloader engine is not installed yet. Install it from Plugins Hub.".into());
    }
    if !mgr.is_engine_ready("media_downloader") {
        return Err("Media Downloader plugin is disabled. Enable it in Plugins Hub.".into());
    }

    let clean_url = validate_media_url(url)?;
    let out_dir = match output_dir.map(str::trim).filter(|s| !s.is_empty()) {
        Some(custom) => crate::agent_tools::resolve_user_path(custom, None)?,
        None => default_downloads_dir(),
    };
    crate::agent_tools::check_path_allowed(&out_dir, true)?;
    if !out_dir.exists() {
        fs::create_dir_all(&out_dir).map_err(|e| format!("Failed creating folder: {e}"))?;
    }

    let ffmpeg_path = mgr.resolve_ffmpeg_path().ok_or_else(|| {
        "FFmpeg dependency is missing. Re-install Lossless Link Downloader or Media / GIF Converter in Plugins.".to_string()
    })?;

    mgr.cancel_task.store(false, Ordering::Relaxed);
    let started_at = SystemTime::now();
    let (label, args) = build_ytdlp_command_args(format, &clean_url, &out_dir, Some(&ffmpeg_path));

    emit_plugin_progress(
        app,
        "media_downloader",
        "running",
        3,
        &format!("Fetching metadata ({label})..."),
    );

    let app_c = app.clone();
    let cancel_watch_done = Arc::new(AtomicBool::new(false));
    let mgr_cancel_poll = &mgr.cancel_task;

    let mut cmd = Command::new(&yt_dlp);
    cmd.args(&args)
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed starting yt-dlp: {e}"))?;
    let child_pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let err_tail = Arc::new(Mutex::new(Vec::<String>::new()));
    let err_tail_c = Arc::clone(&err_tail);
    let stderr_handle = std::thread::spawn(move || {
        if let Some(se) = stderr {
            for_each_lossy_line(se, |line| {
                if let Ok(mut buf) = err_tail_c.lock() {
                    buf.push(line.to_string());
                    if buf.len() > 8 {
                        buf.remove(0);
                    }
                }
            });
        }
    });

    let cancel_mirror = Arc::new(AtomicBool::new(false));
    spawn_cancel_watchdog(child_pid, Arc::clone(&cancel_watch_done), Arc::clone(&cancel_mirror));

    let blocking_fut = tokio::task::spawn_blocking(move || {
        let mut last_printed_file: Option<String> = None;
        if let Some(so) = stdout {
            for_each_lossy_line(so, |trimmed| {
                if let Some((pct, detail)) = parse_ytdlp_progress_line(trimmed) {
                    emit_plugin_progress(&app_c, "media_downloader", "running", pct, &detail);
                } else {
                    let cand_str = trimmed.trim_matches('"');
                    if Path::new(cand_str).is_file() {
                        last_printed_file = Some(cand_str.to_string());
                    }
                }
            });
        }
        let status = child.wait().map_err(|e| format!("Wait error: {e}"))?;
        let _ = stderr_handle.join();
        Ok::<(std::process::ExitStatus, Option<String>), String>((status, last_printed_file))
    });

    while !blocking_fut.is_finished() {
        if mgr_cancel_poll.load(Ordering::Relaxed) {
            cancel_mirror.store(true, Ordering::Relaxed);
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
    let (status, file_opt) = blocking_fut.await.map_err(|e| e.to_string())??;
    cancel_watch_done.store(true, Ordering::Relaxed);

    if mgr.cancel_task.load(Ordering::Relaxed) {
        return Err("Download cancelled.".into());
    }

    if !status.success() {
        let err_msg = err_tail
            .lock()
            .ok()
            .map(|b| b.join("\n"))
            .unwrap_or_default();
        let detail = err_msg.chars().rev().take(280).collect::<String>().chars().rev().collect::<String>();
        return Err(format!("yt-dlp error: {}", detail.trim()));
    }

    let final_path = file_opt
        .or_else(|| find_newest_media_in_dir(&out_dir, started_at))
        .ok_or_else(|| {
            "Download finished, but output file path could not be verified.".to_string()
        })?;
    let file_name = Path::new(&final_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("media")
        .to_string();

    let _ = app.emit(
        "buddy://plugin-progress",
        PluginProgressPayload {
            plugin_id: "media_downloader".into(),
            stage: "done".into(),
            percent: 100,
            detail: file_name.clone(),
        },
    );

    Ok(PluginToolOutput {
        outputs: vec![final_path],
        summary: format!("Saved ({label}): {file_name}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_media_urls_and_builds_lossless_ytdlp_args() {
        assert!(validate_media_url("javascript:alert(1)").is_err());
        assert!(validate_media_url("https://example.com/a b").is_err());
        let ok = validate_media_url("  https://www.youtube.com/watch?v=dQw4w9WgXcQ  ").unwrap();
        assert_eq!(ok, "https://www.youtube.com/watch?v=dQw4w9WgXcQ");

        let out_dir = Path::new("C:\\Users\\Test\\Downloads");
        let ff = Path::new("C:\\AppData\\plugins\\media_gif\\ffmpeg.exe");
        let (lbl_mp4, args_mp4) = build_ytdlp_command_args("mp4_lossless", &ok, out_dir, Some(ff));
        assert_eq!(lbl_mp4, "MP4 (Zero Re-encode)");
        assert!(args_mp4.contains(&"--merge-output-format".to_string()));
        assert!(args_mp4.contains(&"C:\\AppData\\plugins\\media_gif".to_string()));
        assert!(args_mp4.contains(&"--no-playlist".to_string()));

        let (lbl_best, args_best) = build_ytdlp_command_args("audio_best", &ok, out_dir, Some(ff));
        assert_eq!(lbl_best, "Original Audio (Zero Re-encode)");
        assert!(args_best.contains(&"bestaudio/best".to_string()));
        assert!(args_best.contains(&"utf-8".to_string()));
        assert!(!args_best.contains(&"mp3".to_string()));

        let (lbl_flac, args_flac) = build_ytdlp_command_args("audio_flac", &ok, out_dir, Some(ff));
        assert_eq!(lbl_flac, "FLAC (Lossless)");
        assert!(args_flac.contains(&"bestaudio/best".to_string()));
        assert!(args_flac.contains(&"flac".to_string()));

        let (lbl_mp3, args_mp3) = build_ytdlp_command_args("audio_mp3", &ok, out_dir, Some(ff));
        assert_eq!(lbl_mp3, "MP3 320k");
        assert!(args_mp3.contains(&"320K".to_string()));
    }

    #[test]
    fn parses_ytdlp_progress_lines_cleanly() {
        let p = parse_ytdlp_progress_line("[download]  47.8% of   24.30MiB at    3.21MiB/s ETA 00:04").unwrap();
        assert_eq!(p.0, 48);
        assert!(p.1.contains("47.8%"));
        let remux = parse_ytdlp_progress_line("[Merger] Merging formats into \"song.mp4\"").unwrap();
        assert_eq!(remux.0, 94);
        assert!(parse_ytdlp_progress_line("C:\\Users\\Test\\Downloads\\song.mp4").is_none());
    }
}
