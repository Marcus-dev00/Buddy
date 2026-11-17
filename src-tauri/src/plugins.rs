//! Phase 4 Modular Plugin Hub:
//! - Persistent plugin registry & mirror settings (%LOCALAPPDATA%/Buddy/plugins/state.json)
//! - On-demand downloadable engine manager (ffmpeg, qpdf) with streaming progress & SHA-256
//! - Native Windows tools: Windows.Media.Ocr, Restart Manager File Locksmith,
//!   Keep Awake (SetThreadExecutionState), Capsule Timer, Smart Clipboard Parser,
//!   Image Optimizer (batch convert / compress / strip EXIF), Document & PDF Suite.

use crate::media_utils::{self, SelfCopyGuard};
use crate::pin_window::load_dynamic_image_from_source;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::ImageFormat;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex as AsyncMutex;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginInfo {
    pub id: String,
    pub kind: String, // "native" | "engine"
    pub name_en: String,
    pub name_zh: String,
    pub desc_en: String,
    pub desc_zh: String,
    pub enabled: bool,
    pub installed: bool,
    pub version: String,
    pub size_bytes: u64,
    pub sha256: Option<String>,
    pub engine_hint: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct PersistedPluginEntry {
    enabled: bool,
    installed: bool,
    version: String,
    size_bytes: u64,
    sha256: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct PersistedPluginStore {
    #[serde(default)]
    mirror_prefix: String,
    #[serde(default)]
    entries: std::collections::HashMap<String, PersistedPluginEntry>,
}

#[derive(Clone, Serialize)]
pub struct PluginListResponse {
    pub plugins: Vec<PluginInfo>,
    pub mirror_prefix: String,
    pub total_size_bytes: u64,
}

#[derive(Clone, Serialize)]
pub struct PluginProgressPayload {
    pub plugin_id: String,
    pub stage: String, // "downloading" | "extracting" | "running" | "done" | "error"
    pub percent: u32,
    pub detail: String,
}

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

#[derive(Clone, Serialize)]
pub struct TimerTickPayload {
    pub active: bool,
    pub remaining_sec: u64,
    pub total_sec: u64,
    pub label: String,
}

#[derive(Clone, Serialize)]
pub struct PluginToolOutput {
    pub outputs: Vec<String>,
    pub summary: String,
}

pub struct PluginManager {
    plugins_dir: PathBuf,
    state_path: PathBuf,
    store: Mutex<PersistedPluginStore>,
    install_lock: AsyncMutex<()>,
    cancel_download: AtomicBool,
    cancel_task: AtomicBool,
    timer_id: AtomicU64,
    keep_awake_active: AtomicBool,
}

fn default_plugin_defs() -> Vec<PluginInfo> {
    vec![
        PluginInfo {
            id: "offline_ocr".into(),
            kind: "native".into(),
            name_en: "Offline OCR".into(),
            name_zh: "离线 OCR 识字".into(),
            desc_en: "Extract text & code from images in milliseconds via Windows OCR (zero AI tokens).".into(),
            desc_zh: "调用 Windows 原生离线 OCR 毫秒级提取截图与图片文字，不消耗 AI Token。".into(),
            enabled: true,
            installed: true,
            version: "1.0.0".into(),
            size_bytes: 0,
            sha256: None,
            engine_hint: None,
        },
        PluginInfo {
            id: "pin_to_screen".into(),
            kind: "native".into(),
            name_en: "Pin to Screen".into(),
            name_zh: "桌面贴图".into(),
            desc_en: "Pin screenshots above all windows (Win32 native, scroll zoom, Ctrl+scroll opacity).".into(),
            desc_zh: "纯原生 Win32 置顶悬浮贴图，滚轮缩放、Ctrl+滚轮调透明度、双击或右键关闭。".into(),
            enabled: true,
            installed: true,
            version: "1.0.0".into(),
            size_bytes: 0,
            sha256: None,
            engine_hint: None,
        },
        PluginInfo {
            id: "clipboard_parser".into(),
            kind: "native".into(),
            name_en: "Smart Clipboard Parser".into(),
            name_zh: "智能剪贴板解析".into(),
            desc_en: "Offline decoder for Unix timestamps, JSON, Base64, colors, URLs & Clean Paste.".into(),
            desc_zh: "离线秒解时间戳、JSON 格式化、Base64、颜色值、URL 参数与去除 PDF 断行粘贴。".into(),
            enabled: true,
            installed: true,
            version: "1.0.0".into(),
            size_bytes: 0,
            sha256: None,
            engine_hint: None,
        },
        PluginInfo {
            id: "file_locksmith".into(),
            kind: "native".into(),
            name_en: "File Locksmith".into(),
            name_zh: "文件占用解锁".into(),
            desc_en: "Drop locked files to inspect occupying processes (Restart Manager) and unlock them.".into(),
            desc_zh: "拖入删不掉的文件，一键排查占用进程（Windows Restart Manager）并支持结束占用。".into(),
            enabled: true,
            installed: true,
            version: "1.0.0".into(),
            size_bytes: 0,
            sha256: None,
            engine_hint: None,
        },
        PluginInfo {
            id: "color_picker".into(),
            kind: "native".into(),
            name_en: "Screen Color Picker".into(),
            name_zh: "屏幕取色吸管".into(),
            desc_en: "Magnified multi-monitor pixel loupe that copies HEX / RGB / HSL cleanly.".into(),
            desc_zh: "全屏放大镜像素取色器，点击即可复制 HEX / RGB / HSL 色值。".into(),
            enabled: true,
            installed: true,
            version: "1.0.0".into(),
            size_bytes: 0,
            sha256: None,
            engine_hint: None,
        },
        PluginInfo {
            id: "keep_awake_timer".into(),
            kind: "native".into(),
            name_en: "Keep Awake & Capsule Timer".into(),
            name_zh: "防休眠 & 胶囊倒计时".into(),
            desc_en: "Prevent Windows sleep mode and show a glowing countdown ring on the capsule.".into(),
            desc_zh: "防止系统锁屏休眠，并可在灵动岛胶囊边缘显示呼吸倒计时进度条。".into(),
            enabled: true,
            installed: true,
            version: "1.0.0".into(),
            size_bytes: 0,
            sha256: None,
            engine_hint: None,
        },
        PluginInfo {
            id: "image_optimizer".into(),
            kind: "native".into(),
            name_en: "Image Optimizer".into(),
            name_zh: "图片压缩与格式转换".into(),
            desc_en: "Batch compress, resize, convert PNG/JPG/WebP/BMP & strip EXIF metadata.".into(),
            desc_zh: "批量压缩图片体积、调整尺寸、互转 PNG/JPG/WebP/BMP 并清除 EXIF 隐私元数据。".into(),
            enabled: true,
            installed: true,
            version: "1.0.0".into(),
            size_bytes: 0,
            sha256: None,
            engine_hint: None,
        },
        PluginInfo {
            id: "pdf_suite".into(),
            kind: "engine".into(),
            name_en: "Document & PDF Suite".into(),
            name_zh: "文档与 PDF 工具箱".into(),
            desc_en: "Images/Markdown/Office -> PDF built-in; optional QPDF engine (~14 MB) for Merge/Split/Compress.".into(),
            desc_zh: "内置图片/Markdown/Office 转 PDF；可按需下载轻量 QPDF 引擎解锁 PDF 合并/拆分/压缩。".into(),
            enabled: false,
            installed: false,
            version: "latest".into(),
            size_bytes: 0,
            sha256: None,
            engine_hint: Some("qpdf (~14 MB)".into()),
        },
        PluginInfo {
            id: "media_gif".into(),
            kind: "engine".into(),
            name_en: "Media / GIF Converter".into(),
            name_zh: "视频转 GIF 与音视频引擎".into(),
            desc_en: "On-demand FFmpeg static engine to convert screen recordings to GIF, compress MP4 or extract audio.".into(),
            desc_zh: "按需下载独立 FFmpeg 引擎，拖入录屏/视频即可一键转 GIF、压缩 MP4 或提取 MP3。".into(),
            enabled: false,
            installed: false,
            version: "latest".into(),
            size_bytes: 0,
            sha256: None,
            engine_hint: Some("ffmpeg (~95 MB)".into()),
        },
        PluginInfo {
            id: "media_downloader".into(),
            kind: "engine".into(),
            name_en: "Lossless Link Downloader (YTMP3+)".into(),
            name_zh: "音视频链接无损下载".into(),
            desc_en: "On-demand yt-dlp engine (reuses FFmpeg) to save links as Lossless Audio (Original / FLAC), Zero Re-encode MP4, or 320k MP3.".into(),
            desc_zh: "按需下载 yt-dlp 引擎（自动复用 FFmpeg），发链接即可保存原声无损 / FLAC、原画无损封装 MP4 或 320k MP3。".into(),
            enabled: false,
            installed: false,
            version: "latest".into(),
            size_bytes: 0,
            sha256: None,
            engine_hint: Some("yt-dlp (~16 MB) + ffmpeg".into()),
        },
    ]
}

impl PluginManager {
    pub fn init(data_dir: &Path) -> Self {
        let plugins_dir = data_dir.join("plugins");
        let _ = fs::create_dir_all(&plugins_dir);
        let state_path = plugins_dir.join("state.json");

        let store: PersistedPluginStore = fs::read_to_string(&state_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        let mgr = Self {
            plugins_dir,
            state_path,
            store: Mutex::new(store),
            install_lock: AsyncMutex::new(()),
            cancel_download: AtomicBool::new(false),
            cancel_task: AtomicBool::new(false),
            timer_id: AtomicU64::new(0),
            keep_awake_active: AtomicBool::new(false),
        };
        mgr.sync_disk_installs();
        mgr
    }

    fn save_store(&self, guard: &PersistedPluginStore) {
        if let Ok(json) = serde_json::to_string_pretty(guard) {
            let _ = fs::write(&self.state_path, json);
        }
    }

    fn sync_disk_installs(&self) {
        if let Ok(mut g) = self.store.lock() {
            for id in ["pdf_suite", "media_gif", "media_downloader"] {
                let ok = self.engine_binary_path(id).exists();
                let sz = dir_size(&self.plugins_dir.join(id));
                let entry = g.entries.entry(id.into()).or_insert_with(|| PersistedPluginEntry {
                    enabled: ok,
                    installed: ok,
                    version: "latest".into(),
                    size_bytes: sz,
                    sha256: None,
                });
                entry.installed = ok;
                entry.size_bytes = sz;
                if !ok {
                    entry.enabled = false;
                }
            }
            self.save_store(&g);
        }
    }

    pub fn engine_binary_path(&self, id: &str) -> PathBuf {
        match id {
            "pdf_suite" => self.plugins_dir.join("pdf_suite").join("qpdf.exe"),
            "media_gif" => self.plugins_dir.join("media_gif").join("ffmpeg.exe"),
            "media_downloader" => self.plugins_dir.join("media_downloader").join("yt-dlp.exe"),
            _ => self.plugins_dir.join(id),
        }
    }

    pub fn is_engine_ready(&self, id: &str) -> bool {
        let bin_ok = self.engine_binary_path(id).exists();
        if !bin_ok {
            return false;
        }
        self.store
            .lock()
            .ok()
            .and_then(|g| g.entries.get(id).map(|e| e.enabled && e.installed))
            .unwrap_or(false)
    }

    pub fn resolve_ffmpeg_path(&self) -> Option<PathBuf> {
        let bundled = self.engine_binary_path("media_gif");
        if bundled.exists() {
            return Some(bundled);
        }
        let local_copy = self.plugins_dir.join("media_downloader").join("ffmpeg.exe");
        if local_copy.exists() {
            return Some(local_copy);
        }
        if let Some(paths) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&paths) {
                let cand = dir.join("ffmpeg.exe");
                if cand.is_file() {
                    return Some(cand);
                }
            }
        }
        None
    }

    pub fn cancel_active_task(&self) {
        self.cancel_task.store(true, Ordering::Relaxed);
    }

    pub fn list(&self) -> PluginListResponse {
        let g = self.store.lock().map(|x| x.clone()).unwrap_or_default();
        let mut total_size = 0u64;
        let plugins = default_plugin_defs()
            .into_iter()
            .map(|mut p| {
                if let Some(saved) = g.entries.get(&p.id) {
                    p.enabled = saved.enabled;
                    if p.kind == "engine" {
                        p.installed = saved.installed && self.engine_binary_path(&p.id).exists();
                        p.size_bytes = if p.installed { saved.size_bytes } else { 0 };
                        p.sha256 = saved.sha256.clone();
                        if !saved.version.is_empty() {
                            p.version = saved.version.clone();
                        }
                    }
                }
                total_size += p.size_bytes;
                p
            })
            .collect();

        PluginListResponse {
            plugins,
            mirror_prefix: g.mirror_prefix,
            total_size_bytes: total_size,
        }
    }

    pub fn toggle(&self, id: &str, enabled: bool) -> PluginListResponse {
        if let Ok(mut g) = self.store.lock() {
            let base = default_plugin_defs().into_iter().find(|p| p.id == id);
            let entry = g.entries.entry(id.to_string()).or_insert_with(|| {
                let installed = base
                    .as_ref()
                    .map(|b| b.kind == "native")
                    .unwrap_or(false);
                PersistedPluginEntry {
                    enabled,
                    installed,
                    version: "1.0.0".into(),
                    size_bytes: 0,
                    sha256: None,
                }
            });
            entry.enabled = enabled;
            self.save_store(&g);
        }
        self.list()
    }

    pub fn set_mirror_prefix(&self, prefix: &str) -> PluginListResponse {
        if let Ok(mut g) = self.store.lock() {
            g.mirror_prefix = prefix.trim().to_string();
            self.save_store(&g);
        }
        self.list()
    }

    pub fn cancel_active_download(&self) {
        self.cancel_download.store(true, Ordering::Relaxed);
    }

    pub fn uninstall_engine(&self, id: &str) -> Result<PluginListResponse, String> {
        let dir = self.plugins_dir.join(id);
        if dir.exists() {
            fs::remove_dir_all(&dir).map_err(|e| format!("Failed to clean {}: {e}", dir.display()))?;
        }
        if let Ok(mut g) = self.store.lock() {
            if let Some(e) = g.entries.get_mut(id) {
                e.installed = false;
                e.enabled = false;
                e.size_bytes = 0;
                e.sha256 = None;
            }
            self.save_store(&g);
        }
        Ok(self.list())
    }

    async fn stream_download_to_temp(
        &self,
        client: &reqwest::Client,
        app: &AppHandle,
        progress_plugin_id: &str,
        final_url: &str,
        temp_path: &Path,
        pct_start: u32,
        pct_span: u32,
        label: &str,
    ) -> Result<String, String> {
        if temp_path.exists() {
            let _ = fs::remove_file(temp_path);
        }
        let mut resp = client
            .get(final_url)
            .send()
            .await
            .map_err(|e| format!("Download failed ({label}): {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {} downloading {label}", resp.status()));
        }

        let total = resp.content_length().unwrap_or(0);
        let mut downloaded: u64 = 0;
        let mut hasher = Sha256::new();
        let mut file = File::create(temp_path).map_err(|e| e.to_string())?;
        let mut last_pct = 0u32;

        loop {
            if self.cancel_download.load(Ordering::Relaxed) {
                drop(file);
                let _ = fs::remove_file(temp_path);
                return Err("Download cancelled".into());
            }
            let chunk_opt = resp.chunk().await.map_err(|e| {
                let _ = fs::remove_file(temp_path);
                format!("Stream error ({label}): {e}")
            })?;
            let Some(chunk) = chunk_opt else {
                break;
            };
            hasher.update(&chunk);
            file.write_all(&chunk).map_err(|e| e.to_string())?;
            downloaded += chunk.len() as u64;

            let rel_pct = if total > 0 {
                ((downloaded * pct_span as u64) / total).min(pct_span as u64) as u32
            } else {
                pct_span / 2
            };
            let pct = (pct_start + rel_pct).clamp(2, 96);
            if pct != last_pct {
                last_pct = pct;
                let mb = downloaded as f64 / (1024.0 * 1024.0);
                let _ = app.emit(
                    "buddy://plugin-progress",
                    PluginProgressPayload {
                        plugin_id: progress_plugin_id.into(),
                        stage: "downloading".into(),
                        percent: pct,
                        detail: format!("{label}: {mb:.1} MB"),
                    },
                );
            }
        }
        drop(file);
        Ok(format!("{:x}", hasher.finalize()))
    }

    fn record_installed_engine(
        &self,
        id: &str,
        version_tag: String,
        size_bytes: u64,
        sha_hex: String,
    ) {
        if let Ok(mut g) = self.store.lock() {
            g.entries.insert(
                id.to_string(),
                PersistedPluginEntry {
                    enabled: true,
                    installed: true,
                    version: version_tag,
                    size_bytes,
                    sha256: Some(sha_hex),
                },
            );
            self.save_store(&g);
        }
    }

    async fn install_zip_engine(
        &self,
        client: &reqwest::Client,
        app: &AppHandle,
        progress_plugin_id: &str,
        engine_id: &str,
        mirror: &str,
        pct_start: u32,
        pct_span: u32,
    ) -> Result<(), String> {
        let (download_url, version_tag) = if engine_id == "pdf_suite" {
            resolve_qpdf_asset(client).await?
        } else {
            resolve_ffmpeg_asset(client).await?
        };
        let final_url = apply_mirror(&download_url, mirror);
        let temp_zip = self.plugins_dir.join(format!("{engine_id}_download.zip.tmp"));
        let label = if engine_id == "pdf_suite" { "qpdf" } else { "ffmpeg" };

        let sha_hex = self
            .stream_download_to_temp(
                client,
                app,
                progress_plugin_id,
                &final_url,
                &temp_zip,
                pct_start,
                pct_span,
                label,
            )
            .await?;

        let _ = app.emit(
            "buddy://plugin-progress",
            PluginProgressPayload {
                plugin_id: progress_plugin_id.into(),
                stage: "extracting".into(),
                percent: (pct_start + pct_span + 2).min(97),
                detail: format!("Extracting {label}..."),
            },
        );

        let dest_dir = self.plugins_dir.join(engine_id);
        let temp_zip_clone = temp_zip.clone();
        let dest_clone = dest_dir.clone();
        let id_str = engine_id.to_string();
        let extract_res = tokio::task::spawn_blocking(move || {
            extract_engine_zip(&id_str, &temp_zip_clone, &dest_clone)
        })
        .await
        .map_err(|e| e.to_string())?;

        let _ = fs::remove_file(&temp_zip);
        extract_res?;

        if !self.engine_binary_path(engine_id).exists() {
            let _ = fs::remove_dir_all(&dest_dir);
            return Err("Downloaded archive did not contain expected binary".into());
        }

        let size_bytes = dir_size(&dest_dir);
        self.record_installed_engine(engine_id, version_tag, size_bytes, sha_hex);
        Ok(())
    }

    pub async fn install_engine(&self, app: &AppHandle, id: &str) -> Result<PluginListResponse, String> {
        let _install_guard = self.install_lock.lock().await;
        match self.install_engine_inner(app, id).await {
            Ok(res) => Ok(res),
            Err(err) => {
                crate::logger::log_error("plugin", &format!("install `{id}` failed: {err}"));
                let _ = app.emit(
                    "buddy://plugin-progress",
                    PluginProgressPayload {
                        plugin_id: id.into(),
                        stage: "error".into(),
                        percent: 0,
                        detail: err.chars().take(80).collect(),
                    },
                );
                Err(err)
            }
        }
    }

    async fn install_engine_inner(&self, app: &AppHandle, id: &str) -> Result<PluginListResponse, String> {
        if id != "pdf_suite" && id != "media_gif" && id != "media_downloader" {
            return Err("Only engine plugins require download".into());
        }
        self.cancel_download.store(false, Ordering::Relaxed);

        let mirror = self
            .store
            .lock()
            .map(|g| g.mirror_prefix.clone())
            .unwrap_or_default();

        let _ = app.emit(
            "buddy://plugin-progress",
            PluginProgressPayload {
                plugin_id: id.into(),
                stage: "downloading".into(),
                percent: 2,
                detail: "Resolving latest release package...".into(),
            },
        );

        let client = reqwest::Client::builder()
            .user_agent("Buddy-Plugin-Manager/1.0")
            .timeout(Duration::from_secs(600))
            .build()
            .map_err(|e| e.to_string())?;

        if id == "media_downloader" {
            let need_ffmpeg = self.resolve_ffmpeg_path().is_none();
            let yt_span = if need_ffmpeg { 42 } else { 88 };
            let (dl_url, ver_tag) = resolve_ytdlp_asset(&client).await?;
            let final_url = apply_mirror(&dl_url, &mirror);
            let dest_dir = self.plugins_dir.join("media_downloader");
            fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;
            let temp_exe = dest_dir.join("yt-dlp.exe.tmp");
            let sha_hex = self
                .stream_download_to_temp(
                    &client,
                    app,
                    "media_downloader",
                    &final_url,
                    &temp_exe,
                    2,
                    yt_span,
                    "yt-dlp.exe",
                )
                .await?;

            // Validate Windows PE signature ("MZ") and reasonable size before committing
            let valid_pe = File::open(&temp_exe)
                .and_then(|mut f| {
                    let meta_len = f.metadata().map(|m| m.len()).unwrap_or(0);
                    let mut magic = [0u8; 2];
                    let _ = f.read(&mut magic)?;
                    Ok(meta_len >= 1_000_000 && &magic == b"MZ")
                })
                .unwrap_or(false);
            if !valid_pe {
                let _ = fs::remove_file(&temp_exe);
                return Err("Downloaded yt-dlp binary failed PE verification.".into());
            }

            if need_ffmpeg {
                if let Err(ff_err) = self
                    .install_zip_engine(&client, app, "media_downloader", "media_gif", &mirror, 45, 45)
                    .await
                {
                    let _ = fs::remove_file(&temp_exe);
                    return Err(ff_err);
                }
            }

            let final_exe = self.engine_binary_path("media_downloader");
            if final_exe.exists() {
                let _ = fs::remove_file(&final_exe);
            }
            fs::rename(&temp_exe, &final_exe).map_err(|e| e.to_string())?;
            let sz = dir_size(&dest_dir);
            self.record_installed_engine("media_downloader", ver_tag, sz, sha_hex);
        } else {
            self.install_zip_engine(&client, app, id, id, &mirror, 2, 85)
                .await?;
        }

        let _ = app.emit(
            "buddy://plugin-progress",
            PluginProgressPayload {
                plugin_id: id.into(),
                stage: "done".into(),
                percent: 100,
                detail: "Ready".into(),
            },
        );
        Ok(self.list())
    }

    pub fn set_keep_awake(&self, enable: bool) -> Result<bool, String> {
        #[cfg(windows)]
        unsafe {
            use windows::Win32::System::Power::{
                SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
            };
            let flags = if enable {
                ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED
            } else {
                ES_CONTINUOUS
            };
            let prev = SetThreadExecutionState(flags);
            if prev.0 == 0 && enable {
                return Err("SetThreadExecutionState failed".into());
            }
        }
        self.keep_awake_active.store(enable, Ordering::Relaxed);
        Ok(enable)
    }

    pub fn is_keep_awake_active(&self) -> bool {
        self.keep_awake_active.load(Ordering::Relaxed)
    }

    pub fn start_capsule_timer(self: &Arc<Self>, app: AppHandle, seconds: u64, label: String) {
        let secs = seconds.clamp(5, 24 * 3600);
        let tid = self.timer_id.fetch_add(1, Ordering::Relaxed) + 1;
        let this = self.clone();

        std::thread::spawn(move || {
            let mut rem = secs;
            loop {
                if this.timer_id.load(Ordering::Relaxed) != tid {
                    return;
                }
                let _ = app.emit(
                    "buddy://timer-tick",
                    TimerTickPayload {
                        active: rem > 0,
                        remaining_sec: rem,
                        total_sec: secs,
                        label: label.clone(),
                    },
                );
                if rem == 0 {
                    let _ = app.emit("buddy://timer-done", label);
                    return;
                }
                std::thread::sleep(Duration::from_secs(1));
                rem -= 1;
            }
        });
    }

    pub fn cancel_capsule_timer(&self, app: &AppHandle) {
        self.timer_id.fetch_add(1, Ordering::Relaxed);
        let _ = app.emit(
            "buddy://timer-tick",
            TimerTickPayload {
                active: false,
                remaining_sec: 0,
                total_sec: 0,
                label: String::new(),
            },
        );
    }
}

