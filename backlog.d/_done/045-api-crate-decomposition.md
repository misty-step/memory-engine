# Decompose memory-engine-api and test its security invariants

Priority: P1 · Status: pending · Estimate: L

## Goal

The API crate stops being a 3,951-line god module: storage behind one trait,
transport/render/registry separated, and the invariants that would embarrass
us (account isolation, CSRF, idempotency) pinned by tests including the
postgres path in CI.

## Oracle

- [ ] One storage trait with file and postgres implementations; the ~520-line
      backend match dispatch in `memory-engine-api/src/lib.rs` is gone; a bug
      fix in one backend cannot silently miss the other.
- [ ] CI exercises the postgres store (service container or dagger postgres),
      not just the file store.
- [ ] Behavior tests prove: account A cannot read account B's sources/reviews;
      a CSRF-less mutation is rejected; duplicate `idempotencyKey` submits
      produce one attempt under concurrency.
- [ ] HTML rendering, HTTP handlers, and the account registry live in
      separate modules/files; no module both parses forms and runs SQL.
- [ ] Mutex `.expect("account registry lock")` sites (13) replaced with
      poison-tolerant or lock-free design; no panic path reachable from a
      request.

## Notes

Findings from fresh-context audit, vetted: file is 3,951 lines with 22
handlers, 15+ render fns, registry, session logic, and storage dispatch
co-resident; postgres crate is 1,754 LOC with 5 unit tests and zero CI
execution vs the file store's 653-line suite. Decompose by responsibility,
not by ceremony — keep interfaces small (Ousterhout), and let the storage
trait be the one deep seam. Do this after or alongside 042; before 043 grows
the surface further.

## Children

1. Extract storage trait; port both backends; delete the match dispatch.
2. Postgres in CI.
3. Security-invariant test suite (isolation, CSRF, idempotency, session
   fixation).
4. Module split: router/handlers, html render, registry, auth.
5. Panic-path sweep (expect/unwrap audit in request paths).
