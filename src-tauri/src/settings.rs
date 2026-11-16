//! Application configuration persistence (%LOCALAPPDATA%/Buddy/config.json),
//! Windows DPAPI encrypted API Key storage (secret.dpapi), and Windows Startup registry toggle.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::LocalFree;
#[cfg(windows)]
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
};
#[cfg(windows)]
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_WRITE, REG_SZ,
};

fn default_provider() -> String {
    "openai".to_string()
}

fn default_true() -> bool {
    true
}

fn default_ui_lang() -> String {
    "en".to_string()
}

fn default_thinking_level() -> String {
    "medium".to_string()
}

fn default_dock_edge() -> String {
    "top".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default = "default_provider")]
    pub api_provider: String, // "openai" | "anthropic" | "gemini"
    pub base_url: String,
    pub model_id: String,
    pub persona_id: String,
    pub auto_hide_stealth: bool,
    pub music_reactive: bool,
    pub launch_at_startup: bool,
    #[serde(default)]
    pub has_api_key: bool,
    #[serde(default = "default_true")]
    pub agent_mode: bool,
    #[serde(default = "default_thinking_level")]
    pub thinking_level: String, // "off" | "low" | "medium" | "high" | "max"
    #[serde(default)]
    pub agent_workspace: String,
    #[serde(default = "default_ui_lang")]
    pub ui_language: String, // "en" | "zh"
    #[serde(default = "default_dock_edge")]
    pub dock_edge: String, // "top" | "bottom" | "left" | "right" | "free"
    #[serde(default)]
    pub dock_cx: Option<i32>,
    #[serde(default)]
    pub dock_cy: Option<i32>,
    #[serde(default)]
    pub update_endpoint: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            api_provider: default_provider(),
            base_url: "https://api.xiaomimimo.com/v1".to_string(),
            model_id: "mimo-v2-flash".to_string(),
            persona_id: "warm_companion".to_string(),
            auto_hide_stealth: false,
            music_reactive: true,
            launch_at_startup: false,
            has_api_key: false,
            agent_mode: true,
            thinking_level: default_thinking_level(),
            agent_workspace: String::new(),
            ui_language: default_ui_lang(),
            dock_edge: default_dock_edge(),
            dock_cx: None,
            dock_cy: None,
            update_endpoint: String::new(),
        }
    }
}

pub struct SettingsManager {
    config_path: PathBuf,
    secret_path: PathBuf,
    inner: Mutex<AppSettings>,
}

impl SettingsManager {
    pub fn load(data_dir: &Path) -> Self {
        let config_path = data_dir.join("config.json");
        let secret_path = data_dir.join("secret.dpapi");

        let mut settings: AppSettings = std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();

        settings.has_api_key = secret_path.exists()
            && std::fs::read_to_string(&secret_path)
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);

        Self {
            config_path,
            secret_path,
            inner: Mutex::new(settings),
        }
    }

    pub fn get(&self) -> AppSettings {
        self.inner.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn update(&self, mut next: AppSettings, new_api_key: Option<String>) -> Result<AppSettings, String> {
        if let Some(key) = new_api_key {
            let trimmed = key.trim();
            if trimmed.is_empty() {
                let _ = std::fs::remove_file(&self.secret_path);
                next.has_api_key = false;
            } else {
                self.save_encrypted_key(trimmed)?;
                next.has_api_key = true;
            }
        } else {
            next.has_api_key = self.secret_path.exists();
        }

        apply_windows_autostart(next.launch_at_startup);

        let json = serde_json::to_string_pretty(&next).map_err(|e| e.to_string())?;
        std::fs::write(&self.config_path, json).map_err(|e| e.to_string())?;

        if let Ok(mut g) = self.inner.lock() {
            *g = next.clone();
        }
        Ok(next)
    }

    pub fn save_dock_position(&self, edge: &str, cx: i32, cy: i32) {
        if let Ok(mut g) = self.inner.lock() {
            g.dock_edge = edge.to_string();
            g.dock_cx = Some(cx);
            g.dock_cy = Some(cy);
            if let Ok(json) = serde_json::to_string_pretty(&*g) {
                let _ = std::fs::write(&self.config_path, json);
            }
        }
    }

    #[cfg(windows)]
    fn save_encrypted_key(&self, plaintext: &str) -> Result<(), String> {
        let mut bytes = plaintext.as_bytes().to_vec();
        unsafe {
            let input = CRYPT_INTEGER_BLOB {
                cbData: bytes.len() as u32,
                pbData: bytes.as_mut_ptr(),
            };
            let mut output = CRYPT_INTEGER_BLOB::default();
            CryptProtectData(
                &input,
                PCWSTR::null(),
                None,
                None,
                None,
                0,
                &mut output,
            )
            .map_err(|e| format!("DPAPI encrypt failed: {e}"))?;

            let cipher = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
            let _ = LocalFree(windows::Win32::Foundation::HLOCAL(output.pbData as *mut _));
            let encoded = B64.encode(&cipher);
            std::fs::write(&self.secret_path, encoded).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    #[cfg(not(windows))]
    fn save_encrypted_key(&self, plaintext: &str) -> Result<(), String> {
        std::fs::write(&self.secret_path, B64.encode(plaintext.as_bytes())).map_err(|e| e.to_string())
    }

    #[cfg(windows)]
    pub fn load_api_key(&self) -> Option<String> {
        let encoded = std::fs::read_to_string(&self.secret_path).ok()?;
        let mut cipher = B64.decode(encoded.trim()).ok()?;
        unsafe {
            let input = CRYPT_INTEGER_BLOB {
                cbData: cipher.len() as u32,
                pbData: cipher.as_mut_ptr(),
            };
            let mut output = CRYPT_INTEGER_BLOB::default();
            if CryptUnprotectData(&input, None, None, None, None, 0, &mut output).is_err() {
                return None;
            }
            let plain = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
            let _ = LocalFree(windows::Win32::Foundation::HLOCAL(output.pbData as *mut _));
            String::from_utf8(plain).ok()
        }
    }

    #[cfg(not(windows))]
    pub fn load_api_key(&self) -> Option<String> {
        let encoded = std::fs::read_to_string(&self.secret_path).ok()?;
        let bytes = B64.decode(encoded.trim()).ok()?;
        String::from_utf8(bytes).ok()
    }
}

#[cfg(windows)]
fn apply_windows_autostart(enable: bool) {
    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Run\0"
        .encode_utf16()
        .collect();
    let val_name: Vec<u16> = "Buddy\0".encode_utf16().collect();

    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            KEY_WRITE,
            &mut hkey,
        )
        .is_err()
        {
            return;
        }

        if enable {
            if let Ok(exe_path) = std::env::current_exe() {
                let cmd = format!("\"{}\"", exe_path.to_string_lossy());
                let wide: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
                let byte_slice =
                    std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
                let _ = RegSetValueExW(hkey, PCWSTR(val_name.as_ptr()), 0, REG_SZ, Some(byte_slice));
            }
        } else {
            let _ = RegDeleteValueW(hkey, PCWSTR(val_name.as_ptr()));
        }

        let _ = RegCloseKey(hkey);
    }
}

#[cfg(not(windows))]
fn apply_windows_autostart(_enable: bool) {}
