#!/usr/bin/env bash
set -euo pipefail

# Cerberus invokes this absolute path from its detached target worktree. The
# host half performs the shared-Git-metadata check, then runs only the exact
# benchmark half in a pinned, disposable container. Target build.rs/runtime
# code never runs in the runner namespace.
: "${OPENROUTER_API_KEY:?Cerberus must inject only a scoped OpenRouter key}"
: "${GENERATION_HEAD_SHA:?trusted workflow must provide the target SHA}"

literalize_glob() {
  local value="$1"
  value="${value//\\/\\\\}"
  value="${value//\*/\\*}"
  value="${value//\?/\\?}"
  value="${value//\[/\\[}"
  value="${value//\]/\\]}"
  printf '%s' "$value"
}

redact_secret() {
  local text="$1"
  local secret_pattern
  secret_pattern="$(literalize_glob "$2")"
  printf '%s' "${text//$secret_pattern/[redacted]}"
}

if [[ "${GENERATION_LIVE_IN_CONTAINER:-}" == true ]]; then
  : "${GENERATION_OUTPUT_DIR:?container output directory is required}"
  : "${GENERATION_RECEIPT_VALIDATOR:?trusted receipt validator is required}"
  receipt="$GENERATION_OUTPUT_DIR/generation-061-live-comparison-$(date -u +%F).md"

  cargo_bin="$(command -v cargo || true)"
  if [[ -z "$cargo_bin" ]]; then
    for candidate in \
      "${CARGO_HOME:-}/bin/cargo" \
      /usr/local/cargo/bin/cargo /home/runner/.cargo/bin/cargo /root/.cargo/bin/cargo \
      /usr/local/bin/cargo /usr/bin/cargo; do
      if [[ -x "$candidate" ]]; then
        cargo_bin="$candidate"
        break
      fi
    done
  fi
  test -n "$cargo_bin"

  # Capture provider/build output in memory, literalize the scoped credential
  # before any transcript output, and never print a raw failed command log.
  set +e
  output="$("$cargo_bin" run --quiet -p memory-engine-bench -- generation \
    --model google/gemini-3.5-flash \
    --prompt principled \
    --out "$receipt" 2>&1)"
  status=$?
  set -e
  redacted_output="$(redact_secret "$output" "$OPENROUTER_API_KEY")"
  if [[ "$status" -ne 0 ]]; then
    printf '%s\n' "$redacted_output"
    rm -f -- "$receipt"
    exit "$status"
  fi
  if printf '%s' "$output" | grep -Fq -- "$OPENROUTER_API_KEY"; then
    rm -f -- "$receipt"
    echo 'benchmark output contained scoped credential bytes; refusing to emit evidence' >&2
    exit 1
  fi
  printf '%s\n' "$redacted_output"

  if ! "$GENERATION_RECEIPT_VALIDATOR" "$receipt"; then
    rm -f -- "$receipt"
    echo 'benchmark receipt did not satisfy the exact live-generation proof contract' >&2
    exit 1
  fi
  if grep -Fq -- "$OPENROUTER_API_KEY" "$receipt"; then
    rm -f -- "$receipt"
    echo 'benchmark receipt contained scoped credential bytes; refusing to emit evidence' >&2
    exit 1
  fi

  printf '%s\n' '--- GENERATION_061_RECEIPT_BEGIN ---'
  cat "$receipt"
  printf '%s\n' '--- GENERATION_061_RECEIPT_END ---'
  exit 0
fi

shared_git_dir="$(git rev-parse --git-common-dir)"
absolute_git_dir="$(git rev-parse --absolute-git-dir)"
if [[ ! -d "$shared_git_dir" || ! -f "$shared_git_dir/config" ]]; then
  echo 'detached target has no readable shared Git directory; refusing to run' >&2
  exit 1
fi
test "$(git rev-parse HEAD)" = "$GENERATION_HEAD_SHA"

# actions/checkout persist-credentials=false is necessary but not sufficient:
# inspect both the detached worktree config and shared common config immediately
# before untrusted code can run. The container receives neither config path.
for config in "$absolute_git_dir/config" "$shared_git_dir/config"; do
  if [[ -f "$config" ]] && git config --file "$config" --get-regexp 'http\..*extraheader' >/dev/null 2>&1; then
    echo 'detached target or shared Git config contains a GitHub auth extraheader; refusing to run' >&2
    exit 1
  fi
