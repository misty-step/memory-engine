# Fleet onboarding

This is the repo-local receipt for the current fleet integration boundary. It
keeps the old four-item onboarding language honest while pointing at the live
authorities.

## Current decisions

| Surface | Current authority | Decision and proof |
| --- | --- | --- |
| Canary | `memory-engine-api` runtime and `docs/runbook.md` | Shipped. `CANARY_ENDPOINT` and `CANARY_API_KEY` are documented production variables; API 500s report as `memory-engine-api`. |
| Local backlog | [`backlog/`](../backlog/README.md) | `backlog/` is the sole active work ledger under `AGENTS.md`. Item files own priority and workflow state; the index is the board; item files hold proof and closure. |
| Landmark | `.landmark.yml` plus Landmark CLI | Adopted in manifest-only/synthesis-only mode. The repo has no release-secret authority, so no release-mutating workflow is added. Preview with `landmark setup --repo-root . --dry-run` and `landmark run --provider local --repo-root . --dry-run` from a Landmark checkout. |
| Project map | `docs/architecture/memory-engine.map.json` and `workbench.html` | This is the live repo-local architecture-map successor for project structure. It is static, diffable, and explicitly separate from Landmark release intelligence. |
| Cerberus | `.github/workflows/cerberus-review.yml` | CI-native `review-pr` path is wired to Cerberus v0.72.0 with an explicit per-job GitHub token, `container-opencode`, and a capped per-review key minted from `CERBERUS_OPENROUTER_PROVISIONING_KEY`. The job skips visibly when that provisioning key is not provisioned. |

## Cerberus boundary

The current Roster contract separates operator-driven Mode A from event-driven
Mode B. Bitterblossom owns the webhook plane; Cerberus owns the review runner
and its GitHub projection. This repository uses the supported CI-native shape,
which is safe to validate in a pull request and does not require resurrecting a
retired webhook service.

The workflow does not use `pull_request_target`, checkout PR code, ambient `gh`
authentication, or the unsafe plain `--allow-env OPENROUTER_API_KEY` plus
`--harness opencode` combination. `container-opencode` isolates the substrate,
while Cerberus mints and revokes a USD-capped per-review key using the
host-only `CERBERUS_OPENROUTER_PROVISIONING_KEY`. A live posted review still
requires the operator to provision that management key; that external
authority was not available during this onboarding change.

## Verification

```sh
python3 -m json.tool docs/architecture/memory-engine.map.json >/dev/null
test -f docs/architecture/workbench.html
cargo test -p memory-engine-qa --test docs_reconciliation fleet_onboarding_contract_is_declarative_and_current -- --exact
```

The canonical project map is intentionally not generated from Landmark, Roster,
or runtime code. The Rust crates and tests remain runtime truth.