fn dir_size(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    let mut total = 0u64;
    if let Ok(rd) = fs::read_dir(path) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                total += dir_size(&p);
            } else if let Ok(meta) = e.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

fn apply_mirror(url: &str, mirror_prefix: &str) -> String {
    let m = mirror_prefix.trim();
    if m.is_empty() || !url.starts_with("https://github.com/") {
        return url.to_string();
    }
    if m.ends_with('/') {
        format!("{m}{url}")
    } else {
        format!("{m}/{url}")
    }
}

async fn resolve_qpdf_asset(client: &reqwest::Client) -> Result<(String, String), String> {
    let rel: serde_json::Value = client
        .get("https://api.github.com/repos/qpdf/qpdf/releases/latest")
        .send()
        .await
        .map_err(|e| format!("Failed querying qpdf release: {e}"))?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    let tag = rel["tag_name"]
        .as_str()
        .unwrap_or("latest")
        .trim_start_matches('v')
        .to_string();
    let assets = rel["assets"].as_array().ok_or("No assets in qpdf release")?;
    for a in assets {
        let name = a["name"].as_str().unwrap_or("");
        if name.ends_with("msvc64.zip") || name.ends_with("mingw64.zip") {
            if let Some(url) = a["browser_download_url"].as_str() {
                return Ok((url.to_string(), tag));
            }
        }
    }
    Err("No Windows x64 zip found in latest qpdf release".into())
}

