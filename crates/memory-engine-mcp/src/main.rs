//! `memory-engine-mcp` — stdio JSON-RPC MCP server over `memory-engine-api`.
//!
//! Reads one JSON-RPC request per line from stdin, writes one JSON-RPC
//! response per line to stdout. See `docs/dogfood/mcp-review-loop.md` for a
//! captured transcript and `crates/memory-engine-mcp/src/lib.rs` for the
//! tool contract.

use std::io::{self, BufRead, Write};

use memory_engine_mcp::client::MemoryEngineClient;
use memory_engine_mcp::session;
use serde_json::Value;

fn main() {
    let session = match session::resolve() {
        Ok(session) => session,
        Err(error) => {
            eprintln!("memory-engine-mcp: {error}");
            std::process::exit(1);
        }
    };

    eprintln!(
        "memory-engine-mcp: ready (account {}, base url {})",
        session.account_id, session.base_url
    );

    let client =
        MemoryEngineClient::new(session.base_url, session.account_id, session.session_token);

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }

        let request = match serde_json::from_str::<Value>(&line) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("memory-engine-mcp: invalid json: {error}");
                continue;
            }
        };

        if let Some(response) = memory_engine_mcp::handle_json_rpc(&client, &request) {
            if let Ok(line) = serde_json::to_string(&response) {
                let _ = writeln!(stdout, "{line}");
                let _ = stdout.flush();
            }
        }
    }
}
