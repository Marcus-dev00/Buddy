//! Native Windows IFileOpenDialog helpers for file & folder picking.
//!
//! Runs on a dedicated COM STA thread with Buddy's HWND as owner so the dialog
//! appears above Buddy's topmost window while suppressing auto-collapse on blur.

use crate::win32_sensors::AppSensorState;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

struct CollapseSuspendGuard {
    state: Arc<AppSensorState>,
}

impl CollapseSuspendGuard {
    fn new(state: Arc<AppSensorState>) -> Self {
        state.suspend_auto_collapse.store(true, Ordering::Release);
        Self { state }
    }
}

impl Drop for CollapseSuspendGuard {
    fn drop(&mut self) {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        self.state.last_expand_ms.store(now_ms, Ordering::Relaxed);
        self.state
            .suspend_auto_collapse
            .store(false, Ordering::Release);
    }
}

pub async fn pick_folder(
    app: &AppHandle,
    sensor_state: &Arc<AppSensorState>,
    title: Option<String>,
    initial_dir: Option<String>,
) -> Result<Option<String>, String> {
    let items = run_open_dialog(
        app.clone(),
        sensor_state.clone(),
        OpenDialogConfig {
            folder: true,
            multiple: false,
            title,
            initial_dir,
            filter_name: None,
            filter_spec: None,
        },
    )
    .await?;
    Ok(items.into_iter().next())
}

pub async fn pick_files(
    app: &AppHandle,
    sensor_state: &Arc<AppSensorState>,
    title: Option<String>,
    multiple: bool,
    filter_name: Option<String>,
    filter_spec: Option<String>,
    initial_dir: Option<String>,
) -> Result<Vec<String>, String> {
    run_open_dialog(
        app.clone(),
        sensor_state.clone(),
        OpenDialogConfig {
            folder: false,
            multiple,
            title,
            initial_dir,
            filter_name,
            filter_spec,
        },
    )
    .await
}

struct OpenDialogConfig {
    folder: bool,
    multiple: bool,
    title: Option<String>,
    initial_dir: Option<String>,
    filter_name: Option<String>,
    filter_spec: Option<String>,
}

async fn run_open_dialog(
    app: AppHandle,
    sensor_state: Arc<AppSensorState>,
    cfg: OpenDialogConfig,
) -> Result<Vec<String>, String> {
    #[cfg(windows)]
    {
        let owner_raw: isize = app
            .get_webview_window("main")
            .and_then(|w| w.hwnd().ok())
            .map(|h| h.0 as isize)
            .unwrap_or(0);

        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let _guard = CollapseSuspendGuard::new(sensor_state);
            let res = show_com_open_dialog(owner_raw, cfg);
            let _ = tx.send(res);
        });
        rx.await
            .map_err(|_| "File dialog worker ended unexpectedly".to_string())?
    }
    #[cfg(not(windows))]
    {
        let _ = (app, sensor_state, cfg);
        Err("Native file picker is currently implemented for Windows only".to_string())
    }
}

#[cfg(windows)]
fn to_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn show_com_open_dialog(owner_raw: isize, cfg: OpenDialogConfig) -> Result<Vec<String>, String> {
    use std::ffi::c_void;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName,
        FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR,
        FOS_PATHMUSTEXIST, FOS_PICKFOLDERS,
    };

    struct ComInitGuard(bool);
    impl Drop for ComInitGuard {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }

    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let _com_guard = ComInitGuard(hr.is_ok());

        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL)
            .map_err(|e| format!("Failed to open Windows dialog: {e}"))?;

        let mut opts = dialog
            .GetOptions()
            .map_err(|e| format!("GetOptions failed: {e}"))?;
        opts |= FOS_FORCEFILESYSTEM | FOS_NOCHANGEDIR | FOS_PATHMUSTEXIST;
        if cfg.folder {
            opts |= FOS_PICKFOLDERS;
        } else {
            opts |= FOS_FILEMUSTEXIST;
            if cfg.multiple {
                opts |= FOS_ALLOWMULTISELECT;
            }
        }
        dialog
            .SetOptions(opts)
            .map_err(|e| format!("SetOptions failed: {e}"))?;

        if let Some(title) = cfg.title.as_deref().filter(|s| !s.trim().is_empty()) {
            let wide_title = to_wide_null(title);
            let _ = dialog.SetTitle(PCWSTR(wide_title.as_ptr()));
        }

        if let Some(dir) = cfg
            .initial_dir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && Path::new(s).exists())
        {
            let wide_dir = to_wide_null(dir);
            if let Ok(folder_item) =
                SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(wide_dir.as_ptr()), None)
            {
                let _ = dialog.SetFolder(&folder_item);
            }
        }

        // Keep wide filter buffers alive through Show()
        let _filter_buffers: Option<(Vec<u16>, Vec<u16>)> = if !cfg.folder {
            if let Some(spec) = cfg.filter_spec.as_deref().filter(|s| !s.trim().is_empty()) {
                let label = cfg
                    .filter_name
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("Supported Files");
                let wide_name = to_wide_null(label);
                let wide_spec = to_wide_null(spec);
                let filters = [COMDLG_FILTERSPEC {
                    pszName: PCWSTR(wide_name.as_ptr()),
                    pszSpec: PCWSTR(wide_spec.as_ptr()),
                }];
                let _ = dialog.SetFileTypes(&filters);
                Some((wide_name, wide_spec))
            } else {
                None
            }
        } else {
            None
        };

        let owner = HWND(owner_raw as *mut c_void);

        if let Err(err) = dialog.Show(owner) {
            if err.code() == ERROR_CANCELLED.to_hresult() {
                return Ok(Vec::new());
            }
            return Err(format!("Dialog aborted: {err}"));
        }

        let mut results = Vec::new();
        if cfg.multiple && !cfg.folder {
            let arr = dialog
                .GetResults()
                .map_err(|e| format!("GetResults failed: {e}"))?;
            let count = arr
                .GetCount()
                .map_err(|e| format!("GetCount failed: {e}"))?;
            for i in 0..count {
                if let Ok(item) = arr.GetItemAt(i) {
                    if let Some(path) = extract_shell_item_path(&item) {
                        results.push(path);
                    }
                }
            }
        } else if let Ok(item) = dialog.GetResult() {
            if let Some(path) = extract_shell_item_path(&item) {
                results.push(path);
            }
        }
        Ok(results)
    }
}

#[cfg(windows)]
unsafe fn extract_shell_item_path(item: &windows::Win32::UI::Shell::IShellItem) -> Option<String> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::SIGDN_FILESYSPATH;
    let pwstr = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
    let path = pwstr.to_string().ok();
    CoTaskMemFree(Some(pwstr.0 as *const std::ffi::c_void));
    path.filter(|p| !p.trim().is_empty())
}
