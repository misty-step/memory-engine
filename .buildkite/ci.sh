#!/usr/bin/env bash
set -euo pipefail

if ! command -v curl >/dev/null 2>&1; then
  if command -v apt-get >/dev/null 2>&1; then
    apt-get update
    apt-get install -y --no-install-recommends curl ca-certificates unzip
  else
    echo "curl is required" >&2
    exit 127
  fi
fi

curl -fsSL https://bun.sh/install | bash
export BUN_INSTALL="${HOME}/.bun"
export PATH="${BUN_INSTALL}/bin:${PATH}"

curl -fsSL https://dl.dagger.io/dagger/install.sh | BIN_DIR="${HOME}/.local/bin" sh
export PATH="${HOME}/.local/bin:${PATH}"

bun run ci:full

receipt="target/perf/action-latency-postgres-${BUILDKITE_COMMIT}"
mkdir -p "${receipt}"
dagger call action-latency-postgres --source=. --git-sha="${BUILDKITE_COMMIT}" export --path="${receipt}"
