//! MCP (Model Context Protocol) Transport Clients:
//! - `StdioMcpClient`: spawns local MCP servers (npx, uvx, python, exe) via newline-delimited JSON-RPC 2.0.
//! - `HttpMcpClient`: connects to Streamable HTTP / JSON-RPC MCP endpoints (supports both JSON & SSE responses).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpRawTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

pub enum McpClient {
    Stdio(StdioMcpClient),
    Http(HttpMcpClient),
}

impl McpClient {
    pub async fn list_tools(&self) -> Result<Vec<McpRawTool>, String> {
        match self {
            Self::Stdio(c) => c.list_tools().await,
            Self::Http(c) => c.list_tools().await,
        }
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<String, String> {
        match self {
            Self::Stdio(c) => c.call_tool(name, arguments).await,
            Self::Http(c) => c.call_tool(name, arguments).await,
        }
    }

    pub fn shutdown(&self) {
        if let Self::Stdio(c) = self {
            c.shutdown();
        }
    }
}

pub fn parse_tools_list_result(result: &Value) -> Vec<McpRawTool> {
    let mut out = Vec::new();
    if let Some(arr) = result["tools"].as_array() {
        for item in arr {
            let Some(name) = item["name"].as_str().map(str::trim).filter(|s| !s.is_empty()) else {
                continue;
            };
            let description = item["description"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_string();
            let input_schema = if item["inputSchema"].is_object() {
                item["inputSchema"].clone()
            } else {
                json!({ "type": "object", "properties": {} })
            };
            out.push(McpRawTool {
                name: name.to_string(),
                description,
                input_schema,
            });
        }
    }
    out
}

pub fn format_tool_call_result(result: &Value) -> Result<String, String> {
    let is_error = result["isError"].as_bool().unwrap_or(false);
    let mut parts = Vec::new();
    if let Some(arr) = result["content"].as_array() {
        for block in arr {
            let kind = block["type"].as_str().unwrap_or("");
            match kind {
                "text" => {
                    if let Some(t) = block["text"].as_str() {
                        parts.push(t.to_string());
                    }
                }
                "resource" => {
                    if let Some(t) = block["resource"]["text"].as_str() {
                        parts.push(t.to_string());
                    } else if let Some(u) = block["resource"]["uri"].as_str() {
                        parts.push(format!("[Resource: {u}]"));
                    }
                }
                "image" => {
                    let mime = block["mimeType"].as_str().unwrap_or("image");
                    parts.push(format!("[Image attachment: {mime}]"));
                }
                _ => {
                    if let Ok(s) = serde_json::to_string(block) {
                        parts.push(s);
                    }
                }
            }
        }
    } else if !result.is_null() {
        parts.push(result.to_string());
    }

    let joined = if parts.is_empty() {
        "(tool returned empty result)".to_string()
    } else {
        parts.join("\n")
    };
    if is_error {
        Err(joined)
    } else {
        Ok(joined)
    }
}

/// Parses an HTTP response body that may be either plain JSON-RPC or SSE (`data: {...}`).
pub fn parse_http_mcp_body(body: &str, expected_id: u64) -> Result<Value, String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Ok(Value::Null);
    }
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        let v: Value = serde_json::from_str(trimmed)
            .map_err(|e| format!("Invalid MCP JSON response: {e}"))?;
        if let Some(arr) = v.as_array() {
            for item in arr {
                if item["id"].as_u64() == Some(expected_id) {
                    return Ok(item.clone());
                }
            }
            return Ok(arr.first().cloned().unwrap_or(Value::Null));
        }
        return Ok(v);
    }

    // Parse SSE stream lines (`data: {...}`)
    let mut last_msg: Option<Value> = None;
    for line in trimmed.lines() {
        let l = line.trim();
        if let Some(payload) = l.strip_prefix("data:") {
            let p = payload.trim();
            if p.is_empty() || p == "[DONE]" {
                continue;
            }
            if let Ok(val) = serde_json::from_str::<Value>(p) {
                if val["id"].as_u64() == Some(expected_id) {
                    return Ok(val);
                }
                if val.get("result").is_some() || val.get("error").is_some() {
                    last_msg = Some(val);
                }
            }
        }
    }

    last_msg.ok_or_else(|| "MCP HTTP response contained no JSON-RPC payload.".to_string())
}

