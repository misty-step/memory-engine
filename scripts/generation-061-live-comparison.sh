#!/usr/bin/env bash
set -euo pipefail

# Cerberus invokes this absolute path from its detached target worktree. The
# host half performs the shared-Git-metadata check, then runs only the exact
# benchmark half in a pinned, disposable container. Target build.rs/runtime
# code never runs in the runner namespace.
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
  : "${OPENROUTER_PROXY_TOKEN:?trusted provider proxy must inject only a one-run capability}"
 : "${GENERATION_OUTPUT_DIR:?container output directory is required}"
 : "${GENERATION_RECEIPT_VALIDATOR:?trusted receipt validator is required}"
 : "${OPENROUTER_PROXY_SOCKET:?trusted provider proxy socket is required}"
  : "${GENERATION_PREPARED_BINARY:?trusted prebuilt benchmark is required}"
  : "${GENERATION_PREPARED_BINARY_SHA256:?trusted prebuilt benchmark digest is required}"
 receipt="$GENERATION_OUTPUT_DIR/generation-061-live-comparison-$(date -u +%F).md"

  test -f "$GENERATION_PREPARED_BINARY"
  test ! -L "$GENERATION_PREPARED_BINARY"
  test -x "$GENERATION_PREPARED_BINARY"
  prepared_binary_sha256="$(sha256sum "$GENERATION_PREPARED_BINARY" | awk '{print $1}')"
  test "$prepared_binary_sha256" = "$GENERATION_PREPARED_BINARY_SHA256"

  # Capture provider/build output in memory, literalize the scoped credential
  # before any transcript output, and never print a raw failed command log.
  set +e
  output="$("$GENERATION_PREPARED_BINARY" generation \
    --model google/gemini-3.5-flash \
    --prompt principled \
    --out "$receipt" 2>&1)"
  status=$?
  set -e
  redacted_output="$(redact_secret "$output" "$OPENROUTER_PROXY_TOKEN")"
  if [[ "$status" -ne 0 ]]; then
    printf '%s\n' "$redacted_output"
    rm -f -- "$receipt"
    exit "$status"
  fi
  if printf '%s' "$output" | grep -Fq -- "$OPENROUTER_PROXY_TOKEN"; then
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
  if grep -Fq -- "$OPENROUTER_PROXY_TOKEN" "$receipt"; then
    rm -f -- "$receipt"
    echo 'benchmark receipt contained scoped credential bytes; refusing to emit evidence' >&2
    exit 1
  fi

  printf '%s\n' '--- GENERATION_061_RECEIPT_BEGIN ---'
  cat "$receipt"
  printf '%s\n' '--- GENERATION_061_RECEIPT_END ---'
  exit 0
fi

: "${GENERATION_PROVIDER_KEY:?Cerberus must inject only a scoped provider key into the trusted proxy}"

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
: "${GENERATION_TRUSTED_PROXY:?trusted workflow must provide the provider proxy}"
: "${GENERATION_PROVIDER_ATTESTATION:?trusted workflow must provide the attestation path}"
: "${GENERATION_TRUSTED_ATTESTATION_VALIDATOR:?trusted workflow must provide the attestation validator}"
: "${GENERATION_BUILD_TIMEOUT_SECONDS:?trusted workflow must bound dependency preparation}"
: "${GENERATION_CONTAINER_TIMEOUT_SECONDS:?trusted workflow must bound target runtime}"
: "${GENERATION_RUNTIME_CLEANUP_EVIDENCE:?trusted workflow must record cleanup evidence}"

target_root="$(pwd -P)"
mkdir -p "$GENERATION_CACHE_DIR"
cache_root="$(cd -P -- "$GENERATION_CACHE_DIR" && pwd)"
if [[ "$cache_root" == "$target_root" || "$cache_root" == "$target_root/"* ]]; then
  echo 'trusted generation cache must be outside the target tree' >&2
  exit 1
