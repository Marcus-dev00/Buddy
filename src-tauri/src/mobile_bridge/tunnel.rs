//! LAN / Tailscale IP auto-detection, on-demand `cloudflared.exe` downloader & Quick Tunnel runner,
//! and encrypted Cloudflare Worker directory publisher for seamless remote mobile reconnect.

use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const CLOUDFLARED_WIN_URL: &str =
    "https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-windows-amd64.exe";

/// Detects `(lan_ipv4, tailscale_ipv4)` cleanly with zero external network crates.
pub fn detect_local_ips() -> (Option<String>, Option<String>) {
    let lan_ip = UdpSocket::bind("0.0.0.0:0")
        .ok()
        .and_then(|sock| {
            sock.connect("8.8.8.8:80").ok()?;
            let ip = sock.local_addr().ok()?.ip().to_string();
            if ip.starts_with("127.") || ip == "0.0.0.0" {
                None
            } else {
                Some(ip)
            }
        });

    let tailscale_ip = detect_tailscale_ipv4();
    (lan_ip, tailscale_ip)
}

fn is_tailscale_cgnat(ip: &str) -> bool {
    let parts: Vec<u8> = ip.split('.').filter_map(|p| p.parse::<u8>().ok()).collect();
    parts.len() == 4 && parts[0] == 100 && (64..=127).contains(&parts[1])
}

#[cfg(windows)]
fn detect_tailscale_ipv4() -> Option<String> {
    let mut cmd = Command::new("ipconfig.exe");
    cmd.creation_flags(CREATE_NO_WINDOW);
    let out = cmd.output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    extract_tailscale_ip_from_text(&text)
}

#[cfg(not(windows))]
fn detect_tailscale_ipv4() -> Option<String> {
    None
}

pub fn extract_tailscale_ip_from_text(text: &str) -> Option<String> {
    for line in text.lines() {
        for tok in line.split(|c: char| !c.is_ascii_digit() && c != '.') {
            if tok.len() >= 9 && is_tailscale_cgnat(tok) {
                return Some(tok.to_string());
            }
        }
    }
    None
}

pub fn extract_trycloudflare_url(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let rest = &line[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '|' | ')' | ']'))
        .unwrap_or(rest.len());
    let cand = &rest[..end];
    if cand.contains(".trycloudflare.com") {
        Some(cand.trim_end_matches('/').to_string())
    } else {
        None
    }
}

/// Computes the directory write key (SHA-256 of `pair_id:token`) so the Cloudflare directory
/// never sees the raw pairing token.
pub fn compute_directory_auth_hash(pair_id: &str, token: &str) -> String {
    let mut h = Sha256::new();
    h.update(pair_id.as_bytes());
    h.update(b":");
    h.update(token.as_bytes());
    let digest = h.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub async fn publish_tunnel_to_directory(
    directory_base: &str,
    pair_id: &str,
    token: &str,
    tunnel_url: &str,
    lan_url: Option<&str>,
) -> Result<(), String> {
    let base = directory_base.trim().trim_end_matches('/');
    if base.is_empty() || (!base.starts_with("https://") && !base.starts_with("http://")) {
        return Ok(());
    }
    let endpoint = format!("{base}/api/directory/update");
    let auth_hash = compute_directory_auth_hash(pair_id, token);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(&endpoint)
        .json(&serde_json::json!({
            "pair_id": pair_id,
            "auth_hash": auth_hash,
            "remote_url": tunnel_url,
            "lan_url": lan_url.unwrap_or("")
        }))
        .send()
        .await
        .map_err(|e| format!("Directory publish failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Directory returned HTTP {}", resp.status()));
    }
    Ok(())
}

pub async fn ensure_cloudflared_binary<F>(
    data_dir: &Path,
    mirror_prefix: &str,
    on_progress: F,
) -> Result<PathBuf, String>
where
    F: Fn(u8) + Send + Sync,
{
    let bin_dir = data_dir.join("engines").join("cloudflared");
    let _ = std::fs::create_dir_all(&bin_dir);
    let bin_path = bin_dir.join("cloudflared.exe");
    if bin_path.exists()
        && std::fs::metadata(&bin_path)
            .map(|m| m.len() > 2_000_000)
            .unwrap_or(false)
    {
        return Ok(bin_path);
    }

    let m = mirror_prefix.trim();
    let dl_url = if m.is_empty() {
        CLOUDFLARED_WIN_URL.to_string()
    } else if m.ends_with('/') {
        format!("{m}{CLOUDFLARED_WIN_URL}")
    } else {
        format!("{m}/{CLOUDFLARED_WIN_URL}")
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| e.to_string())?;
    let mut resp = client
        .get(&dl_url)
        .header("User-Agent", "Buddy-Desktop")
        .send()
        .await
        .map_err(|e| format!("Failed to download cloudflared: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("cloudflared download HTTP {}", resp.status()));
    }

    let total = resp.content_length().unwrap_or(0);
    let tmp_path = bin_dir.join("cloudflared.exe.part");
    let mut file = std::fs::File::create(&tmp_path).map_err(|e| e.to_string())?;
    let mut received: u64 = 0;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("Download interrupted: {e}"))?
    {
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        received += chunk.len() as u64;
        if total > 0 {
            let pct = ((received * 100) / total).min(99) as u8;
            on_progress(pct);
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);

    if bin_path.exists() {
        let _ = std::fs::remove_file(&bin_path);
    }
    std::fs::rename(&tmp_path, &bin_path).map_err(|e| e.to_string())?;
    on_progress(100);
    Ok(bin_path)
}

pub struct TunnelHandle {
    child: Mutex<Option<Child>>,
}

impl TunnelHandle {
    pub fn spawn_quick_tunnel<F>(
        bin_path: &Path,
        local_port: u16,
        on_url: F,
    ) -> Result<Arc<Self>, String>
    where
        F: Fn(String) + Send + 'static,
    {
        let mut cmd = Command::new(bin_path);
        cmd.args([
            "tunnel",
            "--no-autoupdate",
            "--url",
            &format!("http://127.0.0.1:{local_port}"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

        #[cfg(windows)]
        {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to start cloudflared: {e}"))?;
        let stderr = child.stderr.take();

        if let Some(err_stream) = stderr {
            std::thread::spawn(move || {
                let reader = BufReader::new(err_stream);
                let mut sent = false;
                for line in reader.lines().map_while(Result::ok) {
                    if !sent {
                        if let Some(url) = extract_trycloudflare_url(&line) {
                            sent = true;
                            on_url(url);
                        }
                    }
                }
            });
        }

        Ok(Arc::new(Self {
            child: Mutex::new(Some(child)),
        }))
    }

    pub fn stop(&self) {
        if let Ok(mut g) = self.child.lock() {
            if let Some(mut c) = g.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }
}

impl Drop for TunnelHandle {
    fn drop(&mut self) {
        self.stop();
    }
}
