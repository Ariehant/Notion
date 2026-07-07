//! Model Context Protocol (MCP) surface: the tool registry and a JSON-RPC 2.0
//! dispatcher that external clients (Claude Desktop, Cursor, …) call to search
//! and edit the vault.
//!
//! The transport (a localhost HTTP server) is the thin `notion-mcp` binary; all
//! the logic — method routing, argument validation, tool execution against the
//! injected [`NotebookStorage`] — lives here so it is unit-tested without a
//! socket. Three tools are exposed: `search_notes`, `create_page`, `add_event`.

use serde_json::{json, Value};

use crate::embedding::{fnv1a64, Embedder};
use crate::memory::MemoryService;
use crate::storage::{NotebookCalendarEvent, NotebookStorage};

/// JSON-RPC error codes we emit (subset of the spec).
mod code {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
}

/// The MCP protocol version this server implements.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// The tool definitions advertised via `tools/list`.
pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "search_notes",
            "description": "Semantic + keyword search over the user's notes. Returns matching source ids and scores.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 50}
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "create_page",
            "description": "Create a new page in the notebook.",
            "inputSchema": {
                "type": "object",
                "properties": {"title": {"type": "string"}},
                "required": ["title"]
            }
        }),
        json!({
            "name": "add_event",
            "description": "Add a calendar event. Timestamps are Unix seconds (UTC).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "start_time": {"type": "integer"},
                    "end_time": {"type": "integer"},
                    "all_day": {"type": "boolean"},
                    "location": {"type": "string"}
                },
                "required": ["title", "start_time"]
            }
        }),
    ]
}

/// Handle one JSON-RPC request object and return the response object.
///
/// Never panics on bad input — malformed requests map to JSON-RPC error
/// responses. `now` is injected (Unix seconds) for deterministic id generation.
pub fn handle_request<E: Embedder>(
    store: &dyn NotebookStorage,
    memory: &MemoryService<E>,
    req: &Value,
    now: i64,
) -> Value {
    let id = req.get("id").cloned().unwrap_or(Value::Null);

    let Some(method) = req.get("method").and_then(Value::as_str) else {
        return err(id, code::INVALID_REQUEST, "missing `method`");
    };

    match method {
        "initialize" => ok(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "serverInfo": {"name": "notion-mcp", "version": env!("CARGO_PKG_VERSION")},
                "capabilities": {"tools": {}}
            }),
        ),
        "tools/list" => ok(id, json!({ "tools": tool_definitions() })),
        "tools/call" => {
            let params = req.get("params").cloned().unwrap_or(Value::Null);
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match call_tool(store, memory, name, &args, now) {
                Ok(result) => ok(id, result),
                Err(e) => err(id, e.0, &e.1),
            }
        }
        other => err(
            id,
            code::METHOD_NOT_FOUND,
            &format!("unknown method: {other}"),
        ),
    }
}

/// Convenience: parse a raw JSON string into a request and dispatch it. Returns
/// a JSON-RPC parse error on malformed JSON.
pub fn handle_str<E: Embedder>(
    store: &dyn NotebookStorage,
    memory: &MemoryService<E>,
    raw: &str,
    now: i64,
) -> Value {
    match serde_json::from_str::<Value>(raw) {
        Ok(req) => handle_request(store, memory, &req, now),
        Err(e) => err(Value::Null, code::PARSE_ERROR, &e.to_string()),
    }
}

type ToolErr = (i64, String);

fn call_tool<E: Embedder>(
    store: &dyn NotebookStorage,
    memory: &MemoryService<E>,
    name: &str,
    args: &Value,
    now: i64,
) -> Result<Value, ToolErr> {
    match name {
        "search_notes" => {
            let query = str_arg(args, "query")?;
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(10)
                .clamp(1, 50) as usize;
            let hits = memory
                .search(store, &query, limit)
                .map_err(|e| (code::INTERNAL_ERROR, e.to_string()))?;
            let results: Vec<Value> = hits
                .into_iter()
                .map(|h| json!({"sourceId": h.source_block_id, "score": h.score}))
                .collect();
            Ok(text_result(
                &format!("{} result(s)", results.len()),
                json!({ "results": results }),
            ))
        }
        "create_page" => {
            let title = str_arg(args, "title")?;
            let page_id = gen_id("page", &title, now);
            store
                .create_page(&page_id, &title, now * 1000)
                .map_err(|e| (code::INTERNAL_ERROR, e.to_string()))?;
            Ok(text_result(
                &format!("Created page “{title}”"),
                json!({ "pageId": page_id }),
            ))
        }
        "add_event" => {
            let title = str_arg(args, "title")?;
            let start_time = int_arg(args, "start_time")?;
            // Saturating: start_time is caller-supplied, so `+ 3600` must not
            // overflow on an extreme value.
            let end_time = args
                .get("end_time")
                .and_then(Value::as_i64)
                .filter(|&e| e > start_time)
                .unwrap_or(start_time.saturating_add(3600));
            let all_day = args
                .get("all_day")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let location = args
                .get("location")
                .and_then(Value::as_str)
                .map(String::from)
                .filter(|s| !s.trim().is_empty());
            let event_id = gen_id("event", &format!("{title}:{start_time}"), now);
            store
                .add_calendar_event(&NotebookCalendarEvent {
                    id: event_id.clone(),
                    title: title.clone(),
                    start_time,
                    end_time,
                    all_day,
                    location,
                    description: None,
                    last_modified: now,
                })
                .map_err(|e| (code::INTERNAL_ERROR, e.to_string()))?;
            Ok(text_result(
                &format!("Added event “{title}”"),
                json!({ "eventId": event_id }),
            ))
        }
        "" => Err((code::INVALID_PARAMS, "missing tool `name`".into())),
        other => Err((code::METHOD_NOT_FOUND, format!("unknown tool: {other}"))),
    }
}

