//! `notion-mcp` — a localhost Model Context Protocol bridge.
//!
//! Runs a tiny synchronous HTTP server on `127.0.0.1:8787` (override with
//! `NOTION_MCP_ADDR`) that speaks JSON-RPC 2.0. Every `POST /` body is handed to
//! the tested [`open_notebook_core::mcp`] dispatcher, which exposes three tools —
//! `search_notes`, `create_page`, `add_event` — over the SAME encrypted database
//! the desktop app uses. That lets external clients (Claude Desktop, Cursor, a
//! browser extension) read and edit your notes locally.
//!
//! Security: it binds ONLY to loopback and takes the SQLCipher key from the GNOME
//! Keyring (or `NOTION_SQLCIPHER_KEY_HEX`), never the DEK root. There is no auth
//! beyond loopback + the OS keyring, so do not expose the port off-host.

use std::net::ToSocketAddrs;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use notion_companion::keyring::{EnvKeyProvider, KeyProvider, SecretServiceKeyProvider};
use notion_companion::paths;
use open_notebook_core::storage::SqliteStorage;
use open_notebook_core::{mcp, HashingEmbedder, MemoryService};

const DEFAULT_ADDR: &str = "127.0.0.1:8787";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("notion-mcp: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let addr = std::env::var("NOTION_MCP_ADDR")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_ADDR.to_string());
    // Refuse to bind anything but loopback: this server has no auth.
    if !is_loopback(&addr) {
        return Err(format!(
            "refusing to bind non-loopback address `{addr}` (this server has no auth)"
        ));
    }

    let db_path = match std::env::var("NOTION_DB_PATH") {
        Ok(p) if !p.is_empty() => p,
        _ => paths::db_path()
            .ok_or("cannot resolve the app-data directory (set NOTION_DB_PATH)")?
            .to_string_lossy()
            .into_owned(),
    };
    let key = resolve_key()?;
    let storage =
        SqliteStorage::open(&db_path, &key).map_err(|e| format!("open DB at {db_path}: {e}"))?;
    let memory = MemoryService::new(HashingEmbedder::default());

    let server = tiny_http::Server::http(&addr).map_err(|e| format!("bind {addr}: {e}"))?;
    eprintln!("notion-mcp listening on http://{addr}  (POST JSON-RPC 2.0)");

    for mut request in server.incoming_requests() {
        use tiny_http::Method;
        let response = match request.method() {
            Method::Post => {
                let mut body = String::new();
                if request.as_reader().read_to_string(&mut body).is_err() {
                    json_response(&mcp_parse_error("could not read request body"))
                } else {
                    let result = mcp::handle_str(&storage, &memory, &body, now_secs());
                    json_response(&result)
                }
            }
            Method::Get => json_response(&serde_json::json!({
                "server": "notion-mcp",
                "protocol": mcp::PROTOCOL_VERSION,
                "usage": "POST a JSON-RPC 2.0 request; method tools/list or tools/call",
                "tools": mcp::tool_definitions().iter().map(|t| &t["name"]).collect::<Vec<_>>(),
            })),
            _ => tiny_http::Response::from_string("method not allowed").with_status_code(405),
        };
        let _ = request.respond(response);
    }
    Ok(())
}

fn json_response(value: &serde_json::Value) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let body = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    let header =
        tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    tiny_http::Response::from_string(body).with_header(header)
}

fn mcp_parse_error(msg: &str) -> serde_json::Value {
    serde_json::json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": msg}})
}

/// Whether `addr` binds only loopback. We **resolve** the address the same way
/// `tiny_http` will (`ToSocketAddrs`) and require *every* resolved socket to be
/// loopback. A purely textual check (e.g. `starts_with("127.")`) is unsafe: a
/// hostname like `127.0.0.1.attacker.example` passes a string test but resolves
/// to a public IP, so the server would bind off-host despite having no auth.
fn is_loopback(addr: &str) -> bool {
    match addr.to_socket_addrs() {
        Ok(resolved) => {
            let addrs: Vec<_> = resolved.collect();
            !addrs.is_empty() && addrs.iter().all(|a| a.ip().is_loopback())
        }
        // Unresolvable / missing port ⇒ refuse (fail closed).
        Err(_) => false,
    }
}

fn resolve_key() -> Result<String, String> {
    for provider in [
        &EnvKeyProvider as &dyn KeyProvider,
        &SecretServiceKeyProvider,
    ] {
        match provider.sqlcipher_key_hex() {
            Ok(Some(k)) => return Ok(k.as_str().to_string()),
            Ok(None) => continue,
            Err(e) => return Err(format!("keyring: {e}")),
        }
    }
    Err("vault is locked — unlock the desktop app once, or set NOTION_SQLCIPHER_KEY_HEX".into())
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::is_loopback;

    #[test]
    fn accepts_loopback_literals() {
        assert!(is_loopback("127.0.0.1:8787"));
        assert!(is_loopback("[::1]:8787"));
    }

    #[test]
    fn rejects_non_loopback_and_all_interfaces() {
        // The all-interfaces bind and any routable literal must be refused.
        assert!(!is_loopback("0.0.0.0:8787"));
        assert!(!is_loopback("192.168.1.5:8787"));
        assert!(!is_loopback("8.8.8.8:8787"));
        // Missing port / unresolvable ⇒ fail closed.
        assert!(!is_loopback("127.0.0.1"));
        assert!(!is_loopback("not a host:8787"));
    }
}
