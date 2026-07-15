#!/usr/bin/env bash
set -euo pipefail

receipt="${1:?usage: $0 RECEIPT}"
test -s "$receipt"
if ! grep_bin="$(command -v grep)"; then
  echo 'receipt validator grep is missing; refusing to claim live proof' >&2
  exit 1
fi
grep -Fqx -- '- Corpus: 14 sources' "$receipt"

expected_sources=(
  mitochondria nato-alphabet http-caching rubicon sourdough gdpr-basis
  hope-feathers pythagorean curie git-branching spacing-effect water-boiling
  apostles-creed us-presidents-ordinal
)
for source in "${expected_sources[@]}"; do
  grep -Fq -- "| $source |" "$receipt"
done
grep -Fqx -- '- Provider failures: 0/14 sources' "$receipt"

# A nonempty markdown file is not proof: reject provider, bridge, and any
# error row before the receipt is copied into trusted evidence. The bridge
# score is a gate in its own right, so require the benchmark's explicit PASS.
if ! "$grep_bin" -Eiq -- '^- Bridge fixture: .*[·[:space:]]+PASS[[:space:]]*$' "$receipt"; then
  echo 'live generation receipt is missing an explicit passing bridge fixture' >&2
  exit 1
fi
set +e
"$grep_bin" -nEi -- 'FAILED|ERROR|error|Provider failures: [1-9][0-9]*/|Bridge fixture:.*FAIL' "$receipt" >/dev/null 2>&1
status=$?
set -e
case "$status" in
  0)
    echo 'live generation receipt contains a FAILED/error/bridge-failure row' >&2
    exit 1
    ;;
  1)
    ;;
  *)
    echo "receipt validator grep failed (exit $status); refusing live proof" >&2
    exit 1
    ;;
esac
