//! Built-in WebView2 Companion Browser Manager:
//! - Launches an isolated WebView2 window (`buddy_browser`) with persistent cookies under `<data_dir>/browser_profile`.
//! - Runs silently offscreen while the user chooses not to watch, capturing live thumbnails for the Buddy chat card.
//! - Pops onto the active screen when the user clicks "观看" and magnetically docks Buddy beside it (`win32_sensors`).
//! - Supports human takeover ("我来操作") and immediate memory cleanup when closed or background tasks complete.

pub mod bridge;

use crate::agent_tools::check_path_allowed;
use crate::win32_sensors::{
    attach_dock_to_window, detach_dock_from_window, get_primary_work_area, AppSensorState,
};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use windows::Win32::Foundation::{COLORREF, HWND};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetForegroundWindow, SetLayeredWindowAttributes, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, GWL_EXSTYLE, HWND_TOP, LWA_ALPHA, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOSIZE, SWP_NOZORDER, SW_SHOW, SW_SHOWNOACTIVATE, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW,
};

pub const BROWSER_WINDOW_LABEL: &str = "buddy_browser";

#[derive(Clone, Debug, Serialize)]
pub struct BrowserStatePayload {
    pub active: bool,
    pub watching: bool,
    pub user_takeover: bool,
    pub navigating: bool,
    pub url: String,
    pub title: String,
    pub action_label: String,
    pub needs_attention: bool,
    pub last_error: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct BrowserPreviewPayload {
    pub data_url: String,
    pub timestamp_ms: u64,
}

#[derive(Default)]
struct BrowserRuntime {
    active: bool,
    watching: bool,
    user_takeover: bool,
    navigating: bool,
    url: String,
    title: String,
    action_label: String,
    needs_attention: bool,
    last_error: String,
    window_hwnd: isize,
    generation: u64,
}

pub struct BrowserManager {
    profile_dir: PathBuf,
    runtime: Mutex<BrowserRuntime>,
    closing_guard: AtomicBool,
    gen_counter: AtomicU64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn looks_like_windows_local_path(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

fn looks_like_local_http_host(s: &str) -> bool {
    let low = s.to_ascii_lowercase();
    low == "localhost"
        || low.starts_with("localhost:")
        || low.starts_with("localhost/")
        || low.starts_with("127.")
        || low.starts_with("[::1]")
        || low.starts_with("0.0.0.0")
        || low.starts_with("192.168.")
        || low.starts_with("10.")
        || low.starts_with("172.16.")
        || low.starts_with("172.17.")
        || low.starts_with("172.18.")
        || low.starts_with("172.19.")
        || low.starts_with("172.2")
        || low.starts_with("172.30.")
        || low.starts_with("172.31.")
}

/// Validates and normalizes URLs for Buddy's interactive built-in WebView2 browser.
/// Behaves like a standard desktop browser: allows public http(s), localhost/127.0.0.1/LAN,
/// and local `file://` files (still guarding protected OS/credential paths & dangerous schemes).
pub fn validate_browser_url(raw: &str) -> Result<reqwest::Url, String> {
    let trimmed = raw.trim().trim_matches('"');
    if trimmed.is_empty() {
        return Err("URL cannot be empty.".to_string());
    }
    if trimmed.eq_ignore_ascii_case("about:blank") {
        return reqwest::Url::parse("about:blank").map_err(|e| e.to_string());
    }

    // Bare Windows path like `C:\Users\...\index.html` -> `file:///C:/Users/.../index.html`
    if looks_like_windows_local_path(trimmed) {
        let path = Path::new(trimmed);
        check_path_allowed(path, false)?;
        return reqwest::Url::from_file_path(path)
            .map_err(|_| format!("Invalid local file path: {trimmed}"));
    }

    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else if trimmed.starts_with("javascript:")
        || trimmed.starts_with("vbscript:")
        || trimmed.starts_with("data:")
        || trimmed.starts_with("ms-")
    {
        trimmed.to_string()
    } else if looks_like_local_http_host(trimmed) {
        format!("http://{trimmed}")
    } else {
        format!("https://{trimmed}")
    };

    let url = reqwest::Url::parse(&candidate).map_err(|e| format!("Invalid URL: {e}"))?;
    match url.scheme() {
        "http" | "https" => {
            let host = url
                .host_str()
                .ok_or_else(|| "URL is missing a host.".to_string())?
                .to_ascii_lowercase();
            // Still guard cloud instance metadata IP just in case
            if host == "169.254.169.254" || host == "metadata.google.internal" {
                return Err("Access to cloud metadata endpoints is blocked.".to_string());
            }
            Ok(url)
        }
        "file" => {
            let path = url
                .to_file_path()
                .map_err(|_| "Invalid local file:// URL.".to_string())?;
            check_path_allowed(&path, false)?;
            Ok(url)
        }
        other => Err(format!(
            "Unsupported browser URL scheme `{other}:` (allowed: http, https, file)."
        )),
    }
}

impl BrowserManager {
    pub fn init(data_dir: &std::path::Path) -> Self {
        let profile_dir = data_dir.join("browser_profile");
        let _ = std::fs::create_dir_all(&profile_dir);
        Self {
            profile_dir,
            runtime: Mutex::new(BrowserRuntime::default()),
            closing_guard: AtomicBool::new(false),
            gen_counter: AtomicU64::new(0),
        }
    }

    pub fn get_state(&self) -> BrowserStatePayload {
        if let Ok(rt) = self.runtime.lock() {
            BrowserStatePayload {
                active: rt.active,
                watching: rt.watching,
                user_takeover: rt.user_takeover,
                navigating: rt.navigating,
                url: rt.url.clone(),
                title: rt.title.clone(),
                action_label: rt.action_label.clone(),
                needs_attention: rt.needs_attention,
                last_error: rt.last_error.clone(),
            }
        } else {
            BrowserStatePayload {
                active: false,
                watching: false,
                user_takeover: false,
                navigating: false,
                url: String::new(),
                title: String::new(),
                action_label: String::new(),
                needs_attention: false,
                last_error: String::new(),
            }
        }
    }

    pub fn emit_state(&self, app: &AppHandle) {
        let _ = app.emit("buddy://browser-state", self.get_state());
    }

    pub fn is_user_takeover(&self) -> bool {
        self.runtime
            .lock()
            .map(|rt| rt.active && rt.user_takeover)
            .unwrap_or(false)
    }

    pub fn is_watching(&self) -> bool {
        self.runtime
            .lock()
            .map(|rt| rt.active && rt.watching)
            .unwrap_or(false)
    }

    pub fn set_action_label(&self, app: &AppHandle, label: impl Into<String>) {
        if let Ok(mut rt) = self.runtime.lock() {
            if !rt.active {
                return;
            }
            rt.action_label = label.into();
        }
        self.emit_state(app);
    }

    pub fn set_user_takeover(&self, app: &AppHandle, takeover: bool) -> BrowserStatePayload {
        if let Ok(mut rt) = self.runtime.lock() {
            if rt.active {
                rt.user_takeover = takeover;
                rt.needs_attention = false;
                rt.action_label = if takeover {
                    "User taking control".to_string()
                } else {
                    "Buddy controlling browser".to_string()
                };
            }
        }
        self.emit_state(app);
        self.get_state()
    }

    #[cfg(windows)]
    fn apply_window_mode(&self, win: &WebviewWindow, watching: bool) {
        bridge::set_muted(win, !watching);
        if let Ok(raw_hwnd) = win.hwnd() {
            let hwnd = HWND(raw_hwnd.0 as *mut c_void);
            unsafe {
                let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                let hidden_flags =
                    (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0 | WS_EX_LAYERED.0) as isize;
                if watching {
                    let next_style = style & !hidden_flags;
                    if next_style != style {
                        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next_style);
                    }
                    let (wx, wy, ww, wh) = get_primary_work_area();
                    let scale = win.scale_factor().unwrap_or(1.0);
                    let top_reserve = (62.0 * scale).round() as i32;
                    let bw = (ww - 64).clamp((680.0 * scale) as i32, (1120.0 * scale) as i32);
                    let bh = (wh - top_reserve - 28)
                        .clamp((480.0 * scale) as i32, (760.0 * scale) as i32);
                    let bx = wx + ((ww - bw) / 2).max(16);
                    let by = wy + top_reserve;
                    let _ = SetWindowPos(
                        hwnd,
                        HWND_TOP,
                        bx,
                        by,
                        bw,
                        bh,
                        SWP_FRAMECHANGED,
                    );
                    let _ = ShowWindow(hwnd, SW_SHOW);
                    let _ = SetForegroundWindow(hwnd);
                } else {
                    let next_style = style | hidden_flags;
                    if next_style != style {
                        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next_style);
                    }
                    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA);
                    let _ = SetWindowPos(
                        hwnd,
                        HWND_TOP,
                        -24000,
                        -24000,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                    );
                    // Keep WebView2 compositor active at 0% opacity offscreen without ever appearing on screen
                    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                }
            }
        }
    }

