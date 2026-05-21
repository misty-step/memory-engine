# Memory Engine Monitor Signals

Use repository signals, not production telemetry.

- `bun run ci`: canonical Dagger gate.
- `bun run qa`: full QA sweep, including dogfood lanes and benchmarks.
- `bun test experiments/beta-store/` and `bun test experiments/beta-generation/`: current beta proof lanes.
- `tests/api/`, `tests/testkit/`, and `tests/adapters/`: public package surface drift.
- `backlog.d/` plus `scripts/lib/backlog.sh`: lifecycle contradictions.

Trip means a command fails, an expected proof artifact is missing, or the tracker contradicts shipped evidence. Hand the exact signal to `/diagnose`; do not repair inside `/monitor`.