async fn resolve_ffmpeg_asset(client: &reqwest::Client) -> Result<(String, String), String> {
    if let Ok(resp) = client
        .get("https://api.github.com/repos/BtbN/FFmpeg-Builds/releases/tags/latest")
        .send()
        .await
    {
        if let Ok(rel) = resp.json::<serde_json::Value>().await {
            if let Some(assets) = rel["assets"].as_array() {
                // Prefer lighter gpl-shared archive (~91 MB) before 202 MB static archive
                for preferred in [
                    "ffmpeg-master-latest-win64-gpl-shared.zip",
                    "ffmpeg-master-latest-win64-gpl.zip",
                ] {
                    if let Some(url) = assets.iter().find_map(|a| {
                        let name = a["name"].as_str().unwrap_or("");
                        if name == preferred {
                            a["browser_download_url"].as_str().map(str::to_string)
                        } else {
                            None
                        }
                    }) {
                        return Ok((url, "latest".into()));
                    }
                }
            }
        }
    }
    Ok((
        "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl-shared.zip"
            .into(),
        "latest".into(),
    ))
}

async fn resolve_ytdlp_asset(client: &reqwest::Client) -> Result<(String, String), String> {
    if let Ok(resp) = client
        .get("https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest")
        .send()
        .await
    {
        if let Ok(rel) = resp.json::<serde_json::Value>().await {
            let tag = rel["tag_name"].as_str().unwrap_or("latest").to_string();
            if let Some(assets) = rel["assets"].as_array() {
                for a in assets {
                    let name = a["name"].as_str().unwrap_or("");
                    if name == "yt-dlp.exe" {
                        if let Some(url) = a["browser_download_url"].as_str() {
                            return Ok((url.to_string(), tag));
                        }
                    }
                }
            }
        }
    }
    Ok((
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe".into(),
        "latest".into(),
    ))
}