done

: "${GENERATION_CONTAINER_IMAGE:?trusted workflow must pin the target image}"
: "${GENERATION_CONTAINER_LABEL:?trusted workflow must label the target container}"
: "${GENERATION_CACHE_DIR:?trusted workflow must provide an isolated cache directory}"
: "${GENERATION_TRUSTED_HELPER:?trusted workflow must provide the helper path}"
: "${GENERATION_TRUSTED_VALIDATOR:?trusted workflow must provide the validator path}"
: "${GENERATION_CONTAINER_NAME:?trusted workflow must provide a unique container name}"

target_root="$(pwd -P)"
mkdir -p "$GENERATION_CACHE_DIR"
cache_root="$(cd -P -- "$GENERATION_CACHE_DIR" && pwd)"
if [[ "$cache_root" == "$target_root" || "$cache_root" == "$target_root/"* ]]; then
  echo 'trusted generation cache must be outside the target tree' >&2
  exit 1
fi
output_dir="$cache_root/live-output"
output_dir_owned=false
env_file=''
docker_bin="$(command -v docker || true)"
cleanup_container() {
  if [[ -n "$docker_bin" ]] && "$docker_bin" container inspect "$GENERATION_CONTAINER_NAME" >/dev/null 2>&1; then
    "$docker_bin" container rm -f "$GENERATION_CONTAINER_NAME" >/dev/null 2>&1 || true
  fi
  if [[ -n "$env_file" ]]; then
    rm -f -- "$env_file"
  fi
  if [[ "$output_dir_owned" == true ]]; then
    rm -rf -- "$output_dir"
  fi
}
trap cleanup_container EXIT INT TERM

rm -rf -- "$output_dir"
mkdir "$output_dir"
output_dir_owned=true
mkdir -p "$cache_root/cargo-home" "$cache_root/cargo-target"
: "${docker_bin:?trusted workflow must provide docker}"
env_file="$(mktemp "$cache_root/container-env.XXXXXX")"
chmod 600 "$env_file"
cat > "$env_file" <<EOF
OPENROUTER_API_KEY=$OPENROUTER_API_KEY
GENERATION_HEAD_SHA=$GENERATION_HEAD_SHA
GENERATION_LIVE_IN_CONTAINER=true
GENERATION_OUTPUT_DIR=/output
GENERATION_RECEIPT_VALIDATOR=/trusted/validate-receipt.sh
CARGO_HOME=/cargo-home
CARGO_TARGET_DIR=/cargo-target
EOF

set +e
"$docker_bin" run \
  --rm \
  --init \
  --name "$GENERATION_CONTAINER_NAME" \
  --label "memory-engine.generation-061=$GENERATION_CONTAINER_LABEL" \
  --user "$(id -u):$(id -g)" \
  --network bridge \
  --ipc private \
  --uts private \
  --read-only \
  --tmpfs /tmp:rw,noexec,nosuid,size=64m \
  --cap-drop ALL \
  --security-opt no-new-privileges \
  --pids-limit 256 \
  --mount "type=bind,src=$target_root,dst=/workspace,readonly" \
  --mount "type=bind,src=$output_dir,dst=/output,rw" \
  --mount "type=bind,src=$cache_root/cargo-home,dst=/cargo-home,rw" \
  --mount "type=bind,src=$cache_root/cargo-target,dst=/cargo-target,rw" \
  --mount "type=bind,src=$GENERATION_TRUSTED_HELPER,dst=/trusted/generation-061-live-comparison.sh,readonly" \
  --mount "type=bind,src=$GENERATION_TRUSTED_VALIDATOR,dst=/trusted/validate-receipt.sh,readonly" \
  --env-file "$env_file" \
  --workdir /workspace \
  "$GENERATION_CONTAINER_IMAGE" \
  /bin/bash /trusted/generation-061-live-comparison.sh
status=$?
set -e

# --rm removes a normally exiting container; this explicit postcondition and
# trap also kill a container if the docker client returns abnormally. The
# workflow repeats label cleanup before trusted staging for timeout/SIGKILL
# recovery paths.
if "$docker_bin" container inspect "$GENERATION_CONTAINER_NAME" >/dev/null 2>&1; then
  "$docker_bin" container rm -f "$GENERATION_CONTAINER_NAME" >/dev/null 2>&1 || status=1
fi
exit "$status"
