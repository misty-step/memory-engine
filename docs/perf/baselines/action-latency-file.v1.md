# Core action latency

- Schema: `memory_engine.action_latency_receipt.v1`
- Git SHA: `f79c7102b2be4591c72689b3d891ed02e5bce712`
- Backend: `file`
- Iterations: `5`

| Action | Family | p50 (ms) | p95 (ms) | Max (ms) | Soft p95 | Hard p95 | Status |
| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| `auth.app_home` | auth | 0 | 0 | 0 | 50 | 5000 | PASS |
| `auth.login_request` | auth | 13 | 18 | 18 | 100 | 5000 | PASS |
| `material.capture` | material | 21 | 27 | 27 | 200 | 5000 | PASS |
| `generation.enqueue` | generation | 8 | 34 | 34 | 200 | 5000 | PASS |
| `review.next` | review | 0 | 51 | 51 | 50 | 5000 | SOFT WARN |
| `review.submit` | review | 1 | 4 | 4 | 100 | 5000 | PASS |
| `review.content_feedback` | review | 1 | 3 | 3 | 100 | 5000 | PASS |
| `review.reveal` | review | 0 | 0 | 0 | 50 | 5000 | PASS |
| `review.skip` | review | 1 | 2 | 2 | 50 | 5000 | PASS |
| `review.snooze` | review | 1 | 2 | 2 | 50 | 5000 | PASS |
| `auth.logout` | auth | 5 | 7 | 7 | 50 | 5000 | PASS |