fi
output_dir="$cache_root/live-output"
prepared_dir="$cache_root/prepared"
prepared_binary="$prepared_dir/memory-engine-bench"
provider_socket="$cache_root/provider.sock"
provider_attestation="$GENERATION_PROVIDER_ATTESTATION"
runtime_cleanup_evidence="$GENERATION_RUNTIME_CLEANUP_EVIDENCE"
build_container_name="$GENERATION_CONTAINER_NAME-prepare"
output_dir_owned=false
prepared_dir_owned=false
env_file=''
provider_proxy_pid=''
provider_proxy_stopped=false
attestation_validated=false
build_timed_out=false
runtime_timed_out=false
docker_bin="$(command -v docker || true)"
timeout_bin="$(command -v timeout || true)"
stop_provider_proxy() {
  if [[ -n "$provider_proxy_pid" ]]; then
    kill -TERM "$provider_proxy_pid" >/dev/null 2>&1 || true
    wait "$provider_proxy_pid" >/dev/null 2>&1 || true
    provider_proxy_pid=''
  fi
  provider_proxy_stopped=true
}
write_cleanup_evidence() {
  local temporary
  local output_removed=false
  local prepared_removed=false
  [[ ! -e "$output_dir" && ! -L "$output_dir" ]] && output_removed=true
  [[ ! -e "$prepared_dir" && ! -L "$prepared_dir" ]] && prepared_removed=true
  temporary="${runtime_cleanup_evidence}.tmp.$$"
  umask 077
  printf '{"schema":"memory-engine/generation-061-runtime-cleanup/v1","target_sha":"%s","container_name":"%s","build_timeout_seconds":%s,"runtime_timeout_seconds":%s,"build_timed_out":%s,"runtime_timed_out":%s,"container_removed":%s,"output_removed":%s,"prepared_removed":%s,"provider_proxy_stopped":%s,"attestation_validated":%s}\n' \
    "$GENERATION_HEAD_SHA" "$GENERATION_CONTAINER_NAME" \
    "$GENERATION_BUILD_TIMEOUT_SECONDS" "$GENERATION_CONTAINER_TIMEOUT_SECONDS" \
    "$build_timed_out" "$runtime_timed_out" "$container_removed" \
    "$output_removed" "$prepared_removed" "$provider_proxy_stopped" \
    "$attestation_validated" > "$temporary"
  chmod 600 "$temporary"
  mv -f -- "$temporary" "$runtime_cleanup_evidence"
}
cleanup_container() {
  container_removed=true
  for container_name in "$build_container_name" "$GENERATION_CONTAINER_NAME"; do
    if [[ -n "$docker_bin" ]] && "$docker_bin" container inspect "$container_name" >/dev/null 2>&1; then
      "$docker_bin" container rm -f "$container_name" >/dev/null 2>&1 || true
      if "$docker_bin" container inspect "$container_name" >/dev/null 2>&1; then
        container_removed=false
      fi
    fi
  done
  if [[ -n "$env_file" ]]; then
    rm -f -- "$env_file"
  fi
  stop_provider_proxy
  rm -f -- "$provider_socket" "$cache_root/provider-proxy.log"
  if [[ "$output_dir_owned" == true ]]; then
    rm -rf -- "$output_dir"
  fi
  if [[ "$prepared_dir_owned" == true ]]; then
    rm -rf -- "$prepared_dir"
  fi
  write_cleanup_evidence
}
trap cleanup_container EXIT INT TERM

rm -rf -- "$output_dir"
mkdir "$output_dir"
output_dir_owned=true
rm -rf -- "$prepared_dir"
mkdir "$prepared_dir"
prepared_dir_owned=true
: "${docker_bin:?trusted workflow must provide docker}"
: "${timeout_bin:?trusted workflow must provide timeout}"
python_bin="$(command -v python3 || true)"
: "${python_bin:?trusted workflow must provide python3 for the trusted provider proxy}"
rm -f -- "$provider_attestation" "$provider_socket" "$cache_root/provider-proxy.log" "$runtime_cleanup_evidence"

# Prepare the exact target binary before the provider proxy exists. The build
# image has ephemeral Cargo state and no provider capability; only its digest-
# checked output crosses into the later network-disabled runtime container.
set +e
env -i PATH="$PATH" HOME="$HOME" CARGO_HOME=/cargo-home CARGO_TARGET_DIR=/cargo-target \
  "$timeout_bin" --foreground --signal=TERM --kill-after=30s \
  "${GENERATION_BUILD_TIMEOUT_SECONDS}s" "$docker_bin" run \
  --rm \
  --init \
  --name "$build_container_name" \
  --label "memory-engine.generation-061=$GENERATION_CONTAINER_LABEL" \
  --user "$(id -u):$(id -g)" \
  --network bridge \
  --ipc private \
  --uts private \
  --read-only \
  --tmpfs /tmp:rw,noexec,nosuid,size=64m \
  --tmpfs /cargo-home:rw,noexec,nosuid,size=512m \
  --tmpfs /cargo-target:rw,noexec,nosuid,size=2048m \
  --cap-drop ALL \
  --security-opt no-new-privileges \
  --pids-limit 256 \
  --mount "type=bind,src=$target_root,dst=/workspace,readonly" \
  --mount "type=bind,src=$prepared_dir,dst=/prepared,rw" \
  --workdir /workspace \
  "$GENERATION_CONTAINER_IMAGE" \
  /bin/bash -euc '
    cargo build --quiet --locked -p memory-engine-bench --bin memory-engine-bench
    test -f /cargo-target/debug/memory-engine-bench
    cp -- /cargo-target/debug/memory-engine-bench /prepared/memory-engine-bench
    chmod 0555 /prepared/memory-engine-bench
  '
