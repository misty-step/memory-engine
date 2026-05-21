# Evidence Handling

**Principle:** Evidence is out-of-band from `/deliver` state, but it is
version-controlled when it is part of the work record. Per-phase skills own
their own emission; `/deliver` never writes evidence itself, only records
pointers in the receipt.

## Durable Store

Default path:

```sh
source scripts/lib/evidence.sh
EVIDENCE_DIR="$(evidence_mkdir)"
# .evidence/<branch-slug>/<yyyy-mm-dd>/
```

Key by branch, not PR number. Branch-keyed evidence works offline, in
git-native mode, and in PR mode. Text evidence is normal Git content. Binary
evidence under `.evidence/` is tracked via Git LFS pointer rules in
`.gitattributes`.

## Per-Phase Emission

| Phase | Emits | Where |
|---|---|---|
| `/code-review` | `review-synthesis.md`, `verdict.json`, bench transcript refs | `.evidence/<branch>/<date>/` plus verdict ref when supported |
| `/ci` | dagger logs, failing-check tails | `<state-dir>/ci/` (gitignored) |
| `/qa` | screenshots, walkthroughs, findings | `.evidence/<branch>/<date>/` |
| `/demo` | GIFs, launch videos, no-artifact blurbs | `.evidence/<branch>/<date>/`; optional GitHub draft release upload for PR embedding |
| `/refactor` | None durable | — |
| `/implement` | None durable (test output transient) | — |

## Git Contract

- `.evidence/` is commit-eligible on feature branches.
- `.evidence/**/*.png`, `*.gif`, `*.webm`, `*.mp4`, `*.mov`, `*.jpg`,
  `*.jpeg`, and `*.webp` are LFS-tracked by `.gitattributes`.
- `qa-report.md`, `review-synthesis.md`, `verdict.json`, `trace.ndjson`,
  and command transcripts are normal Git objects.

Review transcripts and CI logs live under `.spellbook/deliver/` which is
gitignored wholesale when they are noisy phase internals. Durable summaries and
operator-facing artifacts live under `.evidence/`.

## Gitignore Convention

`.spellbook/` is gitignored repo-wide. `/deliver` state (`state.json`,
`receipt.json`, `review/`, `ci/`) lands under
`.spellbook/deliver/<ulid>/`. Operator-facing QA/demo/review artifacts land in
`.evidence/<branch>/<date>/`.

Nothing `/deliver` itself emits should be tracked by git. Phase skills may
write durable evidence to `.evidence/<branch>/<date>/`; `/deliver` only records
those paths in `receipt.json`.

## Outer-Loop Override

When `/flywheel` invokes `/deliver`, it may pass `--state-dir` for resumable
composer state. That does not change the evidence contract: durable
operator-facing evidence still defaults to `.evidence/<branch>/<date>/`.

## Composer's Role

`/deliver` itself writes exactly two files: `state.json` and `receipt.json`.
It does not write review transcripts, CI logs, screenshots, or any other
evidence. If the phase skill did not emit it, the receipt does not reference
it.
