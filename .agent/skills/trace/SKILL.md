---
name: trace
description: |
  Capture durable local memory-engine work traces: backlog refs, branch, commits, review/QA/demo evidence, transcript references, redaction notes, and final ship records. Trigger: /trace, /journal.
argument-hint: "[append|final] [--backlog NN] [--transcript-ref ref] [--evidence path]"
---

# /trace

Write the audit trail commit history cannot carry. For `memory-engine`, trace records connect shaped backlog work, branch state, CI/QA/demo evidence, review verdicts, dogfood/beta proof receipts, and final `/ship` closure.

The store is deliberately boring and local-first: append-only JSONL under `.spellbook/traces/traces.jsonl`. No hosted database. No harness-private transcript dependency. Store references and metadata, not raw conversations or private data.

## Contract

Use this skill's writer:

```sh
SKILL_DIR="<resolved path to .agent/skills/trace>"
python3 "$SKILL_DIR/scripts/write-trace.py" append \
  --backlog 030 \
  --kind trace.evidence \
  --evidence docs/qa/backlog-hygiene.md \
  --transcript-ref codex:session:<id> \
  --note "ran backlog hygiene QA receipt"

python3 "$SKILL_DIR/scripts/write-trace.py" final \
  --backlog 030 \
  --merged-sha "$(git rev-parse HEAD)" \
  --qa .evidence/qa/030.txt \
  --review refs/verdicts/cx/030-backlog-hygiene \
  --no-transcript-reason "harness did not expose transcript export"
```

`trace.final` is the record `/ship` should require or write before final reporting. If no transcript reference exists, absence must be explicit with `--no-transcript-reason`.

## Redaction

Trace files must never contain secrets, raw customer data, private transcripts, env dumps, or credentials. The writer refuses obvious token, secret, password, private key, API key, SSN-like, and credit-card-like values. Put bulky evidence in files and reference paths instead.

Active work lives in `backlog.d/`; closed work lives in `backlog.d/_done/`. Work references use `Refs-backlog: NN`; closure uses `Closes-backlog: NN` or `Ships-backlog: NN`. Archive by sourcing `scripts/lib/backlog.sh` and using `backlog_archive`.
