# Complete the Rust cutover

Priority: P0
Status: in-progress
Estimate: M

## Goal
Finish the Rust migration by removing stale TypeScript runtime artifacts, aligning operator docs with the Rust workspace, and proving the Rust beta app is the local/mobile study surface.

## Non-Goals
- Replacing the Dagger TypeScript SDK module.
- Extracting beta app crates into a separate repository.
- Adding a production hosting provider.
- Adding new runtime dependencies.

## Oracle
- [x] No tracked non-Dagger `.ts`, `.tsx`, `.mts`, or `.cts` runtime files remain.
- [x] A Rust test fails if non-Dagger TypeScript runtime files are reintroduced.
- [x] `CLAUDE.md`, `AGENTS.md`, `SPEC.md`, and beta docs describe the Rust workspace and no longer route work through deleted TypeScript paths.
- [x] `cargo test -p memory-engine-qa no_non_dagger_typescript_runtime_files_remain`
- [x] `cargo test --workspace`
- [x] `bun run rust:ci`
- [x] `bun run ci`
- [x] The Rust beta app starts locally and responds over HTTP.

## Notes
Refs-backlog: 39

The Dagger pipeline remains TypeScript because `.dagger/src/index.ts` owns CI behavior. The migration target is no TypeScript runtime, dogfood app, service, or test oracle outside Dagger.
