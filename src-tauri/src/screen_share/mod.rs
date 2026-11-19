//! Live Screen & Target-Window Sharing engine:
//! - Enumerates cursor-following monitor, physical displays, and visible application windows (`capture`).
//! - Captures monitors via BitBlt and GPU/browser windows via PrintWindow with BitBlt fallback (`capture`).
//! - Maintains an in-memory sliding frame buffer (latest + recent comparison frame) with zero disk clutter.
//! - Drives a quiet, settle-debounced proactive sentinel using the configured Watch Vision model (`[SILENT]` when calm).
//! - Renders a click-through, capture-excluded Win32 soft-border overlay around the shared target (`border`).

mod border;
mod capture;

pub use capture::{capture_primary_desktop_jpeg_b64, enumerate_share_targets};

#[cfg(windows)]
use border::spawn_border_overlay_thread;
use border::BorderSpec;
#[cfg(windows)]
use capture::{
    bitblt_rect_image, capture_hwnd_image, compute_luma_thumb, resolve_monitor_rect,
};
use capture::{luma_change_ratio, update_settle_and_should_trigger};

#[cfg(windows)]
use crate::ai_client::encode_dynamic_image_jpeg_base64;
use crate::ai_client::{run_screen_watch_check, AiStreamController, InlineImageFrame};
use crate::settings::SettingsManager;
#[cfg(windows)]
use crate::win32_sensors::get_window_visible_rect;
use crate::win32_sensors::{
    attach_dock_to_window, detach_dock_from_window, set_main_capture_excluded, AppSensorState,
};
use serde::{Deserialize, Serialize};
#[cfg(windows)]
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

#[cfg(windows)]
use windows::Win32::Foundation::HWND;
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_NOREPEAT, MOD_SHIFT,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetMessageW, IsIconic, IsWindow, MSG, WM_HOTKEY,
};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Clone, Debug, Serialize)]
pub struct ShareTargetInfo {
    pub id: String,
    pub kind: String, // "auto_cursor" | "monitor" | "window"
    pub title: String,
    pub app_name: String,
    pub width: i32,
    pub height: i32,
    pub is_primary: bool,
    pub hwnd: isize,
}

