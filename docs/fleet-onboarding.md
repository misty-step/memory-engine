# Fleet onboarding

This is the repo-local receipt for the current fleet integration boundary. It
points at the live authorities.

## Current decisions

| Surface | Current authority | Decision and proof |
| --- | --- | --- |
| Canary | `memory-engine-api` runtime and `docs/runbook.md` | Shipped. `CANARY_ENDPOINT` and `CANARY_API_KEY` are documented production variables; API 500s report as `memory-engine-api`. |
| Work context | `AGENTS.md`, the current operator request, and [`misty-step/scry` issues](https://github.com/misty-step/scry/issues) | Direct work uses current scope, overlap checks, and session or PR evidence. GitHub Issues preserves history; no issue or maintained backlog is required. |
| Landmark | `.landmark.yml` plus Landmark CLI | Adopted in manifest-only/synthesis-only mode. The repo has no release-secret authority, so no release-mutating workflow is added. Preview with `landmark setup --repo-root . --dry-run` and `landmark run --provider local --repo-root . --dry-run` from a Landmark checkout. |
| Project map | `docs/architecture/memory-engine.map.json` and `workbench.html` | This is the live repo-local architecture-map successor for project structure. It is static, diffable, and explicitly separate from Landmark release intelligence. |

## Verification

```sh
python3 -m json.tool docs/architecture/memory-engine.map.json >/dev/null
test -f docs/architecture/workbench.html
cargo test -p memory-engine-qa --test docs_reconciliation fleet_onboarding_contract_is_declarative_and_current -- --exact
```

The canonical project map is intentionally not generated from Landmark or
runtime code. The Rust crates and tests remain runtime truth.
