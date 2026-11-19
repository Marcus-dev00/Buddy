//! Buddy Mobile Companion Bridge:
//! - Built-in zero-dependency HTTP/1.1 + SSE server on local LAN / Tailscale / Cloudflare Tunnel.
//! - High-entropy pairing token auth + QR pairing payload.
//! - Streams live chat deltas, thinking, agent approvals, built-in browser frames, reminders,
//!   and on-demand desktop screenshots directly to BuddyMobile Android APK & Cloudflare `/app`.

pub mod http;
pub mod tunnel;

use crate::agent_tools::{check_path_allowed, AgentApprovalManager, ApprovalDecision};
use crate::ai_client::{spawn_chat_stream, AiStreamController, AttachmentPayload};
use crate::browser::BrowserManager;
use crate::db::{now_ms as db_now_ms, MemoryDb};
use crate::mcp::McpManager;
use crate::media_utils::text_sha256;
use crate::plugins::PluginManager;
use crate::prompts::PromptsManager;
use crate::reminders::{local_utc_offset_ms, parse_fire_at_ms};
use crate::screen_share::{capture_primary_desktop_jpeg_b64, ScreenShareManager};
use crate::settings::SettingsManager;
use crate::skills::SkillsManager;
use crate::win32_sensors::AppSensorState;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use http::{
    parse_http_request, write_cors_preflight, write_error, write_file_response, write_json,
    write_sse_event, write_sse_headers, HttpRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Listener, Manager};
use tokio::sync::broadcast;
use tunnel::{
    detect_local_ips, ensure_cloudflared_binary, publish_tunnel_to_directory, TunnelHandle,
};

pub const DEFAULT_BRIDGE_PORT: u16 = 19425;
pub const DEFAULT_DIRECTORY_URL: &str = "https://buddy-companion.marcus-dev00.workers.dev";

fn default_port() -> u16 {
    DEFAULT_BRIDGE_PORT
}

fn default_directory_url() -> String {
    DEFAULT_DIRECTORY_URL.to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MobileBridgeConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub pair_id: String,
    #[serde(default)]
    pub custom_remote_url: String,
    #[serde(default = "default_directory_url")]
    pub directory_url: String,
    #[serde(default)]
    pub auto_tunnel: bool,
}

impl Default for MobileBridgeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_BRIDGE_PORT,
            token: generate_random_hex(32),
            pair_id: generate_random_hex(8),
            custom_remote_url: String::new(),
            directory_url: default_directory_url(),
            auto_tunnel: false,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct MobileBridgeStatus {
    pub enabled: bool,
    pub running: bool,
    pub port: u16,
    pub token: String,
    pub pair_id: String,
    pub device_name: String,
    pub lan_url: Option<String>,
    pub tailscale_url: Option<String>,
    pub tunnel_url: Option<String>,
    pub tunnel_running: bool,
    pub tunnel_downloading: bool,
    pub tunnel_progress: u8,
    pub custom_remote_url: String,
    pub directory_url: String,
    pub effective_remote_url: Option<String>,
    pub pairing_payload: String,
    pub connected_clients: usize,
    pub last_error: Option<String>,
}

struct BridgeRuntime {
    running: bool,
    stop_flag: Option<Arc<AtomicBool>>,
    tunnel_handle: Option<Arc<TunnelHandle>>,
    tunnel_url: Option<String>,
    tunnel_downloading: bool,
    tunnel_progress: u8,
    last_error: Option<String>,
}

impl Default for BridgeRuntime {
    fn default() -> Self {
        Self {
            running: false,
            stop_flag: None,
            tunnel_handle: None,
            tunnel_url: None,
            tunnel_downloading: false,
            tunnel_progress: 0,
            last_error: None,
        }
    }
}

pub struct MobileBridgeManager {
    data_dir: PathBuf,
    config_path: PathBuf,
    config: Mutex<MobileBridgeConfig>,
    runtime: Mutex<BridgeRuntime>,
    sse_tx: broadcast::Sender<(String, String)>,
    connected_clients: Arc<AtomicUsize>,
}

fn generate_random_hex(byte_len: usize) -> String {
    let mut seed = Vec::new();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    seed.extend_from_slice(&nanos.to_le_bytes());
    seed.extend_from_slice(&std::process::id().to_le_bytes());
    let stack_addr = &seed as *const _ as usize;
    seed.extend_from_slice(&stack_addr.to_le_bytes());
    if let Ok(host) = std::env::var("COMPUTERNAME") {
        seed.extend_from_slice(host.as_bytes());
    }
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::GetTickCount64;
        let ticks = unsafe { GetTickCount64() };
        seed.extend_from_slice(&ticks.to_le_bytes());
    }

    let mut out = String::with_capacity(byte_len * 2);
    let mut counter: u64 = 0;
    while out.len() < byte_len * 2 {
        let mut h = Sha256::new();
        h.update(&seed);
        h.update(counter.to_le_bytes());
        let d = h.finalize();
        for b in d {
            if out.len() >= byte_len * 2 {
                break;
            }
            out.push_str(&format!("{b:02x}"));
        }
        counter += 1;
    }
    out
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    if ab.len() != bb.len() || ab.is_empty() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in ab.iter().zip(bb.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn device_hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Buddy PC".to_string())
}

