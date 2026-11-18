//! Application configuration persistence (%LOCALAPPDATA%/Buddy/config.json),
//! multi-model DPAPI encrypted API Key storage (keys/*.dpapi & secret.dpapi),
//! and fast Windows Login Task Scheduler autostart with HKCU\Run fallback.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[cfg(windows)]
use std::os::windows::process::CommandExt;
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

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;
const AUTOSTART_TASK_NAME: &str = "BuddyAutostart";

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

fn default_chat_profile_id() -> String {
    "prof_default".to_string()
}

fn default_thinking_mode() -> String {
    "auto".to_string()
}

fn default_permission_mode() -> String {
    "default".to_string()
}

pub fn normalize_permission_mode(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "ask" => "ask".to_string(),
        "auto" => "auto".to_string(),
        _ => "default".to_string(),
    }
}

fn normalize_thinking_mode(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "native" => "native".to_string(),
        "prompt" => "prompt".to_string(),
        _ => "auto".to_string(),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelProfile {
    pub id: String,
    pub name: String,
    #[serde(default = "default_provider")]
    pub api_provider: String, // "openai" | "anthropic" | "gemini"
    pub base_url: String,
    pub model_id: String,
    #[serde(default = "default_true")]
    pub supports_vision: bool,
    #[serde(default = "default_thinking_mode")]
    pub thinking_mode: String, // "auto" | "native" | "prompt"
    #[serde(default)]
    pub has_api_key: bool,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ResolvedModelConfig {
    pub profile_id: String,
    pub name: String,
    pub api_provider: String,
    pub base_url: String,
    pub model_id: String,
    pub supports_vision: bool,
    pub thinking_mode: String,
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
    #[serde(default)]
    pub model_profiles: Vec<ModelProfile>,
    #[serde(default = "default_chat_profile_id")]
    pub chat_profile_id: String,
    #[serde(default)]
    pub watch_profile_id: String, // "" => follow chat_profile_id
    #[serde(default)]
    pub auto_collapse_on_blur: bool,
    #[serde(default = "default_permission_mode")]
    pub agent_permission_mode: String, // "ask" | "default" | "auto"
}

impl Default for AppSettings {
    fn default() -> Self {
        let def_profile = ModelProfile {
            id: default_chat_profile_id(),
            name: "Xiaomi MiMo 2.6 Flash".to_string(),
            api_provider: default_provider(),
            base_url: "https://api.xiaomimimo.com/v1".to_string(),
            model_id: "mimo-v2-flash".to_string(),
            supports_vision: true,
            thinking_mode: default_thinking_mode(),
            has_api_key: false,
        };
        Self {
            api_provider: def_profile.api_provider.clone(),
            base_url: def_profile.base_url.clone(),
            model_id: def_profile.model_id.clone(),
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
            model_profiles: vec![def_profile],
            chat_profile_id: default_chat_profile_id(),
            watch_profile_id: String::new(),
            auto_collapse_on_blur: false,
            agent_permission_mode: default_permission_mode(),
        }
    }
}

pub struct SettingsManager {
    config_path: PathBuf,
    secret_path: PathBuf,
    keys_dir: PathBuf,
    inner: Mutex<AppSettings>,
}

fn sanitize_profile_id(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        default_chat_profile_id()
    } else {
        cleaned
    }
}

fn derive_profile_name(provider: &str, model_id: &str) -> String {
    let m = model_id.trim();
    if !m.is_empty() {
        return m.to_string();
    }
    match provider {
        "anthropic" => "Anthropic Model".to_string(),
        "gemini" => "Gemini Model".to_string(),
        _ => "OpenAI Compatible".to_string(),
    }
}

impl SettingsManager {
    pub fn load(data_dir: &Path) -> Self {
        let config_path = data_dir.join("config.json");
        let secret_path = data_dir.join("secret.dpapi");
        let keys_dir = data_dir.join("keys");
        let _ = std::fs::create_dir_all(&keys_dir);

        let mut settings: AppSettings = std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();

        let legacy_key_exists = secret_path.exists()
            && std::fs::read_to_string(&secret_path)
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);

        // Migrate legacy single-model configuration cleanly into model_profiles[0]
        if settings.model_profiles.is_empty() {
            let def_id = default_chat_profile_id();
            let def_key_path = keys_dir.join(format!("{def_id}.dpapi"));
            if legacy_key_exists && !def_key_path.exists() {
                let _ = std::fs::copy(&secret_path, &def_key_path);
            }
            settings.model_profiles.push(ModelProfile {
                id: def_id.clone(),
                name: derive_profile_name(&settings.api_provider, &settings.model_id),
                api_provider: settings.api_provider.clone(),
                base_url: settings.base_url.clone(),
                model_id: settings.model_id.clone(),
                supports_vision: true,
                thinking_mode: default_thinking_mode(),
                has_api_key: legacy_key_exists,
            });
            settings.chat_profile_id = def_id;
        }

        let mgr = Self {
            config_path,
            secret_path,
            keys_dir,
            inner: Mutex::new(settings),
        };
        mgr.hydrate_and_sync_unlocked();
        mgr
    }

    fn key_path_for(&self, profile_id: &str) -> PathBuf {
        self.keys_dir
            .join(format!("{}.dpapi", sanitize_profile_id(profile_id)))
    }

    fn hydrate_and_sync_unlocked(&self) {
        if let Ok(mut g) = self.inner.lock() {
            Self::normalize_settings_with_keys(&self.keys_dir, &self.secret_path, &mut g);
        }
    }

    fn normalize_settings_with_keys(
        keys_dir: &Path,
        secret_path: &Path,
        settings: &mut AppSettings,
    ) {
        for prof in &mut settings.model_profiles {
            prof.id = sanitize_profile_id(&prof.id);
            prof.thinking_mode = normalize_thinking_mode(&prof.thinking_mode);
            let kp = keys_dir.join(format!("{}.dpapi", prof.id));
            let has_kp = kp.exists()
                && std::fs::read_to_string(&kp)
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false);
            let has_legacy = prof.id == "prof_default"
                && secret_path.exists()
                && std::fs::read_to_string(secret_path)
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false);
            if has_legacy && !has_kp {
                let _ = std::fs::copy(secret_path, &kp);
            }
            prof.has_api_key = has_kp || has_legacy;
        }

        if settings.model_profiles.is_empty() {
            let def = AppSettings::default();
            settings.model_profiles = def.model_profiles;
        }

        if !settings
            .model_profiles
            .iter()
            .any(|p| p.id == settings.chat_profile_id)
        {
            settings.chat_profile_id = settings.model_profiles[0].id.clone();
        }

        if !settings.watch_profile_id.is_empty()
            && !settings
                .model_profiles
                .iter()
                .any(|p| p.id == settings.watch_profile_id)
        {
            settings.watch_profile_id.clear();
        }

        if let Some(active) = settings
            .model_profiles
            .iter()
            .find(|p| p.id == settings.chat_profile_id)
            .cloned()
        {
            settings.api_provider = active.api_provider;
            settings.base_url = active.base_url;
            settings.model_id = active.model_id;
            settings.has_api_key = active.has_api_key;
        }
        settings.agent_permission_mode = normalize_permission_mode(&settings.agent_permission_mode);
    }

    fn persist_unlocked(&self, settings: &AppSettings) -> Result<(), String> {
        let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
        std::fs::write(&self.config_path, json).map_err(|e| e.to_string())
    }

    pub fn get(&self) -> AppSettings {
        self.inner.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn set_auto_collapse_on_blur(&self, enabled: bool) -> Result<(), String> {
        let mut guard = self.inner.lock().map_err(|_| "settings lock poisoned".to_string())?;
        guard.auto_collapse_on_blur = enabled;
        self.persist_unlocked(&guard)
    }

    pub fn set_agent_permission_mode(&self, mode: &str) -> Result<String, String> {
        let normalized = normalize_permission_mode(mode);
        let mut guard = self.inner.lock().map_err(|_| "settings lock poisoned".to_string())?;
        guard.agent_permission_mode = normalized.clone();
        self.persist_unlocked(&guard)?;
        Ok(normalized)
    }

    /// Re-verifies / repairs Windows fast logon autostart silently on app launch.
    pub fn sync_autostart_on_boot(&self) {
        let enabled = self.get().launch_at_startup;
        if enabled {
            let _ = apply_windows_autostart(true);
        }
    }

    pub fn update(
        &self,
        mut next: AppSettings,
        new_api_key: Option<String>,
    ) -> Result<AppSettings, String> {
        let prev_startup = self.get().launch_at_startup;

        if next.model_profiles.is_empty() {
            let prev = self.get();
            next.model_profiles = prev.model_profiles;
        }

        let target_prof_id = if next.chat_profile_id.trim().is_empty() {
            default_chat_profile_id()
        } else {
            sanitize_profile_id(&next.chat_profile_id)
        };
        next.chat_profile_id = target_prof_id.clone();

        // Keep active chat profile synchronized when callers update legacy top-level fields
        if let Some(pos) = next
            .model_profiles
            .iter()
            .position(|p| p.id == target_prof_id)
        {
            next.model_profiles[pos].api_provider = next.api_provider.clone();
            next.model_profiles[pos].base_url = next.base_url.clone();
            next.model_profiles[pos].model_id = next.model_id.clone();
            if next.model_profiles[pos].name.trim().is_empty() {
                next.model_profiles[pos].name =
                    derive_profile_name(&next.api_provider, &next.model_id);
            }
        } else {
            next.model_profiles.push(ModelProfile {
                id: target_prof_id.clone(),
                name: derive_profile_name(&next.api_provider, &next.model_id),
                api_provider: next.api_provider.clone(),
                base_url: next.base_url.clone(),
                model_id: next.model_id.clone(),
                supports_vision: true,
                thinking_mode: default_thinking_mode(),
                has_api_key: false,
            });
        }

        if let Some(key) = new_api_key {
            let trimmed = key.trim();
            let kp = self.key_path_for(&target_prof_id);
            if trimmed.is_empty() {
                let _ = std::fs::remove_file(&kp);
                if target_prof_id == "prof_default" {
                    let _ = std::fs::remove_file(&self.secret_path);
                }
            } else {
                self.save_encrypted_key_to(&kp, trimmed)?;
                if target_prof_id == "prof_default" {
                    let _ = self.save_encrypted_key_to(&self.secret_path, trimmed);
                }
            }
        }

        Self::normalize_settings_with_keys(&self.keys_dir, &self.secret_path, &mut next);

        if next.launch_at_startup != prev_startup {
            apply_windows_autostart(next.launch_at_startup)?;
        }

        self.persist_unlocked(&next)?;

        if let Ok(mut g) = self.inner.lock() {
            *g = next.clone();
        }
        Ok(next)
    }

    pub fn save_profile(
        &self,
        mut profile: ModelProfile,
        new_api_key: Option<String>,
        set_as_chat: bool,
        set_as_watch: bool,
    ) -> Result<AppSettings, String> {
        profile.id = sanitize_profile_id(&profile.id);
        profile.thinking_mode = normalize_thinking_mode(&profile.thinking_mode);
        if profile.name.trim().is_empty() {
            profile.name = derive_profile_name(&profile.api_provider, &profile.model_id);
        }

        if let Some(key) = new_api_key {
            let trimmed = key.trim();
            let kp = self.key_path_for(&profile.id);
            if trimmed.is_empty() {
                let _ = std::fs::remove_file(&kp);
                if profile.id == "prof_default" {
                    let _ = std::fs::remove_file(&self.secret_path);
                }
            } else {
                self.save_encrypted_key_to(&kp, trimmed)?;
                if profile.id == "prof_default" {
                    let _ = self.save_encrypted_key_to(&self.secret_path, trimmed);
                }
            }
        }

        let mut next = self.get();
        if let Some(pos) = next.model_profiles.iter().position(|p| p.id == profile.id) {
            next.model_profiles[pos] = profile.clone();
        } else {
            next.model_profiles.push(profile.clone());
        }

        if set_as_chat {
            next.chat_profile_id = profile.id.clone();
        }
        if set_as_watch {
            next.watch_profile_id = profile.id.clone();
        }

        Self::normalize_settings_with_keys(&self.keys_dir, &self.secret_path, &mut next);
        self.persist_unlocked(&next)?;

        if let Ok(mut g) = self.inner.lock() {
            *g = next.clone();
        }
        Ok(next)
    }

    pub fn delete_profile(&self, profile_id: &str) -> Result<AppSettings, String> {
        let pid = sanitize_profile_id(profile_id);
        let mut next = self.get();
        if next.model_profiles.len() <= 1 {
            return Err("At least one model profile must be kept.".to_string());
        }
        next.model_profiles.retain(|p| p.id != pid);

        let kp = self.key_path_for(&pid);
        let _ = std::fs::remove_file(&kp);
        if pid == "prof_default" {
            let _ = std::fs::remove_file(&self.secret_path);
        }

        if next.chat_profile_id == pid {
            next.chat_profile_id = next.model_profiles[0].id.clone();
        }
        if next.watch_profile_id == pid {
            next.watch_profile_id.clear();
        }

        Self::normalize_settings_with_keys(&self.keys_dir, &self.secret_path, &mut next);
        self.persist_unlocked(&next)?;

        if let Ok(mut g) = self.inner.lock() {
            *g = next.clone();
        }
        Ok(next)
    }

    pub fn select_role_models(
        &self,
        chat_profile_id: Option<String>,
        watch_profile_id: Option<String>,
    ) -> Result<AppSettings, String> {
        let mut next = self.get();
        if let Some(cid) = chat_profile_id {
            let cid = sanitize_profile_id(&cid);
            if next.model_profiles.iter().any(|p| p.id == cid) {
                next.chat_profile_id = cid;
            }
        }
        if let Some(wid) = watch_profile_id {
            let trimmed = wid.trim();
            if trimmed.is_empty() || trimmed == "follow_chat" {
                next.watch_profile_id.clear();
            } else {
                let wid = sanitize_profile_id(trimmed);
                if next.model_profiles.iter().any(|p| p.id == wid) {
                    next.watch_profile_id = wid;
                }
            }
        }

        Self::normalize_settings_with_keys(&self.keys_dir, &self.secret_path, &mut next);
        self.persist_unlocked(&next)?;

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
            let _ = self.persist_unlocked(&g);
        }
    }

    pub fn resolve_chat_target(&self) -> (ResolvedModelConfig, String) {
        let cfg = self.get();
        let pid = cfg.chat_profile_id.clone();
        self.resolve_profile_or_default(&cfg, Some(&pid))
    }

    pub fn resolve_watch_target(&self, override_profile_id: Option<&str>) -> (ResolvedModelConfig, String) {
        let cfg = self.get();
        let requested = override_profile_id
            .map(|s| s.trim())
            .filter(|s| !s.is_empty() && *s != "follow_chat")
            .map(|s| s.to_string())
            .or_else(|| {
                let w = cfg.watch_profile_id.trim();
                if w.is_empty() || w == "follow_chat" {
                    None
                } else {
                    Some(w.to_string())
                }
            });
        let target_id = requested.unwrap_or_else(|| cfg.chat_profile_id.clone());
        self.resolve_profile_or_default(&cfg, Some(&target_id))
    }

    fn resolve_profile_or_default(
        &self,
        cfg: &AppSettings,
        profile_id: Option<&str>,
    ) -> (ResolvedModelConfig, String) {
        let chosen = profile_id
            .and_then(|id| cfg.model_profiles.iter().find(|p| p.id == id))
            .or_else(|| {
                cfg.model_profiles
                    .iter()
                    .find(|p| p.id == cfg.chat_profile_id)
            })
            .or_else(|| cfg.model_profiles.first())
            .cloned()
            .unwrap_or_else(|| ModelProfile {
                id: default_chat_profile_id(),
                name: derive_profile_name(&cfg.api_provider, &cfg.model_id),
                api_provider: cfg.api_provider.clone(),
                base_url: cfg.base_url.clone(),
                model_id: cfg.model_id.clone(),
                supports_vision: true,
                thinking_mode: default_thinking_mode(),
                has_api_key: cfg.has_api_key,
            });

        let key = self.load_key_for_profile(&chosen.id).unwrap_or_default();
        (
            ResolvedModelConfig {
                profile_id: chosen.id,
                name: chosen.name,
                api_provider: chosen.api_provider,
                base_url: chosen.base_url,
                model_id: chosen.model_id,
                supports_vision: chosen.supports_vision,
                thinking_mode: normalize_thinking_mode(&chosen.thinking_mode),
            },
            key,
        )
    }

    pub fn load_key_for_profile(&self, profile_id: &str) -> Option<String> {
        let pid = sanitize_profile_id(profile_id);
        let kp = self.key_path_for(&pid);
        if let Some(k) = self.read_encrypted_key_from(&kp) {
            return Some(k);
        }
        if pid == "prof_default" {
            return self.read_encrypted_key_from(&self.secret_path);
        }
        None
    }

    #[cfg(windows)]
    fn save_encrypted_key_to(&self, path: &Path, plaintext: &str) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
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

            let cipher =
                std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
            let _ = LocalFree(windows::Win32::Foundation::HLOCAL(output.pbData as *mut _));
            let encoded = B64.encode(&cipher);
            std::fs::write(path, encoded).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    #[cfg(not(windows))]
    fn save_encrypted_key_to(&self, path: &Path, plaintext: &str) -> Result<(), String> {
        std::fs::write(path, B64.encode(plaintext.as_bytes())).map_err(|e| e.to_string())
    }

    #[cfg(windows)]
    fn read_encrypted_key_from(&self, path: &Path) -> Option<String> {
        let encoded = std::fs::read_to_string(path).ok()?;
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
            let plain =
                std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
            let _ = LocalFree(windows::Win32::Foundation::HLOCAL(output.pbData as *mut _));
            String::from_utf8(plain).ok()
        }
    }

    #[cfg(not(windows))]
    fn read_encrypted_key_from(&self, path: &Path) -> Option<String> {
        let encoded = std::fs::read_to_string(path).ok()?;
        let bytes = B64.decode(encoded.trim()).ok()?;
        String::from_utf8(bytes).ok()
    }
}

