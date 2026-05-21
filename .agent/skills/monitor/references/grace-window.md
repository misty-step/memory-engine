# Memory Engine Monitor Window

There is no production rollout window here. Use a bounded watch over repository signals.

Default sequence:

1. Run or inspect the latest `bun run ci` result.
2. Run or inspect the latest `bun run qa` result when QA is the signal.
3. For beta work, run the focused `experiments/*` test named by the ticket.
4. For lifecycle work, compare `backlog.d/`, `_done/`, and closure trailers.

Stop on the first trip and route to `/diagnose`. Close clean when the named signals stay green for the requested pass.
