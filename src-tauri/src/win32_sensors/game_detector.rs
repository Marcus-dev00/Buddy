//! Foreground game vs desktop-application classification to prevent Buddy from auto-hiding
//! behind fullscreen browsers, video players, editors, or launchers.

use std::path::Path;

#[cfg(windows)]
use windows::core::PWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, HWND, RECT};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
#[cfg(windows)]
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
#[cfg(windows)]
use windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_RUNNING_D3D_FULL_SCREEN};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect, GetWindowTextW,
    GetWindowThreadProcessId, GWL_STYLE, WS_CAPTION,
};

#[cfg(windows)]
pub(super) fn get_window_class(hwnd: HWND) -> String {
    unsafe {
        let mut class_buf = [0u16; 128];
        let len = GetClassNameW(hwnd, &mut class_buf);
        if len > 0 {
            return String::from_utf16_lossy(&class_buf[..len as usize]);
        }
    }
    String::new()
}

#[cfg(windows)]
fn get_window_title_lower(hwnd: HWND) -> String {
    unsafe {
        let mut buf = [0u16; 128];
        let len = GetWindowTextW(hwnd, &mut buf);
        if len > 0 {
            return String::from_utf16_lossy(&buf[..len as usize]).to_ascii_lowercase();
        }
    }
    String::new()
}

