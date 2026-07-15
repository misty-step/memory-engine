#!/usr/bin/env bash
set -euo pipefail

# Cerberus b10bffb6 has no standalone key-lifecycle CLI. This short-lived
# trusted helper calls its reviewed public API instead of keeping the
# provisioning key in the Cerberus review process while target code runs.
: "${1:?usage: $0 mint|revoke STATE_DIR TAG}" # only the trusted workflow calls this
mode="$1"
state_dir="${2:?usage: $0 mint|revoke STATE_DIR TAG}"
tag="${3:-}"
: "${CERBERUS_SOURCE_DIR:?fresh pinned Cerberus checkout is required}"

cerberus_revision='b10bffb6ddb14ec553fbcf4f5e687aee13424717'
test "$(git -C "$CERBERUS_SOURCE_DIR" rev-parse HEAD)" = "$cerberus_revision"
test -s "$CERBERUS_SOURCE_DIR/Cargo.lock"
test -f "$CERBERUS_SOURCE_DIR/Cargo.toml"
test -d "$CERBERUS_SOURCE_DIR/src"

umask 077
mkdir -p "$state_dir"
chmod 700 "$state_dir"
temporary_dir="$(mktemp -d "${TMPDIR:-/tmp}/generation-061-key.XXXXXX")"
temporary_bin="$CERBERUS_SOURCE_DIR/src/bin/memory-engine-061-key-helper.rs"
created_bin_dir=false
if [[ ! -d "$CERBERUS_SOURCE_DIR/src/bin" ]]; then
  created_bin_dir=true
fi
cleanup() {
  rm -f -- "$temporary_bin"
  if [[ "$created_bin_dir" == true ]]; then
    rmdir -- "$CERBERUS_SOURCE_DIR/src/bin" 2>/dev/null || true
  fi
  rm -rf -- "$temporary_dir"
}
trap cleanup EXIT INT TERM
mkdir -p "$CERBERUS_SOURCE_DIR/src/bin"

cat > "$temporary_bin" <<'EOF'
use std::{env, fs, io::Write, path::Path, time::Duration};

use cerberus::{mint_review_key, ProvisioningClient};

fn write_private(path: &Path, value: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(value.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let mode = args.next().ok_or("missing key lifecycle mode")?;
    let state_dir = args.next().ok_or("missing key lifecycle state directory")?;
    let tag = args.next().unwrap_or_default();
    let provisioning_key = env::var("CERBERUS_OPENROUTER_PROVISIONING_KEY")
        .map_err(|_| "CERBERUS_OPENROUTER_PROVISIONING_KEY is missing")?;
    let client = ProvisioningClient::new(provisioning_key);
    let state = Path::new(&state_dir);

    match mode.as_str() {
        "mint" => {
            if tag.is_empty() {
                return Err("missing key tag".into());
            }
            let minted = mint_review_key(&client, &tag, 2.0, Duration::from_secs(1800))?;
            // Persist the hash first: a later secret-write failure still leaves
            // enough authority for the always-run revoke step to clean up.
            write_private(&state.join("scoped-hash"), &minted.hash)?;
            write_private(&state.join("scoped-key"), &minted.secret)?;
        }
        "revoke" => {
            let hash = fs::read_to_string(state.join("scoped-hash"))?;
            client.revoke_key(hash.trim())?;
        }
        _ => return Err("unknown key lifecycle mode".into()),
    }
    Ok(())
}

fn main() {
    if run().is_err() {
        // Never format or print an error that could contain credential bytes.
        eprintln!("trusted Cerberus key lifecycle operation failed");
        std::process::exit(1);
    }
}
EOF

cargo_bin="${CARGO_HOME:-}/bin/cargo"
if [[ ! -x "$cargo_bin" ]]; then
  cargo_bin="$(command -v cargo)"
fi
test -x "$cargo_bin"

# Resolve and build the exact Cerberus dependency graph before the secret is
# read by any build script. Cargo.lock belongs to the pinned checkout and
# --locked forbids registry/git graph mutation.
env -u CERBERUS_OPENROUTER_PROVISIONING_KEY \
  CARGO_TARGET_DIR="$temporary_dir/target" \
  "$cargo_bin" build --quiet --locked \
    --manifest-path "$CERBERUS_SOURCE_DIR/Cargo.toml" \
    --bin memory-engine-061-key-helper

: "${CERBERUS_OPENROUTER_PROVISIONING_KEY:?provisioning key is required only for key lifecycle}"

# Keep the temporary source outside the target worktree and never pass the
# provisioning key through GITHUB_ENV, GITHUB_OUTPUT, argv, or tool output.
CERBERUS_OPENROUTER_PROVISIONING_KEY="$CERBERUS_OPENROUTER_PROVISIONING_KEY" \
  "$temporary_dir/target/debug/memory-engine-061-key-helper" "$mode" "$state_dir" "$tag"
