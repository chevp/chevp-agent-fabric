//! Sub-servers: external MCP servers that Agent Nexus mounts as tools.
//!
//! Heavy or domain-specific tooling (the asset factory, Blender drivers, ...)
//! lives in its own MCP server binary so it can run and be tested alone.
//! Agent Nexus spawns each server listed in `<repo root>/subservers.json`
//! (or `AGENT_NEXUS_SUBSERVERS`), asks it for `tools/list` once at startup,
//! and registers one proxy `Tool` per remote tool. `tools/call` is forwarded
//! over the child's stdio; a crashed child is restarted on the next call.
//!
//! ```json
//! { "servers": [ {
//!     "name": "asset-factory",
//!     "command": "../chevp-agent-fabric/target/release/asset-factory-mcp.exe",
//!     "args": [],
//!     "env": { "ASSET_FACTORY_ROOT": "G:/ft/icc-frost-lib/labs/a56-asset-factory.lab" },
//!     "prefix": "",
//!     "timeoutSecs": 120
//! } ] }
//! ```
//!
//! Relative `command`/`cwd` paths resolve against the config file's folder.

use nexus_tools::{Tool, ToolContext, ToolError};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PROTOCOL_VERSION: &str = "2024-11-05";
pub const CONFIG_FILE: &str = "subservers.json";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub name: String,
    pub command: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    /// Prepended to every remote tool name (e.g. "af_").
    #[serde(default)]
    pub prefix: String,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_timeout() -> u64 {
    120
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub servers: Vec<ServerConfig>,
}

/// Where the config lives: `AGENT_NEXUS_SUBSERVERS` or `<repo root>/subservers.json`.
pub fn config_path(repo_root: &Path) -> PathBuf {
    std::env::var("AGENT_NEXUS_SUBSERVERS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo_root.join(CONFIG_FILE))
}

/// Parses a config and resolves relative paths against `base`.
pub fn parse_config(text: &str, base: &Path) -> Result<Config, String> {
    let mut config: Config = serde_json::from_str(text).map_err(|e| e.to_string())?;
    for s in &mut config.servers {
        if s.command.is_relative() && s.command.components().count() > 1 {
            s.command = base.join(&s.command);
        }
        if let Some(cwd) = &s.cwd {
            if cwd.is_relative() {
                s.cwd = Some(base.join(cwd));
            }
        }
    }
    Ok(config)
}

/// Starts every enabled sub-server and returns its proxy tools. A server that
/// fails to start is reported on stderr and skipped; Agent Nexus keeps serving.
pub fn tools_from_config(repo_root: &Path) -> Vec<Box<dyn Tool>> {
    let path = config_path(repo_root);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let base = path.parent().unwrap_or(Path::new("."));
    let config = match parse_config(&text, base) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("agent-nexus: ignoring {}: {e}", path.display());
            return Vec::new();
        }
    };
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    for cfg in config.servers.into_iter().filter(|s| s.enabled) {
        let name = cfg.name.clone();
        match SubServer::start(cfg).and_then(|s| s.proxy_tools()) {
            Ok(list) => {
                eprintln!("agent-nexus: sub-server \"{name}\" mounted {} tools", list.len());
                tools.extend(list);
            }
            Err(e) => eprintln!("agent-nexus: sub-server \"{name}\" unavailable: {e}"),
        }
    }
    tools
}

struct Conn {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    next_id: u64,
}

impl Drop for Conn {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Conn {
    fn spawn(cfg: &ServerConfig) -> Result<Self, String> {
        let mut cmd = Command::new(&cfg.command);
        cmd.args(&cfg.args)
            .envs(&cfg.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(cwd) = &cfg.cwd {
            cmd.current_dir(cwd);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", cfg.command.display()))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let (tx, rx) = mpsc::channel();
        let label = cfg.name.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                match serde_json::from_str::<Value>(line.trim()) {
                    Ok(v) => {
                        if tx.send(v).is_err() {
                            break;
                        }
                    }
                    Err(_) if line.trim().is_empty() => {}
                    Err(e) => eprintln!("agent-nexus: sub-server \"{label}\" sent non-JSON: {e}"),
                }
            }
        });
        let mut conn = Conn { child, stdin, rx, next_id: 1 };
        let timeout = Duration::from_secs(cfg.timeout_secs.max(1));
        conn.request("initialize", json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "agent-nexus", "version": env!("CARGO_PKG_VERSION") }
        }), timeout)?;
        conn.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))?;
        Ok(conn)
    }

    fn send(&mut self, msg: &Value) -> Result<(), String> {
        writeln!(self.stdin, "{msg}")
            .and_then(|_| self.stdin.flush())
            .map_err(|e| format!("write to sub-server failed: {e}"))
    }

    fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(msg) if msg.get("id") == Some(&json!(id)) => {
                    if let Some(err) = msg.get("error") {
                        return Err(format!("{method}: {}", err["message"].as_str().unwrap_or("error")));
                    }
                    return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
                }
                Ok(_) => continue, // notification or stale response
                Err(RecvTimeoutError::Timeout) => return Err(format!("{method}: timed out after {timeout:?}")),
                Err(RecvTimeoutError::Disconnected) => return Err(format!("{method}: sub-server exited")),
            }
        }
    }
}

