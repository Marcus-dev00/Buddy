//! Model Context Protocol (MCP) Server Manager:
//! - Persists configured MCP servers in `%LOCALAPPDATA%/Buddy/mcp_servers.json`.
//! - Supports importing standard `{ "mcpServers": { ... } }` JSON snippets (Claude Desktop / Cursor format).
//! - Exposes live tools namespaced as `mcp__<server_slug>__<tool_slug>` to OpenAI / Anthropic / Gemini.

pub mod client;

use client::{HttpMcpClient, McpClient, McpRawTool, StdioMcpClient};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

fn default_true() -> bool {
    true
}

fn default_transport() -> String {
    "stdio".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    #[serde(default = "default_transport")]
    pub transport: String, // "stdio" | "http"
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub trusted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpToolInfo {
    pub server_id: String,
    pub server_name: String,
    pub tool_name: String,
    pub qualified_name: String,
    pub description: String,
    pub input_schema: Value,
    pub server_trusted: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct McpServerStatus {
    pub config: McpServerConfig,
    pub connected: bool,
    pub connecting: bool,
    pub tool_count: usize,
    pub tools: Vec<McpToolInfo>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct PersistedMcpStore {
    #[serde(default)]
    servers: Vec<McpServerConfig>,
}

struct LiveServerRuntime {
    client: Option<Arc<McpClient>>,
    tools: Vec<McpToolInfo>,
    connecting: bool,
    last_error: Option<String>,
}

impl Default for LiveServerRuntime {
    fn default() -> Self {
        Self {
            client: None,
            tools: Vec::new(),
            connecting: false,
            last_error: None,
        }
    }
}

pub struct McpManager {
    config_path: PathBuf,
    configs: Mutex<Vec<McpServerConfig>>,
    runtimes: Mutex<HashMap<String, LiveServerRuntime>>,
}

pub fn sanitize_mcp_ident(raw: &str) -> String {
    let mut out = String::new();
    let mut prev_under = false;
    for ch in raw.trim().chars() {
        let low = ch.to_ascii_lowercase();
        if low.is_ascii_alphanumeric() {
            out.push(low);
            prev_under = false;
        } else if !prev_under && !out.is_empty() {
            out.push('_');
            prev_under = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        "srv".to_string()
    } else {
        out.chars().take(28).collect()
    }
}

pub fn make_qualified_tool_name(server_id: &str, tool_name: &str) -> String {
    let sid = sanitize_mcp_ident(server_id);
    let tid = sanitize_mcp_ident(tool_name);
    let full = format!("mcp__{sid}__{tid}");
    // OpenAI/Anthropic/Gemini enforce max 64 chars on tool names
    full.chars().take(64).collect()
}

/// Parses standard `{ "mcpServers": { "name": { ... } } }` or a single server map into `McpServerConfig` entries.
pub fn parse_mcp_servers_json(raw: &str) -> Result<Vec<McpServerConfig>, String> {
    let root: Value =
        serde_json::from_str(raw.trim()).map_err(|e| format!("Invalid JSON: {e}"))?;
    let map_obj = if let Some(m) = root.get("mcpServers").and_then(|v| v.as_object()) {
        m
    } else if let Some(m) = root.as_object() {
        m
    } else {
        return Err("Expected JSON object with `mcpServers` map.".to_string());
    };

    let mut out = Vec::new();
    for (name, val) in map_obj {
        let Some(obj) = val.as_object() else {
            continue;
        };
        let command = obj
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let url = obj
            .get("url")
            .or_else(|| obj.get("serverUrl"))
            .or_else(|| obj.get("endpoint"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let transport = if !url.is_empty() && command.is_empty() {
            "http".to_string()
        } else {
            obj.get("transport")
                .or_else(|| obj.get("type"))
                .and_then(|v| v.as_str())
                .map(|s| {
                    if s.to_ascii_lowercase().contains("http") || s.eq_ignore_ascii_case("sse") {
                        "http".to_string()
                    } else {
                        "stdio".to_string()
                    }
                })
                .unwrap_or_else(|| "stdio".to_string())
        };

        if transport == "stdio" && command.is_empty() {
            continue;
        }
        if transport == "http" && url.is_empty() {
            continue;
        }

        let args: Vec<String> = obj
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let env: HashMap<String, String> = obj
            .get("env")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        let headers: HashMap<String, String> = obj
            .get("headers")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        let enabled = obj
            .get("disabled")
            .and_then(|v| v.as_bool())
            .map(|d| !d)
            .unwrap_or(true);

        out.push(McpServerConfig {
            id: sanitize_mcp_ident(name),
            name: name.trim().to_string(),
            transport,
            command,
            args,
            env,
            url,
            headers,
            enabled,
            trusted: false,
        });
    }

    if out.is_empty() {
        return Err("No valid MCP servers found in JSON (need `command` for stdio or `url` for http).".to_string());
    }
    Ok(out)
}

impl McpManager {
    pub fn init(data_dir: &Path) -> Self {
        let config_path = data_dir.join("mcp_servers.json");
        let persisted: PersistedMcpStore = std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();

        Self {
            config_path,
            configs: Mutex::new(persisted.servers),
            runtimes: Mutex::new(HashMap::new()),
        }
    }

    fn persist_locked(path: &Path, servers: &[McpServerConfig]) {
        let store = PersistedMcpStore {
            servers: servers.to_vec(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&store) {
            let _ = std::fs::write(path, json);
        }
    }

    pub fn list_statuses(&self) -> Vec<McpServerStatus> {
        let cfgs = self
            .configs
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        let rts = self.runtimes.lock().ok();
        cfgs.into_iter()
            .map(|cfg| {
                let rt = rts.as_ref().and_then(|m| m.get(&cfg.id));
                let connected = rt.map(|r| r.client.is_some()).unwrap_or(false);
                let connecting = rt.map(|r| r.connecting).unwrap_or(false);
                let tools = rt.map(|r| r.tools.clone()).unwrap_or_default();
                let last_error = rt.and_then(|r| r.last_error.clone());
                McpServerStatus {
                    config: cfg,
                    connected,
                    connecting,
                    tool_count: tools.len(),
                    tools,
                    last_error,
                }
            })
            .collect()
    }

    pub fn list_active_tools(&self) -> Vec<McpToolInfo> {
        let enabled_ids: HashMap<String, bool> = self
            .configs
            .lock()
            .map(|g| g.iter().filter(|c| c.enabled).map(|c| (c.id.clone(), c.trusted)).collect())
            .unwrap_or_default();

        let Ok(rts) = self.runtimes.lock() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (id, rt) in rts.iter() {
            if let Some(&trusted) = enabled_ids.get(id) {
                if rt.client.is_some() {
                    for mut t in rt.tools.clone() {
                        t.server_trusted = trusted;
                        out.push(t);
                    }
                }
            }
        }
        out
    }

    pub fn find_tool(&self, qualified_name: &str) -> Option<McpToolInfo> {
        self.list_active_tools()
            .into_iter()
            .find(|t| t.qualified_name == qualified_name)
    }

    pub async fn connect_server(&self, server_id: &str) -> Result<McpServerStatus, String> {
        let cfg = self
            .configs
            .lock()
            .ok()
            .and_then(|g| g.iter().find(|c| c.id == server_id).cloned())
            .ok_or_else(|| format!("MCP server `{server_id}` not found."))?;

        self.disconnect_server(server_id);
        if let Ok(mut rts) = self.runtimes.lock() {
            let entry = rts.entry(server_id.to_string()).or_default();
            entry.connecting = true;
            entry.last_error = None;
        }

        let conn_res: Result<(Arc<McpClient>, Vec<McpRawTool>), String> = async {
            let client = if cfg.transport == "http" {
                Arc::new(McpClient::Http(
                    HttpMcpClient::connect(&cfg.url, cfg.headers.clone()).await?,
                ))
            } else {
                Arc::new(McpClient::Stdio(
                    StdioMcpClient::connect(&cfg.command, &cfg.args, &cfg.env).await?,
                ))
            };
            let raw_tools = client.list_tools().await?;
            Ok((client, raw_tools))
        }
        .await;

        match conn_res {
            Ok((client, raw_tools)) => {
                let mapped: Vec<McpToolInfo> = raw_tools
                    .into_iter()
                    .map(|t| McpToolInfo {
                        server_id: cfg.id.clone(),
                        server_name: cfg.name.clone(),
                        qualified_name: make_qualified_tool_name(&cfg.id, &t.name),
                        tool_name: t.name,
                        description: t.description,
                        input_schema: t.input_schema,
                        server_trusted: cfg.trusted,
                    })
                    .collect();

                if let Ok(mut rts) = self.runtimes.lock() {
                    rts.insert(
                        cfg.id.clone(),
                        LiveServerRuntime {
                            client: Some(client),
                            tools: mapped,
                            connecting: false,
                            last_error: None,
                        },
                    );
                }
                crate::logger::log_info(
                    "mcp",
                    &format!("connected server `{}` ({})", cfg.name, cfg.transport),
                );
            }
            Err(err) => {
                crate::logger::log_warn(
                    "mcp",
                    &format!("failed to connect server `{}`: {err}", cfg.name),
                );
                if let Ok(mut rts) = self.runtimes.lock() {
                    rts.insert(
                        cfg.id.clone(),
                        LiveServerRuntime {
                            client: None,
                            tools: Vec::new(),
                            connecting: false,
                            last_error: Some(err),
                        },
                    );
                }
            }
        }

        self.list_statuses()
            .into_iter()
            .find(|s| s.config.id == server_id)
            .ok_or_else(|| "MCP server disappeared.".to_string())
    }

    pub fn disconnect_server(&self, server_id: &str) {
        let old = self
            .runtimes
            .lock()
            .ok()
            .and_then(|mut rts| rts.remove(server_id));
        if let Some(rt) = old {
            if let Some(c) = rt.client {
                c.shutdown();
            }
        }
    }

    pub fn disconnect_all(&self) {
        let clients: Vec<Arc<McpClient>> = self
            .runtimes
            .lock()
            .ok()
            .map(|mut rts| {
                rts.drain()
                    .filter_map(|(_, rt)| rt.client)
                    .collect()
            })
            .unwrap_or_default();
        for c in clients {
            c.shutdown();
        }
    }

    pub async fn connect_all_enabled(self: &Arc<Self>) {
        let ids: Vec<String> = self
            .configs
            .lock()
            .map(|g| g.iter().filter(|c| c.enabled).map(|c| c.id.clone()).collect())
            .unwrap_or_default();
        for id in ids {
            let _ = self.connect_server(&id).await;
        }
    }

    pub async fn save_server(&self, mut cfg: McpServerConfig) -> Result<Vec<McpServerStatus>, String> {
        cfg.name = cfg.name.trim().to_string();
        if cfg.name.is_empty() {
            return Err("MCP server name cannot be empty.".to_string());
        }
        if cfg.id.trim().is_empty() {
            cfg.id = sanitize_mcp_ident(&cfg.name);
        } else {
            cfg.id = sanitize_mcp_ident(&cfg.id);
        }
        cfg.transport = if cfg.transport.eq_ignore_ascii_case("http") {
            "http".to_string()
        } else {
            "stdio".to_string()
        };
        if cfg.transport == "stdio" && cfg.command.trim().is_empty() {
            return Err("Stdio MCP server requires a command (e.g. npx, uvx, python).".to_string());
        }
        if cfg.transport == "http" && cfg.url.trim().is_empty() {
            return Err("HTTP MCP server requires an endpoint URL.".to_string());
        }

        let target_id = cfg.id.clone();
        let should_connect = cfg.enabled;
        if let Ok(mut list) = self.configs.lock() {
            if let Some(existing) = list.iter_mut().find(|c| c.id == target_id) {
                *existing = cfg;
            } else {
                list.push(cfg);
            }
            Self::persist_locked(&self.config_path, &list);
        }

        if should_connect {
            let _ = self.connect_server(&target_id).await;
        } else {
            self.disconnect_server(&target_id);
        }
        Ok(self.list_statuses())
    }

    pub async fn import_json(&self, raw: &str) -> Result<Vec<McpServerStatus>, String> {
        let parsed = parse_mcp_servers_json(raw)?;
        let mut to_connect = Vec::new();
        if let Ok(mut list) = self.configs.lock() {
            for cfg in parsed {
                if cfg.enabled {
                    to_connect.push(cfg.id.clone());
                }
                if let Some(existing) = list.iter_mut().find(|c| c.id == cfg.id) {
                    *existing = cfg;
                } else {
                    list.push(cfg);
                }
            }
            Self::persist_locked(&self.config_path, &list);
        }
        for id in to_connect {
            let _ = self.connect_server(&id).await;
        }
        Ok(self.list_statuses())
    }

    pub async fn toggle_server(&self, server_id: &str, enabled: bool) -> Result<Vec<McpServerStatus>, String> {
        if let Ok(mut list) = self.configs.lock() {
            let Some(item) = list.iter_mut().find(|c| c.id == server_id) else {
                return Err(format!("MCP server `{server_id}` not found."));
            };
            item.enabled = enabled;
            Self::persist_locked(&self.config_path, &list);
        }
        if enabled {
            let _ = self.connect_server(server_id).await;
        } else {
            self.disconnect_server(server_id);
        }
        Ok(self.list_statuses())
    }

    pub fn set_server_trusted(&self, server_id: &str, trusted: bool) -> Result<Vec<McpServerStatus>, String> {
        if let Ok(mut list) = self.configs.lock() {
            let Some(item) = list.iter_mut().find(|c| c.id == server_id) else {
                return Err(format!("MCP server `{server_id}` not found."));
            };
            item.trusted = trusted;
            Self::persist_locked(&self.config_path, &list);
        }
        Ok(self.list_statuses())
    }

    pub fn delete_server(&self, server_id: &str) -> Result<Vec<McpServerStatus>, String> {
        self.disconnect_server(server_id);
        if let Ok(mut list) = self.configs.lock() {
            list.retain(|c| c.id != server_id);
            Self::persist_locked(&self.config_path, &list);
        }
        Ok(self.list_statuses())
    }

    pub async fn call_tool(&self, qualified_name: &str, arguments: Value) -> Result<String, String> {
        let lookup: Option<(Arc<McpClient>, String)> = {
            let rts = self
                .runtimes
                .lock()
                .map_err(|_| "MCP runtime lock poisoned".to_string())?;
            let mut found = None;
            for rt in rts.values() {
                if let Some(tool) = rt.tools.iter().find(|t| t.qualified_name == qualified_name) {
                    if let Some(c) = &rt.client {
                        found = Some((Arc::clone(c), tool.tool_name.clone()));
                        break;
                    }
                }
            }
            found
        };

        let Some((client, raw_name)) = lookup else {
            return Err(format!("MCP tool `{qualified_name}` is not connected."));
        };
        let out = client.call_tool(&raw_name, arguments).await?;
        let capped: String = out.chars().take(16_000).collect();
        if out.chars().count() > 16_000 {
            Ok(format!("{capped}\n... [MCP output truncated]"))
        } else {
            Ok(capped)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::{format_tool_call_result, parse_http_mcp_body, parse_tools_list_result};
    use serde_json::json;

    #[test]
    fn parses_standard_mcp_servers_json_both_stdio_and_http() {
        let raw = r#"{
          "mcpServers": {
            "github": {
              "command": "npx",
              "args": ["-y", "@modelcontextprotocol/server-github"],
              "env": { "GITHUB_TOKEN": "ghp_test" }
            },
            "docs-api": {
              "url": "https://mcp.example.com/rpc",
              "headers": { "Authorization": "Bearer tok" }
            }
          }
        }"#;
        let parsed = parse_mcp_servers_json(raw).unwrap();
        assert_eq!(parsed.len(), 2);
        let gh = parsed.iter().find(|s| s.id == "github").unwrap();
        assert_eq!(gh.transport, "stdio");
        assert_eq!(gh.command, "npx");
        assert_eq!(gh.args.len(), 2);

        let http = parsed.iter().find(|s| s.id == "docs_api").unwrap();
        assert_eq!(http.transport, "http");
        assert_eq!(http.url, "https://mcp.example.com/rpc");
        assert_eq!(
            make_qualified_tool_name(&http.id, "search-docs"),
            "mcp__docs_api__search_docs"
        );
    }

    #[test]
    fn parses_sse_http_response_and_tool_call_blocks() {
        let sse = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"tools\":[{\"name\":\"list_issues\",\"description\":\"List repo issues\",\"inputSchema\":{\"type\":\"object\",\"properties\":{}}}]}}\n\n";
        let rpc = parse_http_mcp_body(sse, 7).unwrap();
        let tools = parse_tools_list_result(&rpc["result"]);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "list_issues");

        let call_ok = format_tool_call_result(&json!({
            "content": [{ "type": "text", "text": "Found 3 issues" }],
            "isError": false
        }))
        .unwrap();
        assert_eq!(call_ok, "Found 3 issues");
    }
}