#[derive(Clone, Debug, Serialize)]
pub struct ShareStatePayload {
    pub active: bool,
    pub paused: bool,
    pub minimized: bool,
    pub analyzing: bool,
    pub target_id: String,
    pub target_kind: String, // "monitor" | "window"
    pub target_label: String,
    pub target_hwnd: isize,
    pub dock_edge: String,
    pub goal: String,
    pub cadence: String, // "quiet" | "balanced" | "active"
    pub show_border: bool,
    pub watch_profile_id: String,
    pub frame_count: u64,
    pub reason: String,
    pub last_error: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ShareWhisperPayload {
    pub id: String,
    pub text: String,
    pub timestamp_ms: u64,
    pub target_label: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StartShareRequest {
    pub target_id: String,
    #[serde(default)]
    pub goal: String,
    #[serde(default = "default_cadence")]
    pub cadence: String,
    #[serde(default = "default_true")]
    pub show_border: bool,
    #[serde(default)]
    pub watch_profile_id: String,
    #[serde(default)]
    pub window_edge: Option<String>,
    #[serde(default)]
    pub window_ratio: Option<f64>,
}

fn default_cadence() -> String {
    "balanced".to_string()
}

fn default_true() -> bool {
    true
}

#[derive(Clone)]
struct CapturedFrame {
    label: String,
    jpeg_b64: String,
    luma: Vec<u8>,
    captured_at_ms: u64,
}

struct ShareSessionRuntime {
    active: bool,
    paused: bool,
    minimized: bool,
    analyzing: bool,
    generation: u64,
    target_id: String,
    target_kind: String, // "monitor" | "window"
    target_label: String,
    target_hwnd: isize,
    monitor_index: Option<usize>,
    goal: String,
    cadence: String,
    show_border: bool,
    watch_profile_id: String,
    frame_count: u64,
    latest_frame: Option<CapturedFrame>,
    prev_frame: Option<CapturedFrame>,
    prev_live_luma: Option<Vec<u8>>,
    last_watched_luma: Option<Vec<u8>>,
    last_watched_frame: Option<InlineImageFrame>,
    last_change_ms: u64,
    pending_settle: bool,
    last_ai_check_ms: u64,
    recent_whispers: Vec<String>,
    last_error: String,
}

impl Default for ShareSessionRuntime {
    fn default() -> Self {
        Self {
            active: false,
            paused: false,
            minimized: false,
            analyzing: false,
            generation: 0,
            target_id: String::new(),
            target_kind: "monitor".to_string(),
            target_label: String::new(),
            target_hwnd: 0,
            monitor_index: None,
            goal: String::new(),
            cadence: "balanced".to_string(),
            show_border: true,
            watch_profile_id: String::new(),
            frame_count: 0,
            latest_frame: None,
            prev_frame: None,
            prev_live_luma: None,
            last_watched_luma: None,
            last_watched_frame: None,
            last_change_ms: 0,
            pending_settle: false,
            last_ai_check_ms: 0,
            recent_whispers: Vec::new(),
            last_error: String::new(),
        }
    }
}

pub struct ScreenShareManager {
    runtime: Mutex<ShareSessionRuntime>,
    border_desired: Arc<Mutex<BorderSpec>>,
    stop_requested: AtomicBool,
    session_counter: AtomicU64,
}

impl Default for ScreenShareManager {
    fn default() -> Self {
        let desired = Arc::new(Mutex::new(BorderSpec::default()));
        #[cfg(windows)]
        spawn_border_overlay_thread(desired.clone());
        Self {
            runtime: Mutex::new(ShareSessionRuntime::default()),
            border_desired: desired,
            stop_requested: AtomicBool::new(false),
            session_counter: AtomicU64::new(0),
        }
    }
}

impl ScreenShareManager {
    pub fn get_state(&self, sensor_state: &AppSensorState) -> ShareStatePayload {
        let g = self.runtime.lock().ok();
        let dock_edge = sensor_state.get_dock_edge();
        if let Some(rt) = g {
            ShareStatePayload {
                active: rt.active,
                paused: rt.paused,
                minimized: rt.minimized,
                analyzing: rt.analyzing,
                target_id: rt.target_id.clone(),
                target_kind: rt.target_kind.clone(),
                target_label: rt.target_label.clone(),
                target_hwnd: rt.target_hwnd,
                dock_edge,
                goal: rt.goal.clone(),
                cadence: rt.cadence.clone(),
                show_border: rt.show_border,
                watch_profile_id: rt.watch_profile_id.clone(),
                frame_count: rt.frame_count,
                reason: String::new(),
                last_error: rt.last_error.clone(),
            }
        } else {
            ShareStatePayload {
                active: false,
                paused: false,
                minimized: false,
                analyzing: false,
                target_id: String::new(),
                target_kind: "monitor".to_string(),
                target_label: String::new(),
                target_hwnd: 0,
                dock_edge,
                goal: String::new(),
                cadence: "balanced".to_string(),
                show_border: true,
                watch_profile_id: String::new(),
                frame_count: 0,
                reason: String::new(),
                last_error: String::new(),
            }
        }
    }

    fn emit_state(&self, app: &AppHandle, sensor_state: &AppSensorState, reason: &str) {
        let mut payload = self.get_state(sensor_state);
        payload.reason = reason.to_string();
        let _ = app.emit("buddy://share-state", payload);
    }

    fn set_border(&self, spec: BorderSpec) {
        if let Ok(mut b) = self.border_desired.lock() {
            *b = spec;
        }
    }

    pub fn update_live_options(
        &self,
        app: &AppHandle,
        sensor_state: &AppSensorState,
        goal: Option<String>,
        cadence: Option<String>,
        show_border: Option<bool>,
        watch_profile_id: Option<String>,
    ) -> ShareStatePayload {
        if let Ok(mut rt) = self.runtime.lock() {
            if let Some(g) = goal {
                rt.goal = g;
            }
            if let Some(c) = cadence {
                rt.cadence = c;
            }
            if let Some(sb) = show_border {
                rt.show_border = sb;
                if !sb || !rt.active || rt.paused || rt.minimized {
                    self.set_border(BorderSpec::default());
                }
            }
            if let Some(wp) = watch_profile_id {
                rt.watch_profile_id = wp;
            }
        }
        self.emit_state(app, sensor_state, "options_updated");
        self.get_state(sensor_state)
    }

    pub fn toggle_pause(&self, app: &AppHandle, sensor_state: &AppSensorState) -> ShareStatePayload {
        if let Ok(mut rt) = self.runtime.lock() {
            if !rt.active {
                return self.get_state(sensor_state);
            }
            rt.paused = !rt.paused;
            if rt.paused {
                rt.pending_settle = false;
                rt.analyzing = false;
                self.set_border(BorderSpec::default());
            }
        }
        self.emit_state(app, sensor_state, "pause_toggled");
        self.get_state(sensor_state)
    }

    pub fn stop(&self, app: &AppHandle, sensor_state: &AppSensorState, reason: &str) -> ShareStatePayload {
        self.stop_requested.store(true, Ordering::Relaxed);
        if let Ok(mut rt) = self.runtime.lock() {
            *rt = ShareSessionRuntime {
                generation: rt.generation.wrapping_add(1),
                ..Default::default()
            };
        }
        self.set_border(BorderSpec::default());

        if let Some(win) = app.get_webview_window("main") {
            set_main_capture_excluded(&win, false);
            detach_dock_from_window(app, &win, sensor_state);
        }

        self.emit_state(app, sensor_state, reason);
        self.get_state(sensor_state)
    }

    pub fn start(
        self: &Arc<Self>,
        app: AppHandle,
        sensor_state: Arc<AppSensorState>,
        settings_mgr: Arc<SettingsManager>,
        ai_ctrl: Arc<AiStreamController>,
        req: StartShareRequest,
    ) -> Result<ShareStatePayload, String> {
        let targets = enumerate_share_targets();
        let (kind, hwnd, mon_idx, label) = if let Some(rest) = req.target_id.strip_prefix("window:") {
            let parsed_hwnd: isize = rest
                .trim()
                .parse()
                .map_err(|_| "Invalid window handle".to_string())?;
            #[cfg(windows)]
            {
                if unsafe { !IsWindow(HWND(parsed_hwnd as *mut c_void)).as_bool() } {
                    return Err("Selected window is no longer open.".to_string());
                }
            }
            let lbl = targets
                .iter()
                .find(|t| t.hwnd == parsed_hwnd)
                .map(|t| {
                    if t.app_name.is_empty() {
                        t.title.clone()
                    } else {
                        format!("{} — {}", t.app_name, t.title)
                    }
                })
                .unwrap_or_else(|| "Selected Window".to_string());
            ("window".to_string(), parsed_hwnd, None, lbl)
        } else if let Some(idx_str) = req.target_id.strip_prefix("monitor:") {
            let idx: usize = idx_str.trim().parse().unwrap_or(0);
            let lbl = targets
                .iter()
                .find(|t| t.id == req.target_id)
                .map(|t| t.title.clone())
                .unwrap_or_else(|| format!("Display {}", idx + 1));
            ("monitor".to_string(), 0, Some(idx), lbl)
        } else {
            (
                "monitor".to_string(),
                0,
                None,
                "Mouse Screen (Auto)".to_string(),
            )
        };

        let gen = self.session_counter.fetch_add(1, Ordering::Relaxed) + 1;
        self.stop_requested.store(false, Ordering::Relaxed);

        if let Ok(mut rt) = self.runtime.lock() {
            *rt = ShareSessionRuntime {
                active: true,
                paused: false,
                minimized: false,
                analyzing: false,
                generation: gen,
                target_id: req.target_id.clone(),
                target_kind: kind.clone(),
                target_label: label.clone(),
                target_hwnd: hwnd,
                monitor_index: mon_idx,
                goal: req.goal.trim().to_string(),
                cadence: match req.cadence.as_str() {
                    "quiet" | "active" => req.cadence.clone(),
                    _ => "balanced".to_string(),
                },
                show_border: req.show_border,
                watch_profile_id: req.watch_profile_id.clone(),
                frame_count: 0,
                latest_frame: None,
                prev_frame: None,
                prev_live_luma: None,
                last_watched_luma: None,
                last_watched_frame: None,
                last_change_ms: 0,
                pending_settle: false,
                last_ai_check_ms: 0,
                recent_whispers: Vec::new(),
                last_error: String::new(),
            };
        }

        if let Some(win) = app.get_webview_window("main") {
            set_main_capture_excluded(&win, true);
            if kind == "window" && hwnd != 0 {
                let edge = req.window_edge.as_deref().unwrap_or("top");
                let ratio = req.window_ratio.unwrap_or(0.5);
                attach_dock_to_window(&app, &win, &sensor_state, hwnd, edge, ratio);
            } else {
                detach_dock_from_window(&app, &win, &sensor_state);
            }
        }

        self.emit_state(&app, &sensor_state, "started");
        let snapshot = self.get_state(&sensor_state);

        let mgr = Arc::clone(self);
        std::thread::spawn(move || {
            mgr.run_session_loop(gen, app, sensor_state, settings_mgr, ai_ctrl);
        });

        Ok(snapshot)
    }

    fn capture_frame_once(
        &self,
        kind: &str,
        hwnd: isize,
        mon_idx: Option<usize>,
        label: &str,
        max_dim: u32,
        quality: u8,
    ) -> Result<Option<(CapturedFrame, (i32, i32, i32, i32), bool)>, String> {
        #[cfg(windows)]
        {
            if kind == "window" && hwnd != 0 {
                let h = HWND(hwnd as *mut c_void);
                if unsafe { !IsWindow(h).as_bool() } {
                    return Err("window_closed".to_string());
                }
                if unsafe { IsIconic(h).as_bool() } {
                    return Ok(None);
                }
                let rect = get_window_visible_rect(h).unwrap_or((0, 0, 800, 600));
                let fg = unsafe { GetForegroundWindow() };
                let dimmed = fg.0 != h.0;
                if let Some(img) = capture_hwnd_image(h) {
                    let b64 = encode_dynamic_image_jpeg_base64(&img, max_dim, quality)?;
                    let luma = compute_luma_thumb(&img);
                    return Ok(Some((
                        CapturedFrame {
                            label: label.to_string(),
                            jpeg_b64: b64,
                            luma,
                            captured_at_ms: now_ms(),
                        },
                        rect,
                        dimmed,
                    )));
                }
                return Ok(None);
            } else {
                let rect = resolve_monitor_rect(mon_idx);
                if let Some(img) = bitblt_rect_image(rect.0, rect.1, rect.2, rect.3) {
                    let b64 = encode_dynamic_image_jpeg_base64(&img, max_dim, quality)?;
                    let luma = compute_luma_thumb(&img);
                    return Ok(Some((
                        CapturedFrame {
                            label: label.to_string(),
                            jpeg_b64: b64,
                            luma,
                            captured_at_ms: now_ms(),
                        },
                        rect,
                        false,
                    )));
                }
                return Ok(None);
            }
        }
        #[cfg(not(windows))]
        Ok(None)
    }

    pub fn grab_frames_for_chat(&self) -> (Vec<InlineImageFrame>, Option<String>, Option<String>) {
        let snapshot = self.runtime.lock().ok().and_then(|rt| {
            if !rt.active || rt.paused || rt.minimized {
                return None;
            }
            Some((
                rt.target_kind.clone(),
                rt.target_hwnd,
                rt.monitor_index,
                rt.target_label.clone(),
                rt.goal.clone(),
                rt.watch_profile_id.clone(),
                rt.latest_frame.clone(),
                rt.prev_frame.clone(),
            ))
        });

        let Some((kind, hwnd, mon_idx, label, goal, watch_prof, mut latest, mut prev)) = snapshot
        else {
            return (Vec::new(), None, None);
        };

        let stale = latest
            .as_ref()
            .map(|f| now_ms().saturating_sub(f.captured_at_ms) > 650)
            .unwrap_or(true);

        if stale {
            if let Ok(Some((fresh, _, _))) =
                self.capture_frame_once(&kind, hwnd, mon_idx, &label, 2304, 88)
            {
                if let Some(old) = &latest {
                    if luma_change_ratio(&old.luma, &fresh.luma) >= 0.025 {
                        prev = Some(old.clone());
                    }
                }
                latest = Some(fresh.clone());
                if let Ok(mut rt) = self.runtime.lock() {
                    if rt.active {
                        rt.prev_frame = prev.clone();
                        rt.latest_frame = Some(fresh);
                        rt.frame_count += 1;
                    }
                }
            }
        }

        let mut frames = Vec::new();
        if let (Some(p), Some(l)) = (&prev, &latest) {
            if luma_change_ratio(&p.luma, &l.luma) >= 0.025 {
                frames.push(InlineImageFrame {
                    label: format!("{} (just before)", p.label),
                    jpeg_b64: p.jpeg_b64.clone(),
                });
            }
        }
        if let Some(l) = &latest {
            frames.push(InlineImageFrame {
                label: format!("{} (live now)", l.label),
                jpeg_b64: l.jpeg_b64.clone(),
            });
        }

        let wp = if watch_prof.trim().is_empty() {
            None
        } else {
            Some(watch_prof)
        };
        (frames, Some(goal), wp)
    }

    fn run_session_loop(
        self: Arc<Self>,
        gen: u64,
        app: AppHandle,
        sensor_state: Arc<AppSensorState>,
        settings_mgr: Arc<SettingsManager>,
        ai_ctrl: Arc<AiStreamController>,
    ) {
        let mut last_capture_ms = 0u64;

        loop {
            if self.stop_requested.load(Ordering::Relaxed) {
                break;
            }

            let state_snap = match self.runtime.lock() {
                Ok(rt) => {
                    if !rt.active || rt.generation != gen {
                        break;
                    }
                    (
                        rt.paused,
                        rt.minimized,
                        rt.analyzing,
                        rt.target_kind.clone(),
                        rt.target_hwnd,
                        rt.monitor_index,
                        rt.target_label.clone(),
                        rt.show_border,
                        rt.cadence.clone(),
                        rt.goal.clone(),
                        rt.watch_profile_id.clone(),
                        rt.last_change_ms,
                        rt.pending_settle,
                        rt.last_ai_check_ms,
                        rt.recent_whispers.clone(),
                    )
                }
                Err(_) => break,
            };

            let (
                paused,
                was_minimized,
                analyzing,
                kind,
                hwnd,
                mon_idx,
                label,
                show_border,
                cadence,
                goal,
                watch_profile_id,
                last_change_ms,
                pending_settle,
                last_ai_check_ms,
                recent_whispers,
            ) = state_snap;

            #[cfg(windows)]
            if kind == "window" && hwnd != 0 {
                let h = HWND(hwnd as *mut c_void);
                if unsafe { !IsWindow(h).as_bool() } {
                    self.stop(&app, &sensor_state, "window_closed");
                    break;
                }
                let is_min = unsafe { IsIconic(h).as_bool() };
                if is_min != was_minimized {
                    if let Ok(mut rt) = self.runtime.lock() {
                        if rt.generation == gen {
                            rt.minimized = is_min;
                        }
                    }
                    self.emit_state(
                        &app,
                        &sensor_state,
                        if is_min { "minimized" } else { "restored" },
                    );
                }

                // Border overlay self-tracks target_hwnd at 140Hz directly; update high-level state here
                if show_border && !paused && !is_min {
                    if let Some(vr) = get_window_visible_rect(h) {
                        let fg = unsafe { GetForegroundWindow() };
                        let dimmed = fg.0 != h.0;
                        self.set_border(BorderSpec {
                            visible: true,
                            is_window_mode: true,
                            target_hwnd: hwnd,
                            rect: vr,
                            dimmed,
                        });
                    }
                } else {
                    self.set_border(BorderSpec::default());
                }
            } else if show_border && !paused {
                let rect = resolve_monitor_rect(mon_idx);
                self.set_border(BorderSpec {
                    visible: true,
                    is_window_mode: false,
                    target_hwnd: 0,
                    rect,
                    dimmed: false,
                });
            } else {
                self.set_border(BorderSpec::default());
            }

            let t_now = now_ms();
            if !paused && !was_minimized && t_now.saturating_sub(last_capture_ms) >= 1050 {
                last_capture_ms = t_now;
                match self.capture_frame_once(&kind, hwnd, mon_idx, &label, 2048, 86) {
                    Err(e) if e == "window_closed" => {
                        self.stop(&app, &sensor_state, "window_closed");
                        break;
                    }
                    Ok(Some((frame, _rect, _dimmed))) => {
                        let mut should_trigger_watch = false;
                        let mut check_prev_frame: Option<InlineImageFrame> = None;
                        let mut check_curr_frame: Option<InlineImageFrame> = None;

                        if let Ok(mut rt) = self.runtime.lock() {
                            if !rt.active || rt.generation != gen {
                                break;
                            }
                            if let Some(prev_live) = &rt.latest_frame {
                                if luma_change_ratio(&prev_live.luma, &frame.luma) >= 0.014 {
                                    rt.prev_frame = Some(prev_live.clone());
                                }
                            }
                            rt.latest_frame = Some(frame.clone());
                            rt.frame_count += 1;

                            let busy_chatting = ai_ctrl.cancel_flag.load(Ordering::Relaxed)
                                || sensor_state.is_dragging.load(Ordering::Relaxed);
                            let cadence_now = rt.cadence.clone();
                            let last_ai_ms = rt.last_ai_check_ms;
                            let analyzing_now = rt.analyzing;
                            let last_watched = rt.last_watched_luma.clone();
                            let mut prev_live = rt.prev_live_luma.take();
                            let mut chg_ms = rt.last_change_ms;
                            let mut pend = rt.pending_settle;

                            let fire = update_settle_and_should_trigger(
                                &cadence_now,
                                &mut prev_live,
                                last_watched.as_deref(),
                                &frame.luma,
                                &mut chg_ms,
                                &mut pend,
                                last_ai_ms,
                                analyzing_now,
                                busy_chatting,
                                t_now,
                            );
                            rt.prev_live_luma = prev_live;
                            rt.last_change_ms = chg_ms;
                            rt.pending_settle = pend;

                            if fire {
                                rt.analyzing = true;
                                rt.pending_settle = false;
                                rt.last_ai_check_ms = t_now;
                                rt.last_watched_luma = Some(frame.luma.clone());
                                check_prev_frame = rt.last_watched_frame.clone();
                                let inline = InlineImageFrame {
                                    label: label.clone(),
                                    jpeg_b64: frame.jpeg_b64.clone(),
                                };
                                rt.last_watched_frame = Some(inline.clone());
                                check_curr_frame = Some(inline);
                                should_trigger_watch = true;
                            }
                        }

                        if should_trigger_watch {
                            self.emit_state(&app, &sensor_state, "analyzing");
                            if let Some(curr) = check_curr_frame {
                                let mgr_async = Arc::clone(&self);
                                let app_c = app.clone();
                                let ss_c = sensor_state.clone();
                                let sm_c = settings_mgr.clone();
                                let wp = watch_profile_id.clone();
                                let lbl_c = label.clone();
                                let goal_c = goal.clone();
                                let cad_c = cadence.clone();
                                let recent_c = recent_whispers.clone();
                                tauri::async_runtime::spawn(async move {
                                    let cfg = sm_c.get();
                                    let (model, key) = sm_c.resolve_watch_target(if wp.trim().is_empty() {
                                        None
                                    } else {
                                        Some(&wp)
                                    });
                                    let res = if !model.supports_vision {
                                        Err("Watch model has Vision disabled in Settings.".to_string())
                                    } else if key.trim().is_empty()
                                        && !model.base_url.contains("localhost")
                                        && !model.base_url.contains("127.0.0.1")
                                    {
                                        Err("No API key configured for Watch model.".to_string())
                                    } else if let Ok(client) = reqwest::Client::builder()
                                        .timeout(Duration::from_secs(45))
                                        .build()
                                    {
                                        run_screen_watch_check(
                                            &client,
                                            &model,
                                            &key,
                                            &lbl_c,
                                            &goal_c,
                                            &cad_c,
                                            &cfg.ui_language,
                                            &recent_c,
                                            check_prev_frame.as_ref(),
                                            &curr,
                                        )
                                        .await
                                    } else {
                                        Err("Failed to build HTTP client.".to_string())
                                    };

                                    let mut emit_whisper: Option<ShareWhisperPayload> = None;
                                    if let Ok(mut rt) = mgr_async.runtime.lock() {
                                        if rt.generation == gen {
                                            rt.analyzing = false;
                                            match res {
                                                Ok(Some(text)) => {
                                                    rt.last_error.clear();
                                                    let clean = text.trim().to_string();
                                                    if !clean.is_empty() {
                                                        rt.recent_whispers.push(clean.clone());
                                                        if rt.recent_whispers.len() > 4 {
                                                            rt.recent_whispers.remove(0);
                                                        }
                                                        emit_whisper = Some(ShareWhisperPayload {
                                                            id: format!("w_{}", now_ms()),
                                                            text: clean,
                                                            timestamp_ms: now_ms(),
                                                            target_label: lbl_c.clone(),
                                                        });
                                                    }
                                                }
                                                Ok(None) => {
                                                    rt.last_error.clear();
                                                }
                                                Err(err) => {
                                                    rt.last_error = err.chars().take(180).collect();
                                                }
                                            }
                                        }
                                    }
                                    mgr_async.emit_state(&app_c, &ss_c, "analysis_done");
                                    if let Some(w) = emit_whisper {
                                        let _ = app_c.emit("buddy://share-whisper", w);
                                    }
                                });
                            }
                        } else {
                            let _ = (cadence, goal, last_change_ms, pending_settle, last_ai_check_ms, analyzing);
                        }
                    }
                    _ => {}
                }
            }

            std::thread::sleep(Duration::from_millis(260));
        }
    }
}

/// Registers global Alt + Shift + P hotkey to toggle Screen Share privacy pause/resume.
pub fn start_share_hotkey_listener(
    app: AppHandle,
    sensor_state: Arc<AppSensorState>,
    share_mgr: Arc<ScreenShareManager>,
) {
    #[cfg(windows)]
    std::thread::spawn(move || unsafe {
        const HOTKEY_PAUSE_ID: i32 = 102;
        // Alt + Shift + P (VK_P = 0x50)
        if RegisterHotKey(None, HOTKEY_PAUSE_ID, MOD_ALT | MOD_SHIFT | MOD_NOREPEAT, 0x50).is_err() {
            return;
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_HOTKEY && msg.wParam.0 as i32 == HOTKEY_PAUSE_ID {
                share_mgr.toggle_pause(&app, &sensor_state);
            }
        }
        let _ = UnregisterHotKey(None, HOTKEY_PAUSE_ID);
    });
}