// ============================================================================
// Stdio MCP Client
// ============================================================================

pub struct StdioMcpClient {
    child: Mutex<Option<Child>>,
    stdin: Mutex<Option<ChildStdin>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>,
    next_id: AtomicU64,
}

impl StdioMcpClient {
    pub async fn connect(
        command: &str,
        args: &[String],
        env_vars: &HashMap<String, String>,
    ) -> Result<Self, String> {
        let clean_cmd = command.trim();
        if clean_cmd.is_empty() {
            return Err("MCP stdio command cannot be empty.".to_string());
        }

        let mut cmd = build_stdio_command(clean_cmd, args);
        for (k, v) in env_vars {
            if !k.trim().is_empty() {
                cmd.env(k.trim(), v);
            }
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        #[cfg(windows)]
        {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to start MCP command `{clean_cmd}`: {e}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "MCP process stdin unavailable.".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "MCP process stdout unavailable.".to_string())?;

        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let pending_reader = Arc::clone(&pending);

        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line_res in reader.lines() {
                let Ok(line) = line_res else {
                    break;
                };
                let trimmed = line.trim();
                if trimmed.is_empty() || !trimmed.starts_with('{') {
                    continue;
                }
                let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
                    continue;
                };
                let Some(id) = msg["id"].as_u64() else {
                    // Server notification — ignore cleanly
                    continue;
                };
                let sender = pending_reader
                    .lock()
                    .ok()
                    .and_then(|mut map| map.remove(&id));
                if let Some(tx) = sender {
                    if let Some(err) = msg.get("error") {
                        let emsg = err["message"]
                            .as_str()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| err.to_string());
                        let _ = tx.send(Err(format!("MCP error: {emsg}")));
                    } else {
                        let _ = tx.send(Ok(msg["result"].clone()));
                    }
                }
            }
            // Fail any in-flight waiters when stdout closes
            if let Ok(mut map) = pending_reader.lock() {
                for (_, tx) in map.drain() {
                    let _ = tx.send(Err("MCP server closed stdout unexpectedly.".to_string()));
                }
            }
        });

        let client = Self {
            child: Mutex::new(Some(child)),
            stdin: Mutex::new(Some(stdin)),
            pending,
            next_id: AtomicU64::new(1),
        };

        // Standard MCP initialize handshake
        let init_params = json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": "Buddy",
                "version": env!("CARGO_PKG_VERSION")
            }
        });
        if let Err(e) = client.request("initialize", init_params, 25).await {
            client.shutdown();
            return Err(format!("MCP initialize failed: {e}"));
        }
        let _ = client.notify("notifications/initialized", json!({}));
        Ok(client)
    }

    fn send_json_line(&self, payload: &Value) -> Result<(), String> {
        let mut line = serde_json::to_string(payload).map_err(|e| e.to_string())?;
        line.push('\n');
        let mut guard = self
            .stdin
            .lock()
            .map_err(|_| "MCP stdin mutex poisoned".to_string())?;
        let stdin = guard
            .as_mut()
            .ok_or_else(|| "MCP server process is closed.".to_string())?;
        stdin
            .write_all(line.as_bytes())
            .map_err(|e| format!("Failed to write to MCP server: {e}"))?;
        stdin
            .flush()
            .map_err(|e| format!("Failed to flush MCP stdin: {e}"))?;
        Ok(())
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.send_json_line(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }))
    }

    async fn request(&self, method: &str, params: Value, timeout_secs: u64) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        if let Ok(mut map) = self.pending.lock() {
            map.insert(id, tx);
        }
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        if let Err(e) = self.send_json_line(&req) {
            if let Ok(mut map) = self.pending.lock() {
                map.remove(&id);
            }
            return Err(e);
        }

        match tokio::time::timeout(Duration::from_secs(timeout_secs), rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => Err("MCP response channel dropped.".to_string()),
            Err(_) => {
                if let Ok(mut map) = self.pending.lock() {
                    map.remove(&id);
                }
                Err(format!("MCP request `{method}` timed out after {timeout_secs}s."))
            }
        }
    }

    pub async fn list_tools(&self) -> Result<Vec<McpRawTool>, String> {
        let res = self.request("tools/list", json!({}), 25).await?;
        Ok(parse_tools_list_result(&res))
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<String, String> {
        let args_obj = if arguments.is_object() {
            arguments
        } else {
            json!({})
        };
        let res = self
            .request(
                "tools/call",
                json!({
                    "name": name,
                    "arguments": args_obj
                }),
                45,
            )
            .await?;
        format_tool_call_result(&res)
    }

    pub fn shutdown(&self) {
        if let Ok(mut stdin_guard) = self.stdin.lock() {
            stdin_guard.take();
        }
        if let Ok(mut child_guard) = self.child.lock() {
            if let Some(mut c) = child_guard.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }
}

impl Drop for StdioMcpClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn build_stdio_command(command: &str, args: &[String]) -> Command {
    #[cfg(windows)]
    {
        let low = command.to_ascii_lowercase();
        let needs_cmd_shim = matches!(
            low.as_str(),
            "npx" | "npm" | "pnpm" | "pnpx" | "yarn" | "bunx"
        ) || low.ends_with(".cmd")
            || low.ends_with(".bat");
        if needs_cmd_shim {
            let mut c = Command::new("cmd.exe");
            c.arg("/C").arg(command).args(args);
            return c;
        }
    }
    let mut c = Command::new(command);
    c.args(args);
    c
}

// ============================================================================
// Streamable HTTP / JSON-RPC MCP Client
// ============================================================================

pub struct HttpMcpClient {
    url: String,
    headers: HashMap<String, String>,
    session_id: Mutex<Option<String>>,
    http: reqwest::Client,
    next_id: AtomicU64,
}

impl HttpMcpClient {
    pub async fn connect(url: &str, headers: HashMap<String, String>) -> Result<Self, String> {
        let clean = url.trim().to_string();
        if !clean.starts_with("http://") && !clean.starts_with("https://") {
            return Err("MCP HTTP URL must start with http:// or https://".to_string());
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(45))
            .build()
            .map_err(|e| e.to_string())?;
        let client = Self {
            url: clean,
            headers,
            session_id: Mutex::new(None),
            http,
            next_id: AtomicU64::new(1),
        };
        let init_params = json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": "Buddy",
                "version": env!("CARGO_PKG_VERSION")
            }
        });
        client.request("initialize", init_params).await?;
        let _ = client.notify("notifications/initialized", json!({})).await;
        Ok(client)
    }

    async fn post_rpc(&self, body: Value, expected_id: Option<u64>) -> Result<Value, String> {
        let mut req = self
            .http
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .json(&body);

        for (k, v) in &self.headers {
            if !k.trim().is_empty() {
                req = req.header(k.trim(), v.as_str());
            }
        }
        if let Ok(guard) = self.session_id.lock() {
            if let Some(sid) = guard.as_deref() {
                req = req.header("Mcp-Session-Id", sid);
            }
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("MCP HTTP request failed: {e}"))?;

        if let Some(sid) = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if let Ok(mut guard) = self.session_id.lock() {
                *guard = Some(sid.to_string());
            }
        }

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            let preview: String = text.chars().take(240).collect();
            return Err(format!("MCP HTTP {status}: {preview}"));
        }

        let Some(id) = expected_id else {
            return Ok(Value::Null);
        };
        let msg = parse_http_mcp_body(&text, id)?;
        if let Some(err) = msg.get("error") {
            let emsg = err["message"]
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| err.to_string());
            return Err(format!("MCP error: {emsg}"));
        }
        Ok(msg["result"].clone())
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        let _ = self
            .post_rpc(
                json!({
                    "jsonrpc": "2.0",
                    "method": method,
                    "params": params
                }),
                None,
            )
            .await?;
        Ok(())
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.post_rpc(
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params
            }),
            Some(id),
        )
        .await
    }

    pub async fn list_tools(&self) -> Result<Vec<McpRawTool>, String> {
        let res = self.request("tools/list", json!({})).await?;
        Ok(parse_tools_list_result(&res))
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<String, String> {
        let args_obj = if arguments.is_object() {
            arguments
        } else {
            json!({})
        };
        let res = self
            .request(
                "tools/call",
                json!({
                    "name": name,
                    "arguments": args_obj
                }),
            )
            .await?;
        format_tool_call_result(&res)
    }
}
