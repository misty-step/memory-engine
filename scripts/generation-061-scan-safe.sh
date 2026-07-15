#!/usr/bin/env bash
set -euo pipefail

source_dir="${1:?usage: $0 SOURCE_DIR SAFE_DIR}"
safe_dir="${2:?usage: $0 SOURCE_DIR SAFE_DIR}"
trap 'rm -rf -- "$source_dir"' EXIT
if ! scanner="$(command -v rg)"; then
  echo 'credential scanner rg is missing; refusing to stage evidence' >&2
  exit 1
fi

rm -rf -- "$safe_dir"
mkdir -p "$safe_dir"
mapfile -d '' sources < <(find "$source_dir" -type f -print0)
if ((${#sources[@]} == 0)); then
  echo 'no live evidence was produced; refusing to upload anything' >&2
  exit 1
fi

for path in "${sources[@]}"; do
  set +e
  "$scanner" -n --hidden -F \
    -e 'CERBERUS_OPENROUTER_PROVISIONING_KEY' \
    -e 'OPENROUTER_API_KEY=' \
    -e 'sk-or-v1-' \
    -- "$path" >/dev/null 2>&1
  status=$?
  set -e
  case "$status" in
    0)
      rm -rf -- "$safe_dir"
      echo 'live evidence contained credential-shaped bytes; refusing to upload it' >&2
      exit 1
      ;;
    1)
      cp -- "$path" "$safe_dir/$(basename "$path")"
      ;;
    *)
      rm -rf -- "$safe_dir"
      echo "credential scanner failed (rg exit $status); refusing to upload evidence" >&2
      exit 1
      ;;
  esac
done

(cd "$safe_dir" && sha256sum -- * > SHA256SUMS)
