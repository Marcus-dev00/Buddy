//! Path resolution, protected OS/credential path guards, permission risk classification,
//! destructive shell command detection, and SSRF protection for public URL fetching.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::path::{Component, Path, PathBuf};

pub fn user_home_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\"))
}

pub fn resolve_user_path(raw: &str, workspace_hint: Option<&Path>) -> Result<PathBuf, String> {
    let trimmed = raw.trim().trim_matches('"');
    if trimmed.is_empty() {
        return Ok(workspace_hint
            .map(|p| p.to_path_buf())
            .unwrap_or_else(user_home_dir));
    }

    let home = user_home_dir();
    let lower = trimmed.to_ascii_lowercase();

    let expanded = if trimmed == "~" {
        home
    } else if let Some(rest) = trimmed.strip_prefix("~/").or_else(|| trimmed.strip_prefix("~\\")) {
        home.join(rest)
    } else if lower == "desktop" || lower.starts_with("desktop\\") || lower.starts_with("desktop/") {
        let rest = &trimmed[7..].trim_start_matches(['\\', '/']);
        if rest.is_empty() {
            home.join("Desktop")
        } else {
            home.join("Desktop").join(rest)
        }
    } else if lower == "documents"
        || lower.starts_with("documents\\")
        || lower.starts_with("documents/")
    {
        let rest = &trimmed[9..].trim_start_matches(['\\', '/']);
        if rest.is_empty() {
            home.join("Documents")
        } else {
            home.join("Documents").join(rest)
        }
    } else if lower == "downloads"
        || lower.starts_with("downloads\\")
        || lower.starts_with("downloads/")
    {
        let rest = &trimmed[9..].trim_start_matches(['\\', '/']);
        if rest.is_empty() {
            home.join("Downloads")
        } else {
            home.join("Downloads").join(rest)
        }
    } else {
        let p = PathBuf::from(trimmed);
        if p.is_absolute() {
            p
        } else if let Some(base) = workspace_hint {
            base.join(p)
        } else {
            home.join(p)
        }
    };

    Ok(normalize_lexical(&expanded))
}

fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    out.components().next_back(),
                    Some(Component::Prefix(_)) | Some(Component::RootDir) | None
                ) {
                    out.pop();
                }
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}

pub(super) fn is_blocked_path(path: &Path, is_write: bool) -> bool {
    let s = path.to_string_lossy().to_ascii_lowercase().replace('/', "\\");
    let blocked_prefixes = [
        "c:\\windows",
        "c:\\$recycle.bin",
        "c:\\system volume information",
    ];
    if blocked_prefixes.iter().any(|p| s == *p || s.starts_with(&format!("{p}\\"))) {
        return true;
    }
    if is_write {
        let write_protected = ["c:\\program files", "c:\\program files (x86)"];
        if write_protected
            .iter()
            .any(|p| s == *p || s.starts_with(&format!("{p}\\")))
        {
            return true;
        }
    }
    let sensitive_fragments = [
        "\\buddy\\secret.dpapi",
        "\\.ssh\\",
        "\\.gnupg\\",
        "\\.aws\\credentials",
    ];
    if sensitive_fragments.iter().any(|f| s.contains(f) || s.ends_with(f.trim_end_matches('\\'))) {
        return true;
    }
    false
}

