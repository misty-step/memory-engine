#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WRITER="$SCRIPT_DIR/write-trace.py"
PASS=0
FAIL=0

assert_eq() {
  local desc="$1" expected="$2" actual="$3"
  if [ "$expected" = "$actual" ]; then
    PASS=$((PASS + 1))
    echo "  PASS  $desc"
  else
    FAIL=$((FAIL + 1))
    echo "  FAIL  $desc (expected '$expected', got '$actual')"
  fi
}

setup_repo() {
  TEST_DIR="$(mktemp -d)"
  git -C "$TEST_DIR" init -q
  mkdir -p "$TEST_DIR/.empty-hooks"
  git -C "$TEST_DIR" config core.hooksPath .empty-hooks
  git -C "$TEST_DIR" config user.email test@example.com
  git -C "$TEST_DIR" config user.name "Trace Test"
  printf 'hello\n' > "$TEST_DIR/README.md"
  git -C "$TEST_DIR" add README.md
  git -C "$TEST_DIR" commit -q -m 'init'
}

teardown_repo() {
  rm -rf "$TEST_DIR"
}

test_append_writes_jsonl() {
  setup_repo
  python3 "$WRITER" append --repo "$TEST_DIR" --backlog 056 \
    --evidence .evidence/qa.txt --transcript-ref codex:session:test \
    --note "focused trace note" >/tmp/trace-append.json
  local log="$TEST_DIR/.spellbook/traces/traces.jsonl"
  local count kind backlog
  count="$(wc -l < "$log" | tr -d ' ')"
  kind="$(python3 -c "import json,sys; print(json.loads(sys.stdin.read())['kind'])" < "$log")"
  backlog="$(python3 -c "import json,sys; print(json.loads(sys.stdin.read())['backlog_id'])" < "$log")"
  assert_eq "append writes one JSONL line" "1" "$count"
  assert_eq "append kind defaults to trace.note" "trace.note" "$kind"
  assert_eq "append records backlog id" "056" "$backlog"
  teardown_repo
}

test_final_requires_transcript_or_reason() {
  setup_repo
  local exit_code=0
  python3 "$WRITER" final --repo "$TEST_DIR" --backlog 056 \
    --merged-sha abc123 >/tmp/trace-final.json 2>/tmp/trace-final.err || exit_code=$?
  assert_eq "final without transcript reason fails" "1" "$exit_code"
  teardown_repo
}

test_final_requires_merged_sha() {
  setup_repo
  local exit_code=0
  python3 "$WRITER" final --repo "$TEST_DIR" --backlog 056 \
    --no-transcript-reason "harness did not expose transcript export" >/tmp/trace-final.json 2>/tmp/trace-final.err || exit_code=$?
  assert_eq "final without merged sha fails" "1" "$exit_code"
  teardown_repo
}

test_final_accepts_no_transcript_reason() {
  setup_repo
  python3 "$WRITER" final --repo "$TEST_DIR" --backlog 056 \
    --merged-sha abc123 --qa .evidence/qa.txt \
    --no-transcript-reason "harness did not expose transcript export" >/tmp/trace-final.json
  local log="$TEST_DIR/.spellbook/traces/traces.jsonl"
  local kind reason
  kind="$(python3 -c "import json,sys; print(json.loads(sys.stdin.read())['kind'])" < "$log")"
  reason="$(python3 -c "import json,sys; print(json.loads(sys.stdin.read())['no_transcript_reason'])" < "$log")"
  assert_eq "final writes trace.final" "trace.final" "$kind"
  assert_eq "final records no transcript reason" "harness did not expose transcript export" "$reason"
  teardown_repo
}

test_secret_refusal() {
  setup_repo
  local exit_code=0
  python3 "$WRITER" append --repo "$TEST_DIR" --backlog 056 \
    --note "token sk-abcdefghijklmnopqrstuvwxyz" >/tmp/trace-secret.json 2>/tmp/trace-secret.err || exit_code=$?
  assert_eq "secret-shaped note fails" "1" "$exit_code"
  if [ -e "$TEST_DIR/.spellbook/traces/traces.jsonl" ]; then
    assert_eq "secret-shaped note writes no log" "missing" "present"
  else
    assert_eq "secret-shaped note writes no log" "missing" "missing"
  fi
  teardown_repo
}

test_secret_assignment_refusal() {
  setup_repo
  local exit_code=0
  python3 "$WRITER" append --repo "$TEST_DIR" --backlog 056 \
    --note "OPENAI_TOKEN=abc123" >/tmp/trace-secret-assignment.json 2>/tmp/trace-secret-assignment.err || exit_code=$?
  assert_eq "secret assignment note fails" "1" "$exit_code"
  if [ -e "$TEST_DIR/.spellbook/traces/traces.jsonl" ]; then
    assert_eq "secret assignment writes no log" "missing" "present"
  else
    assert_eq "secret assignment writes no log" "missing" "missing"
  fi
  teardown_repo
}

test_card_like_value_refusal() {
  setup_repo
  local exit_code=0
  python3 "$WRITER" append --repo "$TEST_DIR" --backlog 056 \
    --note "card 4111 1111 1111 1111" >/tmp/trace-card.json 2>/tmp/trace-card.err || exit_code=$?
  assert_eq "card-shaped note fails" "1" "$exit_code"
  if [ -e "$TEST_DIR/.spellbook/traces/traces.jsonl" ]; then
    assert_eq "card-shaped note writes no log" "missing" "present"
  else
    assert_eq "card-shaped note writes no log" "missing" "missing"
  fi
  teardown_repo
}

test_numeric_trace_id_is_allowed() {
  setup_repo
  python3 "$WRITER" append --repo "$TEST_DIR" --backlog 056 \
    --branch numeric --head-sha 1234567890123456 \
    --note "focused trace note" >/tmp/trace-numeric-id.json
  local log="$TEST_DIR/.spellbook/traces/traces.jsonl"
  local count
  count="$(wc -l < "$log" | tr -d ' ')"
  assert_eq "numeric structural id writes log" "1" "$count"
  teardown_repo
}

test_append_writes_jsonl
test_final_requires_transcript_or_reason
test_final_requires_merged_sha
test_final_accepts_no_transcript_reason
test_secret_refusal
test_secret_assignment_refusal
test_card_like_value_refusal
test_numeric_trace_id_is_allowed

if [ "$FAIL" -gt 0 ]; then
  echo "$FAIL failed, $PASS passed"
  exit 1
fi

echo "$PASS passed"
