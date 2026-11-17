//! Lightweight rolling diagnostic logger for Buddy.
//!
//! Writes redacted operational events to `<data_dir>/logs/buddy.log` with a
//! 2 MB size cap (rolling the previous log into `buddy.prev.log`).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug)]
struct LoggerState {
    log_dir: PathBuf,
    log_file: PathBuf,
    prev_file: PathBuf,
    max_bytes: u64,
}

static LOGGER: OnceLock<Mutex<LoggerState>> = OnceLock::new();

/// Initialize the global rolling logger under `<data_dir>/logs`.
/// Safe to call more than once (subsequent calls are no-ops).
pub fn init_logger(data_dir: &Path) -> Result<PathBuf, String> {
    let log_dir = data_dir.join("logs");
    fs::create_dir_all(&log_dir).map_err(|e| format!("create logs dir: {e}"))?;
    let log_file = log_dir.join("buddy.log");
    let prev_file = log_dir.join("buddy.prev.log");

    let _ = LOGGER.get_or_init(|| {
        Mutex::new(LoggerState {
            log_dir: log_dir.clone(),
            log_file: log_file.clone(),
            prev_file,
            max_bytes: MAX_LOG_BYTES,
        })
    });

    log_info("logger", "diagnostic logger initialized");
    Ok(log_dir)
}

/// Return the initialized logs directory path, or `<LOCALAPPDATA>/Buddy/logs` fallback.
pub fn logs_dir() -> PathBuf {
    if let Some(lock) = LOGGER.get() {
        if let Ok(state) = lock.lock() {
            return state.log_dir.clone();
        }
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("Buddy").join("logs")
}

pub fn log_info(component: &str, message: &str) {
    write_record("INFO", component, message);
}

pub fn log_warn(component: &str, message: &str) {
    write_record("WARN", component, message);
}

pub fn log_error(component: &str, message: &str) {
    write_record("ERROR", component, message);
}

fn write_record(level: &str, component: &str, message: &str) {
    let Some(lock) = LOGGER.get() else {
        return;
    };
    let Ok(mut state) = lock.lock() else {
        return;
    };
    let clean = sanitize_log_message(message);
    let ts = format_utc_timestamp();
    let line = format!("[{ts}] [{level}] [{component}] {clean}\n");
    let _ = append_line_with_rotation(&mut state, line.as_bytes());
}

fn append_line_with_rotation(state: &mut LoggerState, bytes: &[u8]) -> std::io::Result<()> {
    fs::create_dir_all(&state.log_dir)?;
    if let Ok(meta) = fs::metadata(&state.log_file) {
        if meta.len().saturating_add(bytes.len() as u64) > state.max_bytes {
            let _ = fs::remove_file(&state.prev_file);
            let _ = fs::rename(&state.log_file, &state.prev_file);
        }
    }
    let mut file: File = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&state.log_file)?;
    file.write_all(bytes)?;
    Ok(())
}

/// Redact likely API keys, bearer tokens, URL query credentials, and truncate overly long text.
pub fn sanitize_log_message(raw: &str) -> String {
    let single_line = raw
        .replace('\r', " ")
        .replace('\n', " | ")
        .trim()
        .to_string();

    let mut tokens: Vec<String> = Vec::new();
    let mut redact_next = false;

    for token in single_line.split_whitespace() {
        if redact_next {
            tokens.push("[REDACTED]".to_string());
            redact_next = false;
            continue;
        }
        let lower = token.to_ascii_lowercase();
        if lower == "bearer" || lower == "authorization:" {
            tokens.push(token.to_string());
            redact_next = true;
            continue;
        }
        if let Some(masked) = redact_token(token) {
            tokens.push(masked);
        } else {
            tokens.push(token.to_string());
        }
    }

    let joined = tokens.join(" ");
    const MAX_MSG_CHARS: usize = 600;
    if joined.chars().count() > MAX_MSG_CHARS {
        let mut out: String = joined.chars().take(MAX_MSG_CHARS).collect();
        out.push_str("...[truncated]");
        out
    } else {
        joined
    }
}

fn redact_token(token: &str) -> Option<String> {
    // Strip trailing punctuation gently when checking prefixes
    let trimmed = token.trim_matches(|c: char| matches!(c, ',' | ';' | '"' | '\'' | ')' | ']'));
    let lower = trimmed.to_ascii_lowercase();

    for prefix in [
        "sk-",
        "sk-ant-",
        "aiza",
        "ghp_",
        "gho_",
        "xoxb-",
        "xoxp-",
    ] {
        if lower.starts_with(prefix) && trimmed.len() >= 12 {
            let visible: String = trimmed.chars().take(prefix.len().min(6)).collect();
            return Some(format!("{visible}***[REDACTED]"));
        }
    }

    for kv_key in [
        "api_key=",
        "apikey=",
        "token=",
        "secret=",
        "password=",
        "key=",
        "authorization=",
    ] {
        if let Some(idx) = lower.find(kv_key) {
            let split_at = idx + kv_key.len();
            if trimmed.len() > split_at {
                let prefix = &trimmed[..split_at];
                return Some(format!("{prefix}[REDACTED]"));
            }
        }
    }

    None
}

fn format_utc_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Civil date/time from Unix seconds (Howard Hinnant algorithm)
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    format!("{year:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_api_keys_and_bearer_tokens() {
        let msg = "Failed calling https://example.com?key=AIzaSySecretValue12345 with Bearer my-secret-token and sk-proj-abcdef1234567890";
        let sanitized = sanitize_log_message(msg);
        assert!(!sanitized.contains("AIzaSySecretValue12345"));
        assert!(!sanitized.contains("my-secret-token"));
        assert!(!sanitized.contains("abcdef1234567890"));
        assert!(sanitized.contains("[REDACTED]"));
    }

    #[test]
    fn rolls_over_when_size_limit_exceeded() {
        let base = std::env::temp_dir().join(format!(
            "buddy_log_test_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let log_dir = base.join("logs");
        let log_file = log_dir.join("buddy.log");
        let prev_file = log_dir.join("buddy.prev.log");
        let mut state = LoggerState {
            log_dir: log_dir.clone(),
            log_file: log_file.clone(),
            prev_file: prev_file.clone(),
            max_bytes: 64,
        };

        append_line_with_rotation(&mut state, b"first line that takes roughly fifty bytes\n").unwrap();
        append_line_with_rotation(&mut state, b"second line pushing beyond sixty four bytes!\n").unwrap();

        assert!(prev_file.exists(), "previous log should exist after rollover");
        assert!(log_file.exists(), "active log should exist after rollover");
        let _ = fs::remove_dir_all(&base);
    }
}
