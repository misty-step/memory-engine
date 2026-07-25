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
dependency-preparation container from the pinned
`rust:1.88-bookworm@sha256:4727898c104ecd2e22d780925832502faee9fe4e70581b8572af081370b315a0`
build image with network access only for Cargo resolution. The target worktree
is read-only, Cargo home and target are fresh tmpfs mounts, and the container
receives no provider credential. It builds the exact `memory-engine-bench`
binary with `cargo build --locked` and copies only that binary to a trusted
cache. The networked build stage is target-controlled, so the trusted
wrapper requires the prepared directory to hold
exactly one regular executable — the digest-checked benchmark — and fails
closed on any extra entry a hostile `build.rs` may have planted. The build container and its
dependency state are removed, with explicit fail-closed absence proof, before
provider capability creation.

The exact prepared binary then runs inside the digest-pinned minimal
`debian:bookworm-slim@sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818`
runtime image, which carries no Cargo, rustc, or toolchain the target could
use to rebuild and execute new code. The runtime container uses `--rm
--init`, private PID/IPC/UTS namespaces, and no network. It mounts no
repository source: only the eval corpus data directory (read-only, at the
manifest path baked into the binary), the exact prepared binary file
(read-only — never the prepared directory), the trusted in-container
helper/validator, and the Unix-socket provider capability. Its only writable
report surface is a bounded `noexec,nosuid` tmpfs at `/output`; the validated
receipt leaves the container exclusively through the size-capped, redacted
proof markers on stdout, so no host output bind mount exists at all. A
preexisting target receipt therefore cannot enter evidence, and the target
cannot read `/workspace` sources to smuggle them into provider calls or
reports. The target never receives the scoped provider key. A trusted proxy
owns the upstream OpenRouter connection, refuses every upstream redirect
(the bearer header can never follow a Location), enforces a strict
allowlisted request schema and model plus global call/byte/concurrency
budgets, and writes request/response hashes, call counts, and the attestation
outside the target tree. The container has no Docker socket, dropped
capabilities, no-new-privileges, and a PID limit.
The trusted wrapper explicitly kills any container with the run label before
staging. Both build and runtime have bounded timeouts. The wrapper writes
trusted runtime-cleanup evidence proving container removal, timeout state,
proxy shutdown, attestation validation, and prepared-directory deletion;
staging requires that evidence to be true. Cleanup proof fails closed: only
an explicit docker "no such container" report counts as absence, any docker
daemon error keeps `container_removed` false and fails the run, and a
label-filter sweep must succeed and come back empty. Native target
descendants therefore cannot continue modifying evidence or inspect the
runner's `/proc`/workspace after the benchmark.

The helper requires all 14 exact corpus scenarios, `Provider failures: 0/14
sources`, and no `FAILED`/error rows before printing proof markers. Receipt and
credential scans use explicit status handling: 0 means a match and rejects, 1
means no match, and any larger status means scanner failure and rejects. The
receipt validator uses `grep` available in the pinned image; the evidence
scanner preflights `rg` on the hosted runner.

## Evidence and threat model

The workflow has read-only `contents` and `checks` permission, no write
permission, and no `pull_request` trigger. Actions are pinned to
reviewed full commit SHAs. Benchmark and receipt publication must both
succeed before evidence staging; failures do not satisfy the live-lane
upload. Published evidence is only trusted schema-validated size-capped
fields: the proxy's provider attestation, the wrapper's runtime-cleanup
proof, and the validated dated receipt (whose stdout capture, extraction,
and validator all enforce a 256 KiB cap). The raw target transcript,
Cerberus artifact, and receipt bundle never leave the runner, and never
historical `docs/evals` receipts. Candidates are staged descriptor-first and
scanned as immutable safe copies. Upload additionally requires container
stop, revocation, command-file/runner-state cleanup, scanner success, and an
immediate SHA-256 recheck and rescan.
Each secret-bearing step traps the shared command-file audit helper before
exit. The final transport audit runs after publish, so mint, benchmark, revoke,
and publish snapshots all remain readable until they have been audited before
evidence staging; those prior-step snapshots are not trusted evidence by
themselves.