impl MobileBridgeManager {
    pub fn init(data_dir: &Path) -> Self {
        let config_path = data_dir.join("mobile_bridge.json");
        let mut cfg: MobileBridgeConfig = std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        let mut changed = false;
        if cfg.token.len() < 32 {
            cfg.token = generate_random_hex(32);
            changed = true;
        }
        if cfg.pair_id.len() < 12 {
            cfg.pair_id = generate_random_hex(8);
            changed = true;
        }
        if cfg.port == 0 {
            cfg.port = DEFAULT_BRIDGE_PORT;
            changed = true;
        }
        if changed {
            if let Ok(s) = serde_json::to_string_pretty(&cfg) {
                let _ = std::fs::write(&config_path, s);
            }
        }
        let (sse_tx, _) = broadcast::channel(256);
        Self {
            data_dir: data_dir.to_path_buf(),
            config_path,
            config: Mutex::new(cfg),
            runtime: Mutex::new(BridgeRuntime::default()),
            sse_tx,
            connected_clients: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn persist_config(&self, cfg: &MobileBridgeConfig) {
        if let Ok(s) = serde_json::to_string_pretty(cfg) {
            let _ = std::fs::write(&self.config_path, s);
        }
    }

    /// Hooks into Tauri's app event bus once on startup so every desktop event fans out to connected phones.
    pub fn attach_event_forwarders(self: &Arc<Self>, app: &AppHandle) {
        const EVENTS: &[&str] = &[
            "buddy://ai-chunk",
            "buddy://ai-thought",
            "buddy://ai-done",
            "buddy://ai-error",
            "buddy://agent-step",
            "buddy://browser-state",
            "buddy://browser-preview",
            "buddy://reminder-due",
            "buddy://reminders-changed",
            "buddy://memory-captured",
            "buddy://conversations-changed",
        ];
        for &ev_name in EVENTS {
            let tx = self.sse_tx.clone();
            let short = ev_name.trim_start_matches("buddy://").to_string();
            app.listen(ev_name, move |ev| {
                let payload = ev.payload().to_string();
                let _ = tx.send((short.clone(), payload));
            });
        }
    }

    pub fn get_status(&self) -> MobileBridgeStatus {
        let cfg = self.config.lock().map(|g| g.clone()).unwrap_or_default();
        let (lan_ip, ts_ip) = detect_local_ips();
        let lan_url = lan_ip.map(|ip| format!("http://{ip}:{}", cfg.port));
        let tailscale_url = ts_ip.map(|ip| format!("http://{ip}:{}", cfg.port));

        let (running, tunnel_url, tunnel_running, tunnel_downloading, tunnel_progress, last_error) =
            self.runtime
                .lock()
                .map(|rt| {
                    (
                        rt.running,
                        rt.tunnel_url.clone(),
                        rt.tunnel_handle.is_some(),
                        rt.tunnel_downloading,
                        rt.tunnel_progress,
                        rt.last_error.clone(),
                    )
                })
                .unwrap_or((false, None, false, false, 0, None));

        let effective_remote_url = if !cfg.custom_remote_url.trim().is_empty() {
            Some(cfg.custom_remote_url.trim().trim_end_matches('/').to_string())
        } else if let Some(tu) = tunnel_url.clone() {
            Some(tu)
        } else {
            tailscale_url.clone()
        };

        let device_name = device_hostname();
        let pairing_payload = json!({
            "v": 1,
            "name": device_name,
            "pairId": cfg.pair_id,
            "token": cfg.token,
            "lan": lan_url.clone().unwrap_or_default(),
            "remote": effective_remote_url.clone().unwrap_or_default(),
            "dir": cfg.directory_url.trim().trim_end_matches('/'),
        })
        .to_string();

        MobileBridgeStatus {
            enabled: cfg.enabled,
            running,
            port: cfg.port,
            token: cfg.token,
            pair_id: cfg.pair_id,
            device_name,
            lan_url,
            tailscale_url,
            tunnel_url,
            tunnel_running,
            tunnel_downloading,
            tunnel_progress,
            custom_remote_url: cfg.custom_remote_url,
            directory_url: cfg.directory_url,
            effective_remote_url,
            pairing_payload,
            connected_clients: self.connected_clients.load(Ordering::Relaxed),
            last_error,
        }
    }

    fn emit_status(&self, app: &AppHandle) {
        let _ = app.emit("buddy://mobile-bridge-status", self.get_status());
    }

    pub fn start_if_enabled(self: &Arc<Self>, app: &AppHandle) {
        let (enabled, auto_tunnel) = self
            .config
            .lock()
            .map(|c| (c.enabled, c.auto_tunnel))
            .unwrap_or((false, false));
        if enabled {
            let _ = self.start_server(app);
            if auto_tunnel {
                let mgr = Arc::clone(self);
                let app_c = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = mgr.start_tunnel(&app_c).await;
                });
            }
        }
    }

