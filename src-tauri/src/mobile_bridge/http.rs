//! Minimal zero-dependency HTTP/1.1 + SSE + Range file helpers for Buddy's Mobile Bridge.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;

pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

const MAX_BODY_BYTES: usize = 25 * 1024 * 1024; // 25 MB max upload from mobile

pub fn parse_http_request(stream: &TcpStream) -> Result<HttpRequest, String> {
    let mut reader = BufReader::new(stream);
    let mut req_line = String::new();
    let n = reader
        .read_line(&mut req_line)
        .map_err(|e| format!("read request line failed: {e}"))?;
    if n == 0 {
        return Err("EOF".to_string());
    }

    let parts: Vec<&str> = req_line.trim().split_whitespace().collect();
    if parts.len() < 2 {
        return Err("Malformed HTTP request line".to_string());
    }
    let method = parts[0].to_ascii_uppercase();
    let full_target = parts[1];
    let (raw_path, raw_query) = match full_target.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (full_target, None),
    };
    let path = percent_decode(raw_path);
    let mut query = HashMap::new();
    if let Some(q) = raw_query {
        for pair in q.split('&') {
            if pair.is_empty() {
                continue;
            }
            if let Some((k, v)) = pair.split_once('=') {
                query.insert(percent_decode(k), percent_decode(v));
            } else {
                query.insert(percent_decode(pair), String::new());
            }
        }
    }

    let mut headers = HashMap::new();
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).map_err(|e| e.to_string())?;
        if read == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_ascii_lowercase();
            let val = v.trim().to_string();
            if key == "content-length" {
                content_length = val.parse::<usize>().unwrap_or(0);
            }
            headers.insert(key, val);
        }
    }

    if content_length > MAX_BODY_BYTES {
        return Err("Request body exceeds 25 MB limit".to_string());
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader
            .read_exact(&mut body)
            .map_err(|e| format!("read body failed: {e}"))?;
    }

    Ok(HttpRequest {
        method,
        path,
        query,
        headers,
        body,
    })
}

pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
        } else if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
            out.push(bytes[i]);
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub fn write_cors_preflight(stream: &mut TcpStream) {
    let resp = concat!(
        "HTTP/1.1 204 No Content\r\n",
        "Access-Control-Allow-Origin: *\r\n",
        "Access-Control-Allow-Methods: GET, POST, DELETE, OPTIONS\r\n",
        "Access-Control-Allow-Headers: Authorization, Content-Type, Range\r\n",
        "Access-Control-Max-Age: 86400\r\n",
        "Content-Length: 0\r\n",
        "Connection: close\r\n\r\n"
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

pub fn write_json(stream: &mut TcpStream, status: u16, body: &serde_json::Value) {
    let payload = serde_json::to_string(body).unwrap_or_else(|_| "{}".to_string());
    let reason = status_reason(status);
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: application/json; charset=utf-8\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Headers: Authorization, Content-Type\r\n\
         Cache-Control: no-store\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n",
        payload.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(payload.as_bytes());
    let _ = stream.flush();
}

pub fn write_error(stream: &mut TcpStream, status: u16, message: &str) {
    write_json(stream, status, &serde_json::json!({ "ok": false, "error": message }));
}

pub fn write_sse_headers(stream: &mut TcpStream) -> std::io::Result<()> {
    let head = concat!(
        "HTTP/1.1 200 OK\r\n",
        "Content-Type: text/event-stream; charset=utf-8\r\n",
        "Cache-Control: no-cache\r\n",
        "Connection: keep-alive\r\n",
        "X-Accel-Buffering: no\r\n",
        "Access-Control-Allow-Origin: *\r\n\r\n"
    );
    stream.write_all(head.as_bytes())?;
    stream.flush()
}

pub fn write_sse_event(stream: &mut TcpStream, event: &str, data_json: &str) -> std::io::Result<()> {
    let chunk = format!("event: {event}\ndata: {data_json}\n\n");
    stream.write_all(chunk.as_bytes())?;
    stream.flush()
}

pub fn write_file_response(
    stream: &mut TcpStream,
    path: &Path,
    range_header: Option<&str>,
    as_download: bool,
) {
    let Ok(bytes) = std::fs::read(path) else {
        write_error(stream, 404, "File not found");
        return;
    };
    let total = bytes.len();
    let mime = guess_mime_type(path);
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .replace('"', "_");
    let disp = if as_download {
        format!("attachment; filename=\"{file_name}\"")
    } else {
        format!("inline; filename=\"{file_name}\"")
    };

    if let Some(rh) = range_header {
        if let Some((start, end)) = parse_byte_range(rh, total) {
            let slice = &bytes[start..=end];
            let head = format!(
                "HTTP/1.1 206 Partial Content\r\n\
                 Content-Type: {mime}\r\n\
                 Content-Disposition: {disp}\r\n\
                 Accept-Ranges: bytes\r\n\
                 Content-Range: bytes {start}-{end}/{total}\r\n\
                 Access-Control-Allow-Origin: *\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\r\n",
                slice.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(slice);
            let _ = stream.flush();
            return;
        }
    }

    let head = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: {mime}\r\n\
         Content-Disposition: {disp}\r\n\
         Accept-Ranges: bytes\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Content-Length: {total}\r\n\
         Connection: close\r\n\r\n"
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&bytes);
    let _ = stream.flush();
}

fn parse_byte_range(header: &str, total: usize) -> Option<(usize, usize)> {
    if total == 0 {
        return None;
    }
    let rest = header.trim().strip_prefix("bytes=")?;
    let (s, e) = rest.split_once('-')?;
    let start = if s.is_empty() {
        0
    } else {
        s.parse::<usize>().ok()?
    };
    let end = if e.is_empty() {
        total - 1
    } else {
        e.parse::<usize>().ok()?.min(total - 1)
    };
    if start <= end && end < total {
        Some((start, end))
    } else {
        None
    }
}

pub fn guess_mime_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("mp3") => "audio/mpeg",
        Some("m4a" | "aac") => "audio/mp4",
        Some("flac") => "audio/flac",
        Some("wav") => "audio/wav",
        Some("ogg" | "opus") => "audio/ogg",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mkv") => "video/x-matroska",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("pdf") => "application/pdf",
        Some("txt" | "md" | "log") => "text/plain; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        206 => "Partial Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    }
}
