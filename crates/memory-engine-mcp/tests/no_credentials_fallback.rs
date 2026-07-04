use std::process::{Command, Stdio};

/// Mirrors `powder-mcp`'s `refuses_to_start_without_a_persistence_mode`: a
/// stdio MCP server that silently picked an account, or silently ran against
/// nothing, would leave an agent believing its study state persisted when it
/// did not. Prove instead that running the binary with no credential path
/// configured fails loudly before reading any stdin.
#[test]
fn refuses_to_start_without_any_credential_path() {
    let binary = env!("CARGO_BIN_EXE_memory-engine-mcp");

    let output = Command::new(binary)
        .env_remove("MEMORY_ENGINE_ACCOUNT_ID")
        .env_remove("MEMORY_ENGINE_SESSION_TOKEN")
        .env_remove("MEMORY_ENGINE_MCP_EMAIL")
        .env("MEMORY_ENGINE_HOME", unique_missing_home())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn memory-engine-mcp");

    assert!(
        !output.status.success(),
        "must exit non-zero with no credential path configured"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("MEMORY_ENGINE_ACCOUNT_ID") && stderr.contains("MEMORY_ENGINE_MCP_EMAIL"),
        "error must name the credential paths tried: {stderr}"
    );
    assert!(
        output.stdout.is_empty(),
        "must not emit any JSON-RPC output before exiting"
    );
}

fn unique_missing_home() -> String {
    std::env::temp_dir()
        .join(format!(
            "memory-engine-mcp-no-creds-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos())
        ))
        .display()
        .to_string()
}