The publish receipt is created under the trusted generation cache, outside the
target worktree. Evidence staging is descriptor-based, not check-then-copy:
sources are opened `O_NOFOLLOW` and `fstat`-verified as regular single-link
files, destinations are created `O_CREAT|O_EXCL|O_NOFOLLOW` inside verified
real directories (openat-anchored on the hosted runner), executable modes are
preserved, and every staged file is bounded by an explicit size cap. A
symlink or pre-existing file planted at either end fails closed with no
copy, and the credential scan runs on the immutable safe copy rather than
the racy source path. The provider proxy also bounds each Unix-socket
request read to 16 MiB and 30 seconds, on top of its global budgets.

An arbitrary fork cannot invoke this workflow because there is no pull-request
trigger, the job is gated to `misty-step/scry` on `master`, checkout
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

From a checked-out repository, after the final card061 pull request is
squash-merged so its merge commit is the current `master` tip:

```sh
gh workflow run generation-061-live.yml --ref master \
  -f head_sha="$(git rev-parse refs/remotes/origin/master)" \
  -f pull_request_number=<merged card061 PR number>
gh run watch
```

The pull request number is required: the workflow refuses to run unless that
PR is merged, its squash commit is exactly `head_sha`, and the reviewed PR
head tree equals the `head_sha` tree.

The live receipt is the required proof for memory-engine-061. Card 098 does not
claim a live receipt while this workflow is still draft; successful live
dispatch and its artifact remain downstream card061 acceptance work. A
missing-secret run is expected to fail and is not live-generation proof; it
demonstrates the fail-closed precondition only.

## Trust anchor

The trusted computing base of this lane — the outer comparison wrapper, the
provider proxy, the descriptor-based staging/copy helpers, the receipt and
attestation validators, the command-file audit helper, and the key-lifecycle
helper — executes from the same validated `refs/remotes/origin/master` commit
that is being evaluated. There is no separately hosted copy of these files;
the enforceable protected-master trust model is therefore explicit:

- The submitted SHA must equal the exact current `refs/remotes/origin/master`
  object, so the trusted files are always the reviewed tip of the protected
  branch, never an arbitrary reachable commit.
- `master` is protected (see the protection proof below): strict required
  `ci` and Cerberus `review` status checks, linear history, administrator
  enforcement, and no force pushes or deletions. Every change to a trusted
  file passes those gates and the QA regression suite that pins this lane's
  security contract before it can become the trusted revision.
- The workflow does not assume protection was applied, and it does not
  accept a bare check run on the master commit as review proof: a squash
  merge mints a fresh master SHA while Cerberus `review` runs on the
  reviewed PR head, and a `workflow_dispatch` of the review workflow against
  master could plant a green `review` on master while reviewing an unrelated
  diff. Before any provider authority exists, the workflow instead binds the
  evaluated commit to the exact merged pull request named in the dispatch:
  the PR must be merged into `master` of this repository, its
  `merge_commit_sha` must equal the evaluated SHA, the reviewed PR head tree
  must equal the evaluated commit's tree (protected master requires
  up-to-date branches and linear history, so a faithful squash preserves the
  tree), a successful Cerberus `review` check run must exist on that exact
  PR head SHA, and a successful `ci` check run must exist on the evaluated
  master SHA itself. It then prints the SHA-256 digest of every trusted file
  into the run log so the trusted-file set of any run is auditable after the
  fact.
- The residual trust statement is deliberate: an attacker who can land a
  malicious commit on protected `master` — passing hosted CI, Cerberus
  review, and the QA contract tests — already owns the repository's release
  path; this lane does not create that authority and cannot exceed its USD 2
  scoped-key bound.

The target binary remains untrusted even under this anchor: it is built from
the same commit, but `build.rs`, proc-macros, and the benchmark runtime are
treated as hostile and confined by the container, mount, proxy-schema, and
budget boundaries above.

## Master branch protection proof

Read-only verification on 2026-07-15, using
`gh api repos/misty-step/scry/branches/master/protection`, returned:

- strict required status checks: exactly `ci` and `review`;
- required approving reviews: `0`;
- conversation resolution required: `true`;
- linear history required: `true`;
- administrator enforcement: `true`;
- force pushes: disabled; deletions: disabled.

This policy is infrastructure authority for the default branch and was not
mutated by the 098 repair.
