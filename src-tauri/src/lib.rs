mod agent_tools;
mod ai_client;
mod browser;
mod commands;
mod db;
mod file_dialog;
mod llm_protocols;
mod logger;
mod mcp;
mod media_utils;
mod mobile_bridge;
mod music_sensor;
mod pin_window;
mod plugins;
mod prompts;
mod reminders;
mod screen_share;
mod settings;
mod skills;
mod snip;
mod thinking;
mod watchers;
mod win32_sensors;

use agent_tools::AgentApprovalManager;
use ai_client::AiStreamController;
use browser::BrowserManager;
use commands::chat::classify_dropped_paths;
use commands::now_ms;
use commands::system::{build_updater, UpdateInfo};
use db::MemoryDb;
use mcp::McpManager;
use media_utils::SelfCopyGuard;
use mobile_bridge::MobileBridgeManager;
use plugins::PluginManager;
use prompts::PromptsManager;
use screen_share::{start_share_hotkey_listener, ScreenShareManager};
use settings::SettingsManager;
use skills::SkillsManager;
use snip::{
    begin_snip_capture, cancel_snip, finish_snip, get_snip_preview, start_snip_hotkey_listener,
    SnipState,
};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{DragDropEvent, Emitter, Manager, WindowEvent};
use win32_sensors::{
    apply_interaction_mode, apply_window_layout, show_window_no_activate, start_sensor_loops,
    start_toggle_hotkey_listener, AppSensorState,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let db = Arc::new(MemoryDb::open().expect("failed to initialize buddy.db"));
    let _ = logger::init_logger(&db.data_dir);
    let settings_mgr = Arc::new(SettingsManager::load(&db.data_dir));
    let prompts_mgr = Arc::new(PromptsManager::init(&db.data_dir));
    let plugin_mgr = Arc::new(PluginManager::init(&db.data_dir));
    let skills_mgr = Arc::new(SkillsManager::init(&db.data_dir));
    let initial_cfg = settings_mgr.get();

    let sensor_state = Arc::new(AppSensorState::default());
    sensor_state
        .auto_hide_enabled
        .store(initial_cfg.auto_hide_stealth, Ordering::Relaxed);
    sensor_state
        .auto_collapse_on_blur
        .store(initial_cfg.auto_collapse_on_blur, Ordering::Relaxed);
    sensor_state
        .music_reactive_enabled
        .store(initial_cfg.music_reactive, Ordering::Relaxed);
    if matches!(
        initial_cfg.dock_edge.as_str(),
        "top" | "bottom" | "left" | "right" | "free"
    ) {
        sensor_state.set_dock_edge(&initial_cfg.dock_edge);
    }
    if let (Some(cx), Some(cy)) = (initial_cfg.dock_cx, initial_cfg.dock_cy) {
        sensor_state.anchor_cx.store(cx, Ordering::Relaxed);
        sensor_state.anchor_cy.store(cy, Ordering::Relaxed);
        sensor_state.has_anchor.store(true, Ordering::Relaxed);
    }

    let copy_guard = Arc::new(SelfCopyGuard::default());
    let snip_state = Arc::new(SnipState::default());
    let ai_controller = Arc::new(AiStreamController::default());
    let approval_mgr = Arc::new(AgentApprovalManager::default());
    let share_mgr = Arc::new(ScreenShareManager::default());
    let browser_mgr = Arc::new(BrowserManager::init(&db.data_dir));
    let mcp_mgr = Arc::new(McpManager::init(&db.data_dir));
    let mobile_mgr = Arc::new(MobileBridgeManager::init(&db.data_dir));

    let boot_settings = settings_mgr.clone();
    std::thread::spawn(move || {
        boot_settings.sync_autostart_on_boot();
    });

    let _ = db.cleanup_expired();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                show_window_no_activate(&win);
                let _ = app.emit("buddy://expand", ());
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(sensor_state.clone())
        .manage(db.clone())
        .manage(settings_mgr.clone())
        .manage(prompts_mgr.clone())
        .manage(copy_guard.clone())
        .manage(snip_state.clone())
        .manage(ai_controller.clone())
        .manage(approval_mgr.clone())
        .manage(plugin_mgr.clone())
        .manage(skills_mgr.clone())
        .manage(share_mgr.clone())
        .manage(browser_mgr.clone())
        .manage(mcp_mgr.clone())
        .manage(mobile_mgr.clone())
        .invoke_handler(tauri::generate_handler![
            commands::system::set_expanded,
            commands::system::set_window_mode,
            commands::system::toggle_window_maximize,
            commands::system::start_window_mode_drag,
            commands::system::update_pill_bounds,
            commands::system::get_dock_state,
            commands::system::begin_buddy_drag,
            commands::system::step_buddy_drag,
            commands::system::end_buddy_drag,
            commands::system::reset_dock_position,
            commands::system::get_app_version,
            commands::system::check_app_update,
            commands::system::install_app_update,
            commands::system::set_auto_hide_stealth,
            commands::system::set_auto_collapse_on_blur,
            commands::system::set_agent_permission_mode,
            commands::system::set_music_reactive,
            commands::system::set_music_playing_flag,
            commands::system::get_expanded_bounds,
            commands::system::list_memories,
            commands::system::list_memory_facts,
            commands::system::add_memory_fact,
            commands::system::toggle_memory_star,
            commands::system::delete_memory,
            commands::system::copy_memory_to_clipboard,
            commands::system::read_memory_image,
            commands::system::get_settings,
            commands::system::save_settings,
            commands::system::verify_and_save_model_config,
            commands::system::save_model_profile,
            commands::system::verify_and_save_model_profile,
            commands::system::delete_model_profile,
            commands::system::select_role_models,
            commands::chat::list_personalities,
            commands::chat::open_prompts_folder,
            commands::chat::ensure_active_conversation,
            commands::chat::ensure_fresh_startup_conversation,
            commands::chat::list_conversations,
            commands::chat::create_conversation,
            commands::chat::delete_conversation,
            commands::chat::rename_conversation,
            commands::chat::search_conversations,
            commands::chat::truncate_conversation_from_user_turn,
            commands::chat::list_browser_bookmarks,
            commands::chat::add_browser_bookmark,
            commands::chat::delete_browser_bookmark,
            commands::chat::list_reminders,
            commands::chat::create_reminder,
            commands::chat::toggle_reminder,
            commands::chat::snooze_reminder,
            commands::chat::delete_reminder,
            commands::chat::list_chat_history,
            commands::chat::send_chat_message,
            commands::chat::steer_chat_stream,
            commands::chat::resolve_agent_approval,
            commands::chat::cancel_chat_stream,
            commands::chat::get_browser_state,
            commands::chat::set_browser_watching,
            commands::chat::set_browser_takeover,
            commands::chat::browser_navigate,
            commands::chat::open_url_in_builtin_browser,
            commands::chat::open_external_url,
            commands::chat::save_pasted_image_attachment,
            commands::chat::browser_nav_action,
            commands::chat::close_builtin_browser,
            commands::chat::clear_browser_data,
            commands::chat::open_local_media_file,
            commands::chat::read_local_audio_data_url,
            commands::chat::list_share_targets,
            commands::chat::get_share_state,
            commands::chat::start_screen_share,
            commands::chat::stop_screen_share,
            commands::chat::toggle_screen_share_pause,
            commands::chat::update_screen_share_options,
            commands::chat::set_window_dock_preset,
            commands::chat::set_whisper_bounds,
            commands::chat::accept_share_whisper,
            commands::chat::trigger_snip,
            commands::chat::pick_chat_attachments,
            commands::chat::export_conversation_markdown,
            get_snip_preview,
            finish_snip,
            cancel_snip,
            commands::plugins::list_plugins,
            commands::mcp::list_mcp_servers,
            commands::mcp::save_mcp_server,
            commands::mcp::import_mcp_servers_json,
            commands::mcp::toggle_mcp_server,
            commands::mcp::set_mcp_server_trusted,
            commands::mcp::reconnect_mcp_server,
            commands::mcp::reconnect_all_mcp_servers,
            commands::mcp::delete_mcp_server,
            commands::mobile::get_mobile_bridge_status,
            commands::mobile::set_mobile_bridge_enabled,
            commands::mobile::start_mobile_tunnel,
            commands::mobile::stop_mobile_tunnel,
            commands::mobile::update_mobile_remote_settings,
            commands::mobile::reset_mobile_pairing_token,
            commands::plugins::toggle_plugin,
            commands::plugins::set_plugin_mirror,
            commands::plugins::install_plugin_engine,
            commands::plugins::cancel_plugin_download,
            commands::plugins::uninstall_plugin_engine,
            commands::plugins::plugin_offline_ocr,
            commands::plugins::plugin_pin_image,
            commands::plugins::plugin_close_all_pins,
            commands::plugins::plugin_parse_clipboard,
            commands::plugins::plugin_clean_paste,
            commands::plugins::plugin_inspect_locks,
            commands::plugins::plugin_unlock_pid,
            commands::plugins::plugin_start_color_picker,
            commands::plugins::plugin_set_keep_awake,
            commands::plugins::plugin_get_keep_awake,
            commands::plugins::plugin_start_timer,
            commands::plugins::plugin_cancel_timer,
            commands::plugins::plugin_optimize_images,
            commands::plugins::plugin_run_pdf_tool,
            commands::plugins::plugin_run_media_tool,
            commands::plugins::plugin_download_media,
            commands::plugins::cancel_plugin_task,
            commands::plugins::plugin_open_folder,
            commands::plugins::open_logs_folder,
            commands::plugins::pick_folder,
            commands::plugins::pick_files,
            commands::plugins::list_skills,
            commands::plugins::get_skill_detail,
            commands::plugins::toggle_skill,
            commands::plugins::uninstall_skill,
            commands::plugins::import_local_skill,
            commands::plugins::search_github_skills_cmd,
            commands::plugins::install_github_skill_cmd,
            commands::plugins::open_skills_folder
        ])
        .setup(move |app| {
            let main_win = app
                .get_webview_window("main")
                .expect("main window should exist");

            apply_window_layout(&main_win, &sensor_state, false);
            apply_interaction_mode(&main_win, false);

            // Silent background update check ~4 seconds after launch
            let app_upd = app.handle().clone();
            let mgr_upd = settings_mgr.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                let cfg = mgr_upd.get();
                let custom = cfg.update_endpoint.trim().to_string();
                let mut skip = false;
                if custom.is_empty() {
                    if let Some(u_cfg) = app_upd.config().plugins.0.get("updater") {
                        if u_cfg.to_string().contains("YOUR_OWNER") {
                            skip = true;
                        }
                    }
                }
                if !skip {
                    if let Ok(updater) = build_updater(&app_upd, &mgr_upd) {
                        if let Ok(Some(upd)) = updater.check().await {
                            let current = app_upd.package_info().version.to_string();
                            let _ = app_upd.emit(
                                "buddy://update-available",
                                UpdateInfo {
                                    version: upd.version,
                                    current_version: current,
                                    body: upd.body.unwrap_or_default(),
                                    date: upd.date.map(|d| d.to_string()),
                                },
                            );
                        }
                    }
                }
            });

            let expand_item = MenuItem::with_id(app, "expand", "Open Buddy", true, None::<&str>)?;
            let snip_item = MenuItem::with_id(
                app,
                "snip",
                "Quick Screenshot (Alt+Shift+S)",
                true,
                None::<&str>,
            )?;
            let music_toggle = CheckMenuItem::with_id(
                app,
                "music_reactive",
                "Music Reactive",
                true,
                initial_cfg.music_reactive,
                None::<&str>,
            )?;
            let stealth_toggle = CheckMenuItem::with_id(
                app,
                "auto_hide",
                "Auto-Hide During Games",
                true,
                initial_cfg.auto_hide_stealth,
                None::<&str>,
            )?;
            let capture_item = MenuItem::with_id(
                app,
                "sim_capture",
                "Test Capture Animation",
                true,
                None::<&str>,
            )?;
            let sleep_item =
                MenuItem::with_id(app, "sim_sleep", "Toggle Sleep Mode", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit Buddy", true, None::<&str>)?;

            let tray_menu = Menu::with_items(
                app,
                &[
                    &expand_item,
                    &snip_item,
                    &music_toggle,
                    &stealth_toggle,
                    &capture_item,
                    &sleep_item,
                    &quit_item,
                ],
            )?;

            let tray_state = sensor_state.clone();
            let tray_db = db.clone();
            let tray_snip = snip_state.clone();
            let tray_settings = settings_mgr.clone();
            let tray_plugins = plugin_mgr.clone();
            let tray_share = share_mgr.clone();
            let tray_browser = browser_mgr.clone();
            let tray_mcp = mcp_mgr.clone();
            let tray_mobile = mobile_mgr.clone();
            let stealth_item_ref = stealth_toggle.clone();
            let music_item_ref = music_toggle.clone();
            let mut tray_builder = TrayIconBuilder::new()
                .menu(&tray_menu)
                .tooltip("Buddy — Desktop AI Companion");

            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            }

            tray_builder
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "expand" => {
                        if let Some(win) = app.get_webview_window("main") {
                            let _vis_guard = tray_state.visibility_lock.lock();
                            tray_state.is_stealth.store(false, Ordering::Release);
                            show_window_no_activate(&win);
                            let _ = app.emit("buddy://expand", ());
                        }
                    }
                    "snip" => {
                        begin_snip_capture(app, &tray_snip);
                    }
                    "music_reactive" => {
                        let checked = music_item_ref.is_checked().unwrap_or(true);
                        tray_state
                            .music_reactive_enabled
                            .store(checked, Ordering::Relaxed);
                        if !checked {
                            tray_state.is_music_playing.store(false, Ordering::Relaxed);
                        }
                        let mut cfg = tray_settings.get();
                        cfg.music_reactive = checked;
                        let _ = tray_settings.update(cfg, None);
                        let _ = app.emit("buddy://music-reactive-changed", checked);
                    }
                    "auto_hide" => {
                        let checked = stealth_item_ref.is_checked().unwrap_or(false);
                        tray_state
                            .auto_hide_enabled
                            .store(checked, Ordering::Relaxed);
                        let mut cfg = tray_settings.get();
                        cfg.auto_hide_stealth = checked;
                        let _ = tray_settings.update(cfg, None);
                        let _ = app.emit("buddy://auto-hide-changed", checked);
                    }
                    "sim_capture" => {
                        let _ = app.emit("buddy://simulate-capture", ());
                    }
                    "sim_sleep" => {
                        let next = !tray_state.is_sleeping.load(Ordering::Relaxed);
                        tray_state.manual_sleep.store(next, Ordering::Relaxed);
                        tray_state.is_sleeping.store(next, Ordering::Relaxed);
                        let _ = app.emit(
                            "buddy://idle-sleep",
                            win32_sensors::SleepPayload { sleeping: next },
                        );
                    }
                    "quit" => {
                        tray_share.stop(app, &tray_state, "quit");
                        tray_browser.close_browser(app, &tray_state);
                        tray_mcp.disconnect_all();
                        tray_mobile.stop_all(app);
                        let _ = tray_plugins.set_keep_awake(false);
                        pin_window::close_all_pins();
                        tray_db.checkpoint_wal();
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            let win_handle = app.handle().clone();
            let focus_state = sensor_state.clone();
            main_win.on_window_event(move |event| match event {
                WindowEvent::Focused(false) => {
                    // Compact Dynamic Island auto-collapses on blur; Large Window Mode stays open like a normal app.
                    if !focus_state.is_window_mode.load(Ordering::Relaxed)
                        && focus_state.is_expanded.load(Ordering::Relaxed)
                        && !focus_state.is_dragging.load(Ordering::Relaxed)
                        && !focus_state.suspend_auto_collapse.load(Ordering::Acquire)
                    {
                        let elapsed =
                            now_ms().saturating_sub(focus_state.last_expand_ms.load(Ordering::Relaxed));
                        if elapsed > 280 {
                            let _ = win_handle.emit("buddy://collapse", ());
                        }
                    }
                }
                WindowEvent::ScaleFactorChanged { .. } => {
                    if let Some(win) = win_handle.get_webview_window("main") {
                        apply_window_layout(
                            &win,
                            &focus_state,
                            focus_state.is_expanded.load(Ordering::Relaxed),
                        );
                    }
                }
                WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) => {
                    let attachments = classify_dropped_paths(paths);
                    if !attachments.is_empty() {
                        let _ = win_handle.emit("buddy://files-dropped", attachments);
                    }
                }
                _ => {}
            });

            start_sensor_loops(app.handle().clone(), sensor_state.clone());
            watchers::start_clipboard_watcher(
                app.handle().clone(),
                db.clone(),
                copy_guard.clone(),
            );
            watchers::start_screenshot_folder_watcher(app.handle().clone(), db.clone());
            start_snip_hotkey_listener(app.handle().clone(), snip_state.clone());
            start_share_hotkey_listener(app.handle().clone(), sensor_state.clone(), share_mgr.clone());
            start_toggle_hotkey_listener(app.handle().clone(), sensor_state.clone());
            music_sensor::start_music_sensor(app.handle().clone(), sensor_state.clone());
            reminders::start_reminder_loop(app.handle().clone(), db.clone());
            mobile_mgr.attach_event_forwarders(app.handle());
            mobile_mgr.start_if_enabled(app.handle());
            let boot_mcp = mcp_mgr.clone();
            tauri::async_runtime::spawn(async move {
                boot_mcp.connect_all_enabled().await;
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Buddy");
}
