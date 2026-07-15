# Trusted hosted live-generation lane

The `Generation 061 trusted live comparison` workflow is the operator-only
proof lane for the live OpenRouter generation benchmark. It is intentionally a
manual workflow on `master`; it is not a pull-request workflow.

## Boundary

The workflow checks out `master` as its trusted source with
`persist-credentials: false` and accepts one full 40-character commit SHA for
the final card061 commit on `origin/master`. Before any credential is available
it requires the input to equal the exact `refs/remotes/origin/master` object. A
fork SHA, abbreviated SHA, mutable ref, or target from any other branch fails
closed.

Cerberus is pinned to
`b10bffb6ddb14ec553fbcf4f5e687aee13424717`. Its fresh source checkout is also
the locked dependency graph for the temporary key-helper bin: the helper uses
that checkout's `Cargo.lock` and `cargo build --locked`, with the provisioning
secret absent while dependencies/build scripts are compiled. Cerberus creates
the detached target worktree and runs the absolute helper from trusted
`master`. The helper inspects both the detached worktree config and shared Git
directory before target execution; any `http.*extraheader` auth config fails.
The helper then verifies detached `HEAD` equals the immutable SHA.

The Cerberus request evaluates the exact current commit against its first
parent (`HEAD_SHA^..HEAD_SHA`), not `origin/master..HEAD_SHA`: exact master
validation intentionally makes the latter an empty range.

The exact target behavior remains the following command arguments, executed by
the digest-checked prepared binary after the trusted build:

```text
memory-engine-bench generation --model google/gemini-3.5-flash --prompt principled --out <dated receipt>
```

The workflow first runs the pinned key helper in a short-lived mint process.
Cerberus `mint_review_key` performs orphan cleanup, mints one key capped at
USD 2, and writes only a mode-0600 scoped key/hash pair to runner-temp state.
That process exits before target review. The target receives only a one-run
`OPENROUTER_PROXY_TOKEN` capability over the trusted Unix socket; scoped and
provisioning keys are absent from its process tree, environment, command files,
and container. After the target and every labeled
target container stop, an `always()` step calls pinned
`ProvisioningClient::revoke_key`. Its cleanup trap removes scoped-key,
scoped-hash, and temporary container-env files even if revocation fails. A
future mint's orphan sweep covers a runner crash between mint and revoke.

Before any provider proxy or scoped key exists, the trusted helper runs a
dependency-preparation container from the pinned image with network access
only for Cargo resolution. The target worktree is read-only, Cargo home and
target are fresh tmpfs mounts, and the container receives no provider
credential. It builds the exact `memory-engine-bench` binary with
`cargo build --locked`, copies only that binary to a trusted cache, and records
its SHA-256 digest. The build container and its dependency state are removed
before provider capability creation.

The exact prepared binary then runs inside the pinned
`rust:1.88-bookworm@sha256:4727898c104ecd2e22d780925832502faee9fe4e70581b8572af081370b315a0`
container with `--rm --init`, private PID/IPC/UTS namespaces, no network,
a read-only detached worktree, the digest-checked binary mounted read-only, and
one freshly removed and recreated writable output mount under the trusted cache
directory (never the target's `docs/evals` tree). A preexisting target receipt
therefore cannot enter `/output`, and the output directory is removed during
wrapper cleanup. The runtime has no Cargo home or target mount, so build.rs
cannot retain credentials or write persistent host Cargo state. The only
trusted boundary mounted into the target is a Unix-socket provider capability;
the target never receives the scoped provider key. A trusted proxy owns the
upstream OpenRouter connection, request/response hashes, call count, and
attestation written outside the target tree. The container has no Docker
socket, dropped capabilities, no-new-privileges, and a PID limit.
The trusted wrapper explicitly kills any container with the run label before
staging. Both build and runtime have bounded timeouts. The wrapper writes
trusted runtime-cleanup evidence proving container removal, timeout state,
proxy shutdown, attestation validation, and deletion of prepared/output
directories; staging requires that evidence to be true. Native target
descendants therefore cannot continue modifying evidence or inspect the
runner's `/proc`/workspace after the benchmark.

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
Each secret-bearing step traps the shared command-file audit helper before
exit. The final transport audit runs after publish, so mint, benchmark, revoke,
and publish snapshots all remain readable until they have been audited before
evidence staging; those prior-step snapshots are not trusted evidence by
themselves.

The publish receipt is created under the trusted generation cache, outside the
target worktree. The trusted staging helper rejects every existing symlink or
non-regular source before copying and uses no-dereference copying; a target
symlink cannot redirect staging into a runner-owned file. The provider proxy
also bounds each Unix-socket request read to 16 MiB and 30 seconds.

An arbitrary fork cannot invoke this workflow because there is no pull-request
trigger, the job is gated to `misty-step/memory-engine` on `master`, checkout
credentials are disabled, and the submitted SHA must be an exact
`refs/remotes/origin/master` object. The same-repository target is still
treated as untrusted benchmark code: it receives only a one-run local proxy
capability and no provisioning key, Cerberus, GitHub, or general network
authority. A compromised benchmark can spend at most USD 2 through the proxy;
the lane is not a general-purpose execution grant.

The target's receipt and stdout are untrusted report material, not canonical
provider proof. Canonical acceptance requires the trusted proxy's attestation
to show at least 15 successful upstream calls and a matching immutable target
SHA. A preseeded or forged `/output` receipt, stdout marker, zero-call report,
malicious `build.rs`, cache retention, or direct-egress attempt cannot create
that attestation. The final published evidence includes the report only as
supporting target behavior; the provider attestation is the acceptance oracle.

## Dispatch

From a checked-out repository, after the final card061 commit is present on
`master`:

```sh
gh workflow run generation-061-live.yml --ref master -f head_sha="$(git rev-parse refs/remotes/origin/master)"
gh run watch
```

The live receipt is the required proof for memory-engine-061. Card 098 does not
claim a live receipt while this workflow is still draft; successful live
dispatch and its artifact remain downstream card061 acceptance work. A
missing-secret run is expected to fail and is not live-generation proof; it
demonstrates the fail-closed precondition only.

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