fn extract_engine_zip(id: &str, zip_path: &Path, dest_dir: &Path) -> Result<(), String> {
    let parent = dest_dir.parent().unwrap_or_else(|| Path::new("."));
    let staging = parent.join(format!(".staging_{id}_{}", std::process::id()));
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    fs::create_dir_all(&staging).map_err(|e| e.to_string())?;

    let work_res: Result<(), String> = (|| {
        let file = File::open(zip_path).map_err(|e| format!("Cannot open downloaded zip: {e}"))?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(|e| format!("Invalid zip archive: {e}"))?;

        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
            if entry.is_dir() {
                continue;
            }
            let raw_name = entry.name().replace('\\', "/");
            let file_name = Path::new(&raw_name)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();

            let keep = match id {
                "media_gif" => {
                    (raw_name.contains("/bin/") || !raw_name.contains('/'))
                        && (file_name == "ffmpeg.exe" || file_name.ends_with(".dll"))
                }
                "pdf_suite" => {
                    raw_name.contains("/bin/")
                        && (file_name.ends_with(".exe") || file_name.ends_with(".dll"))
                }
                _ => false,
            };

            if keep {
                let out_path = staging.join(
                    Path::new(&raw_name)
                        .file_name()
                        .ok_or("Invalid filename in archive")?,
                );
                let mut out_file = File::create(&out_path).map_err(|e| e.to_string())?;
                std::io::copy(&mut entry, &mut out_file).map_err(|e| e.to_string())?;
            }
        }

        let expected = match id {
            "media_gif" => "ffmpeg.exe",
            "pdf_suite" => "qpdf.exe",
            _ => "",
        };
        if !expected.is_empty() && !staging.join(expected).exists() {
            return Err(format!("Downloaded archive did not contain `{expected}`"));
        }
        Ok(())
    })();

    if let Err(e) = work_res {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }

    if dest_dir.exists() {
        fs::remove_dir_all(dest_dir).map_err(|e| {
            let _ = fs::remove_dir_all(&staging);
            format!("Failed clearing old plugin directory: {e}")
        })?;
    }
    fs::rename(&staging, dest_dir).map_err(|e| {
        let _ = fs::remove_dir_all(&staging);
        format!("Failed committing extracted plugin: {e}")
    })?;
    Ok(())
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
    // Object numbering:
    // 1 = Catalog, 2 = Pages
    // For page i (0..n): page_obj = 3 + i*3, content_obj = 4 + i*3, img_obj = 5 + i*3
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
        // Scale into readable PDF points (max 595 x 842 A4 bounds)
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
            // compress
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
    // qpdf returns 0 on success, 3 on warnings-only success
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
            // Best native original audio stream (no lossy re-encoding; never fetch full 4K video)
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

    // Forward stderr lines lossily in background so pipes never deadlock or stop on non-UTF-8
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
    fn extract_engine_zip_handles_shared_ffmpeg_and_cleans_on_invalid_zip() {
        let base = std::env::temp_dir().join(format!("buddy_extract_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        let dest = base.join("media_gif");
        let bad_zip = base.join("bad.zip");
        fs::write(&bad_zip, b"not a zip").unwrap();
        assert!(extract_engine_zip("media_gif", &bad_zip, &dest).is_err());
        assert!(!dest.exists(), "failed extraction must never leave empty folder");

        let ok_zip = base.join("ok.zip");
        {
            let f = File::create(&ok_zip).unwrap();
            let mut zw = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default();
            zw.start_file("ffmpeg-master-latest-win64-gpl-shared/bin/ffmpeg.exe", opts).unwrap();
            zw.write_all(b"MZ_FAKE_FFMPEG").unwrap();
            zw.start_file("ffmpeg-master-latest-win64-gpl-shared/bin/avcodec-63.dll", opts).unwrap();
            zw.write_all(b"FAKE_DLL").unwrap();
            zw.start_file("ffmpeg-master-latest-win64-gpl-shared/bin/ffplay.exe", opts).unwrap();
            zw.write_all(b"SKIP_ME").unwrap();
            zw.finish().unwrap();
        }
        extract_engine_zip("media_gif", &ok_zip, &dest).unwrap();
        assert!(dest.join("ffmpeg.exe").exists());
        assert!(dest.join("avcodec-63.dll").exists());
        assert!(!dest.join("ffplay.exe").exists());

        let _ = fs::remove_dir_all(&base);
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