    pub fn set_enabled(self: &Arc<Self>, app: &AppHandle, enabled: bool) -> Result<MobileBridgeStatus, String> {
        if let Ok(mut cfg) = self.config.lock() {
            cfg.enabled = enabled;
            self.persist_config(&cfg);
        }
        if enabled {
            self.start_server(app)?;
        } else {
            self.stop_all(app);
        }
        self.emit_status(app);
        Ok(self.get_status())
    }

    pub fn update_remote_settings(
        self: &Arc<Self>,
        app: &AppHandle,
        custom_remote_url: String,
        directory_url: String,
        auto_tunnel: bool,
    ) -> MobileBridgeStatus {
        if let Ok(mut cfg) = self.config.lock() {
            cfg.custom_remote_url = custom_remote_url.trim().to_string();
            cfg.directory_url = directory_url.trim().to_string();
            cfg.auto_tunnel = auto_tunnel;
            self.persist_config(&cfg);
        }
        self.emit_status(app);
        self.get_status()
    }

    pub fn reset_pairing_token(self: &Arc<Self>, app: &AppHandle) -> MobileBridgeStatus {
        if let Ok(mut cfg) = self.config.lock() {
            cfg.token = generate_random_hex(32);
            cfg.pair_id = generate_random_hex(8);
            self.persist_config(&cfg);
        }
        self.emit_status(app);
        self.get_status()
    }

    pub fn start_server(self: &Arc<Self>, app: &AppHandle) -> Result<(), String> {
        let port = self
            .config
            .lock()
            .map(|c| c.port)
            .unwrap_or(DEFAULT_BRIDGE_PORT);

        if let Ok(rt) = self.runtime.lock() {
            if rt.running {
                return Ok(());
            }
        }

        let listener = TcpListener::bind(("0.0.0.0", port))
            .map_err(|e| format!("Failed to bind Mobile Bridge port {port}: {e}"))?;
        let _ = listener.set_nonblocking(true);

        let stop_flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut rt) = self.runtime.lock() {
            rt.running = true;
            rt.stop_flag = Some(Arc::clone(&stop_flag));
            rt.last_error = None;
        }