pub(crate) fn check_path_allowed(path: &Path, is_write: bool) -> Result<(), String> {
    if is_blocked_path(path, is_write) {
        return Err(format!(
            "Access blocked for protected OS or credential path: {}",
            path.display()
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentPermissionMode {
    Ask,
    Default,
    Auto,
}

impl AgentPermissionMode {
    pub fn from_str(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "ask" => Self::Ask,
            "auto" => Self::Auto,
            _ => Self::Default,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ToolRiskKind {
    ReadOnly,
    ReversibleWrite,
    ExternalSideEffect,
    DangerousOrDelete,
}

pub(super) fn approval_flags_for(
    mode: AgentPermissionMode,
    risk: ToolRiskKind,
) -> (bool, bool, bool) {
    let is_destructive = risk == ToolRiskKind::DangerousOrDelete;
    match (mode, risk) {
        (_, ToolRiskKind::ReadOnly) => (false, false, false),
        (
            AgentPermissionMode::Ask,
            ToolRiskKind::ReversibleWrite
            | ToolRiskKind::ExternalSideEffect
            | ToolRiskKind::DangerousOrDelete,
        ) => (true, false, is_destructive),
        (AgentPermissionMode::Default, ToolRiskKind::ReversibleWrite) => (true, true, false),
        (
            AgentPermissionMode::Default,
            ToolRiskKind::ExternalSideEffect | ToolRiskKind::DangerousOrDelete,
        ) => (true, false, is_destructive),
        (
            AgentPermissionMode::Auto,
            ToolRiskKind::ReversibleWrite | ToolRiskKind::ExternalSideEffect,
        ) => (false, false, false),
        (AgentPermissionMode::Auto, ToolRiskKind::DangerousOrDelete) => (true, false, true),
    }
}

pub(super) fn is_dangerous_shell_command(cmd: &str) -> bool {
    let normalized: String = cmd
        .to_ascii_lowercase()
        .replace('`', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        return true;
    }

    const SUBSTR_PATTERNS: &[&str] = &[
        "remove-item",
        "clear-content",
        "clear-item",
        "remove-itemproperty",
        "set-itemproperty",
        "new-itemproperty",
        "stop-process",
        "stop-service",
        "set-service",
        "remove-service",
        "restart-computer",
        "stop-computer",
        "format-volume",
        "format-disk",
        "clear-disk",
        "remove-partition",
        "invoke-expression",
        "-encodedcommand",
        "frombase64string",
        "start-bitstransfer",
        "file]::delete",
        "directory]::delete",
        ".delete(",
        "os.remove",
        "os.unlink",
        "shutil.rmtree",
        "fs.unlink",
        "fs.rm",
        "rmsync",
        "rmdirsync",
        "unlinksync",
        "uninstall-package",
        "uninstall-module",
        "winget uninstall",
        "npm uninstall",
        "pip uninstall",
        "cargo clean",
        "git clean",
        "git reset --hard",
        "git push",
        "git branch -d",
        "git checkout --",
        "git restore",
        "git rebase",
        "reg delete",
        "reg add",
        "reg import",
        "reg restore",
        "-verb runas",
        "-verb 'runas'",
        "-verb \"runas\"",
        " -force",
        " --force",
    ];
    if SUBSTR_PATTERNS.iter().any(|p| normalized.contains(p)) {
        return true;
    }
    if normalized.contains("cipher") && normalized.contains("/w") {
        return true;
    }
    if normalized.contains(" -enc ") || normalized.ends_with(" -enc") {
        return true;
    }

    const DANGEROUS_CMDS: &[&str] = &[
        "rm",
        "del",
        "erase",
        "rd",
        "rmdir",
        "ri",
        "clc",
        "kill",
        "taskkill",
        "shutdown",
        "logoff",
        "diskpart",
        "bcdedit",
        "vssadmin",
        "wbadmin",
        "reg",
        "regedit",
        "schtasks",
        "netsh",
        "net",
        "icacls",
        "takeown",
        "iex",
        "runas",
        "format",
        "sfc",
        "dism",
        "sc",
    ];

    for segment in normalized.split(|c| matches!(c, ';' | '|' | '&' | '\n' | '\r' | '(' | '{')) {
        let mut iter = segment.split_whitespace().peekable();
        // Skip shell wrappers / invocation prefixes to inspect the actual verb
        while let Some(raw_tok) = iter.peek().copied() {
            let tok = raw_tok
                .trim_matches(|c| matches!(c, '\'' | '"' | '.'))
                .trim_end_matches(".exe");
            if matches!(
                tok,
                "" | "cmd" | "powershell" | "pwsh" | "/c" | "/k" | "-c" | "-command" | "-noprofile"
                    | "-nologo" | "-noninteractive" | "sudo"
            ) {
                iter.next();
                continue;
            }
            if DANGEROUS_CMDS.contains(&tok) {
                return true;
            }
            break;
        }
    }

    false
}

// ============================================================================
// SSRF-Safe Public URL / IP Validators
// ============================================================================

fn is_blocked_ipv4(ip: Ipv4Addr) -> bool {
    let oct = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        // CGNAT 100.64.0.0/10
        || (oct[0] == 100 && (oct[1] & 0b1100_0000) == 64)
        // Reserved / protocol / bench ranges
        || (oct[0] == 192 && oct[1] == 0 && oct[2] == 0)
        || (oct[0] == 198 && (oct[1] == 18 || oct[1] == 19))
        || oct[0] >= 240
}

fn is_blocked_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_blocked_ipv4(v4);
    }
    let seg0 = ip.segments()[0];
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (seg0 & 0xfe00) == 0xfc00 // fc00::/7 unique local
        || (seg0 & 0xffc0) == 0xfe80 // fe80::/10 link-local
}

pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_ipv4(v4),
        IpAddr::V6(v6) => is_blocked_ipv6(v6),
    }
}

pub fn validate_public_url(raw_url: &str, resolve_dns: bool) -> Result<reqwest::Url, String> {
    let trimmed = raw_url.trim();
    let parsed = reqwest::Url::parse(trimmed).map_err(|e| format!("Invalid URL: {e}"))?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!("Blocked non-HTTP URL scheme: {scheme}"));
    }
    let host_raw = parsed
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;
    let host = host_raw.trim_matches(|c| c == '[' || c == ']').to_ascii_lowercase();
    if host.is_empty()
        || host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with(".lan")
    {
        return Err(format!("Blocked local/private host: {host}"));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_blocked_ip(ip) {
            return Err(format!("Blocked non-public IP: {ip}"));
        }
        return Ok(parsed);
    }
    if !host.contains('.') {
        return Err(format!("Blocked single-label intranet host: {host}"));
    }
    if resolve_dns {
        let port = parsed.port_or_known_default().unwrap_or(80);
        let addrs: Vec<_> = (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|e| format!("DNS resolution failed for `{host}`: {e}"))?
            .collect();
        if addrs.is_empty() {
            return Err(format!("No DNS addresses resolved for `{host}`"));
        }
        for sa in addrs {
            if is_blocked_ip(sa.ip()) {
                return Err(format!(
                    "Blocked host `{host}` resolving to private/local IP {}",
                    sa.ip()
                ));
            }
        }
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_blocking_protects_os_and_secrets() {
        assert!(is_blocked_path(Path::new("C:\\Windows\\System32\\cmd.exe"), false));
        assert!(is_blocked_path(Path::new("C:\\Program Files\\App\\a.dll"), true));
        assert!(is_blocked_path(Path::new("C:\\Users\\Test\\AppData\\Local\\Buddy\\secret.dpapi"), false));
        assert!(!is_blocked_path(Path::new("C:\\Users\\Test\\Desktop\\notes.md"), true));
    }

    #[test]
    fn ssrf_blocks_local_private_and_metadata_urls() {
        assert!(validate_public_url("http://127.0.0.1:8080/admin", false).is_err());
        assert!(validate_public_url("http://localhost:1420/", false).is_err());
        assert!(validate_public_url("http://169.254.169.254/latest/meta-data", false).is_err());
        assert!(validate_public_url("http://192.168.1.1/", false).is_err());
        assert!(validate_public_url("http://[::1]:3000/", false).is_err());
        assert!(validate_public_url("file:///C:/Windows/win.ini", false).is_err());
        assert!(validate_public_url("https://example.com/docs", false).is_ok());
    }
}