pub struct SubServer {
    cfg: ServerConfig,
    conn: Mutex<Option<Conn>>,
}

impl SubServer {
    pub fn start(cfg: ServerConfig) -> Result<Arc<Self>, String> {
        let conn = Conn::spawn(&cfg)?;
        Ok(Arc::new(Self { cfg, conn: Mutex::new(Some(conn)) }))
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(self.cfg.timeout_secs.max(1))
    }

    /// Sends a request, restarting the child once if it died or hung.
    pub fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let mut guard = self.conn.lock().map_err(|_| "sub-server lock poisoned".to_string())?;
        if guard.is_none() {
            *guard = Some(Conn::spawn(&self.cfg)?);
        }
        let first = guard.as_mut().expect("set above").request(method, params.clone(), self.timeout());
        match first {
            Ok(v) => Ok(v),
            Err(e) if e.ends_with("exited") || e.contains("write to sub-server") => {
                *guard = None;
                let mut conn = Conn::spawn(&self.cfg)?;
                let out = conn.request(method, params, self.timeout());
                *guard = Some(conn);
                out
            }
            Err(e) => {
                // A timed-out child may still answer later; start clean next time.
                *guard = None;
                Err(e)
            }
        }
    }

    pub fn proxy_tools(self: Arc<Self>) -> Result<Vec<Box<dyn Tool>>, String> {
        let list = self.request("tools/list", json!({}))?;
        let tools = list["tools"].as_array().cloned().unwrap_or_default();
        Ok(tools
            .into_iter()
            .filter_map(|t| {
                let remote = t["name"].as_str()?.to_string();
                Some(Box::new(ProxyTool {
                    exposed: format!("{}{remote}", self.cfg.prefix),
                    description: format!(
                        "[{}] {}",
                        self.cfg.name,
                        t["description"].as_str().unwrap_or_default()
                    ),
                    schema: t.get("inputSchema").cloned().unwrap_or_else(|| json!({ "type": "object" })),
                    remote,
                    server: Arc::clone(&self),
                }) as Box<dyn Tool>)
            })
            .collect())
    }
}

pub struct ProxyTool {
    exposed: String,
    remote: String,
    description: String,
    schema: Value,
    server: Arc<SubServer>,
}

impl Tool for ProxyTool {
    fn name(&self) -> &str {
        &self.exposed
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        self.schema.clone()
    }

    fn call(&self, _ctx: &ToolContext<'_>, input: Value) -> Result<Value, ToolError> {
        let result = self
            .server
            .request("tools/call", json!({ "name": self.remote, "arguments": input }))
            .map_err(ToolError::Other)?;
        unwrap_call_result(&result)
    }
}

/// MCP `tools/call` result -> tool value. Text content that is JSON is
/// returned as JSON (Agent Nexus re-serialises it), anything else as string.
pub fn unwrap_call_result(result: &Value) -> Result<Value, ToolError> {
    let text: Vec<&str> = result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["type"] == "text")
        .filter_map(|c| c["text"].as_str())
        .collect();
    let text = text.join("\n");
    if result["isError"] == json!(true) {
        return Err(ToolError::Other(text));
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_resolves_relative_paths_against_its_folder() {
        let base = Path::new("/cfg");
        let c = parse_config(
            r#"{ "servers": [
                { "name": "a", "command": "bin/a.exe", "cwd": "work" },
                { "name": "b", "command": "b-on-path", "enabled": false, "prefix": "b_" } ] }"#,
            base,
        )
        .unwrap();
        assert_eq!(c.servers[0].command, base.join("bin/a.exe"));
        assert_eq!(c.servers[0].cwd.as_deref(), Some(base.join("work").as_path()));
        assert_eq!(c.servers[0].timeout_secs, 120);
        assert_eq!(c.servers[1].command, PathBuf::from("b-on-path"));
        assert!(!c.servers[1].enabled);
    }

    #[test]
    fn json_text_content_becomes_json() {
        let v = unwrap_call_result(&json!({ "content": [{ "type": "text", "text": "{\"a\":1}" }] })).unwrap();
        assert_eq!(v, json!({ "a": 1 }));
        let v = unwrap_call_result(&json!({ "content": [{ "type": "text", "text": "plain" }] })).unwrap();
        assert_eq!(v, json!("plain"));
    }

    #[test]
    fn remote_errors_become_tool_errors() {
        let r = unwrap_call_result(&json!({ "content": [{ "type": "text", "text": "boom" }], "isError": true }));
        assert!(matches!(r, Err(ToolError::Other(m)) if m == "boom"));
    }

    #[test]
    fn missing_config_mounts_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::env::remove_var("AGENT_NEXUS_SUBSERVERS");
        assert!(tools_from_config(dir.path()).is_empty());
    }

    #[test]
    fn unstartable_server_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(CONFIG_FILE),
            r#"{ "servers": [ { "name": "ghost", "command": "./does-not-exist.exe" } ] }"#,
        )
        .unwrap();
        std::env::remove_var("AGENT_NEXUS_SUBSERVERS");
        assert!(tools_from_config(dir.path()).is_empty());
    }
}
