#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
receipt="${repo_root}/.evidence/learner-trust-091-driver.txt"
mkdir -p "$(dirname "${receipt}")"
{
  printf 'learner-trust-091 local real-collaborator driver\n'
  printf 'repo: %s\n' "${repo_root}"
  printf 'command: cargo test -p memory-engine-persistence learner_trust_driver_keeps_pending_decisions_and_exports_after_reload -- --nocapture\n'
  printf 'flow: generation pending -> keep + edit-and-keep + reject -> due queue reload -> provenance export\n'
  cargo test --manifest-path "${repo_root}/Cargo.toml" -p memory-engine-persistence learner_trust_driver_keeps_pending_decisions_and_exports_after_reload -- --nocapture
  printf 'result: PASS; only kept and edited-kept drafts are due; rejected draft is present in reload/export\n'
} | tee "${receipt}"
