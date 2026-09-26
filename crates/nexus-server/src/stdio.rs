//! MCP stdio transport: newline-delimited JSON-RPC messages on stdin,
//! newline-delimited JSON-RPC responses on stdout. Logging goes to stderr so
//! it never corrupts the protocol stream.

use crate::app::McpApp;
use crate::rpc::handle_message;
use serde_json::Value;
use std::io::{self, BufRead, Write};

pub fn serve(app: &McpApp) -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let message: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(err) => {
                eprintln!("agent-nexus: dropping malformed JSON-RPC message: {err}");
                continue;
            }
        };

        if let Some(response) = handle_message(app, &message) {
            writeln!(out, "{response}")?;
            out.flush()?;
        }
    }

    Ok(())
}
