//! Fake Lightpanda native MCP child for browser integration tests.
//!
//! Speaks newline-delimited JSON-RPC over stdio, mimicking the pinned
//! Lightpanda 0.3.7 MCP surface. Mode is `argv[1]`:
//!
//! ```text
//! normal        full required tool set + admin extras; calls echo their params
//! missing-tool  omits waitForScript from discovery
//! malformed     answers tools/list with non-JSON garbage
//! crash         exits right after the initialize handshake
//! delay <ms>    tools/call sleeps <ms> before answering
//! noisy-stderr  emits sensitive-looking stderr lines at startup
//! ```
use std::io::{BufRead, Write};
use std::time::Duration;

use serde_json::{json, Value};

const REQUIRED_TOOLS: &[&str] = &[
    "goto",
    "getUrl",
    "getCookies",
    "consoleLogs",
    "tree",
    "markdown",
    "html",
    "nodeDetails",
    "findElement",
    "interactiveElements",
    "links",
    "extract",
    "structuredData",
    "search",
    "click",
    "hover",
    "fill",
    "selectOption",
    "setChecked",
    "press",
    "scroll",
    "detectForms",
    "evaluate",
    "waitForSelector",
    "waitForState",
    "waitForScript",
];

const ADMIN_EXTRAS: &[&str] = &[
    "getEnv",
    "save",
    "session_new",
    "session_list",
    "session_close",
];

fn tool_entries(missing_required: bool) -> Vec<Value> {
    let mut tools: Vec<Value> = REQUIRED_TOOLS
        .iter()
        .filter(|name| !(missing_required && **name == "waitForScript"))
        .map(|name| json!({ "name": name, "inputSchema": { "type": "object" } }))
        .collect();
    for name in ADMIN_EXTRAS {
        tools.push(json!({ "name": name, "inputSchema": { "type": "object" } }));
    }
    tools
}

fn send(stdout: &mut std::io::Stdout, value: &Value) -> bool {
    let mut line = serde_json::to_string(value).unwrap();
    line.push('\n');
    stdout.write_all(line.as_bytes()).is_ok() && stdout.flush().is_ok()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("normal");
    let delay_ms: u64 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);

    if mode == "noisy-stderr" {
        eprintln!("cookie file /var/lib/local-workspaces/browser/cookies/page-1.json loaded");
        eprintln!("authorization: Bearer secret-token-abc123");
    }

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { return };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let method = value
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or_default()
            .to_string();
        let Some(id) = value.get("id").cloned() else {
            continue; // notification
        };
        match method.as_str() {
            "initialize" => {
                if !send(
                    &mut stdout,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "protocolVersion": "2025-03-26",
                            "capabilities": {},
                            "serverInfo": { "name": "fake-lightpanda", "version": "0.3.7" }
                        }
                    }),
                ) {
                    return;
                }
                if mode == "crash" {
                    return; // clean EOF right after the handshake
                }
            }
            "tools/list" => {
                if mode == "malformed" {
                    let _ = stdout.write_all(b"this is not json-rpc\n");
                    let _ = stdout.flush();
                    continue;
                }
                if !send(
                    &mut stdout,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "tools": tool_entries(mode == "missing-tool") }
                    }),
                ) {
                    return;
                }
            }
            "tools/call" => {
                if mode == "delay" {
                    std::thread::sleep(Duration::from_millis(delay_ms));
                }
                let params = value.get("params").cloned().unwrap_or_else(|| json!({}));
                if !send(
                    &mut stdout,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{ "type": "text", "text": params.to_string() }],
                            "isError": false
                        }
                    }),
                ) {
                    return;
                }
            }
            _ => {
                if !send(
                    &mut stdout,
                    &json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
                ) {
                    return;
                }
            }
        }
    }
}
