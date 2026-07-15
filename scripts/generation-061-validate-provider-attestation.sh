#!/usr/bin/env bash
set -euo pipefail

attestation="${1:?usage: $0 ATTESTATION EXPECTED_HEAD_SHA}"
expected_head="${2:?usage: $0 ATTESTATION EXPECTED_HEAD_SHA}"
jq_bin="$(command -v jq || true)"
if [[ -z "$jq_bin" ]]; then
  echo 'trusted provider attestation jq is missing; refusing canonical acceptance' >&2
  exit 1
fi
test -s "$attestation"
test "$("$jq_bin" -er '.schema' "$attestation")" = 'memory-engine/generation-061-provider-attestation/v1'
test "$("$jq_bin" -er '.target_sha' "$attestation")" = "$expected_head"
calls="$("$jq_bin" -er '.provider_calls | numbers' "$attestation")"
successful="$("$jq_bin" -er '.successful_provider_calls | numbers' "$attestation")"
test "$calls" -ge 15
test "$successful" = "$calls"
test "$("$jq_bin" -er '.canonical_acceptance' "$attestation")" = 'provider-calls-observed'
test "$("$jq_bin" -er '.calls | length' "$attestation")" = "$calls"
