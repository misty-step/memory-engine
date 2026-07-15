#!/usr/bin/env bash
set -euo pipefail

destination="${1:?usage: $0 DESTINATION [SOURCE ...]}"
shift

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
  if [[ -L "$source" || ! -f "$source" ]]; then
    echo "evidence source is not a regular non-symlink file: $source" >&2
    exit 1
  fi
  name="$(basename -- "$source")"
  target="$destination/$name"
  if [[ -L "$target" || -e "$target" ]]; then
    echo "evidence destination already exists: $target" >&2
    exit 1
  fi
  cp --no-dereference -- "$source" "$target"
  if [[ -L "$target" || ! -f "$target" ]]; then
    echo "evidence copy did not produce a regular file: $target" >&2
    exit 1
  fi
done
