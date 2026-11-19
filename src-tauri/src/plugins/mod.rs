//! Phase 4 Modular Plugin Hub:
//! - Persistent plugin registry & mirror settings (%LOCALAPPDATA%/Buddy/plugins/state.json)
//! - On-demand downloadable engine manager (ffmpeg, qpdf) with streaming progress & SHA-256
//! - Native Windows tools: Windows.Media.Ocr, Restart Manager File Locksmith,
//!   Keep Awake (SetThreadExecutionState), Capsule Timer, Smart Clipboard Parser,
//!   Image Optimizer (batch convert / compress / strip EXIF), Document & PDF Suite.

mod media;
mod system_tools;

pub use media::{
    optimize_images, run_media_download, run_media_tool, run_pdf_tool, PluginToolOutput,
};
pub use system_tools::{
    apply_clean_paste_text, inspect_file_locks, open_output_in_explorer, parse_clipboard_text,
    run_offline_ocr, terminate_locking_process, ClipboardParseResult, LockProcessInfo, OcrResult,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex as AsyncMutex;

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
pub struct TimerTickPayload {
    pub active: bool,
    pub remaining_sec: u64,
    pub total_sec: u64,
    pub label: String,
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

#[cfg(test)]
mod tests {
    use super::*;

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

}