build_status=$?
set -e
if [[ "$build_status" == 124 || "$build_status" == 137 ]]; then
  build_timed_out=true
fi
if "$docker_bin" container inspect "$build_container_name" >/dev/null 2>&1; then
  "$docker_bin" container rm -f "$build_container_name" >/dev/null 2>&1 || build_status=1
fi
if [[ "$build_status" -ne 0 ]]; then
  exit "$build_status"
fi
test -f "$prepared_binary"
test ! -L "$prepared_binary"
test -x "$prepared_binary"
prepared_binary_sha256="$(sha256sum "$prepared_binary" | awk '{print $1}')"
test -n "$prepared_binary_sha256"

proxy_token="$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')"
"$python_bin" "$GENERATION_TRUSTED_PROXY" \
  --socket "$provider_socket" \
  --attestation "$provider_attestation" \
  --target-sha "$GENERATION_HEAD_SHA" \
  --token "$proxy_token" \
  3<<<"$GENERATION_PROVIDER_KEY" \
  >"$cache_root/provider-proxy.log" 2>&1 &
provider_proxy_pid=$!
for _ in {1..100}; do
  [[ -S "$provider_socket" ]] && break
  sleep 0.1
done
test -S "$provider_socket"
env_file="$(mktemp "$cache_root/container-env.XXXXXX")"
chmod 600 "$env_file"
cat > "$env_file" <<EOF
OPENROUTER_PROXY_TOKEN=$proxy_token
OPENROUTER_PROXY_SOCKET=/provider.sock
GENERATION_HEAD_SHA=$GENERATION_HEAD_SHA
GENERATION_LIVE_IN_CONTAINER=true
GENERATION_OUTPUT_DIR=/output
GENERATION_RECEIPT_VALIDATOR=/trusted/validate-receipt.sh
GENERATION_PREPARED_BINARY=/prepared/memory-engine-bench
GENERATION_PREPARED_BINARY_SHA256=$prepared_binary_sha256
EOF

set +e
"$timeout_bin" --foreground --signal=TERM --kill-after=30s \
  "${GENERATION_CONTAINER_TIMEOUT_SECONDS}s" "$docker_bin" run \
  --rm \
  --init \
  --name "$GENERATION_CONTAINER_NAME" \
  --label "memory-engine.generation-061=$GENERATION_CONTAINER_LABEL" \
  --user "$(id -u):$(id -g)" \
  --network none \
  --ipc private \
  --uts private \
  --read-only \
  --tmpfs /tmp:rw,noexec,nosuid,size=64m \
  --cap-drop ALL \
  --security-opt no-new-privileges \
  --pids-limit 256 \
  --mount "type=bind,src=$target_root,dst=/workspace,readonly" \
  --mount "type=bind,src=$output_dir,dst=/output,rw" \
  --mount "type=bind,src=$prepared_dir,dst=/prepared,readonly" \
  --mount "type=bind,src=$provider_socket,dst=/provider.sock,readonly" \
  --mount "type=bind,src=$GENERATION_TRUSTED_HELPER,dst=/trusted/generation-061-live-comparison.sh,readonly" \
  --mount "type=bind,src=$GENERATION_TRUSTED_VALIDATOR,dst=/trusted/validate-receipt.sh,readonly" \
  --env-file "$env_file" \
  --workdir /workspace \
  "$GENERATION_CONTAINER_IMAGE" \
  /bin/bash /trusted/generation-061-live-comparison.sh
status=$?
set -e
if [[ "$status" == 124 || "$status" == 137 ]]; then
  runtime_timed_out=true
fi

# --rm removes a normally exiting container; this explicit postcondition and
# trap also kill a container if the docker client returns abnormally. The
# workflow repeats label cleanup before trusted staging for timeout/SIGKILL
# recovery paths.
if "$docker_bin" container inspect "$GENERATION_CONTAINER_NAME" >/dev/null 2>&1; then
  "$docker_bin" container rm -f "$GENERATION_CONTAINER_NAME" >/dev/null 2>&1 || status=1
fi
stop_provider_proxy
if ! "$GENERATION_TRUSTED_ATTESTATION_VALIDATOR" "$provider_attestation" "$GENERATION_HEAD_SHA"; then
  status=1
else
  attestation_validated=true
fi
cleanup_container
trap - EXIT INT TERM
exit "$status"