#[cfg(windows)]
fn query_process_image_path_lower(pid: u32) -> Option<String> {
    unsafe {
        let proc_handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut path_buf = [0u16; 512];
        let mut len = path_buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            proc_handle,
            PROCESS_NAME_WIN32,
            PWSTR(path_buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(proc_handle);
        if ok && len > 0 {
            return Some(String::from_utf16_lossy(&path_buf[..len as usize]).to_ascii_lowercase());
        }
    }
    None
}

#[cfg(windows)]
fn is_shell_or_desktop_class(cls: &str) -> bool {
    matches!(
        cls,
        "Progman"
            | "WorkerW"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "NotifyIconOverflowWindow"
            | "TopLevelWindowForOverflowXamlIsland"
            | "Windows.UI.Core.CoreWindow"
            | "MultitaskingViewFrame"
            | "ForegroundStaging"
    )
}

fn is_non_game_window_class(win_class: &str) -> bool {
    matches!(
        win_class,
        "Chrome_WidgetWin_0"
            | "Chrome_WidgetWin_1"
            | "Chrome_RenderWidgetHostHWND"
            | "MozillaWindowClass"
            | "MozillaDialogClass"
            | "ApplicationFrameWindow"
            | "CASCADIA_HOSTING_WINDOW_CLASS"
            | "ConsoleWindowClass"
            | "VirtualConsoleClass"
            | "CabinetWClass"
            | "ExploreWClass"
            | "Electron"
    ) || (win_class.starts_with("Qt") && win_class.ends_with("QWindowIcon"))
        || win_class.starts_with("HwndWrapper[")
}

fn is_known_game_class(cls: &str) -> bool {
    matches!(
        cls,
        "UnityWndClass"
            | "UnrealWindow"
            | "RiotWindowClass"
            | "Valve001"
            | "TslGame"
            | "Respawn001"
            | "CryENGINE"
            | "Frostbite"
            | "LaunchUnrealUWindowsClient"
            | "grcWindow"
            | "sgaWinClass"
            | "LWJGL"
    )
}

fn is_non_game_exe(exe_name: &str) -> bool {
    const NON_GAME_EXES: &[&str] = &[
        "chrome.exe",
        "chrome_proxy.exe",
        "msedge.exe",
        "msedge_proxy.exe",
        "msedgewebview2.exe",
        "firefox.exe",
        "zen.exe",
        "floorp.exe",
        "librewolf.exe",
        "waterfox.exe",
        "thorium.exe",
        "brave.exe",
        "opera.exe",
        "opera_gx.exe",
        "vivaldi.exe",
        "arc.exe",
        "comet.exe",
        "yandex.exe",
        "quark.exe",
        "360se.exe",
        "360chrome.exe",
        "qqbrowser.exe",
        "sogouexplorer.exe",
        "2345explorer.exe",
        "liebao.exe",
        "centbrowser.exe",
        "code.exe",
        "devenv.exe",
        "idea64.exe",
        "clion64.exe",
        "pycharm64.exe",
        "goland64.exe",
        "webstorm64.exe",
        "rider64.exe",
        "cursor.exe",
        "windsurf.exe",
        "antigravity.exe",
        "trae.exe",
        "kiro.exe",
        "zed.exe",
        "sublime_text.exe",
        "subl.exe",
        "explorer.exe",
        "applicationframehost.exe",
        "notepad.exe",
        "notepad++.exe",
        "obsidian.exe",
        "notion.exe",
        "typora.exe",
        "logseq.exe",
        "siyuan.exe",
        "wps.exe",
        "et.exe",
        "wpp.exe",
        "winword.exe",
        "excel.exe",
        "powerpnt.exe",
        "acrord32.exe",
        "acrobat.exe",
        "sumatrapdf.exe",
        "foxitpdfreader.exe",
        "foxitreader.exe",
        "vlc.exe",
        "potplayer.exe",
        "potplayermini64.exe",
        "potplayermini.exe",
        "mpv.exe",
        "mpv.net.exe",
        "iina.exe",
        "mpc-hc64.exe",
        "mpc-be64.exe",
        "kodi.exe",
        "plex.exe",
        "plexhtpc.exe",
        "jellyfinmediaplayer.exe",
        "bilibili.exe",
        "douyin.exe",
        "qqlive.exe",
        "tencentvideo.exe",
        "iqiyi.exe",
        "qiyi.exe",
        "youku.exe",
        "qq.exe",
        "wechat.exe",
        "weixin.exe",
        "tim.exe",
        "feishu.exe",
        "lark.exe",
        "dingtalk.exe",
        "wemeetapp.exe",
        "zoom.exe",
        "teams.exe",
        "ms-teams.exe",
        "discord.exe",
        "spotify.exe",
        "cloudmusic.exe",
        "qqmusic.exe",
        "kugou.exe",
        "kwmusic.exe",
        "windowsterminal.exe",
        "powershell.exe",
        "pwsh.exe",
        "cmd.exe",
        "conhost.exe",
        "taskmgr.exe",
        "buddy.exe",
        "mstsc.exe",
        "parsec.exe",
        "moonlight.exe",
        "sunshine.exe",
        "todesk.exe",
        "sunloginclient.exe",
        "anydesk.exe",
        "teamviewer.exe",
        "rustdesk.exe",
        "snipaste.exe",
        "pixpin.exe",
        "sharex.exe",
        // Launchers, live wallpapers, overlays, and streaming tools (not games themselves)
        "steam.exe",
        "steamwebhelper.exe",
        "epicgameslauncher.exe",
        "epicwebhelper.exe",
        "riotclientservices.exe",
        "riotclientux.exe",
        "wegame.exe",
        "hoyoplay.exe",
        "launcher.exe",
        "battlenet.exe",
        "uu.exe",
        "leigod.exe",
        "wallpaper32.exe",
        "wallpaper64.exe",
        "wallpaperui.exe",
        "ui32.exe",
        "webwallpaper32.exe",
        "losslessscaling.exe",
        "obs64.exe",
        "obs32.exe",
        "ciscocollabhost.exe",
        "gamebar.exe",
        "nvidia overlay.exe",
        "nvidiashare.exe",
    ];
    NON_GAME_EXES.contains(&exe_name)
}

fn is_known_game_signature(exe_path_lower: &str, exe_name: &str, cls: &str, title_lower: &str) -> bool {
    if is_known_game_class(cls) {
        return true;
    }

    if exe_name.starts_with("unins")
        || exe_name.contains("crashhandler")
        || exe_name.contains("crashpad")
        || exe_name.contains("crashreport")
    {
        return false;
    }

    const PATH_FRAGMENTS: &[&str] = &[
        "\\steamapps\\common\\",
        "\\steamlibrary\\",
        "\\epic games\\",
        "\\riot games\\",
        "\\wegameapps\\",
        "\\xboxgames\\",
        "\\ubisoft game launcher\\games\\",
        "\\ea games\\",
        "\\origin games\\",
        "\\battle.net\\",
        "\\hoyoverse\\",
        "\\hoyoplay\\games\\",
        "\\mihoyo\\",
        "\\genshin impact\\",
        "\\kuro games\\",
        "\\rockstar games\\",
        "\\gog galaxy\\games\\",
    ];
    if PATH_FRAGMENTS.iter().any(|f| exe_path_lower.contains(f)) {
        return true;
    }

    if exe_name.ends_with("-win64-shipping.exe") || exe_name.ends_with("-win32-shipping.exe") {
        return true;
    }
    if (exe_name == "javaw.exe" || exe_name == "java.exe") && title_lower.contains("minecraft") {
        return true;
    }

    const GAME_EXES: &[&str] = &[
        "valorant-win64-shipping.exe",
        "league of legends.exe",
        "yuanshen.exe",
        "genshinimpact.exe",
        "starrail.exe",
        "zenlesszonezero.exe",
        "bh3.exe",
        "tslgame.exe",
        "cs2.exe",
        "csgo.exe",
        "dota2.exe",
        "overwatch.exe",
        "r5apex.exe",
        "eldenring.exe",
        "sekiro.exe",
        "cyberpunk2077.exe",
        "gtav.exe",
        "gta5.exe",
        "rdr2.exe",
        "helldivers2.exe",
        "fc24.exe",
        "fc25.exe",
        "minecraft.windows.exe",
        "robloxplayerbeta.exe",
        "deltaforceclient-win64-shipping.exe",
        "narakabladepoint.exe",
        "naraka.exe",
        "b1-win64-shipping.exe",
        "client-win64-shipping.exe",
        "palworld-win64-shipping.exe",
        "marvel-win64-shipping.exe",
        "wow.exe",
        "wowclassic.exe",
        "ffxiv_dx11.exe",
        "diablo iv.exe",
        "hearthstone.exe",
        "starcraft2.exe",
        "starcraft2_x64.exe",
        "crossfire.exe",
        "dnf.exe",
        "pathofexile.exe",
        "pathofexile_x64.exe",
        "pathofexile2.exe",
        "bg3.exe",
        "bg3_dx11.exe",
        "witcher3.exe",
        "mhworld.exe",
        "monsterhunterwilds.exe",
    ];
    GAME_EXES.contains(&exe_name)
}

#[cfg(not(windows))]
fn is_shell_or_desktop_class(_cls: &str) -> bool {
    false
}

/// Distinguishes real games (known engines/paths/exes, anti-cheat protected game classes, or
/// genuine D3D exclusive fullscreen non-app windows) from normal fullscreen browsers, editors, or players.
pub(super) fn classify_foreground_game(
    exe_path_opt: Option<&str>,
    win_class: &str,
    title_lower: &str,
    covers_screen: bool,
    is_d3d_exclusive: bool,
    has_caption: bool,
    large_window: bool,
) -> bool {
    if is_shell_or_desktop_class(win_class) || is_non_game_window_class(win_class) {
        return false;
    }

    let Some(raw_path) = exe_path_opt else {
        if is_known_game_class(win_class) {
            return large_window || covers_screen || is_d3d_exclusive;
        }
        return is_d3d_exclusive && covers_screen && !has_caption;
    };

    let path_lower = raw_path.to_ascii_lowercase();
    let exe_name = Path::new(&path_lower)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    if is_non_game_exe(exe_name) {
        return false;
    }

    if is_known_game_signature(&path_lower, exe_name, win_class, title_lower) {
        return large_window || covers_screen || is_d3d_exclusive;
    }

    is_d3d_exclusive && covers_screen && !has_caption
}

#[cfg(windows)]
pub(super) fn detect_active_game(own_pid: u32) -> Option<String> {
    unsafe {
        let d3d_exclusive = SHQueryUserNotificationState()
            .map(|s| s == QUNS_RUNNING_D3D_FULL_SCREEN)
            .unwrap_or(false);

        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return None;
        }
        let mut fg_pid: u32 = 0;
        GetWindowThreadProcessId(fg, Some(&mut fg_pid));
        if fg_pid == own_pid || fg_pid == 0 {
            return None;
        }

        let cls = get_window_class(fg);
        if is_shell_or_desktop_class(&cls) || is_non_game_window_class(&cls) {
            return None;
        }

        let mut fg_rect = RECT::default();
        if GetWindowRect(fg, &mut fg_rect).is_err() {
            return None;
        }
        let w = fg_rect.right - fg_rect.left;
        let h = fg_rect.bottom - fg_rect.top;
        let large_window = w >= 640 && h >= 480;

        let mut covers_screen = false;
        let hmon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmon, &mut mi).as_bool() {
            let m = mi.rcMonitor;
            covers_screen = fg_rect.left <= m.left + 3
                && fg_rect.top <= m.top + 3
                && fg_rect.right >= m.right - 3
                && fg_rect.bottom >= m.bottom - 3;
        }

        let style = GetWindowLongPtrW(fg, GWL_STYLE) as u32;
        let has_caption = (style & WS_CAPTION.0) == WS_CAPTION.0;
        let exe_path = query_process_image_path_lower(fg_pid);
        let title_lower = get_window_title_lower(fg);

        if classify_foreground_game(
            exe_path.as_deref(),
            &cls,
            &title_lower,
            covers_screen,
            d3d_exclusive,
            has_caption,
            large_window,
        ) {
            let exe_label = exe_path
                .as_deref()
                .and_then(|p| Path::new(p).file_name().and_then(|n| n.to_str()))
                .unwrap_or("<denied>");
            return Some(format!(
                "exe={}, class={}, covers={}, d3d={}, caption={}",
                exe_label, cls, covers_screen, d3d_exclusive, has_caption
            ));
        }
        None
    }
}

