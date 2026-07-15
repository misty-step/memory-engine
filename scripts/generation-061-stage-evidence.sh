#!/usr/bin/env bash
set -euo pipefail

# Trusted evidence staging. Every copy goes through the descriptor-based
# helper: source opened O_NOFOLLOW and fstat-verified regular, destination
# created O_CREAT|O_EXCL|O_NOFOLLOW inside a verified real directory
# (openat-anchored where the platform supports dir_fd). A symlink or
# pre-existing file at either end fails closed with no partial copy, and
# source permission bits (including executable modes) are preserved.
destination="${1:?usage: $0 DESTINATION [SOURCE ...]}"
shift

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
copy_helper="$script_dir/generation-061-copy-regular.py"
test -f "$copy_helper"
python_bin="$(command -v python3 || true)"
: "${python_bin:?trusted staging requires python3 for descriptor-based copies}"

# Evidence files are small (receipt, attestation, cleanup proof); anything
# larger is not trusted evidence.
max_evidence_bytes=1048576

if [[ -L "$destination" || ( -e "$destination" && ! -d "$destination" ) ]]; then
  echo 'evidence destination is not a real directory; refusing staging' >&2
  exit 1
fi
mkdir -p -- "$destination"
if [[ -L "$destination" || ! -d "$destination" ]]; then
  echo 'evidence destination is not a real directory; refusing staging' >&2
  exit 1
fi

for source in "$@"; do
  if [[ ! -e "$source" && ! -L "$source" ]]; then
    continue
  fi
  name="$(basename -- "$source")"
  "$python_bin" "$copy_helper" "$source" "$destination" "$name" \
    --max-bytes "$max_evidence_bytes"
done