#[cfg(windows)]
fn escape_xml(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(windows)]
fn remove_legacy_run_key() {
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
        .is_ok()
        {
            let _ = RegDeleteValueW(hkey, PCWSTR(val_name.as_ptr()));
            let _ = RegCloseKey(hkey);
        }
    }
}

#[cfg(windows)]
fn write_legacy_run_key(exe_path: &Path) -> Result<(), String> {
    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Run\0"
        .encode_utf16()
        .collect();
    let val_name: Vec<u16> = "Buddy\0".encode_utf16().collect();
    unsafe {
        let mut hkey = HKEY::default();
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            KEY_WRITE,
            &mut hkey,
        )
        .ok()
        .map_err(|e| format!("Open Run registry failed: {e}"))?;

        let cmd = format!("\"{}\"", exe_path.to_string_lossy());
        let wide: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
        let byte_slice = std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
        let res = RegSetValueExW(hkey, PCWSTR(val_name.as_ptr()), 0, REG_SZ, Some(byte_slice));
        let _ = RegCloseKey(hkey);
        res.ok().map_err(|e| format!("Write Run registry failed: {e}"))
    }
}

#[cfg(windows)]
fn delete_autostart_task() {
    let _ = std::process::Command::new("schtasks.exe")
        .args(["/Delete", "/TN", AUTOSTART_TASK_NAME, "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

#[cfg(windows)]
fn create_fast_logon_task(exe_path: &Path) -> Result<(), String> {
    let exe_str = exe_path.to_string_lossy().to_string();
    let work_dir = exe_path
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    let whoami_user = std::process::Command::new("whoami")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());

    let user_id = whoami_user.unwrap_or_else(|| {
        let domain = std::env::var("USERDOMAIN").unwrap_or_default();
        let user = std::env::var("USERNAME").unwrap_or_default();
        if !domain.is_empty() && !user.is_empty() {
            format!("{domain}\\{user}")
        } else {
            user
        }
    });
    if user_id.trim().is_empty() {
        return Err("Missing current Windows username".to_string());
    }

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.3" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Buddy fast logon launch (bypasses HKCU Run throttling)</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user_xml}</UserId>
      <Delay>PT2S</Delay>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user_xml}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>false</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>5</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe_xml}</Command>
      <WorkingDirectory>{wd_xml}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>"#,
        user_xml = escape_xml(&user_id),
        exe_xml = escape_xml(&exe_str),
        wd_xml = escape_xml(&work_dir),
    );

    let tmp_xml = std::env::temp_dir().join(format!("buddy_autostart_{}.xml", std::process::id()));
    let mut utf16_bytes = vec![0xFFu8, 0xFEu8]; // UTF-16 LE BOM expected by schtasks /XML
    for u in xml.encode_utf16() {
        utf16_bytes.extend_from_slice(&u.to_le_bytes());
    }
    std::fs::write(&tmp_xml, &utf16_bytes).map_err(|e| format!("Write task XML failed: {e}"))?;

    let output = std::process::Command::new("schtasks.exe")
        .args([
            "/Create",
            "/TN",
            AUTOSTART_TASK_NAME,
            "/XML",
            &tmp_xml.to_string_lossy(),
            "/F",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    let _ = std::fs::remove_file(&tmp_xml);

    let out = output.map_err(|e| format!("Run schtasks failed: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        Err(format!(
            "schtasks /Create failed: {} {}",
            stderr, stdout
        ))
    }
}

