#!/usr/bin/env bash
set -euo pipefail

source_dir="${1:?usage: $0 SOURCE_DIR SAFE_DIR}"
safe_dir="${2:?usage: $0 SOURCE_DIR SAFE_DIR}"
trap 'rm -rf -- "$source_dir"' EXIT
if ! scanner="$(command -v rg)"; then
  echo 'credential scanner rg is missing; refusing to stage evidence' >&2
  exit 1
fi
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
copy_helper="$script_dir/generation-061-copy-regular.py"
test -f "$copy_helper"
python_bin="$(command -v python3 || true)"
: "${python_bin:?safe staging requires python3 for descriptor-based copies}"
max_evidence_bytes=1048576

rm -rf -- "$safe_dir"
mkdir -p "$safe_dir"

# Copy first through the descriptor-based helper, then scan the immutable
# safe copy. Scanning the source path first would leave a window where the
# scanned content and the copied content differ.
copied=0
while IFS= read -r -d '' path; do
  name="$(basename -- "$path")"
  if ! "$python_bin" "$copy_helper" "$path" "$safe_dir" "$name" \
    --max-bytes "$max_evidence_bytes"; then
    rm -rf -- "$safe_dir"
    echo 'live evidence could not be staged safely; refusing to upload it' >&2
    exit 1
  fi
  copied=$((copied + 1))
  set +e
  "$scanner" -n --hidden -F \
    -e 'CERBERUS_OPENROUTER_PROVISIONING_KEY' \
    -e 'GENERATION_PROVIDER_KEY=' \
    -e 'OPENROUTER_PROXY_TOKEN=' \
    -e 'OPENROUTER_API_KEY=' \
    -e 'sk-or-v1-' \
    -- "$safe_dir/$name" >/dev/null 2>&1
  status=$?
  set -e
  case "$status" in
    0)
      rm -rf -- "$safe_dir"
      echo 'live evidence contained credential-shaped bytes; refusing to upload it' >&2
      exit 1
      ;;
    1)
      ;;
    *)
      rm -rf -- "$safe_dir"
      echo "credential scanner failed (rg exit $status); refusing to upload evidence" >&2
      exit 1
      ;;
  esac
done < <(find "$source_dir" -type f -print0)

if [[ "$copied" -eq 0 ]]; then
  rm -rf -- "$safe_dir"
  echo 'no live evidence was produced; refusing to upload anything' >&2
  exit 1
fi

(cd "$safe_dir" && sha256sum -- * > SHA256SUMS)