// --- helpers ---

fn str_arg(args: &Value, key: &str) -> Result<String, ToolErr> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| (code::INVALID_PARAMS, format!("missing/empty `{key}`")))
}

fn int_arg(args: &Value, key: &str) -> Result<i64, ToolErr> {
    args.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| (code::INVALID_PARAMS, format!("missing integer `{key}`")))
}

/// Deterministic id from a namespace, payload, and the injected clock.
fn gen_id(ns: &str, payload: &str, now: i64) -> String {
    format!(
        "{ns}:{:016x}",
        fnv1a64(format!("{payload}:{now}").as_bytes())
    )
}

/// The MCP `tools/call` result envelope: a human-readable text block plus a
/// machine-readable `structuredContent`.
fn text_result(text: &str, structured: Value) -> Value {
    json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": structured,
        "isError": false
    })
}

fn ok(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn err(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::HashingEmbedder;
    use crate::storage::MemStorage;

    fn memory() -> MemoryService<HashingEmbedder> {
        MemoryService::new(HashingEmbedder::default())
    }

    #[test]
    fn tools_list_returns_three_tools() {
        let store = MemStorage::new();
        let mem = memory();
        let resp = handle_str(
            &store,
            &mem,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            0,
        );
        let tools = resp["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 3);
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"search_notes"));
        assert!(names.contains(&"add_event"));
    }

    #[test]
    fn initialize_reports_protocol_version() {
        let store = MemStorage::new();
        let mem = memory();
        let resp = handle_request(
            &store,
            &mem,
            &json!({"jsonrpc":"2.0","id":9,"method":"initialize"}),
            0,
        );
        assert_eq!(resp["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(resp["id"], 9);
    }

    #[test]
    fn add_event_tool_writes_to_calendar() {
        let store = MemStorage::new();
        let mem = memory();
        let resp = handle_request(
            &store,
            &mem,
            &json!({
                "jsonrpc":"2.0","id":2,"method":"tools/call",
                "params":{"name":"add_event","arguments":{"title":"Sync","start_time":1000}}
            }),
            500,
        );
        assert_eq!(resp["result"]["isError"], false);
        assert_eq!(store.event_count(), 1);
        let ev = &store.events()[0];
        assert_eq!(ev.title, "Sync");
        assert_eq!(ev.end_time, 1000 + 3600); // defaulted
    }

    #[test]
    fn create_page_tool_inserts_page() {
        let store = MemStorage::new();
        let mem = memory();
        let resp = handle_request(
            &store,
            &mem,
            &json!({
                "jsonrpc":"2.0","id":3,"method":"tools/call",
                "params":{"name":"create_page","arguments":{"title":"Ideas"}}
            }),
            1,
        );
        assert!(resp["result"]["structuredContent"]["pageId"].is_string());
        assert_eq!(store.page_count(), 1);
    }

    #[test]
    fn search_notes_tool_finds_indexed_content() {
        let store = MemStorage::new();
        let mem = memory();
        mem.index(&store, "b1", "Invoices", "the acme invoice is overdue", 1)
            .unwrap();
        let resp = handle_request(
            &store,
            &mem,
            &json!({
                "jsonrpc":"2.0","id":4,"method":"tools/call",
                "params":{"name":"search_notes","arguments":{"query":"overdue invoice"}}
            }),
            1,
        );
        let results = resp["result"]["structuredContent"]["results"]
            .as_array()
            .unwrap();
        assert_eq!(results[0]["sourceId"], "b1");
    }

    #[test]
    fn unknown_method_and_tool_error_cleanly() {
        let store = MemStorage::new();
        let mem = memory();
        let r1 = handle_request(
            &store,
            &mem,
            &json!({"jsonrpc":"2.0","id":1,"method":"nope"}),
            0,
        );
        assert_eq!(r1["error"]["code"], code::METHOD_NOT_FOUND);

        let r2 = handle_request(
            &store,
            &mem,
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"frobnicate"}}),
            0,
        );
        assert_eq!(r2["error"]["code"], code::METHOD_NOT_FOUND);
    }

    #[test]
    fn missing_required_arg_is_invalid_params() {
        let store = MemStorage::new();
        let mem = memory();
        let resp = handle_request(
            &store,
            &mem,
            &json!({
                "jsonrpc":"2.0","id":1,"method":"tools/call",
                "params":{"name":"add_event","arguments":{"title":"NoStart"}}
            }),
            0,
        );
        assert_eq!(resp["error"]["code"], code::INVALID_PARAMS);
        assert_eq!(store.event_count(), 0);
    }

    #[test]
    fn malformed_json_is_parse_error() {
        let store = MemStorage::new();
        let mem = memory();
        let resp = handle_str(&store, &mem, "{not json", 0);
        assert_eq!(resp["error"]["code"], code::PARSE_ERROR);
    }
}