#[cfg(windows)]
fn apply_windows_autostart(enable: bool) -> Result<(), String> {
    if enable {
        let exe_path = std::env::current_exe().map_err(|e| e.to_string())?;
        // Prefer fast Task Scheduler ONLOGON (launches ~2s after logon, never throttled by HKCU\Run queue);
        // remove legacy Run entry so Buddy never double-launches. Fallback to HKCU\Run if Task Scheduler denies it.
        match create_fast_logon_task(&exe_path) {
            Ok(()) => {
                remove_legacy_run_key();
                Ok(())
            }
            Err(_task_err) => write_legacy_run_key(&exe_path),
        }
    } else {
        delete_autostart_task();
        remove_legacy_run_key();
        Ok(())
    }
}

#[cfg(not(windows))]
fn apply_windows_autostart(_enable: bool) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_legacy_config_and_switches_chat_and_watch_models() {
        let dir = std::env::temp_dir().join(format!("buddy_settings_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let legacy_json = r#"{
            "api_provider": "gemini",
            "base_url": "https://generativelanguage.googleapis.com",
            "model_id": "gemini-2.5-pro",
            "persona_id": "warm_companion",
            "auto_hide_stealth": false,
            "music_reactive": true,
            "launch_at_startup": false
        }"#;
        std::fs::write(dir.join("config.json"), legacy_json).unwrap();

        let mgr = SettingsManager::load(&dir);
        let cfg = mgr.get();
        assert!(!cfg.auto_collapse_on_blur);
        assert_eq!(cfg.agent_permission_mode, "default");
        assert_eq!(cfg.model_profiles.len(), 1);
        assert_eq!(cfg.chat_profile_id, "prof_default");
        assert_eq!(cfg.model_profiles[0].model_id, "gemini-2.5-pro");

        mgr.set_auto_collapse_on_blur(true).unwrap();
        assert!(SettingsManager::load(&dir).get().auto_collapse_on_blur);
        assert_eq!(mgr.set_agent_permission_mode("auto").unwrap(), "auto");
        assert_eq!(SettingsManager::load(&dir).get().agent_permission_mode, "auto");
        assert_eq!(mgr.set_agent_permission_mode("turbo").unwrap(), "default");

        let (watch_follow, _) = mgr.resolve_watch_target(None);
        assert_eq!(watch_follow.model_id, "gemini-2.5-pro");

        let prof2 = ModelProfile {
            id: "prof_flash".to_string(),
            name: "Flash Watch".to_string(),
            api_provider: "gemini".to_string(),
            base_url: "https://generativelanguage.googleapis.com".to_string(),
            model_id: "gemini-2.5-flash".to_string(),
            supports_vision: true,
            thinking_mode: default_thinking_mode(),
            has_api_key: false,
        };
        let saved = mgr.save_profile(prof2, None, false, true).unwrap();
        assert_eq!(saved.model_profiles.len(), 2);
        assert_eq!(saved.chat_profile_id, "prof_default");
        assert_eq!(saved.watch_profile_id, "prof_flash");

        let (chat_target, _) = mgr.resolve_chat_target();
        let (watch_target, _) = mgr.resolve_watch_target(None);
        assert_eq!(chat_target.model_id, "gemini-2.5-pro");
        assert_eq!(watch_target.model_id, "gemini-2.5-flash");

        // Deleting active watch resets watch cleanly to follow chat
        let after_del = mgr.delete_profile("prof_flash").unwrap();
        assert_eq!(after_del.watch_profile_id, "");
        assert!(mgr.delete_profile("prof_default").is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