    #[cfg(not(windows))]
    fn apply_window_mode(&self, win: &WebviewWindow, watching: bool) {
        if watching {
            let _ = win.show();
        } else {
            let _ = win.hide();
        }
    }

    fn ensure_or_navigate_window(
        self: &Arc<Self>,
        app: &AppHandle,
        sensor_state: &Arc<AppSensorState>,
        parsed_url: reqwest::Url,
        watch_now: bool,
    ) -> Result<(WebviewWindow, u64), String> {
        if let Some(existing) = app.get_webview_window(BROWSER_WINDOW_LABEL) {
            let gen = if let Ok(mut rt) = self.runtime.lock() {
                rt.navigating = true;
                rt.needs_attention = false;
                rt.url = parsed_url.to_string();
                rt.last_error.clear();
                rt.generation
            } else {
                0
            };
            existing
                .navigate(parsed_url)
                .map_err(|e| format!("Failed to navigate browser: {e}"))?;
            if watch_now {
                self.set_watching(app, sensor_state, true);
            }
            self.emit_state(app);
            return Ok((existing, gen));
        }

        let gen = self.gen_counter.fetch_add(1, Ordering::Relaxed) + 1;
        let url_str = parsed_url.to_string();
        let app_nav = app.clone();
        let mgr_nav = Arc::clone(self);
        let app_pop = app.clone();
        let mgr_pop = Arc::clone(self);

        let builder = WebviewWindowBuilder::new(
            app,
            BROWSER_WINDOW_LABEL,
            WebviewUrl::External(parsed_url),
        )
        .title("Buddy Browser")
        .inner_size(1020.0, 720.0)
        .min_inner_size(520.0, 420.0)
        .position(-24000.0, -24000.0)
        .visible(watch_now)
        .skip_taskbar(!watch_now)
        .decorations(true)
        .resizable(true)
        .focused(watch_now)
        .data_directory(self.profile_dir.clone())
        .initialization_script(bridge::CONSOLE_CAPTURE_INIT_JS)
        .on_navigation(move |nav_url| {
            let raw = nav_url.as_str();
            if raw == "about:blank" || raw == "about:srcdoc" || raw.starts_with("blob:") {
                return true;
            }
            if validate_browser_url(raw).is_err() {
                crate::logger::log_warn("browser", &format!("blocked unsafe navigation target: {raw}"));
                return false;
            }
            if let Ok(mut rt) = mgr_nav.runtime.lock() {
                if rt.generation == gen {
                    rt.url = raw.to_string();
                    rt.needs_attention = false;
                }
            }
            let _ = app_nav.emit("buddy://browser-state", mgr_nav.get_state());
            true
        })
        .on_new_window(move |popup_url, _features| {
            let raw = popup_url.as_str().to_string();
            if validate_browser_url(&raw).is_ok() {
                if let Some(w) = app_pop.get_webview_window(BROWSER_WINDOW_LABEL) {
                    let _ = w.navigate(popup_url);
                }
                if let Ok(mut rt) = mgr_pop.runtime.lock() {
                    if rt.generation == gen {
                        rt.url = raw;
                    }
                }
            }
            tauri::webview::NewWindowResponse::Deny
        });

        let win = builder
            .build()
            .map_err(|e| format!("Failed to launch built-in browser: {e}"))?;

        #[cfg(windows)]
        let hwnd_val = win
            .hwnd()
            .map(|h| h.0 as isize)
            .unwrap_or(0);
        #[cfg(not(windows))]
        let hwnd_val = 0isize;

        if let Ok(mut rt) = self.runtime.lock() {
            *rt = BrowserRuntime {
                active: true,
                watching: watch_now,
                user_takeover: false,
                navigating: true,
                url: url_str,
                title: "Loading...".to_string(),
                action_label: "Opening page...".to_string(),
                needs_attention: false,
                last_error: String::new(),
                window_hwnd: hwnd_val,
                generation: gen,
            };
        }

        self.apply_window_mode(&win, watch_now);
        if watch_now {
            if let Some(main_win) = app.get_webview_window("main") {
                if hwnd_val != 0 {
                    attach_dock_to_window(app, &main_win, sensor_state, hwnd_val, "top", 0.5);
                }
            }
        }

        let app_close = app.clone();
        let ss_close = Arc::clone(sensor_state);
        let mgr_close = Arc::clone(self);
        win.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { .. } = event {
                mgr_close.handle_window_closed(&app_close, &ss_close, gen);
            }
        });

        self.spawn_preview_loop(app.clone(), gen);
        self.emit_state(app);
        Ok((win, gen))
    }

    fn handle_window_closed(&self, app: &AppHandle, sensor_state: &AppSensorState, gen: u64) {
        if self.closing_guard.swap(true, Ordering::SeqCst) {
            return;
        }
        let target_hwnd = {
            let Ok(mut rt) = self.runtime.lock() else {
                self.closing_guard.store(false, Ordering::SeqCst);
                return;
            };
            if rt.generation != gen {
                self.closing_guard.store(false, Ordering::SeqCst);
                return;
            }
            let h = rt.window_hwnd;
            *rt = BrowserRuntime {
                generation: rt.generation.wrapping_add(1),
                ..Default::default()
            };
            h
        };
        if target_hwnd != 0 {
            let docked_to_browser = sensor_state
                .window_dock
                .lock()
                .ok()
                .and_then(|g| g.as_ref().map(|d| d.target_hwnd == target_hwnd))
                .unwrap_or(false);
            if docked_to_browser {
                if let Some(main_win) = app.get_webview_window("main") {
                    detach_dock_from_window(app, &main_win, sensor_state);
                }
            }
        }
        self.closing_guard.store(false, Ordering::SeqCst);
        self.emit_state(app);
    }

    fn spawn_preview_loop(self: &Arc<Self>, app: AppHandle, gen: u64) {
        let mgr = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(1200)).await;
                let (still_active, watching) = mgr
                    .runtime
                    .lock()
                    .map(|rt| (rt.active && rt.generation == gen, rt.watching))
                    .unwrap_or((false, false));
                if !still_active {
                    break;
                }
                let Some(win) = app.get_webview_window(BROWSER_WINDOW_LABEL) else {
                    break;
                };
                // Sync title in case page navigated dynamically via user clicks
                if let Ok(title_val) = bridge::eval_json(&win, "(() => ({ title: document.title || '', url: location.href }))()").await {
                    if let Ok(mut rt) = mgr.runtime.lock() {
                        if rt.generation == gen {
                            if let Some(t) = title_val["title"].as_str() {
                                if !t.trim().is_empty() {
                                    rt.title = t.trim().to_string();
                                }
                            }
                            if let Some(u) = title_val["url"].as_str() {
                                if !u.trim().is_empty() {
                                    rt.url = u.trim().to_string();
                                }
                            }
                        }
                    }
                    mgr.emit_state(&app);
                }
                // Only capture lightweight preview thumbnail for the mini card when not in big Watch window
                if !watching {
                    if let Ok(b64) = bridge::capture_preview_jpeg(&win, 520, 74).await {
                        let _ = app.emit(
                            "buddy://browser-preview",
                            BrowserPreviewPayload {
                                data_url: format!("data:image/jpeg;base64,{b64}"),
                                timestamp_ms: now_ms(),
                            },
                        );
                    }
                }
            }
        });
    }

    pub async fn refresh_preview_once(&self, app: &AppHandle) {
        if let Some(win) = app.get_webview_window(BROWSER_WINDOW_LABEL) {
            if let Ok(b64) = bridge::capture_preview_jpeg(&win, 520, 76).await {
                let _ = app.emit(
                    "buddy://browser-preview",
                    BrowserPreviewPayload {
                        data_url: format!("data:image/jpeg;base64,{b64}"),
                        timestamp_ms: now_ms(),
                    },
                );
            }
        }
    }

    pub fn set_watching(
        self: &Arc<Self>,
        app: &AppHandle,
        sensor_state: &AppSensorState,
        watching: bool,
    ) -> BrowserStatePayload {
        let Some(win) = app.get_webview_window(BROWSER_WINDOW_LABEL) else {
            return self.get_state();
        };
        #[cfg(windows)]
        let hwnd_val = win.hwnd().map(|h| h.0 as isize).unwrap_or(0);
        #[cfg(not(windows))]
        let hwnd_val = 0isize;

        if let Ok(mut rt) = self.runtime.lock() {
            if !rt.active {
                return self.get_state();
            }
            rt.watching = watching;
            if watching {
                rt.needs_attention = false;
            }
        }

        let _ = win.set_skip_taskbar(!watching);
        self.apply_window_mode(&win, watching);

        if let Some(main_win) = app.get_webview_window("main") {
            if watching && hwnd_val != 0 {
                attach_dock_to_window(app, &main_win, sensor_state, hwnd_val, "top", 0.5);
            } else {
                let docked_to_browser = sensor_state
                    .window_dock
                    .lock()
                    .ok()
                    .and_then(|g| g.as_ref().map(|d| d.target_hwnd == hwnd_val))
                    .unwrap_or(false);
                if docked_to_browser {
                    detach_dock_from_window(app, &main_win, sensor_state);
                }
            }
        }

        self.emit_state(app);
        self.get_state()
    }

    pub fn close_browser(&self, app: &AppHandle, sensor_state: &AppSensorState) -> BrowserStatePayload {
        let target_hwnd = if let Ok(mut rt) = self.runtime.lock() {
            let h = rt.window_hwnd;
            *rt = BrowserRuntime {
                generation: rt.generation.wrapping_add(1),
                ..Default::default()
            };
            h
        } else {
            0
        };

        if target_hwnd != 0 {
            let docked_to_browser = sensor_state
                .window_dock
                .lock()
                .ok()
                .and_then(|g| g.as_ref().map(|d| d.target_hwnd == target_hwnd))
                .unwrap_or(false);
            if docked_to_browser {
                if let Some(main_win) = app.get_webview_window("main") {
                    detach_dock_from_window(app, &main_win, sensor_state);
                }
            }
        }

        if let Some(win) = app.get_webview_window(BROWSER_WINDOW_LABEL) {
            self.closing_guard.store(true, Ordering::SeqCst);
            let _ = win.destroy();
            self.closing_guard.store(false, Ordering::SeqCst);
        }

        self.emit_state(app);
        self.get_state()
    }

    /// Destroys hidden background browser immediately at the end of an agent turn to keep RAM at 0MB,
    /// while preserving an actively watched window so the user isn't interrupted mid-read.
    pub fn on_agent_turn_finished(&self, app: &AppHandle, sensor_state: &AppSensorState) {
        let (is_active, watching) = self
            .runtime
            .lock()
            .map(|rt| (rt.active, rt.watching))
            .unwrap_or((false, false));
        if !is_active {
            return;
        }
        if !watching {
            self.close_browser(app, sensor_state);
        } else {
            if let Ok(mut rt) = self.runtime.lock() {
                rt.navigating = false;
                rt.user_takeover = false;
                rt.action_label = "Task finished".to_string();
            }
            self.emit_state(app);
        }
    }

    pub fn clear_profile_data(&self, app: &AppHandle, sensor_state: &AppSensorState) -> Result<(), String> {
        self.close_browser(app, sensor_state);
        if self.profile_dir.exists() {
            let _ = std::fs::remove_dir_all(&self.profile_dir);
        }
        std::fs::create_dir_all(&self.profile_dir).map_err(|e| e.to_string())
    }

    pub async fn wait_until_loaded(
        &self,
        win: &WebviewWindow,
        max_wait_ms: u64,
        prev_url: Option<&str>,
    ) {
        let start = tokio::time::Instant::now();
        let deadline = start + Duration::from_millis(max_wait_ms);
        loop {
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            if let Ok(val) = bridge::eval_json(
                win,
                "(() => ({ ready: document.readyState, href: location.href, len: document.body ? (document.body.innerText || '').trim().length : 0 }))()",
            )
            .await
            {
                let ready = val["ready"].as_str().unwrap_or("");
                let href = val["href"].as_str().unwrap_or("");
                let len = val["len"].as_u64().unwrap_or(0);
                let url_changed = prev_url.map(|p| p.is_empty() || href != p).unwrap_or(true);
                let elapsed_ms = start.elapsed().as_millis() as u64;
                if ready == "complete" && (url_changed || elapsed_ms >= 900) && (len > 20 || elapsed_ms >= 1800) {
                    tokio::time::sleep(Duration::from_millis(380)).await;
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(220)).await;
        }
    }

    pub fn format_page_snapshot(snap: &Value) -> String {
        let title = snap["title"].as_str().unwrap_or("(untitled)");
        let url = snap["url"].as_str().unwrap_or("");
        let canonical = snap["canonical_url"].as_str().unwrap_or("").trim();
        let scroll_y = snap["scroll_y"].as_i64().unwrap_or(0);
        let scroll_h = snap["scroll_height"].as_i64().unwrap_or(0);
        let vh = snap["viewport_height"].as_i64().unwrap_or(0);
        let text = snap["text"].as_str().unwrap_or("");
        let needs_att = snap["needs_attention"].as_bool().unwrap_or(false);
        let att_reason = snap["attention_reason"].as_str().unwrap_or("");
        let console_errors = snap["console_error_count"].as_u64().unwrap_or(0);
        let console_warns = snap["console_warn_count"].as_u64().unwrap_or(0);

        let mut elements_lines = Vec::new();
        if let Some(arr) = snap["elements"].as_array() {
            for item in arr.iter().take(60) {
                let id = item["id"].as_str().unwrap_or("");
                let tag = item["tag"].as_str().unwrap_or("");
                let typ = item["type"].as_str().unwrap_or("");
                let label = item["label"].as_str().unwrap_or("");
                let href = item["href"].as_str().unwrap_or("");
                let tag_disp = if typ.is_empty() {
                    tag.to_string()
                } else {
                    format!("{tag}[{typ}]")
                };
                if href.is_empty() {
                    elements_lines.push(format!("[{id}] <{tag_disp}> \"{label}\""));
                } else {
                    elements_lines.push(format!("[{id}] <{tag_disp}> \"{label}\" -> {href}"));
                }
            }
        }

        let canon_line = if !canonical.is_empty() && canonical != url {
            format!("\nCanonical URL: {canonical}")
        } else {
            String::new()
        };
        let att_banner = if needs_att {
            format!("\n[BROWSER ATTENTION REQUIRED: {att_reason} — Ask the user to click Watch in the card to finish login/CAPTCHA manually, then continue.]")
        } else {
            String::new()
        };
        let console_banner = if console_errors > 0 || console_warns > 0 {
            format!("\n[Browser Console: {console_errors} error(s), {console_warns} warning(s) — call `browser_console` to inspect stack traces]")
        } else {
            String::new()
        };

        format!(
            "Page Title: {title}\nCurrent URL: {url}{canon_line}\nScroll: {scroll_y}px / {scroll_h}px (viewport {vh}px){console_banner}{att_banner}\n\n[Interactive Elements]\n{}\n\n[Visible Text]\n{text}",
            if elements_lines.is_empty() {
                "(none)".to_string()
            } else {
                elements_lines.join("\n")
            }
        )
    }

    pub async fn execute_open(
        self: &Arc<Self>,
        app: &AppHandle,
        sensor_state: &Arc<AppSensorState>,
        raw_url: &str,
        wait_ms: u64,
    ) -> Result<(String, String), String> {
        let parsed = validate_browser_url(raw_url)?;
        let prev_url = self.runtime.lock().map(|rt| rt.url.clone()).unwrap_or_default();
        let watch_now = self.is_watching();
        let (win, gen) = self.ensure_or_navigate_window(app, sensor_state, parsed, watch_now)?;
        self.set_action_label(app, "Loading page...");
        tokio::time::sleep(Duration::from_millis(320)).await;
        self.wait_until_loaded(&win, wait_ms.clamp(1200, 12_000), Some(&prev_url)).await;

        let snap = bridge::eval_json(&win, &bridge::build_read_page_script(10_000)).await?;
        self.sync_snapshot_meta(app, &win, gen, &snap);
        self.refresh_preview_once(app).await;
        let formatted = Self::format_page_snapshot(&snap);
        let detail = snap["title"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Page loaded")
            .to_string();
        Ok((formatted, detail))
    }

    pub async fn execute_read(
        self: &Arc<Self>,
        app: &AppHandle,
        _sensor_state: &Arc<AppSensorState>,
        max_chars: usize,
    ) -> Result<(String, String), String> {
        let win = app
            .get_webview_window(BROWSER_WINDOW_LABEL)
            .ok_or_else(|| "Built-in browser is not open yet. Call `browser_open` first.".to_string())?;
        self.set_action_label(app, "Reading rendered page...");
        let gen = self.runtime.lock().map(|rt| rt.generation).unwrap_or(0);
        let snap = bridge::eval_json(&win, &bridge::build_read_page_script(max_chars)).await?;
        self.sync_snapshot_meta(app, &win, gen, &snap);
        self.refresh_preview_once(app).await;
        Ok((Self::format_page_snapshot(&snap), "Page read".to_string()))
    }

    pub async fn execute_click(
        self: &Arc<Self>,
        app: &AppHandle,
        _sensor_state: &Arc<AppSensorState>,
        target: &str,
    ) -> Result<(String, String), String> {
        let win = app
            .get_webview_window(BROWSER_WINDOW_LABEL)
            .ok_or_else(|| "Built-in browser is not open yet. Call `browser_open` first.".to_string())?;
        let prev_url = self.runtime.lock().map(|rt| rt.url.clone()).unwrap_or_default();
        self.set_action_label(app, format!("Clicking {target}..."));
        let res = bridge::eval_json(&win, &bridge::build_click_script(target)?).await?;
        if res["sensitive_password"].as_bool().unwrap_or(false) {
            self.notify_user_attention(app, "Password field requires manual input");
            return Err(res["error"].as_str().unwrap_or("Password field blocked").to_string());
        }
        if !res["ok"].as_bool().unwrap_or(false) {
            return Err(res["error"].as_str().unwrap_or("Click failed").to_string());
        }
        tokio::time::sleep(Duration::from_millis(650)).await;
        self.wait_until_loaded(&win, 4500, Some(&prev_url)).await;
        let gen = self.runtime.lock().map(|rt| rt.generation).unwrap_or(0);
        let snap = bridge::eval_json(&win, &bridge::build_read_page_script(9000)).await?;
        self.sync_snapshot_meta(app, &win, gen, &snap);
        self.refresh_preview_once(app).await;
        let clicked = res["clicked_label"].as_str().unwrap_or(target);
        Ok((
            format!("Clicked: `{clicked}`\n\n{}", Self::format_page_snapshot(&snap)),
            format!("Clicked {clicked}"),
        ))
    }

    pub async fn execute_type(
        self: &Arc<Self>,
        app: &AppHandle,
        _sensor_state: &Arc<AppSensorState>,
        target: &str,
        text: &str,
        submit: bool,
    ) -> Result<(String, String), String> {
        let win = app
            .get_webview_window(BROWSER_WINDOW_LABEL)
            .ok_or_else(|| "Built-in browser is not open yet. Call `browser_open` first.".to_string())?;
        let prev_url = self.runtime.lock().map(|rt| rt.url.clone()).unwrap_or_default();
        self.set_action_label(app, format!("Typing into {target}..."));
        let res = bridge::eval_json(&win, &bridge::build_type_script(target, text, submit)?).await?;
        if res["sensitive_password"].as_bool().unwrap_or(false) {
            self.notify_user_attention(app, "Please sign in manually in Watch view");
            return Err(res["error"].as_str().unwrap_or("Password input blocked").to_string());
        }
        if !res["ok"].as_bool().unwrap_or(false) {
            return Err(res["error"].as_str().unwrap_or("Type failed").to_string());
        }
        tokio::time::sleep(Duration::from_millis(if submit { 900 } else { 300 })).await;
        if submit {
            self.wait_until_loaded(&win, 5000, Some(&prev_url)).await;
        }
        let gen = self.runtime.lock().map(|rt| rt.generation).unwrap_or(0);
        let snap = bridge::eval_json(&win, &bridge::build_read_page_script(9000)).await?;
        self.sync_snapshot_meta(app, &win, gen, &snap);
        self.refresh_preview_once(app).await;
        Ok((
            format!("Typed into `{target}`{}\n\n{}", if submit { " and submitted" } else { "" }, Self::format_page_snapshot(&snap)),
            format!("Typed into {target}"),
        ))
    }

    pub async fn execute_scroll(
        self: &Arc<Self>,
        app: &AppHandle,
        _sensor_state: &Arc<AppSensorState>,
        direction: &str,
        amount: i32,
    ) -> Result<(String, String), String> {
        let win = app
            .get_webview_window(BROWSER_WINDOW_LABEL)
            .ok_or_else(|| "Built-in browser is not open yet. Call `browser_open` first.".to_string())?;
        self.set_action_label(app, format!("Scrolling {direction}..."));
        let _ = bridge::eval_json(&win, &bridge::build_scroll_script(direction, amount)).await?;
        tokio::time::sleep(Duration::from_millis(450)).await;
        let gen = self.runtime.lock().map(|rt| rt.generation).unwrap_or(0);
        let snap = bridge::eval_json(&win, &bridge::build_read_page_script(9000)).await?;
        self.sync_snapshot_meta(app, &win, gen, &snap);
        self.refresh_preview_once(app).await;
        Ok((Self::format_page_snapshot(&snap), format!("Scrolled {direction}")))
    }

    pub async fn execute_screenshot_b64(&self, app: &AppHandle) -> Result<String, String> {
        let win = app
            .get_webview_window(BROWSER_WINDOW_LABEL)
            .ok_or_else(|| "Built-in browser is not open yet.".to_string())?;
        self.set_action_label(app, "Capturing viewport snapshot...");
        bridge::capture_preview_jpeg(&win, 1280, 82).await
    }

    pub async fn execute_console(
        &self,
        app: &AppHandle,
        clear: bool,
    ) -> Result<(String, String), String> {
        let win = app
            .get_webview_window(BROWSER_WINDOW_LABEL)
            .ok_or_else(|| "Built-in browser is not open yet. Call `browser_open` first.".to_string())?;
        self.set_action_label(app, "Reading console logs...");
        let res = bridge::eval_json(&win, &bridge::build_read_console_script(clear)).await?;
        let url = res["url"].as_str().unwrap_or("");
        let arr = res["logs"].as_array().cloned().unwrap_or_default();
        if arr.is_empty() {
            return Ok((
                format!("URL: {url}\nNo console errors, warnings, or failed fetch requests captured."),
                "0 console issues".to_string(),
            ));
        }
        let mut lines = Vec::with_capacity(arr.len());
        for entry in &arr {
            let lvl = entry["level"].as_str().unwrap_or("log").to_uppercase();
            let msg = entry["msg"].as_str().unwrap_or("");
            lines.push(format!("[{lvl}] {msg}"));
        }
        let summary = format!("{} console entries", arr.len());
        Ok((
            format!("URL: {url}\nCaptured {} console/network entries:\n{}", arr.len(), lines.join("\n")),
            summary,
        ))
    }

    pub fn navigate_user_url(
        self: &Arc<Self>,
        app: &AppHandle,
        sensor_state: &Arc<AppSensorState>,
        raw_url: &str,
    ) -> Result<BrowserStatePayload, String> {
        let watch_now = self.is_watching();
        self.open_user_url(app, sensor_state, raw_url, watch_now)
    }

    pub fn open_user_url(
        self: &Arc<Self>,
        app: &AppHandle,
        sensor_state: &Arc<AppSensorState>,
        raw_url: &str,
        watch_now: bool,
    ) -> Result<BrowserStatePayload, String> {
        let parsed = validate_browser_url(raw_url)?;
        let _ = self.ensure_or_navigate_window(app, sensor_state, parsed, watch_now)?;
        Ok(self.get_state())
    }

    pub fn browser_history_action(&self, app: &AppHandle, action: &str) -> Result<(), String> {
        let win = app
            .get_webview_window(BROWSER_WINDOW_LABEL)
            .ok_or_else(|| "Built-in browser is not open.".to_string())?;
        let script = match action {
            "back" => "history.back()",
            "forward" => "history.forward()",
            _ => "location.reload()",
        };
        win.eval(script).map_err(|e| e.to_string())
    }

    fn notify_user_attention(&self, app: &AppHandle, reason: &str) {
        if let Ok(mut rt) = self.runtime.lock() {
            if rt.active {
                rt.needs_attention = true;
                rt.user_takeover = true;
                rt.action_label = reason.to_string();
            }
        }
        self.emit_state(app);
    }

    fn sync_snapshot_meta(
        &self,
        app: &AppHandle,
        win: &WebviewWindow,
        gen: u64,
        snap: &Value,
    ) {
        let needs_att = snap["needs_attention"].as_bool().unwrap_or(false);
        let mut clean_title: Option<String> = None;
        if let Ok(mut rt) = self.runtime.lock() {
            if rt.generation == gen {
                rt.navigating = false;
                if let Some(u) = snap["url"].as_str() {
                    rt.url = u.to_string();
                }
                if let Some(t) = snap["title"].as_str() {
                    let trimmed = t.trim();
                    if !trimmed.is_empty() {
                        rt.title = trimmed.to_string();
                        clean_title = Some(trimmed.chars().take(48).collect());
                    }
                }
                rt.needs_attention = needs_att;
                if needs_att {
                    rt.action_label = "Needs your verification / sign-in".to_string();
                } else if !rt.user_takeover {
                    rt.action_label = "Page ready".to_string();
                }
            }
        }
        if let Some(t) = clean_title {
            let _ = win.set_title(&format!("{t} — Buddy Browser"));
        }
        // Never force-pop the browser window onto the user's screen; only highlight inside the Watch card.
        self.emit_state(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formats_browser_snapshot_and_attention_flags_cleanly() {
        let snap = json!({
            "title": "Music Search - Video",
            "url": "https://example.com/search?q=song",
            "scroll_y": 120,
            "scroll_height": 2400,
            "viewport_height": 720,
            "text": "Official MV 04:12",
            "needs_attention": true,
            "attention_reason": "captcha_or_challenge",
            "elements": [
                { "id": "e1", "tag": "a", "type": "", "label": "Official Song", "href": "https://example.com/watch?v=123" },
                { "id": "e2", "tag": "input", "type": "search", "label": "Search", "href": "" }
            ]
        });

        let formatted = BrowserManager::format_page_snapshot(&snap);
        assert!(formatted.contains("Page Title: Music Search - Video"));
        assert!(formatted.contains("[e1] <a> \"Official Song\" -> https://example.com/watch?v=123"));
        assert!(formatted.contains("[e2] <input[search]> \"Search\""));
        assert!(formatted.contains("[BROWSER ATTENTION REQUIRED: captcha_or_challenge"));
    }

    #[test]
    fn builds_safe_escaped_browser_action_scripts() {
        let click_js = bridge::build_click_script("a[title=\"Play 'Now'\"]").unwrap();
        assert!(click_js.contains("sensitive_password"));
        assert!(click_js.contains("data-buddy-id"));

        let type_js = bridge::build_type_script("e2", "hello \"world\"\n", true).unwrap();
        assert!(type_js.contains("sensitive_password"));
        assert!(type_js.contains("HTMLInputElement.prototype"));

        let scroll_js = bridge::build_scroll_script("down", 99999);
        assert!(scroll_js.contains("3000"));
    }

    #[test]
    fn validates_browser_urls_including_localhost_lan_and_local_files() {
        assert_eq!(
            validate_browser_url("localhost:5173").unwrap().as_str(),
            "http://localhost:5173/"
        );
        assert_eq!(
            validate_browser_url("127.0.0.1:8080/api").unwrap().as_str(),
            "http://127.0.0.1:8080/api"
        );
        assert_eq!(
            validate_browser_url("192.168.1.20:3000").unwrap().as_str(),
            "http://192.168.1.20:3000/"
        );
        assert_eq!(
            validate_browser_url("example.com").unwrap().as_str(),
            "https://example.com/"
        );
        let file_url = validate_browser_url(r"C:\Users\Marcus\demo\index.html").unwrap();
        assert_eq!(file_url.scheme(), "file");
        assert!(file_url.as_str().ends_with("/index.html"));

        // Still blocks dangerous schemes, metadata IP, and OS/credential paths
        assert!(validate_browser_url("javascript:alert(1)").is_err());
        assert!(validate_browser_url("http://169.254.169.254/latest/meta-data").is_err());
        assert!(validate_browser_url(r"file:///C:/Windows/System32/drivers/etc/hosts").is_err());
    }
}
