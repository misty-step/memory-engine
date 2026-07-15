# Trusted hosted live-generation lane

The `Generation 061 trusted live comparison` workflow is the operator-only
proof lane for the live OpenRouter generation benchmark. It is intentionally a
manual workflow on `master`; it is not a pull-request workflow.

## Boundary

The workflow checks out `master` as its trusted source with
`persist-credentials: false` and accepts one full 40-character commit SHA.
Before any credential is available it fetches only `origin` heads and requires
the SHA to be the exact head of a branch in `misty-step/memory-engine`. A fork
SHA, abbreviated SHA, mutable ref, or target not reachable from the same
repository fails closed.

Cerberus is pinned to
`b10bffb6ddb14ec553fbcf4f5e687aee13424717`. Its fresh source checkout is also
the locked dependency graph for the temporary key-helper bin: the helper uses
that checkout's `Cargo.lock` and `cargo build --locked`, with the provisioning
secret absent while dependencies/build scripts are compiled. Cerberus creates
the detached target worktree and runs the absolute helper from trusted
`master`. The helper inspects both the detached worktree config and shared Git
directory before target execution; any `http.*extraheader` auth config fails.
The helper then verifies detached `HEAD` equals the immutable SHA.

The exact command remains:

```text
cargo run --quiet -p memory-engine-bench -- generation --model google/gemini-3.5-flash --prompt principled --out <dated receipt>
```

The workflow first runs the pinned key helper in a short-lived mint process.
Cerberus `mint_review_key` performs orphan cleanup, mints one key capped at
USD 2, and writes only a mode-0600 scoped key/hash pair to runner-temp state.
That process exits before target review. The target receives only
`OPENROUTER_API_KEY`; the provisioning key is absent from its process tree,
environment, command files, and container. After the target and every labeled
target container stop, an `always()` step calls pinned
`ProvisioningClient::revoke_key`. Its cleanup trap removes scoped-key,
scoped-hash, and temporary container-env files even if revocation fails. A
future mint's orphan sweep covers a runner crash between mint and revoke.

The exact helper runs inside the pinned
`rust:1.88-bookworm@sha256:4727898c104ecd2e22d780925832502faee9fe4e70581b8572af081370b315a0`
container with `--rm --init`, private PID/IPC/UTS namespaces, a read-only
detached worktree, isolated writable Cargo caches, and one freshly removed and
recreated writable output mount under the trusted cache directory (never the
target's `docs/evals` tree). A preexisting target receipt therefore cannot
enter `/output`, and the output directory is removed during wrapper cleanup.
The container has no Docker socket, dropped capabilities, no-new-privileges,
and a PID limit.
The trusted wrapper explicitly kills any container with the run label before
staging. Native target descendants therefore cannot continue modifying
evidence or inspect the runner's `/proc`/workspace after the benchmark.

The helper requires all 14 exact corpus scenarios, `Provider failures: 0/14
sources`, and no `FAILED`/error rows before printing proof markers. Receipt and
credential scans use explicit status handling: 0 means a match and rejects, 1
means no match, and any larger status means scanner failure and rejects. The
receipt validator uses `grep` available in the pinned image; the evidence
scanner preflights `rg` on the hosted runner.

## Evidence and threat model

The workflow has `contents: read` only, no `pull_request` trigger, and no write
permission. Actions are pinned to reviewed full commit SHAs. Benchmark and
receipt publication must both succeed before evidence staging; failures do
not satisfy the live-lane upload. Staging copies only the exact current-run
receipt path plus current-run transcript/artifacts, never historical
`docs/evals` receipts. Raw candidates are scanned, deleted, and copied into a
separate safe directory. Upload additionally requires container stop,
revocation, command-file/runner-state cleanup, scanner success, and an
immediate SHA-256 recheck and rescan.

An arbitrary fork cannot invoke this workflow because there is no pull-request
trigger, the job is gated to `misty-step/memory-engine` on `master`, checkout
credentials are disabled, and the submitted SHA must be an exact
same-repository branch head. The same-repository target is still treated as untrusted
benchmark code: it receives only the bounded, revocable key and no Cerberus or
GitHub authority. A compromised benchmark can spend at most USD 2 during the
run; the lane is not a general-purpose execution grant.

## Dispatch

From a checked-out repository, after the target commit is pushed to a branch in
the same repository:

```sh
gh workflow run generation-061-live.yml --ref master -f head_sha="$(git rev-parse HEAD)"
gh run watch
```

The live receipt is the required proof for memory-engine-061. A missing-secret
run is expected to fail and is not live-generation proof; it demonstrates the
fail-closed precondition only.

## Master branch protection proof

Read-only verification on 2026-07-15, using
`gh api repos/misty-step/memory-engine/branches/master/protection`, returned:

- strict required status checks: exactly `ci` and `review`;
- required approving reviews: `0`;
- conversation resolution required: `true`;
- linear history required: `true`;
- administrator enforcement: `true`;
- force pushes: disabled; deletions: disabled.

This policy is infrastructure authority for the default branch and was not
mutated by the 098 repair.