#[cfg(not(windows))]
pub(super) fn detect_active_game(_own_pid: u32) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_games_without_hiding_fullscreen_browsers_or_editors() {
        assert!(!classify_foreground_game(
            Some(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
            "Chrome_WidgetWin_1",
            "youtube - google chrome",
            true,
            false,
            false,
            true
        ));
        assert!(!classify_foreground_game(
            Some(r"C:\Users\Marcus\AppData\Local\Programs\Microsoft VS Code\Code.exe"),
            "Chrome_WidgetWin_1",
            "app.tsx",
            true,
            true,
            false,
            true
        ));
        assert!(!classify_foreground_game(
            Some(r"C:\Program Files\VideoLAN\VLC\vlc.exe"),
            "Qt5QWindowIcon",
            "movie",
            true,
            false,
            false,
            true
        ));
        assert!(!classify_foreground_game(
            Some(r"C:\Program Files (x86)\Steam\steam.exe"),
            "SDL_app",
            "steam",
            true,
            true,
            false,
            true
        ));
        assert!(!classify_foreground_game(
            Some(r"C:\Program Files (x86)\Steam\steamapps\common\wallpaper_engine\wallpaper64.exe"),
            "WallpaperClass",
            "wallpaper engine",
            true,
            true,
            false,
            true
        ));
        assert!(!classify_foreground_game(
            Some(r"C:\Program Files\obs-studio\bin\64bit\obs64.exe"),
            "Qt662QWindowIcon",
            "obs studio",
            true,
            false,
            false,
            true
        ));
        assert!(!classify_foreground_game(
            Some(r"C:\Users\Marcus\AppData\Local\Discord\app-1.0\Discord.exe"),
            "Chrome_WidgetWin_1",
            "discord",
            true,
            false,
            false,
            true
        ));
        assert!(!classify_foreground_game(
            Some(r"C:\Program Files\SomeUnlistedApp\SomeViewer.exe"),
            "SomeViewerWindowClass",
            "full screen presentation",
            true,
            false,
            false,
            true
        ));
        assert!(!classify_foreground_game(
            Some(r"C:\Program Files\SomeUnlistedApp\SomeViewer.exe"),
            "SomeViewerWindowClass",
            "small window with lingering d3d state",
            false,
            true,
            true,
            true
        ));
        assert!(!classify_foreground_game(
            None,
            "CASCADIA_HOSTING_WINDOW_CLASS",
            "administrator: powershell",
            true,
            false,
            false,
            true
        ));
        assert!(!classify_foreground_game(
            None,
            "CustomElevatedApp",
            "fullscreen admin app",
            true,
            false,
            false,
            true
        ));
        assert!(!classify_foreground_game(
            Some(r"D:\Games\Genshin Impact\launcher.exe"),
            "Qt5152QWindowIcon",
            "genshin launcher",
            false,
            false,
            false,
            true
        ));
        assert!(classify_foreground_game(
            Some(r"D:\SteamLibrary\steamapps\common\EldenRing\Game\eldenring.exe"),
            "ELDEN RING™",
            "elden ring",
            false,
            false,
            true,
            true
        ));
        assert!(classify_foreground_game(
            Some(r"C:\Games\Custom\Game-Win64-Shipping.exe"),
            "UnrealWindow",
            "game",
            true,
            true,
            false,
            true
        ));
        assert!(classify_foreground_game(
            None,
            "RiotWindowClass",
            "valorant",
            true,
            false,
            false,
            true
        ));
        assert!(classify_foreground_game(
            None,
            "CustomProtectedGameWnd",
            "protected d3d game",
            true,
            true,
            false,
            true
        ));
    }
}
