# Weave into the fleet — onboarding checklist epic

Priority: P3 · Status: pending · Estimate: S

## Goal

Connect Memory Engine to the standard fleet tooling other actively-developed
repos use: observability (Canary), release intelligence (Landmark), backlog
board sync (Powder), and automated PR review (Cerberus).

## Oracle

- [ ] **Canary: already done.** `CANARY_ENDPOINT`/`CANARY_API_KEY` are live
      production secrets per `docs/runbook.md`; every API 500 reports to
      Canary as service `memory-engine-api`. No action needed — recorded here
      so this epic doesn't re-do it.
- [ ] Landmark: repo adopts Landmark's local-CLI or GitHub Action mode for
      conventional-commit release intelligence (version decisions, changelog,
      release notes) — memory-engine's commit history already uses
      conventional-commit-shaped prefixes (`feat:`, `fix:`, `docs:`, `ci:`),
      so this should be close to a drop-in per `landmark/README.md`'s
      adoption-mode ladder (start with local CLI preview, no secrets needed).
- [ ] Powder: memory-engine's `backlog.d/` is importable into a Powder board
      via `powder-cli import` (fixture path confirmed working against
      `crates/powder-core/tests/fixtures/backlog.d` in powder's own repo) —
      confirm the format compatibility against memory-engine's actual
      `backlog.d/` (numbered `.md` files, `_done/` archive) and either import
      or document why not.
- [ ] Cerberus: a review webhook is wired on the `misty-step/memory-engine`
      GitHub repo so PRs get automated review per Cerberus's `review-pr`
      GitHub adapter path.

## Notes

Operator's words, relayed via groom dispatch 2026-07-02: "the afternoon-
onboarding checklist" — "canary key + landmark workflow + powder backlog
import + cerberus review webhook."

I found no single existing document titled an "afternoon-onboarding
checklist" in this repo, the fleet checkouts, or the daybook vault — I
reconstructed the four items from each tool's own README/AGENTS.md
(`canary/`, `landmark/`, `powder/`, `cerberus/` under `~/Development`). If a
canonical checklist doc exists elsewhere, point future delivery at it instead
of this reconstruction.

This is the lowest-priority of the four epics filed today — it's plumbing,
not product, and nothing else in this backlog depends on it landing first.

## Children

1. Confirm Canary is done (no code change — a verification-only item).
2. Land Landmark local-CLI preview; decide on GitHub Action mode.
3. Attempt `powder-cli import` against memory-engine's `backlog.d/`; fix
   format mismatches or document the gap.
4. Wire Cerberus's `review-pr` GitHub adapter onto the repo.
