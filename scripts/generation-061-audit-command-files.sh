#!/usr/bin/env bash
set -euo pipefail

audit_generation_command_files() {
  local scanner command_file status
  if ! scanner="$(command -v rg)"; then
    echo 'credential scanner rg is missing; refusing to continue' >&2
    return 1
  fi
  for command_file in "$@"; do
    [[ -f "$command_file" ]] || continue
    set +e
    "$scanner" -n --hidden -F \
      -e 'CERBERUS_OPENROUTER_PROVISIONING_KEY' \
      -e 'GENERATION_PROVIDER_KEY=' \
      -e 'OPENROUTER_PROXY_TOKEN=' \
      -e 'OPENROUTER_API_KEY=' \
      -e 'sk-or-v1-' \
      -- "$command_file" >/dev/null 2>&1
    status=$?
    set -e
    case "$status" in
      0)
        echo "credential-shaped bytes found in $command_file" >&2
        return 1
        ;;
      1) ;;
      *)
        echo "credential scanner failed on $command_file (rg exit $status)" >&2
        return 1
        ;;
    esac
  done
}
