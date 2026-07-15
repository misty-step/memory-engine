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
  # Published fields are size-capped: anything larger than this is not
  # trusted evidence, whatever it contains.
  max_report_bytes=262144

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
  if (( ${#output} > max_report_bytes )); then
    rm -f -- "$receipt"
    echo 'benchmark output exceeded the trusted report size cap; refusing to emit evidence' >&2
    exit 1
  fi
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

: "${GENERATION_CONTAINER_IMAGE:?trusted workflow must pin the build image}"
: "${GENERATION_RUNTIME_IMAGE:?trusted workflow must pin the minimal runtime image}"
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
corpus_dir="$target_root/crates/memory-engine-bench/corpus"
test -d "$corpus_dir"
mkdir -p "$GENERATION_CACHE_DIR"
cache_root="$(cd -P -- "$GENERATION_CACHE_DIR" && pwd)"
if [[ "$cache_root" == "$target_root" || "$cache_root" == "$target_root/"* ]]; then
  echo 'trusted generation cache must be outside the target tree' >&2
  exit 1
fi
prepared_dir="$cache_root/prepared"
prepared_binary="$prepared_dir/memory-engine-bench"
provider_socket="$cache_root/provider.sock"
provider_attestation="$GENERATION_PROVIDER_ATTESTATION"
runtime_cleanup_evidence="$GENERATION_RUNTIME_CLEANUP_EVIDENCE"
build_container_name="$GENERATION_CONTAINER_NAME-prepare"
prepared_dir_owned=false
env_file=''
provider_proxy_pid=''
provider_proxy_stopped=false
attestation_validated=false
build_timed_out=false
runtime_timed_out=false
container_removed=false
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
  local prepared_removed=false
  [[ ! -e "$prepared_dir" && ! -L "$prepared_dir" ]] && prepared_removed=true
  temporary="${runtime_cleanup_evidence}.tmp.$$"
  umask 077
  printf '{"schema":"memory-engine/generation-061-runtime-cleanup/v1","target_sha":"%s","container_name":"%s","build_timeout_seconds":%s,"runtime_timeout_seconds":%s,"build_timed_out":%s,"runtime_timed_out":%s,"container_removed":%s,"output_host_mount":false,"prepared_removed":%s,"provider_proxy_stopped":%s,"attestation_validated":%s}\n' \
    "$GENERATION_HEAD_SHA" "$GENERATION_CONTAINER_NAME" \
    "$GENERATION_BUILD_TIMEOUT_SECONDS" "$GENERATION_CONTAINER_TIMEOUT_SECONDS" \
    "$build_timed_out" "$runtime_timed_out" "$container_removed" \
    "$prepared_removed" "$provider_proxy_stopped" \
    "$attestation_validated" > "$temporary"
  chmod 600 "$temporary"
  mv -f -- "$temporary" "$runtime_cleanup_evidence"
}
# Distinguish "definitively absent" from "docker failed". Returns 0 when the
# daemon explicitly reports no such container, 1 when the container exists,
# and 2 on any other docker failure — which must always fail closed. Both
# this function and every call site are errexit-safe: status is captured
# with `|| status=$?`, never by toggling `set -e`.
container_absent() {
  local name="$1" inspect_output inspect_status=0
  inspect_output="$("$docker_bin" container inspect --format '{{.Id}}' "$name" 2>&1)" || inspect_status=$?
  if [[ "$inspect_status" -eq 0 ]]; then
    return 1
  fi
  case "$inspect_output" in
    *'No such container'*|*'no such container'*) return 0 ;;
  esac
  return 2
}
cleanup_container() {
  # Cleanup proof fails closed: container_removed becomes true only when
  # every named container is explicitly reported absent and the label sweep
  # succeeds and is empty. Any docker error keeps it false.
  container_removed=true
  if [[ -z "$docker_bin" ]]; then
    container_removed=false
  else
    local absence_status labeled labeled_status
    for container_name in "$build_container_name" "$GENERATION_CONTAINER_NAME"; do
      absence_status=0
      container_absent "$container_name" || absence_status=$?
      if [[ "$absence_status" -eq 1 ]]; then
        "$docker_bin" container rm -f "$container_name" >/dev/null 2>&1 || true
        absence_status=0
        container_absent "$container_name" || absence_status=$?
      fi
      if [[ "$absence_status" -ne 0 ]]; then
        container_removed=false
      fi
    done
    labeled_status=0
    labeled="$("$docker_bin" container ls -aq --filter "label=memory-engine.generation-061=$GENERATION_CONTAINER_LABEL" 2>/dev/null)" || labeled_status=$?
    if [[ "$labeled_status" -ne 0 || -n "$labeled" ]]; then
      container_removed=false
    fi
  fi
  if [[ -n "$env_file" ]]; then
    rm -f -- "$env_file"
  fi
  stop_provider_proxy
  rm -f -- "$provider_socket" "$cache_root/provider-proxy.log"
  if [[ "$prepared_dir_owned" == true ]]; then
    rm -rf -- "$prepared_dir"
  fi
  write_cleanup_evidence
}
trap cleanup_container EXIT INT TERM

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
build_absence=0
container_absent "$build_container_name" || build_absence=$?
if [[ "$build_absence" -eq 1 ]]; then
  "$docker_bin" container rm -f "$build_container_name" >/dev/null 2>&1 || build_status=1
  build_absence=0
  container_absent "$build_container_name" || build_absence=$?
fi
if [[ "$build_absence" -ne 0 ]]; then
  echo 'docker could not prove the build container is gone; refusing to continue' >&2
  build_status=1
fi
if [[ "$build_status" -ne 0 ]]; then
  exit "$build_status"
fi
# The networked build stage is untrusted: its writable /prepared mount must
# contain exactly the digest-checked benchmark and nothing else, and only
# that exact file is ever mounted into the runtime container.
unexpected_prepared_entry="$(find "$prepared_dir" -mindepth 1 ! -path "$prepared_binary" -print 2>/dev/null | head -n 1)"
if [[ -n "$unexpected_prepared_entry" ]]; then
  echo "prepared directory must contain exactly the digest-checked benchmark; found: $unexpected_prepared_entry" >&2
  exit 1
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
  --model google/gemini-3.5-flash \
  --max-calls 128 \
  --max-total-bytes 268435456 \
  --max-concurrency 4 \
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

# The runtime container is a digest-pinned minimal image with no toolchain:
# it sees only the eval corpus data (never repository source), the exact
# prepared binary, the trusted in-container helper/validator, the provider
# socket, and a bounded noexec tmpfs for its report. The corpus path matches
# the manifest directory baked into the prepared binary at build time.
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
  --tmpfs /output:rw,noexec,nosuid,size=16m \
  --cap-drop ALL \
  --security-opt no-new-privileges \
  --pids-limit 256 \
  --mount "type=bind,src=$corpus_dir,dst=/workspace/crates/memory-engine-bench/corpus,readonly" \
  --mount "type=bind,src=$prepared_binary,dst=/prepared/memory-engine-bench,readonly" \
  --mount "type=bind,src=$provider_socket,dst=/provider.sock,readonly" \
  --mount "type=bind,src=$GENERATION_TRUSTED_HELPER,dst=/trusted/generation-061-live-comparison.sh,readonly" \
  --mount "type=bind,src=$GENERATION_TRUSTED_VALIDATOR,dst=/trusted/validate-receipt.sh,readonly" \
  --env-file "$env_file" \
  --workdir /tmp \
  "$GENERATION_RUNTIME_IMAGE" \
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
runtime_absence=0
container_absent "$GENERATION_CONTAINER_NAME" || runtime_absence=$?
if [[ "$runtime_absence" -eq 1 ]]; then
  "$docker_bin" container rm -f "$GENERATION_CONTAINER_NAME" >/dev/null 2>&1 || status=1
  runtime_absence=0
  container_absent "$GENERATION_CONTAINER_NAME" || runtime_absence=$?
fi
if [[ "$runtime_absence" -ne 0 ]]; then
  echo 'docker could not prove the target container is gone; refusing trusted staging' >&2
  status=1
fi
stop_provider_proxy
if ! "$GENERATION_TRUSTED_ATTESTATION_VALIDATOR" "$provider_attestation" "$GENERATION_HEAD_SHA"; then
  status=1
else
  attestation_validated=true
fi
cleanup_container
trap - EXIT INT TERM
if [[ "$container_removed" != true ]]; then
  echo 'cleanup could not prove every target container was removed' >&2
  status=1
fi
exit "$status"