        let mgr = Arc::clone(self);
        let app_handle = app.clone();
        std::thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _addr)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
                        let mgr_c = Arc::clone(&mgr);
                        let app_c = app_handle.clone();
                        std::thread::spawn(move || {
                            mgr_c.handle_connection(app_c, stream);
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(35));
                    }
                    Err(_) => {
                        std::thread::sleep(Duration::from_millis(120));
                    }
                }
            }
        });

        crate::logger::log_info("mobile", &format!("Mobile Bridge listening on 0.0.0.0:{port}"));
        self.emit_status(app);
        Ok(())
    }

    pub async fn start_tunnel(self: &Arc<Self>, app: &AppHandle) -> Result<MobileBridgeStatus, String> {
        if let Ok(mut cfg) = self.config.lock() {
            if !cfg.enabled {
                cfg.enabled = true;
                self.persist_config(&cfg);
            }
        }
        self.start_server(app)?;
        self.stop_tunnel(app);

        let (port, pair_id, token, directory_url) = self
            .config
            .lock()
            .map(|c| (c.port, c.pair_id.clone(), c.token.clone(), c.directory_url.clone()))
            .unwrap_or_else(|_| (DEFAULT_BRIDGE_PORT, String::new(), String::new(), String::new()));

        let mirror_prefix = app
            .try_state::<Arc<PluginManager>>()
            .map(|pm| pm.list().mirror_prefix)
            .unwrap_or_default();

        if let Ok(mut rt) = self.runtime.lock() {
            rt.tunnel_downloading = true;
            rt.tunnel_progress = 2;
            rt.last_error = None;
        }
        self.emit_status(app);

        let mgr_prog = Arc::clone(self);
        let app_prog = app.clone();
        let bin_res = ensure_cloudflared_binary(&self.data_dir, &mirror_prefix, move |pct| {
            if let Ok(mut rt) = mgr_prog.runtime.lock() {
                rt.tunnel_progress = pct;
            }
            mgr_prog.emit_status(&app_prog);
        })
        .await;

        let bin_path = match bin_res {
            Ok(p) => p,
            Err(e) => {
                if let Ok(mut rt) = self.runtime.lock() {
                    rt.tunnel_downloading = false;
                    rt.tunnel_progress = 0;
                    rt.last_error = Some(e.clone());
                }
                self.emit_status(app);
                return Err(e);
            }
        };

        if let Ok(mut rt) = self.runtime.lock() {
            rt.tunnel_downloading = false;
            rt.tunnel_progress = 100;
        }

        let (url_tx, url_rx) = tokio::sync::oneshot::channel::<String>();
        let url_tx_cell = Arc::new(Mutex::new(Some(url_tx)));
        let mgr_cb = Arc::clone(self);
        let app_cb = app.clone();
        let lan_url_opt = detect_local_ips().0.map(|ip| format!("http://{ip}:{port}"));

        let handle = TunnelHandle::spawn_quick_tunnel(&bin_path, port, move |turl| {
            if let Ok(mut rt) = mgr_cb.runtime.lock() {
                rt.tunnel_url = Some(turl.clone());
                rt.last_error = None;
            }
            mgr_cb.emit_status(&app_cb);
            if let Ok(mut g) = url_tx_cell.lock() {
                if let Some(tx) = g.take() {
                    let _ = tx.send(turl.clone());
                }
            }
            // Publish updated tunnel URL to Cloudflare Worker directory so phone auto-reconnects
            let dir = directory_url.clone();
            let pid = pair_id.clone();
            let tok = token.clone();
            let lan = lan_url_opt.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) =
                    publish_tunnel_to_directory(&dir, &pid, &tok, &turl, lan.as_deref()).await
                {
                    crate::logger::log_warn("mobile", &format!("directory sync skipped: {e}"));
                }
            });
        })?;

        if let Ok(mut rt) = self.runtime.lock() {
            rt.tunnel_handle = Some(handle);
        }
        self.emit_status(app);

        // Wait up to 18s for cloudflared to emit the quick tunnel URL
        match tokio::time::timeout(Duration::from_secs(18), url_rx).await {
            Ok(Ok(_)) => Ok(self.get_status()),
            _ => Ok(self.get_status()),
        }
    }

    pub fn stop_tunnel(&self, app: &AppHandle) {
        let old = self.runtime.lock().ok().and_then(|mut rt| {
            rt.tunnel_url = None;
            rt.tunnel_downloading = false;
            rt.tunnel_progress = 0;
            rt.tunnel_handle.take()
        });
        if let Some(h) = old {
            h.stop();
        }
        self.emit_status(app);
    }

    pub fn stop_all(&self, app: &AppHandle) {
        self.stop_tunnel(app);
        if let Ok(mut rt) = self.runtime.lock() {
            rt.running = false;
            if let Some(flag) = rt.stop_flag.take() {
                flag.store(true, Ordering::Relaxed);
            }
        }
        self.emit_status(app);
    }

    fn verify_auth(&self, req: &HttpRequest) -> bool {
        let expected = self
            .config
            .lock()
            .map(|c| c.token.clone())
            .unwrap_or_default();
        if let Some(auth) = req.headers.get("authorization") {
            if let Some(bearer) = auth.strip_prefix("Bearer ").or_else(|| auth.strip_prefix("bearer ")) {
                if constant_time_eq(bearer.trim(), &expected) {
                    return true;
                }
            }
        }
        if let Some(qt) = req.query.get("token") {
            if constant_time_eq(qt.trim(), &expected) {
                return true;
            }
        }
        false
    }

    fn handle_connection(&self, app: AppHandle, mut stream: TcpStream) {
        let req = match parse_http_request(&stream) {
            Ok(r) => r,
            Err(_) => return,
        };

        if req.method == "OPTIONS" {
            write_cors_preflight(&mut stream);
            return;
        }

        // Public unauthenticated health probe (leaks no private data)
        if req.method == "GET" && req.path == "/health" {
            write_json(&mut stream, 200, &json!({ "ok": true, "app": "Buddy" }));
            return;
        }

        if !self.verify_auth(&req) {
            write_error(&mut stream, 401, "Unauthorized: invalid pairing token");
            return;
        }

        if req.method == "GET" && req.path == "/api/events" {
            self.handle_sse(stream);
            return;
        }

        if req.method == "GET" && req.path == "/api/memory-image" {
            let Some(id_str) = req.query.get("id") else {
                write_error(&mut stream, 400, "Missing `id` query parameter");
                return;
            };
            let Ok(mem_id) = id_str.trim().parse::<i64>() else {
                write_error(&mut stream, 400, "Invalid `id` query parameter");
                return;
            };
            let full = req
                .query
                .get("full")
                .map(|v| v == "1" || v == "true")
                .unwrap_or(false);
            let db = app.state::<Arc<MemoryDb>>();
            let Ok(item) = db.get_memory_by_id(mem_id) else {
                write_error(&mut stream, 404, "Memory not found");
                return;
            };
            let chosen = if full {
                item.file_path.as_deref().or(item.thumb_path.as_deref())
            } else {
                item.thumb_path.as_deref().or(item.file_path.as_deref())
            };
            let Some(raw_path) = chosen.filter(|s| !s.trim().is_empty()) else {
                write_error(&mut stream, 404, "Memory has no image");
                return;
            };
            let p = PathBuf::from(raw_path);
            let resolved = if p.is_file() {
                p
            } else if let Some(fb) = item.file_path.as_deref().map(PathBuf::from).filter(|x| x.is_file()) {
                fb
            } else {
                write_error(&mut stream, 404, "Image file not found on disk");
                return;
            };
            let as_dl = req.query.get("download").map(|v| v == "1" || v == "true").unwrap_or(false);
            let range = req.headers.get("range").map(|s| s.as_str());
            write_file_response(&mut stream, &resolved, range, as_dl);
            return;
        }

        if req.method == "GET" && req.path == "/api/file" {
            let Some(raw_path) = req.query.get("path").map(|s| s.trim()).filter(|s| !s.is_empty()) else {
                write_error(&mut stream, 400, "Missing `path` query parameter");
                return;
            };
            let p = PathBuf::from(raw_path);
            let in_buddy_data = p
                .canonicalize()
                .ok()
                .and_then(|cp| self.data_dir.canonicalize().ok().map(|rd| cp.starts_with(&rd)))
                .unwrap_or(false);
            if (!in_buddy_data && check_path_allowed(&p, false).is_err()) || !p.is_file() {
                write_error(&mut stream, 403, "File path not permitted or missing");
                return;
            }
            let as_dl = req.query.get("download").map(|v| v == "1" || v == "true").unwrap_or(false);
            let range = req.headers.get("range").map(|s| s.as_str());
            write_file_response(&mut stream, &p, range, as_dl);
            return;
        }

        let res = tauri::async_runtime::block_on(self.dispatch_api(&app, &req));
        match res {
            Ok(val) => write_json(&mut stream, 200, &val),
            Err((code, msg)) => write_error(&mut stream, code, &msg),
        }
    }

    fn handle_sse(&self, mut stream: TcpStream) {
        if write_sse_headers(&mut stream).is_err() {
            return;
        }
        let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
        self.connected_clients.fetch_add(1, Ordering::Relaxed);
        let mut rx = self.sse_tx.subscribe();

        // Send initial connected heartbeat
        if write_sse_event(&mut stream, "connected", &json!({ "ok": true }).to_string()).is_err() {
            self.connected_clients.fetch_sub(1, Ordering::Relaxed);
            return;
        }

        loop {
            match rx.try_recv() {
                Ok((ev, payload)) => {
                    if write_sse_event(&mut stream, &ev, &payload).is_err() {
                        break;
                    }
                }
                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(broadcast::error::TryRecvError::Closed) => break,
                Err(broadcast::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(60));
                    // Periodic keep-alive comment every ~15s would also detect closed sockets
                    static TICK: AtomicUsize = AtomicUsize::new(0);
                    if TICK.fetch_add(1, Ordering::Relaxed) % 240 == 0 {
                        use std::io::Write;
                        if stream.write_all(b": ping\n\n").is_err() || stream.flush().is_err() {
                            break;
                        }
                    }
                }
            }
        }
        self.connected_clients.fetch_sub(1, Ordering::Relaxed);
    }

    async fn dispatch_api(
        &self,
        app: &AppHandle,
        req: &HttpRequest,
    ) -> Result<Value, (u16, String)> {
        let db = app.state::<Arc<MemoryDb>>();
        let settings_mgr = app.state::<Arc<SettingsManager>>();
        let prompts_mgr = app.state::<Arc<PromptsManager>>();
        let skills_mgr = app.state::<Arc<SkillsManager>>();
        let plugin_mgr = app.state::<Arc<PluginManager>>();
        let browser_mgr = app.state::<Arc<BrowserManager>>();
        let mcp_mgr = app.state::<Arc<McpManager>>();
        let sensor_state = app.state::<Arc<AppSensorState>>();
        let share_mgr = app.state::<Arc<ScreenShareManager>>();
        let controller = app.state::<Arc<AiStreamController>>();
        let approval_mgr = app.state::<Arc<AgentApprovalManager>>();

        let body_json = || -> Value {
            serde_json::from_slice(&req.body).unwrap_or(Value::Null)
        };

        match (req.method.as_str(), req.path.as_str()) {
            ("GET", "/api/ping") => {
                let st = self.get_status();
                let bstate = browser_mgr.get_state();
                let cfg = settings_mgr.get();
                Ok(json!({
                    "ok": true,
                    "version": env!("CARGO_PKG_VERSION"),
                    "device_name": st.device_name,
                    "lan_url": st.lan_url,
                    "remote_url": st.effective_remote_url,
                    "thinking": controller.is_streaming.load(Ordering::Relaxed),
                    "browser": bstate,
                    "model_id": cfg.model_id,
                    "chat_profile_id": cfg.chat_profile_id,
                    "ui_language": cfg.ui_language
                }))
            }
            ("GET", "/api/conversations") => {
                let list = db.list_conversations(50).map_err(|e| (500, e))?;
                Ok(json!({ "ok": true, "conversations": list }))
            }
            ("POST", "/api/conversations") => {
                let created = db
                    .create_conversation_with_source(None, "mobile")
                    .map_err(|e| (500, e))?;
                let _ = app.emit(
                    "buddy://conversations-changed",
                    json!({ "conversation_id": created.id, "source": "mobile" }),
                );
                Ok(json!({ "ok": true, "conversation": created }))
            }
            ("POST", "/api/chat/send") => {
                let b = body_json();
                let prompt = b["prompt"].as_str().unwrap_or("").trim().to_string();
                let attachments: Vec<AttachmentPayload> =
                    serde_json::from_value(b["attachments"].clone()).unwrap_or_default();
                if prompt.is_empty() && attachments.is_empty() {
                    return Err((400, "Prompt or attachments required".to_string()));
                }
                let conv_id = match b["conversation_id"].as_i64() {
                    Some(id) if id > 0 => id,
                    _ => db
                        .create_conversation_with_source(None, "mobile")
                        .map_err(|e| (500, e))?
                        .id,
                };

                if controller.is_streaming.load(Ordering::Relaxed) {
                    if !prompt.is_empty() {
                        let _ = db.append_chat_message(conv_id, "user", &prompt, None, None, None);
                        let _ = app.emit(
                            "buddy://conversations-changed",
                            json!({ "conversation_id": conv_id, "source": "mobile" }),
                        );
                        controller.push_steer(prompt);
                    }
                    return Ok(json!({ "ok": true, "steered": true, "conversation_id": conv_id }));
                }

                let stream_id = b["stream_id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| db_now_ms().to_string());

                let (summary_opt, prior_msgs) = db
                    .list_chat_context(conv_id, 20)
                    .unwrap_or_default();
                let prior = prior_msgs
                    .into_iter()
                    .map(|m| (m.role, m.content))
                    .collect::<Vec<_>>();
                let att_json = if attachments.is_empty() {
                    None
                } else {
                    serde_json::to_string(&attachments).ok()
                };
                let stored_prompt = if prompt.is_empty() {
                    "Analyze attached item".to_string()
                } else {
                    prompt
                };
                let _ = db.append_chat_message(
                    conv_id,
                    "user",
                    &stored_prompt,
                    att_json.as_deref(),
                    None,
                    None,
                );
                let _ = app.emit(
                    "buddy://conversations-changed",
                    json!({ "conversation_id": conv_id, "source": "mobile" }),
                );

                let (live_frames, screen_goal, watch_override) = share_mgr.grab_frames_for_chat();
                spawn_chat_stream(
                    app.clone(),
                    settings_mgr.inner().clone(),
                    prompts_mgr.inner().clone(),
                    skills_mgr.inner().clone(),
                    plugin_mgr.inner().clone(),
                    browser_mgr.inner().clone(),
                    mcp_mgr.inner().clone(),
                    sensor_state.inner().clone(),
                    db.inner().clone(),
                    controller.inner().clone(),
                    approval_mgr.inner().clone(),
                    conv_id,
                    stream_id.clone(),
                    stored_prompt,
                    attachments,
                    prior,
                    summary_opt,
                    live_frames,
                    screen_goal,
                    watch_override,
                );
                Ok(json!({ "ok": true, "conversation_id": conv_id, "stream_id": stream_id }))
            }
            ("POST", "/api/chat/stop") => {
                controller.cancel_flag.store(true, Ordering::Relaxed);
                controller.clear_steers();
                approval_mgr.reject_all();
                if browser_mgr.is_user_takeover() {
                    browser_mgr.set_user_takeover(app, false);
                }
                Ok(json!({ "ok": true }))
            }
            ("POST", "/api/approvals/resolve") => {
                let b = body_json();
                let step_id = b["step_id"].as_str().unwrap_or("");
                let dec: ApprovalDecision = serde_json::from_value(b["decision"].clone())
                    .map_err(|e| (400, format!("Invalid decision: {e}")))?;
                approval_mgr.resolve(step_id, dec).map_err(|e| (400, e))?;
                Ok(json!({ "ok": true }))
            }
            ("POST", "/api/upload") => {
                let b = body_json();
                let filename = b["filename"].as_str().unwrap_or("mobile_item.bin");
                let mime = b["mime_type"].as_str().unwrap_or("application/octet-stream");
                let raw_b64 = b["data_base64"].as_str().unwrap_or("");
                let save_to_mem = b["save_to_memory"].as_bool().unwrap_or(false);
                let payload = raw_b64
                    .split_once(",")
                    .map(|(_, r)| r)
                    .unwrap_or(raw_b64)
                    .trim();
                let bytes = B64
                    .decode(payload)
                    .map_err(|e| (400, format!("Invalid base64 upload: {e}")))?;
                let upload_dir = self.data_dir.join("mobile_uploads");
                let _ = std::fs::create_dir_all(&upload_dir);
                let safe_name: String = filename
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                            c
                        } else {
                            '_'
                        }
                    })
                    .take(64)
                    .collect();
                let final_name = format!("{}_{}", db_now_ms(), if safe_name.is_empty() { "file.bin".to_string() } else { safe_name });
                let full_path = upload_dir.join(&final_name);
                std::fs::write(&full_path, &bytes).map_err(|e| (500, e.to_string()))?;
                let full_str = full_path.to_string_lossy().to_string();
                let is_img = mime.starts_with("image/")
                    || matches!(
                        full_path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()).as_deref(),
                        Some("png" | "jpg" | "jpeg" | "webp" | "bmp")
                    );

                if save_to_mem && is_img {
                    let sha = text_sha256(&format!("mobile:{final_name}:{}", bytes.len()));
                    let _ = db.upsert_memory(
                        "clipboard_image",
                        None,
                        Some(&full_str),
                        Some(&full_str),
                        &sha,
                        "BuddyMobile",
                    );
                    let _ = app.emit("buddy://memory-cleaned", ());
                }

                let att = AttachmentPayload {
                    kind: if is_img { "image".to_string() } else { "file".to_string() },
                    label: filename.to_string(),
                    text_content: if is_img {
                        None
                    } else {
                        std::str::from_utf8(&bytes).ok().map(|t| t.chars().take(12_000).collect())
                    },
                    file_path: Some(full_str),
                };
                Ok(json!({ "ok": true, "attachment": att }))
            }
            ("GET", "/api/browser/state") => {
                Ok(json!({ "ok": true, "browser": browser_mgr.get_state() }))
            }
            ("GET", "/api/browser/frame") => {
                let b64 = browser_mgr
                    .execute_screenshot_b64(app)
                    .await
                    .map_err(|e| (400, e))?;
                Ok(json!({
                    "ok": true,
                    "data_url": format!("data:image/jpeg;base64,{b64}"),
                    "browser": browser_mgr.get_state()
                }))
            }
            ("POST", "/api/browser/open") => {
                let b = body_json();
                let url = b["url"].as_str().unwrap_or("");
                let watch = b["watch"].as_bool().unwrap_or(false);
                let st = browser_mgr
                    .open_user_url(app, &sensor_state, url, watch)
                    .map_err(|e| (400, e))?;
                Ok(json!({ "ok": true, "browser": st }))
            }
            ("POST", "/api/browser/nav") => {
                let b = body_json();
                let action = b["action"].as_str().unwrap_or("reload");
                browser_mgr
                    .browser_history_action(app, action)
                    .map_err(|e| (400, e))?;
                Ok(json!({ "ok": true, "browser": browser_mgr.get_state() }))
            }
            ("POST", "/api/browser/takeover") => {
                let b = body_json();
                let takeover = b["takeover"].as_bool().unwrap_or(false);
                let st = browser_mgr.set_user_takeover(app, takeover);
                Ok(json!({ "ok": true, "browser": st }))
            }
            ("POST", "/api/browser/close") => {
                let st = browser_mgr.close_browser(app, &sensor_state);
                Ok(json!({ "ok": true, "browser": st }))
            }
            ("GET", "/api/browser/bookmarks") => {
                let list = db.list_bookmarks().map_err(|e| (500, e))?;
                Ok(json!({ "ok": true, "bookmarks": list }))
            }
            ("POST", "/api/browser/bookmarks") => {
                let b = body_json();
                let title = b["title"].as_str().unwrap_or("");
                let url = b["url"].as_str().unwrap_or("");
                let bm = db.add_bookmark(title, url).map_err(|e| (400, e))?;
                Ok(json!({ "ok": true, "bookmark": bm }))
            }
            ("GET", "/api/desktop/screenshot") => {
                let b64 = tauri::async_runtime::spawn_blocking(|| {
                    capture_primary_desktop_jpeg_b64(1280, 76)
                })
                .await
                .map_err(|e| (500, e.to_string()))?
                .map_err(|e| (500, e))?;
                Ok(json!({
                    "ok": true,
                    "data_url": format!("data:image/jpeg;base64,{b64}"),
                    "timestamp_ms": db_now_ms()
                }))
            }
            ("GET", "/api/memories") => {
                let q = req.query.get("q").map(|s| s.as_str()).filter(|s| !s.trim().is_empty());
                let list = db.list_memories(q, 50).map_err(|e| (500, e))?;
                Ok(json!({ "ok": true, "memories": list }))
            }
            ("POST", "/api/memories/note") => {
                let b = body_json();
                let text = b["text"].as_str().unwrap_or("").trim();
                if text.is_empty() {
                    return Err((400, "Note text cannot be empty".to_string()));
                }
                let sha = text_sha256(text);
                let (item, _) = db
                    .upsert_memory("clipboard_text", Some(text), None, None, &sha, "BuddyMobile")
                    .map_err(|e| (500, e))?;
                let _ = app.emit("buddy://memory-captured", &item);
                Ok(json!({ "ok": true, "memory": item }))
            }
            ("GET", "/api/facts") => {
                let list = db.list_active_facts(100).map_err(|e| (500, e))?;
                Ok(json!({ "ok": true, "facts": list }))
            }
            ("POST", "/api/facts") => {
                let b = body_json();
                let content = b["content"].as_str().unwrap_or("");
                let cat = b["category"].as_str();
                let fact = db.save_fact(content, cat).map_err(|e| (400, e))?;
                Ok(json!({ "ok": true, "fact": fact }))
            }
            ("GET", "/api/reminders") => {
                let list = db.list_reminders(true).map_err(|e| (500, e))?;
                Ok(json!({ "ok": true, "reminders": list }))
            }
            ("POST", "/api/reminders") => {
                let b = body_json();
                let title = b["title"].as_str().unwrap_or("");
                let delay = b["delay_minutes"].as_f64();
                let at_time = b["at_time"].as_str();
                let repeat = b["repeat_rule"].as_str();
                let fire_ms = parse_fire_at_ms(delay, at_time, db_now_ms(), local_utc_offset_ms())
                    .map_err(|e| (400, e))?;
                let rem = db
                    .create_reminder(title, fire_ms, repeat)
                    .map_err(|e| (400, e))?;
                let _ = app.emit("buddy://reminders-changed", ());
                Ok(json!({ "ok": true, "reminder": rem }))
            }
            ("GET", "/api/models") => {
                let cfg = settings_mgr.get();
                Ok(json!({
                    "ok": true,
                    "chat_profile_id": cfg.chat_profile_id,
                    "model_id": cfg.model_id,
                    "profiles": cfg.model_profiles
                }))
            }
            ("POST", "/api/models/select") => {
                let b = body_json();
                let pid = b["profile_id"].as_str().unwrap_or("").to_string();
                let cfg = settings_mgr.get();
                let updated = settings_mgr
                    .select_role_models(Some(pid), Some(cfg.watch_profile_id))
                    .map_err(|e| (400, e))?;
                Ok(json!({ "ok": true, "chat_profile_id": updated.chat_profile_id, "model_id": updated.model_id }))
            }
            _ => {
                // Parameterized routes
                if req.method == "GET"
                    && req.path.starts_with("/api/conversations/")
                    && req.path.ends_with("/messages")
                {
                    let seg = req
                        .path
                        .trim_start_matches("/api/conversations/")
                        .trim_end_matches("/messages");
                    if let Ok(cid) = seg.parse::<i64>() {
                        let msgs = db.list_chat_messages(cid, 100).map_err(|e| (500, e))?;
                        return Ok(json!({ "ok": true, "messages": msgs }));
                    }
                }
                if req.method == "DELETE" && req.path.starts_with("/api/conversations/") {
                    if let Ok(cid) = req.path.trim_start_matches("/api/conversations/").parse::<i64>() {
                        db.delete_conversation(cid).map_err(|e| (500, e))?;
                        let _ = app.emit(
                            "buddy://conversations-changed",
                            json!({ "conversation_id": cid, "deleted": true }),
                        );
                        return Ok(json!({ "ok": true }));
                    }
                }
                if req.method == "DELETE" && req.path.starts_with("/api/memories/") {
                    if let Ok(id) = req.path.trim_start_matches("/api/memories/").parse::<i64>() {
                        db.delete_memory(id).map_err(|e| (500, e))?;
                        let _ = app.emit("buddy://memory-cleaned", ());
                        return Ok(json!({ "ok": true }));
                    }
                }
                if req.method == "DELETE" && req.path.starts_with("/api/facts/") {
                    if let Ok(id) = req.path.trim_start_matches("/api/facts/").parse::<i64>() {
                        db.delete_memory(id).map_err(|e| (500, e))?;
                        return Ok(json!({ "ok": true }));
                    }
                }
                if req.method == "DELETE" && req.path.starts_with("/api/reminders/") {
                    if let Ok(id) = req.path.trim_start_matches("/api/reminders/").parse::<i64>() {
                        db.delete_reminder(id).map_err(|e| (500, e))?;
                        let _ = app.emit("buddy://reminders-changed", ());
                        return Ok(json!({ "ok": true }));
                    }
                }
                if req.method == "POST"
                    && req.path.starts_with("/api/reminders/")
                    && req.path.ends_with("/snooze")
                {
                    let seg = req
                        .path
                        .trim_start_matches("/api/reminders/")
                        .trim_end_matches("/snooze");
                    if let Ok(id) = seg.parse::<i64>() {
                        let mins = body_json()["minutes"].as_i64().unwrap_or(5);
                        let rem = db.snooze_reminder(id, mins).map_err(|e| (400, e))?;
                        let _ = app.emit("buddy://reminders-changed", ());
                        return Ok(json!({ "ok": true, "reminder": rem }));
                    }
                }
                Err((404, format!("Route not found: {} {}", req.method, req.path)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tunnel::{compute_directory_auth_hash, extract_tailscale_ip_from_text, extract_trycloudflare_url};

    #[test]
    fn parses_tunnel_url_tailscale_ip_and_directory_hash() {
        let log = "2026-11-19T12:00:00Z INF |  https://warm-sky-island-42.trycloudflare.com  |";
        assert_eq!(
            extract_trycloudflare_url(log).as_deref(),
            Some("https://warm-sky-island-42.trycloudflare.com")
        );

        let ipcfg = "Wireless LAN adapter Wi-Fi:\n  IPv4 Address. . . : 192.168.1.88\nUnknown adapter Tailscale:\n  IPv4 Address. . . : 100.84.12.9\n";
        assert_eq!(
            extract_tailscale_ip_from_text(ipcfg).as_deref(),
            Some("100.84.12.9")
        );

        let h1 = compute_directory_auth_hash("pair1", "secretA");
        let h2 = compute_directory_auth_hash("pair1", "secretB");
        assert_eq!(h1.len(), 64);
        assert_ne!(h1, h2);
        assert!(constant_time_eq(&h1, &h1));
        assert!(!constant_time_eq(&h1, &h2));
    }

    #[test]
    fn percent_decodes_paths_and_query_cleanly() {
        assert_eq!(
            http::percent_decode("C%3A%5CUsers%5CMarcus%5CMy%20Song.mp3"),
            "C:\\Users\\Marcus\\My Song.mp3"
        );
    }
}
