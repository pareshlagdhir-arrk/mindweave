//! The Model Context Protocol over stdio: newline-delimited JSON-RPC 2.0
//! messages, one per line, on stdin and stdout.
//!
//! Handles `initialize`, `ping`, `tools/list` and `tools/call`;
//! notifications (such as `notifications/initialized`) need no answer.
//! Protocol problems are JSON-RPC errors; a tool that fails answers with
//! a result marked `isError` whose text says what went wrong.

use std::io::{BufRead, Write};

use serde_json::{json, Value};

use crate::tools::{Context, ToolError};

/// Protocol versions the server speaks, newest first.
pub const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
/// The longest message accepted, in bytes.
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// What the server tells clients about itself on `initialize`.
const INSTRUCTIONS: &str = "Mindweave edits .mindweave files (mind maps, diagrams, artboards). \
Every tool takes `path`, an absolute path to the file; each call opens, changes and saves it. \
Work in small steps: create_document or document_summary, build with add_mind_map / add_topics \
or add_shape / add_text / connect, then call export_png to look at the result and fix what is off. \
Ids come back from the tools that create objects and from list_objects.";

/// A JSON-RPC error response to `id`.
fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

/// A JSON-RPC result response to `id`.
fn result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// The server: tool context and whether the client initialised.
pub struct Server {
    ctx: Context,
}

impl Server {
    /// A server using `ctx`.
    pub fn new(ctx: Context) -> Server {
        Server { ctx }
    }

    /// Answers one line of input; `None` for notifications and blank
    /// lines.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        if line.trim().is_empty() {
            return None;
        }
        if line.len() > MAX_MESSAGE_BYTES {
            return Some(error(Value::Null, INVALID_REQUEST, "Message too large.").to_string());
        }
        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                return Some(
                    error(Value::Null, PARSE_ERROR, format!("Parse error: {e}")).to_string(),
                )
            }
        };
        let reply = match msg {
            Value::Array(batch) => {
                let replies: Vec<Value> =
                    batch.into_iter().filter_map(|m| self.handle(m)).collect();
                (!replies.is_empty()).then(|| Value::Array(replies))
            }
            m => self.handle(m),
        };
        reply.map(|r| r.to_string())
    }

    /// Answers one message; `None` for notifications and responses.
    fn handle(&mut self, msg: Value) -> Option<Value> {
        let Some(obj) = msg.as_object() else {
            return Some(error(
                Value::Null,
                INVALID_REQUEST,
                "Expected a JSON-RPC object.",
            ));
        };
        let method = obj.get("method").and_then(Value::as_str);
        let id = obj.get("id").cloned();
        let (Some(method), Some(id)) = (method, id) else {
            // A notification, or a response to a request we never send.
            if method.is_none() && obj.get("id").is_none() {
                return Some(error(Value::Null, INVALID_REQUEST, "Missing method."));
            }
            return None;
        };
        let params = obj.get("params").cloned().unwrap_or(Value::Null);
        Some(match method {
            "initialize" => result(id, initialize(&params)),
            "ping" => result(id, json!({})),
            "tools/list" => result(id, json!({"tools": self.ctx.tool_list()})),
            "tools/call" => {
                let Some(name) = params.get("name").and_then(Value::as_str) else {
                    return Some(error(id, INVALID_PARAMS, "tools/call needs a tool `name`."));
                };
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                match self.ctx.call(name, &args) {
                    Ok(content) => result(id, json!({"content": content, "isError": false})),
                    Err(ToolError::UnknownTool) => {
                        error(id, INVALID_PARAMS, format!("Unknown tool: {name}"))
                    }
                    Err(ToolError::Failed(message)) => result(
                        id,
                        json!({"content": [{"type": "text", "text": message}], "isError": true}),
                    ),
                }
            }
            m if m.starts_with("notifications/") => return None,
            _ => error(id, METHOD_NOT_FOUND, format!("Method not found: {method}")),
        })
    }

    /// Serves `input` until it ends, writing replies to `output`.
    pub fn run(&mut self, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if let Some(reply) = self.handle_line(&line) {
                writeln!(output, "{reply}")?;
                output.flush()?;
            }
        }
        Ok(())
    }
}

/// The `initialize` result: the client's protocol version when the
/// server speaks it, else the newest the server speaks.
fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let version = asked
        .filter(|v| PROTOCOL_VERSIONS.contains(v))
        .unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": "mindweave", "title": "Mindweave", "version": env!("CARGO_PKG_VERSION")},
        "instructions": INSTRUCTIONS,
    })
}
