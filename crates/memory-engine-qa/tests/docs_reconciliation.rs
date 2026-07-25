use std::{fmt::Write as _, fs, path::Path};

#[cfg(unix)]
use std::os::unix::fs::symlink;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("qa crate is under crates/")
}

fn read_repo_file(relative: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("failed to read {relative}: {error}"))
}

fn test_temp_dir(label: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("memory-engine-qa-{label}-{stamp}"));
    fs::create_dir_all(&path).expect("create temporary test directory");
    path
}

fn github_workflows() -> Vec<(String, String)> {
    let directory = repo_root().join(".github/workflows");
    fs::read_dir(directory)
        .expect("read GitHub workflows")
        .map(|entry| entry.expect("read workflow entry").path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("yml" | "yaml")
            )
        })
        .map(|path| {
            let name = path
                .file_name()
                .expect("workflow filename")
                .to_string_lossy()
                .into_owned();
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            (name, text)
        })
        .collect()
}

fn assert_contains_all(relative: &str, text: &str, expected: &[&str]) {
    for needle in expected {
        assert!(text.contains(needle), "{relative} is missing `{needle}`");
    }
}

#[test]
fn agent_docs_match_post_cutover_contract() {
    let agents = read_repo_file("AGENTS.md");

    assert!(
        !agents.contains("/settle"),
        "AGENTS.md must not route lifecycle work through an unavailable /settle command"
    );
    assert!(
        !agents.contains("Complete the Rust cutover before adding new product scope"),
        "AGENTS.md must not describe the completed Rust cutover as future work"
    );
    assert!(
        agents.contains("docs/runbook.md"),
        "AGENTS.md must point cold agents at the production deployment runbook"
    );
    assert!(
        agents.contains("historical extraction context"),
        "AGENTS.md must identify SLICE docs and exemplars as historical context"
    );
}

#[test]
fn historical_shape_packets_are_explicitly_marked() {
    for relative in [
        "SLICE-1-KERNEL.md",
        "SLICE-2-PROGRESSION.md",
        "SLICE-3-RUBRIC.md",
        "SLICE-4-SERVICE-PROTOTYPE.md",
        "exemplars.md",
    ] {
        let text = read_repo_file(relative);
        assert_contains_all(
            relative,
            &text,
            &[
                "Historical note",
                "not active ground truth",
                "SPEC.md",
                "docs/runbook.md",
            ],
        );
    }
}

#[test]
fn historical_slice_frontmatter_is_not_actionable() {
    for relative in [
        "SLICE-1-KERNEL.md",
        "SLICE-2-PROGRESSION.md",
        "SLICE-3-RUBRIC.md",
    ] {
        let text = read_repo_file(relative);
        assert!(
            text.contains("status: historical"),
            "{relative} frontmatter must not advertise an active implementation status"
        );
    }
}

#[test]
fn readme_quickstart_points_to_current_rust_and_deployed_surface() {
    let readme = read_repo_file("README.md");

    assert_contains_all(
        "README.md",
        &readme,
        &[
            "## Quickstart",
            "cargo fmt --all --check",
            "cargo test --workspace",
            "cargo clippy --workspace --all-targets -- -D warnings",
            "cargo doc --workspace --no-deps",
            "bun run ci:local",
            "bun run ci",
            "bun run ci:full",
            "MEMORY_ENGINE_ENABLE_FILE_STORE=true",
            "cargo run -p memory-engine-api",
            "curl -fsS http://127.0.0.1:18080/healthz",
            "docs/runbook.md",
        ],
    );
    assert!(
        !readme.contains("Roadmap and shaping docs"),
        "README must not present historical slice packets as the active roadmap"
    );
}

#[test]
fn runbook_contains_reproducible_digitalocean_smoke_commands() {
    let runbook = read_repo_file("docs/runbook.md");

    assert_contains_all(
        "docs/runbook.md",
        &runbook,
        &[
            "App: `memory-engine-api`",
            "Platform: DigitalOcean App Platform",
            "direct origin `https://memory-engine-api-i2xcr.ondigitalocean.app`",
            "MEMORY_ENGINE_POSTGRES_URL",
            "MEMORY_ENGINE_ENABLE_FILE_STORE=true",
            "MEMORY_ENGINE_AUTH_ALLOWED_EMAILS",
            "do-connecting-ip",
            "## Deployed smoke",
            "base=\"https://scry.study\"",
            "curl -fsS --max-time 15 -o /tmp/memory-engine-healthz -w \"%{http_code}\" \"$base/healthz\"",
            "curl -fsS --max-time 15 -o /tmp/memory-engine-home -w \"%{http_code}\" \"$base/\"",
            "curl -fsS --max-time 15 -o /tmp/memory-engine-auth-boundary -w \"%{http_code}\" -X POST \"$base/app/generate\"",
            "case \"$status\" in 4??)",
        ],
    );

    assert!(
        !repo_root().join(".github/workflows/deploy.yml").exists(),
        "the retired provider deploy workflow must not be restored after the DigitalOcean cutover"
    );
    assert!(
        !repo_root().join("fly.toml").exists(),
        "the retired provider manifest must not remain a runnable rollback path"
    );
    for (workflow, text) in github_workflows() {
        for retired_surface in ["flyctl", "FLY_API_TOKEN", "memory-engine-api.fly.dev"] {
            assert!(
                !text.contains(retired_surface),
                ".github/workflows/{workflow} restores retired Fly surface `{retired_surface}`"
            );
        }
    }
    assert!(
        !runbook.contains("still deployed by CI on every push"),
        "the runbook must not advertise the retired provider as a live deployment target"
    );
}

#[test]
fn current_runtime_contract_has_no_retired_provider_recreation_path() {
    for relative in [
        "AGENTS.md",
        "README.md",
        "VISION.md",
        "docs/runbook.md",
        ".agents/skills/scry-qa/SKILL.md",
        "docs/api/openapi.v1.json",
        "bin/send-magic-link",
        "crates/memory-engine-api-state/src/lib.rs",
        "crates/memory-engine-api/src/tests/mod.rs",
        "crates/memory-engine-contract/src/main.rs",
    ] {
        let text = read_repo_file(relative).to_ascii_lowercase();
        for retired_surface in [
            "memory-engine-api.fly.dev",
            "flyctl",
            "fly.toml",
            "fly-client-ip",
            "fly machines",
            "temporary fly standby",
            "rollback platform: fly",
            "canary-obs",
        ] {
            assert!(
                !text.contains(retired_surface),
                "{relative} retains active retired-provider surface `{retired_surface}`"
            );
        }
    }
}

#[test]
fn fleet_onboarding_contract_is_declarative_and_current() {
    let landmark = read_repo_file(".landmark.yml");
    assert_contains_all(
        ".landmark.yml",
        &landmark,
        &[
            "product:",
            "name: Scry",
            "changelog:",
            "source: auto",
            "release:",
            "profile: synthesis-only",
        ],
    );

    let cerberus = read_repo_file(".github/workflows/cerberus-review.yml");
    assert_contains_all(
        ".github/workflows/cerberus-review.yml",
        &cerberus,
        &[
            "pull_request:",
            "misty-step/cerberus",
            "v0.72.0",
            "review-pr",
            "CERBERUS_GH_TOKEN",
            "CERBERUS_OPENROUTER_PROVISIONING_KEY",
            "--harness container-opencode",
            "--container-binary",
            "--openrouter-scoped-key",
            "--openrouter-provisioning-key-env CERBERUS_OPENROUTER_PROVISIONING_KEY",
            "--openrouter-key-limit-usd 2",
            "--summary-target status",
            "--post",
        ],
    );
    assert!(
        cerberus.contains("if: steps.preflight.outputs.ready == 'true'"),
        "Cerberus must skip cleanly when its provisioning key is unavailable"
    );
    assert!(
        !cerberus.contains("--allow-env OPENROUTER_API_KEY"),
        "Cerberus must not forward a long-lived provider key to an untrusted PR"
    );
    assert!(
        !cerberus.contains("--harness opencode"),
        "Cerberus must not use the unsandboxed OpenCode harness for PR reviews"
    );

    let onboarding = read_repo_file("docs/fleet-onboarding.md");
    assert_contains_all(
        "docs/fleet-onboarding.md",
        &onboarding,
        &[
            "memory-engine.map.json",
            "CANARY_ENDPOINT",
            "memory-engine-api",
            "Powder",
            "Cerberus",
            "Bitterblossom",
        ],
    );

    let map = read_repo_file("docs/architecture/memory-engine.map.json");
    assert_contains_all(
        "docs/architecture/memory-engine.map.json",
        &map,
        &[
            "fleet-integration",
            "node.fleet.landmark",
            "node.fleet.cerberus",
            "node.fleet.canary",
            "node.fleet.powder",
        ],
    );
    assert!(
        !map.contains("edge.fleet.powder-to-card"),
        "the Powder fleet node already references memory-engine-067; do not retain a stale historical-card edge"
    );
}

#[test]
fn trusted_live_generation_lane_is_default_branch_only_and_fail_closed() {
    let workflow = read_repo_file(".github/workflows/generation-061-live.yml");

    assert_contains_all(
        ".github/workflows/generation-061-live.yml",
        &workflow,
        &[
            "workflow_dispatch:",
            "head_sha:",
            "persist-credentials: false",
            "git config --get-regexp 'http\\..*extraheader' >/dev/null 2>&1",
            "github.ref == 'refs/heads/master'",
            "ref: master",
            "[[ \"$HEAD_SHA\" =~ ^[0-9a-f]{40}$ ]]",
            "test \"$HEAD_SHA\" = \"$(git rev-parse refs/remotes/origin/master)\"",
            "--local-runtime-command \"$GITHUB_WORKSPACE/scripts/generation-061-live-comparison.sh\"",
            "--allow-env GENERATION_PROVIDER_KEY",
            "CERBERUS_OPENROUTER_PROVISIONING_KEY is required; refusing to run live generation",
            "receipt=\"$published_dir/generation-061-live-comparison-${date_utc}.md\"",
            "id: sanitize_evidence",
            "steps.run_benchmark.outcome == 'success'",
            "steps.publish_receipt.outcome == 'success'",
            "target/generation-061-live-safe/**",
            "if-no-files-found: error",
            "uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683",
            "uses: dtolnay/rust-toolchain@fa04a1451ff1842e2626ccb99004d0195b455a88",
            "uses: actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02",
        ],
    );
    assert!(
        !workflow.contains("pull_request:"),
        "the trusted live lane must never receive fork or PR workflow code"
    );
    assert!(
        !workflow.contains("pull_request_target:"),
        "the trusted live lane must never receive fork or PR workflow code"
    );
    assert!(
        !workflow.contains("uses: actions/checkout@v")
            && !workflow.contains("uses: dtolnay/rust-toolchain@stable")
            && !workflow.contains("uses: actions/upload-artifact@v"),
        "secret-bearing actions must be pinned to immutable commits"
    );
    assert!(
        !workflow.contains("contents: write"),
        "the trusted live lane cannot need repository write authority"
    );
    assert!(
        !workflow.contains("--allow-env CERBERUS_OPENROUTER_PROVISIONING_KEY"),
        "the provisioning key must never enter the target subprocess"
    );
    assert!(
        !workflow.contains("--allow-env GITHUB_TOKEN"),
        "the target subprocess must not receive GitHub authority"
    );
    let upload = workflow
        .split("- name: Upload dated live evidence")
        .nth(1)
        .expect("upload step exists");
    assert!(
        !upload.contains("target/generation-061-live/transcript.txt")
            && !upload.contains("target/generation-061-live/artifact.json"),
        "the upload step must never publish raw early-failure evidence"
    );
    let staging = workflow
        .split("- name: Stage only scanned live evidence")
        .nth(1)
        .and_then(|rest| rest.split("\n      - name:").next())
        .expect("staging step exists");
    assert!(
        !staging.contains("transcript.txt")
            && !staging.contains("artifact.json")
            && !staging.contains("cerberus-receipt.json"),
        "published evidence is only trusted schema-validated fields: the raw target transcript and harness bundles must never be staged"
    );
    assert!(
        staging.contains("provider-attestation.json")
            && staging.contains("runtime-cleanup.json")
            && staging.contains("$RECEIPT_PATH"),
        "staging must cover exactly the trusted attestation, cleanup proof, and validated receipt"
    );
    assert!(
        !workflow.contains("find docs/evals -maxdepth 1") && !workflow.contains("! rg -n -F"),
        "staging and scans must be exact and explicit, never historical/globbed or inverted"
    );
    assert!(
        !workflow.contains("target SHA is not a head of any branch"),
        "the target must not be accepted merely because it heads another remote branch"
    );
}

#[test]
fn trusted_live_request_uses_a_nonempty_parent_to_exact_head_range() {
    let workflow = read_repo_file(".github/workflows/generation-061-live.yml");
    let request = workflow
        .split("- name: Build the exact live-generation request")
        .nth(1)
        .expect("request step exists")
        .split("- name: Pin the Cerberus fixture substrate")
        .next()
        .expect("request step ends before fixture setup");
    assert!(
        request.contains("base_sha=\"$(git rev-parse \"$HEAD_SHA^1\")\"")
            && request.contains("--base \"$base_sha\"")
            && request.contains("--head \"$HEAD_SHA\""),
        "the exact current master commit must be evaluated against its parent, not an empty master-to-self range"
    );
    assert!(
        request.contains("if git diff --quiet \"$base_sha\" \"$HEAD_SHA\"; then"),
        "an empty tree diff (e.g. an empty commit on master) must be refused, not merely a parent != head SHA comparison"
    );
}

#[test]
fn trusted_live_lane_verifies_protected_master_gates_before_any_secret() {
    let workflow = read_repo_file(".github/workflows/generation-061-live.yml");
    let gates = workflow
        .find("- name: Verify protected-master gates passed on the evaluated commit")
        .expect("protected-master gate verification step exists");
    let mint = workflow
        .find("- name: Mint bounded key in short-lived process")
        .expect("mint step exists");
    assert!(
        gates < mint,
        "gate verification must run before the provisioning secret is exposed"
    );
    let section = workflow
        .split("- name: Verify protected-master gates passed on the evaluated commit")
        .nth(1)
        .and_then(|rest| rest.split("\n      - name:").next())
        .expect("gate verification step body");
    // A squash merge mints a fresh master SHA while Cerberus review check
    // runs land on the reviewed PR head, and a workflow_dispatch of the
    // review workflow against master could plant a green `review` on master
    // while reviewing an unrelated diff. The trust anchor must therefore
    // bind the evaluated master commit to the exact merged pull request:
    // merged, merge_commit_sha equality, reviewed-head tree equality, a
    // successful review check on that exact PR head, and a successful ci
    // check on the master commit itself.
    assert_contains_all(
        "protected-master gate verification",
        section,
        &[
            "[[ \"$PR_NUMBER\" =~ ^[0-9]+$ ]]",
            "test \"$merged\" = true",
            "test \"$base_repo\" = \"$GITHUB_REPOSITORY\"",
            "test \"$base_ref\" = master",
            "test \"$merge_commit_sha\" = \"$HEAD_SHA\"",
            "[[ \"$pr_head_sha\" =~ ^[0-9a-f]{40}$ ]]",
            "test \"$head_tree\" = \"$master_tree\"",
            "commits/${pr_head_sha}/check-runs?check_name=review",
            "commits/${HEAD_SHA}/check-runs?check_name=ci",
            "refusing to expose provider authority",
            "sha256sum",
        ],
    );
    assert!(
        !section.contains("commits/${HEAD_SHA}/check-runs?check_name=review")
            && !section.contains("for gate in ci review"),
        "a review check run on the master commit is forgeable via workflow_dispatch; the review gate must be checked on the merged PR head SHA"
    );
    assert!(
        workflow.contains("pull_request_number:")
            && workflow
                .split("pull_request_number:")
                .nth(1)
                .and_then(|rest| rest.split("type:").next())
                .is_some_and(|input| input.contains("required: true")),
        "the dispatch must require the merged pull request number that produced the evaluated commit"
    );
    assert!(
        workflow.contains(
            "permissions:\n  contents: read\n  checks: read\n  pull-requests: read\n\nconcurrency:"
        ),
        "the workflow may hold only read authority: repository contents, check runs, and pull requests"
    );
}

#[test]
fn trusted_live_executed_helpers_have_executable_modes() {
    for relative in [
        "scripts/generation-061-live-comparison.sh",
        "scripts/generation-061-validate-provider-attestation.sh",
        "scripts/generation-061-stage-evidence.sh",
    ] {
        let mode = fs::metadata(repo_root().join(relative))
            .unwrap_or_else(|error| panic!("read {relative} metadata: {error}"))
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o111,
            0o111,
            "workflow-executed helper {relative} must be committed executable"
        );
    }
}

#[test]
fn trusted_staging_rejects_malicious_symlink_origins() {
    let staging = repo_root().join("scripts/generation-061-stage-evidence.sh");
    assert!(
        staging.is_file(),
        "trusted staging must be an executable repo-owned boundary"
    );
    let root = test_temp_dir("symlink-stage");
    let source = root.join("target-receipt.md");
    let outside = root.join("trusted-runner-file");
    let destination = root.join("safe");
    fs::write(&outside, "must never be copied through a target symlink\n")
        .expect("write trusted runner file");
    symlink(&outside, &source).expect("create malicious target symlink");
    let result = std::process::Command::new("bash")
        .arg(&staging)
        .arg(&destination)
        .arg(&source)
        .output()
        .expect("run trusted staging helper");
    assert!(
        !result.status.success(),
        "trusted staging must reject a target-controlled symlink before any copy: {result:?}"
    );
    assert!(!destination.join("target-receipt.md").exists());
    fs::remove_dir_all(root).expect("remove symlink staging fixture");
}

#[cfg(unix)]
#[test]
fn trusted_staging_is_descriptor_based_exclusive_and_mode_preserving() {
    let staging = repo_root().join("scripts/generation-061-stage-evidence.sh");
    let root = test_temp_dir("descriptor-stage");
    let destination = root.join("safe");
    let tool = root.join("tool.sh");
    fs::write(&tool, "#!/bin/sh\nexit 0\n").expect("write executable evidence source");
    let mut permissions = fs::metadata(&tool).expect("tool metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&tool, permissions).expect("make evidence source executable");

    let first = std::process::Command::new("bash")
        .arg(&staging)
        .arg(&destination)
        .arg(&tool)
        .output()
        .expect("run trusted staging helper");
    assert!(
        first.status.success(),
        "staging a regular executable evidence file must succeed: {first:?}"
    );
    let staged_mode = fs::metadata(destination.join("tool.sh"))
        .expect("staged file metadata")
        .permissions()
        .mode();
    assert_eq!(
        staged_mode & 0o111,
        0o111,
        "descriptor staging must preserve executable modes"
    );

    // A destination name that already exists must fail via O_EXCL at the
    // descriptor — there is no check-then-copy window to race.
    let second = std::process::Command::new("bash")
        .arg(&staging)
        .arg(&destination)
        .arg(&tool)
        .output()
        .expect("re-run trusted staging helper");
    assert!(
        !second.status.success(),
        "staging over an existing destination name must fail closed: {second:?}"
    );

    // A symlink planted at the destination name must fail even though the
    // source is honest.
    let planted = root.join("planted");
    let honest = root.join("honest.md");
    fs::write(&honest, "honest evidence\n").expect("write honest evidence");
    fs::create_dir_all(&planted).expect("create planted destination");
    symlink(root.join("outside.md"), planted.join("honest.md")).expect("plant destination symlink");
    let symlinked = std::process::Command::new("bash")
        .arg(&staging)
        .arg(&planted)
        .arg(&honest)
        .output()
        .expect("run staging against planted destination symlink");
    assert!(
        !symlinked.status.success(),
        "a symlink planted at the destination name must fail closed: {symlinked:?}"
    );
    assert!(
        !root.join("outside.md").exists(),
        "the planted symlink target must never be created or written"
    );
    fs::remove_dir_all(root).expect("remove descriptor staging fixture");
}

#[test]
fn trusted_staging_and_scanning_are_descriptor_based_and_scan_the_safe_copy() {
    let staging = read_repo_file("scripts/generation-061-stage-evidence.sh");
    let scanner = read_repo_file("scripts/generation-061-scan-safe.sh");
    let helper = read_repo_file("scripts/generation-061-copy-regular.py");
    for (name, text) in [
        ("scripts/generation-061-stage-evidence.sh", &staging),
        ("scripts/generation-061-scan-safe.sh", &scanner),
    ] {
        assert_contains_all(
            name,
            text,
            &["generation-061-copy-regular.py", "--max-bytes"],
        );
        assert!(
            !text.contains("cp --no-dereference") && !text.contains("cp -- \"$path\""),
            "{name} must not fall back to racy path-based cp"
        );
    }
    assert_contains_all(
        "scripts/generation-061-copy-regular.py",
        &helper,
        &[
            "O_NOFOLLOW",
            "O_EXCL",
            "dir_fd",
            "fstat",
            "S_ISREG",
            "fchmod",
        ],
    );
    let copy_position = scanner
        .find("generation-061-copy-regular.py")
        .expect("scan-safe copies through the descriptor helper");
    let scan_position = scanner
        .find("$scanner\" -n --hidden -F")
        .expect("scan-safe scans staged evidence");
    assert!(
        copy_position < scan_position && scanner.contains("$safe_dir/$name"),
        "scan-safe must copy first and scan the immutable safe copy, not the racy source path"
    );
}

#[test]
fn trusted_live_lane_isolates_target_lifecycle_and_audits_transport() {
    let workflow = read_repo_file(".github/workflows/generation-061-live.yml");
    let comparison = read_repo_file("scripts/generation-061-live-comparison.sh");

    let target = workflow
        .split("- name: Run exact benchmark with scoped key only")
        .nth(1)
        .expect("target step exists")
        .split("- name: Revoke scoped key (always)")
        .next()
        .expect("target step ends before revoke");
    assert!(
        !target.contains("CERBERUS_OPENROUTER_PROVISIONING_KEY"),
        "the target step must not have provisioning authority"
    );
    assert!(
        comparison.contains("docker_bin\" run") && comparison.contains("--rm"),
        "the exact target/helper execution must be in a disposable container"
    );
    assert!(
        comparison.contains("--read-only")
            && comparison.contains("--network")
            && comparison.contains("--cap-drop")
            && comparison.contains("--security-opt")
            && comparison.contains("--mount")
            && comparison.contains("--env-file"),
        "the target container must declare its isolation and restricted mounts"
    );
    assert!(
        comparison.contains("--mount") && comparison.contains("dst=/workspace,readonly"),
        "the target worktree must be read-only inside the build container"
    );
    assert!(
        comparison.contains("--tmpfs /output:rw,noexec,nosuid"),
        "the only writable output surface must be a bounded noexec tmpfs"
    );
    assert!(
        workflow.contains("rust:1.88-bookworm@sha256:4727898c104ecd2e22d780925832502faee9fe4e70581b8572af081370b315a0"),
        "the build stage must use a pinned image"
    );
    assert!(
        workflow.contains("debian:bookworm-slim@sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818"),
        "the runtime stage must use a digest-pinned minimal image without Cargo or rustc"
    );
    assert!(
        workflow.contains("id: mint_key")
            && workflow.contains("- name: Revoke scoped key (always)")
            && workflow.contains("if: always()"),
        "mint and revoke must be separate lifecycle steps"
    );
    assert!(
        workflow.contains("command -v rg")
            && workflow.contains("scanner failed")
            && workflow.contains("status=$?"),
        "scanner failures must be distinct from no-match results"
    );
    assert!(
        workflow.contains("steps.revoke_key.outputs.revoked == 'true'")
            && workflow.contains("steps.audit_transport.outputs.clean == 'true'")
            && workflow.contains("steps.sanitize_evidence.outputs.safe == 'true'"),
        "upload must require revoke, transport audit, and safe scanning"
    );
    assert!(
        !workflow.contains("CERBERUS_OPENROUTER_PROVISIONING_KEY >>")
            && !workflow.contains("CERBERUS_OPENROUTER_PROVISIONING_KEY=\"$"),
        "provisioning key bytes must not be written as a workflow assignment"
    );
    assert!(
        workflow.contains("GITHUB_ENV") && workflow.contains("GITHUB_OUTPUT"),
        "command files must be explicitly audited"
    );
}

#[test]
fn trusted_live_helpers_enforce_key_lifecycle_redaction_and_validation() {
    let key_helper = read_repo_file("scripts/generation-061-cerberus-key.sh");
    let comparison = read_repo_file("scripts/generation-061-live-comparison.sh");
    let scanner = read_repo_file("scripts/generation-061-scan-safe.sh");
    let validator = read_repo_file("scripts/generation-061-validate-receipt.sh");
    assert!(
        key_helper.contains("b10bffb6ddb14ec553fbcf4f5e687aee13424717")
            && key_helper.contains("mint_review_key")
            && key_helper.contains("revoke_key")
            && key_helper.contains("2.0")
            && key_helper.contains("0o600")
            && key_helper.contains("src/bin/memory-engine-061-key-helper.rs")
            && key_helper.contains("cargo_bin\" build --quiet --locked"),
        "key lifecycle must use the pinned Cerberus API and private state"
    );
    assert!(
        comparison.contains("literalize_glob")
            && !comparison.contains("${output//$OPENROUTER_API_KEY/[redacted]}")
            && comparison.contains("GENERATION_OUTPUT_DIR"),
        "scoped-key redaction must be literal and output must be isolated"
    );
    assert!(
        validator.contains("command -v grep")
            && validator.contains("Corpus: 14 sources")
            && validator.contains("Provider failures: [1-9]")
            && validator.contains("FAILED")
            && validator.contains("Bridge fixture:.*FAIL")
            && validator.contains("error"),
        "receipt validation must reject failed/error rows"
    );
    assert!(
        scanner.contains("command -v rg")
            && scanner.contains("0)")
            && scanner.contains("1)")
            && scanner.contains("*)"),
        "safe-evidence scanning must fail closed on scanner errors"
    );
}

#[test]
fn trusted_live_secret_steps_audit_their_command_files_before_exit() {
    let workflow = read_repo_file(".github/workflows/generation-061-live.yml");
    for (step, secret_names) in [
        (
            "Mint bounded key in short-lived process",
            "CERBERUS_OPENROUTER_PROVISIONING_KEY",
        ),
        (
            "Run exact benchmark with scoped key only",
            "scoped_key GENERATION_PROVIDER_KEY",
        ),
        (
            "Revoke scoped key (always)",
            "CERBERUS_OPENROUTER_PROVISIONING_KEY",
        ),
        ("Publish a dated redacted eval receipt", ""),
    ] {
        let section = workflow
            .split(&format!("- name: {step}"))
            .nth(1)
            .and_then(|rest| rest.split("\n      - name:").next())
            .unwrap_or_else(|| panic!("workflow step missing: {step}"));
        assert!(
            section.contains("source scripts/generation-061-audit-command-files.sh")
                && section
                    .contains("command_file_dir=\"$RUNNER_TEMP/generation-061-command-files\"")
                && section.contains("capture_command_files")
                && section.contains("cp --")
                && section.contains("trap ")
                && section.contains("audit_generation_command_files")
                && section.contains(secret_names),
            "{step} must audit its own GitHub command files before exiting"
        );
    }
}

#[test]
fn trusted_live_contract_pins_repository_permissions_and_tool_discovery() {
    let workflow = read_repo_file(".github/workflows/generation-061-live.yml");
    let comparison = read_repo_file("scripts/generation-061-live-comparison.sh");
    assert!(
        workflow.contains(
            "permissions:\n  contents: read\n  checks: read\n  pull-requests: read\n\nconcurrency:"
        ),
        "the workflow must have the exact top-level read-only permissions block"
    );
    assert!(
        workflow.contains(
            "if: github.repository == 'misty-step/scry' && github.ref == 'refs/heads/master'"
        ),
        "the trusted job must be gated to the exact repository and master branch"
    );
    assert!(
        comparison.contains("cargo build --quiet --locked -p memory-engine-bench")
            && comparison.contains("--network bridge")
            && comparison.contains("GENERATION_PREPARED_BINARY")
            && comparison.contains("sha256sum \"$prepared_binary\"")
            && comparison.contains("! -d \"$shared_git_dir\"")
            && comparison.contains("! -f \"$shared_git_dir/config\""),
        "the trusted boundary must prepare and digest the exact benchmark before runtime"
    );
    assert!(
        comparison.contains("cache_root=\"$(cd -P -- \"$GENERATION_CACHE_DIR\" && pwd)\"")
            && comparison.contains("--tmpfs /output:rw,noexec,nosuid")
            && comparison.contains("prepared_dir=\"$cache_root/prepared\"")
            && comparison.contains("GENERATION_RUNTIME_CLEANUP_EVIDENCE"),
        "benchmark output must live in a bounded noexec tmpfs, never a host bind mount"
    );
    assert!(
        !comparison.contains("dst=/output"),
        "no host directory may be bind-mounted at /output"
    );
}

#[test]
fn trusted_live_audits_prior_step_command_file_snapshots() {
    let workflow = read_repo_file(".github/workflows/generation-061-live.yml");
    let publish = workflow
        .find("- name: Publish a dated redacted eval receipt")
        .expect("publish step exists");
    let final_audit = workflow
        .find("- name: Final audit command files and runner state before staging")
        .expect("final audit step exists");
    assert!(
        final_audit > publish,
        "final aggregate command-file audit must run after publish snapshots are created"
    );
    assert!(
        workflow.contains("generation-061-command-files")
            && workflow.contains("capture_command_files")
            && workflow.contains("command_files+=(\"$command_file\")")
            && workflow
                .contains("find \"$RUNNER_TEMP/generation-061-command-files\" -type f -print0")
            && workflow.contains("rm -rf -- \"${RUNNER_TEMP:-}/generation-061-command-files\""),
        "transport audit must scan prior-step command-file snapshots and clean them up"
    );
}

#[test]
fn trusted_live_target_cannot_retain_provider_key_or_use_unrestricted_egress() {
    let workflow = read_repo_file(".github/workflows/generation-061-live.yml");
    let comparison = read_repo_file("scripts/generation-061-live-comparison.sh");
    let proxy = read_repo_file("scripts/generation-061-trusted-provider-proxy.py");
    let attestation = read_repo_file("scripts/generation-061-validate-provider-attestation.sh");
    assert_contains_all(
        "trusted live target boundary",
        &workflow,
        &["GENERATION_PROVIDER_KEY=", "provider-attestation.json"],
    );
    assert_contains_all(
        "scripts/generation-061-live-comparison.sh",
        &comparison,
        &[
            "--network none",
            "--tmpfs /cargo-home:rw",
            "--tmpfs /cargo-target:rw",
            "OPENROUTER_PROXY_SOCKET=/provider.sock",
        ],
    );
    assert!(
        !workflow.contains("OPENROUTER_API_KEY=\"$scoped_key\"")
            && !workflow.contains("--allow-env OPENROUTER_API_KEY")
            && !comparison.contains("OPENROUTER_API_KEY=$OPENROUTER_API_KEY")
            && !comparison.contains("dst=/cargo-home")
            && !comparison.contains("dst=/cargo-target"),
        "target build/runtime canaries must not receive the provider key or persistent caches"
    );
    let runtime = comparison
        .split("--network none")
        .nth(1)
        .expect("network-none runtime invocation exists");
    let runtime = runtime
        .split("GENERATION_RUNTIME_IMAGE")
        .next()
        .expect("runtime invocation ends at its image");
    assert!(
        runtime.contains("dst=/workspace/crates/memory-engine-bench/corpus,readonly")
            && runtime.contains("dst=/prepared/memory-engine-bench,readonly")
            && runtime.contains("--tmpfs /output:rw,noexec,nosuid"),
        "the runtime container may mount only the eval corpus data, the exact prepared binary, trusted helpers, and a bounded noexec output tmpfs"
    );
    assert!(
        !runtime.contains("dst=/workspace,readonly") && !runtime.contains("cargo"),
        "the runtime container must not see repository source or any Cargo surface"
    );
    assert!(
        comparison.contains("prepared directory must contain exactly the digest-checked benchmark"),
        "the prepared directory must be verified to hold exactly one regular executable"
    );
    assert_contains_all(
        "scripts/generation-061-trusted-provider-proxy.py",
        &proxy,
        &[
            "provider-key-fd",
            "ThreadingUnixStreamServer",
            "UPSTREAM_URL",
            "SOCKET_READ_TIMEOUT_SECONDS",
            "settimeout",
            "readline(MAX_REQUEST_BYTES + 1)",
            "provider_calls",
            "request_sha256",
            "response_sha256",
            "os.replace",
        ],
    );
    assert_contains_all(
        "runtime cleanup contract",
        &workflow,
        &[
            "GENERATION_BUILD_TIMEOUT_SECONDS",
            "GENERATION_CONTAINER_TIMEOUT_SECONDS",
            "GENERATION_RUNTIME_CLEANUP_EVIDENCE",
            "prepared_removed",
            "attestation_validated",
            "generation-061-stage-evidence.sh",
        ],
    );
    assert_contains_all(
        "scripts/generation-061-validate-provider-attestation.sh",
        &attestation,
        &["provider-calls-observed", "-ge 15", ".calls | length"],
    );
}

const STALE_RECEIPT_DOCKER: &str = r#"#!/bin/bash
set -euo pipefail
if [[ "${1:-}" == container ]]; then
  case "${2:-}" in
    inspect) echo "Error: No such container: ${*: -1}" >&2; exit 1 ;;
    ls) exit 0 ;;
    rm) exit 0 ;;
  esac
  exit 1
fi
[[ "${1:-}" == run ]]
network=''
output_dir=''
target_mount=''
prepared_mount=''
prepared_binary=''
helper_mount=''
validator_mount=''
env_file=''
while (($#)); do
  case "$1" in
    --network) network="$2"; shift 2 ;;
    --env-file) env_file="$2"; shift 2 ;;
    --tmpfs)
      case "$2" in
        /output:*) output_dir="$(mktemp -d)" ;;
      esac
      shift 2
      ;;
    --mount)
      mount="$2"
      case "$mount" in
        *",dst=/workspace,readonly")
          target_mount="${mount#type=bind,src=}"
          target_mount="${target_mount%,dst=/workspace,readonly}"
          ;;
        *",dst=/prepared,rw")
          prepared_mount="${mount#type=bind,src=}"
          prepared_mount="${prepared_mount%,dst=/prepared,rw}"
          ;;
        *",dst=/prepared/memory-engine-bench,readonly")
          prepared_binary="${mount#type=bind,src=}"
          prepared_binary="${prepared_binary%,dst=/prepared/memory-engine-bench,readonly}"
          ;;
        *",dst=/trusted/generation-061-live-comparison.sh,readonly")
          helper_mount="${mount#type=bind,src=}"
          helper_mount="${helper_mount%,dst=/trusted/generation-061-live-comparison.sh,readonly}"
          ;;
        *",dst=/trusted/validate-receipt.sh,readonly")
          validator_mount="${mount#type=bind,src=}"
          validator_mount="${validator_mount%,dst=/trusted/validate-receipt.sh,readonly}"
          ;;
      esac
      shift 2
      ;;
    *) shift ;;
  esac
done
if [[ "$network" == bridge ]]; then
  mkdir -p "$prepared_mount"
  if [[ -f "$target_mount/forge-receipt.txt" ]]; then
    cat > "$prepared_mount/memory-engine-bench" <<EOF
#!/bin/sh
set -eu
out=''
while [[ "\$#" -gt 0 ]]; do
  if [[ "\$1" == --out ]]; then out="\$2"; shift 2; else shift; fi
done
cat "$target_mount/forge-receipt.txt" > "\$out"
EOF
  else
    cat > "$prepared_mount/memory-engine-bench" <<'EOF'
#!/bin/sh
exit 0
EOF
  fi
  chmod 0555 "$prepared_mount/memory-engine-bench"
  exit 0
fi
printf '%s' "$output_dir" > "$DOCKER_OUTPUT_MOUNT_LOG"
set -a
. "$env_file"
set +a
GENERATION_LIVE_IN_CONTAINER=true \
GENERATION_OUTPUT_DIR="$output_dir" \
GENERATION_RECEIPT_VALIDATOR="$validator_mount" \
GENERATION_PREPARED_BINARY="$prepared_binary" \
PATH="$FAKE_BIN:/usr/bin:/bin" \
bash "$helper_mount"
"#;

const STALE_RECEIPT_GIT: &str = r#"#!/bin/sh
set -eu
if [ "${1:-}" = rev-parse ]; then
  case "${2:-}" in
    --git-common-dir) printf '%s\n' "$FAKE_GIT_DIR" ;;
    --absolute-git-dir) printf '%s/worktree\n' "$FAKE_GIT_DIR" ;;
    HEAD) printf '%s\n' "$GENERATION_HEAD_SHA" ;;
    *) exit 1 ;;
  esac
  exit 0
fi
if [ "${1:-}" = config ]; then exit 1; fi
exit 1
"#;

fn prepare_stale_receipt_fixture() -> (
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
    String,
) {
    let root = test_temp_dir("stale-live-receipt");
    let scripts = root.join("scripts");
    let fake_bin = root.join("fake-bin");
    let cache = fs::canonicalize(test_temp_dir("stale-live-cache")).expect("canonicalize cache");
    let fake_git_dir = root.join("git-common");
    fs::create_dir_all(root.join("docs/evals")).expect("create target eval directory");
    fs::create_dir_all(root.join("crates/memory-engine-bench/corpus/generation"))
        .expect("create target corpus directory");
    fs::write(
        root.join("crates/memory-engine-bench/corpus/generation/fixture.md"),
        "fixture corpus source\n",
    )
    .expect("write fixture corpus source");
    fs::create_dir_all(&scripts).expect("create temporary scripts directory");
    fs::create_dir_all(&fake_bin).expect("create fake command directory");
    fs::create_dir_all(&fake_git_dir).expect("create fake shared git directory");
    fs::write(fake_git_dir.join("config"), "[core]\n").expect("write fake git config");
    fs::copy(
        repo_root().join("scripts/generation-061-live-comparison.sh"),
        scripts.join("generation-061-live-comparison.sh"),
    )
    .expect("copy trusted helper");
    fs::copy(
        repo_root().join("scripts/generation-061-validate-receipt.sh"),
        scripts.join("generation-061-validate-receipt.sh"),
    )
    .expect("copy trusted validator");
    fs::copy(
        repo_root().join("scripts/generation-061-trusted-provider-proxy.py"),
        scripts.join("generation-061-trusted-provider-proxy.py"),
    )
    .expect("copy trusted provider proxy");
    fs::copy(
        repo_root().join("scripts/generation-061-validate-provider-attestation.sh"),
        scripts.join("generation-061-validate-provider-attestation.sh"),
    )
    .expect("copy trusted provider attestation validator");
    let date = std::process::Command::new("date")
        .args(["-u", "+%F"])
        .output()
        .expect("read UTC date");
    let date = String::from_utf8(date.stdout)
        .expect("date output utf8")
        .trim()
        .to_owned();
    let target_receipt = root.join(format!(
        "docs/evals/generation-061-live-comparison-{date}.md"
    ));
    let sources = [
        "mitochondria",
        "nato-alphabet",
        "http-caching",
        "rubicon",
        "sourdough",
        "gdpr-basis",
        "hope-feathers",
        "pythagorean",
        "curie",
        "git-branching",
        "spacing-effect",
        "water-boiling",
        "apostles-creed",
        "us-presidents-ordinal",
    ];
    let mut stale = String::from(
        "# Generation eval receipt\n\n- Corpus: 14 sources\n- Provider failures: 0/14 sources\n",
    );
    for source in sources {
        writeln!(&mut stale, "| {source} | fixture | pass |").expect("write stale row");
    }
    fs::write(&target_receipt, &stale).expect("preseed target receipt");
    fs::write(
        fake_bin.join("cargo"),
        "#!/bin/sh\n# malicious benchmark succeeds without producing a receipt\nexit 0\n",
    )
    .expect("write fake cargo");
    fs::write(fake_bin.join("git"), STALE_RECEIPT_GIT).expect("write fake git");
    fs::write(fake_bin.join("docker"), STALE_RECEIPT_DOCKER).expect("write fake docker");
    for name in ["cargo", "docker", "git"] {
        let path = fake_bin.join(name);
        let mut permissions = fs::metadata(&path)
            .expect("fake command metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("make fake command executable");
    }
    (root, scripts, fake_bin, cache, target_receipt, stale)
}

fn prepare_forged_receipt_fixture() -> (
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let (root, scripts, fake_bin, cache, _target_receipt, stale) = prepare_stale_receipt_fixture();
    let mut forged = stale;
    forged.push_str("- Bridge fixture: easier 100% · faithful 100% · duplicate 0% · PASS\n");
    let script = format!(
        "#!/bin/sh\nset -eu\n# Malicious build.rs/runtime canary: only the one-run proxy capability may exist.\ntest -z \"${{GENERATION_PROVIDER_KEY:-}}\"\ntest -z \"${{OPENROUTER_API_KEY:-}}\"\nout=''\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = --out ]; then out=\"$2\"; shift 2; else shift; fi\ndone\nprintf '%s' '{forged}' > \"$out\"\nprintf '%s\\n' 'malicious target stdout marker; provider_calls=0; attempted cache retention and direct egress'\nexit 0\n",
        forged = forged.replace('\'', "'\\''")
    );
    fs::write(root.join("forge-receipt.txt"), &forged).expect("write forged build receipt marker");
    fs::write(fake_bin.join("cargo"), script).expect("write forged benchmark");
    let path = fake_bin.join("cargo");
    let mut permissions = fs::metadata(&path)
        .expect("forged benchmark metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("make forged benchmark executable");
    (root, scripts, fake_bin, cache)
}

const HONEST_BUILD_DOCKER: &str = r#"#!/bin/sh
set -eu
fixture_cache='__FIXTURE_CACHE__'
if [ "${1:-}" = container ]; then
  if [ -f "$fixture_cache/fixture-docker-broken-cleanup" ]; then
    echo 'Cannot connect to the Docker daemon at unix:///var/run/docker.sock' >&2
    exit 1
  fi
  case "${2:-}" in
    inspect) echo "Error: No such container" >&2; exit 1 ;;
    ls) exit 0 ;;
    rm) exit 0 ;;
  esac
  exit 1
fi
network=''
env_file=''
output_dir=''
helper_mount=''
validator_mount=''
prepared_mount=''
prepared_binary=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    --network) network="$2"; shift 2 ;;
    --env-file) env_file="$2"; shift 2 ;;
    --tmpfs)
      case "$2" in
        /output:*)
          output_dir="$fixture_cache/fixture-output"
          mkdir -p "$output_dir"
          ;;
      esac
      shift 2
      ;;
    --mount)
      mount="$2"
      case "$mount" in
        *",dst=/trusted/generation-061-live-comparison.sh,readonly")
          helper_mount="${mount#type=bind,src=}"
          helper_mount="${helper_mount%,dst=/trusted/generation-061-live-comparison.sh,readonly}"
          ;;
        *",dst=/trusted/validate-receipt.sh,readonly")
          validator_mount="${mount#type=bind,src=}"
          validator_mount="${validator_mount%,dst=/trusted/validate-receipt.sh,readonly}"
          ;;
        *",dst=/prepared/memory-engine-bench,readonly")
          prepared_binary="${mount#type=bind,src=}"
          prepared_binary="${prepared_binary%,dst=/prepared/memory-engine-bench,readonly}"
          ;;
        *",dst=/prepared,rw")
          prepared_mount="${mount#type=bind,src=}"
          prepared_mount="${prepared_mount%,dst=/prepared,rw}"
          ;;
      esac
      shift 2
      ;;
    *) shift ;;
  esac
done
printf '%s\n' "$network" >> "$fixture_cache/honest-build-network.log"
if [ "$network" = bridge ]; then
  test -z "${GENERATION_PROVIDER_KEY:-}"
  mkdir -p "$prepared_mount"
  cat > "$prepared_mount/memory-engine-bench" <<'EOF'
#!/bin/sh
set -eu
out=''
while [ "$#" -gt 0 ]; do
  if [ "$1" = --out ]; then out="$2"; shift 2; else shift; fi
done
cat > "$out" <<'RECEIPT'
# Generation eval receipt

- Corpus: 14 sources
- Provider failures: 0/14 sources
| mitochondria | fixture | pass |
| nato-alphabet | fixture | pass |
| http-caching | fixture | pass |
| rubicon | fixture | pass |
| sourdough | fixture | pass |
| gdpr-basis | fixture | pass |
| hope-feathers | fixture | pass |
| pythagorean | fixture | pass |
| curie | fixture | pass |
| git-branching | fixture | pass |
| spacing-effect | fixture | pass |
| water-boiling | fixture | pass |
| apostles-creed | fixture | pass |
| us-presidents-ordinal | fixture | pass |
- Bridge fixture: quality · PASS
RECEIPT
EOF
  chmod 0555 "$prepared_mount/memory-engine-bench"
  if [ -f "$fixture_cache/fixture-extra-entry" ]; then
    : > "$prepared_mount/fixture-extra"
  fi
  exit 0
fi
test "$network" = none
test -s "$env_file"
if grep -Eq 'GENERATION_PROVIDER_KEY|CARGO_HOME|CARGO_TARGET_DIR' "$env_file"; then
  exit 1
fi
test -n "$prepared_binary"
test -x "$prepared_binary"
test -n "$output_dir"
exit 0
"#;

const HONEST_BUILD_PROXY: &str = r"#!/usr/bin/env python3
import json
import signal
import socket
import sys
import time
from pathlib import Path

socket_path = Path(sys.argv[sys.argv.index('--socket') + 1])

def remove_socket_path(path):
    try:
        path.unlink()
    except FileNotFoundError:
        pass

remove_socket_path(socket_path)
server_socket = socket.socket(socket.AF_UNIX)
server_socket.bind(str(socket_path))
server_socket.listen(1)
attestation = Path(sys.argv[sys.argv.index('--attestation') + 1])
target_sha = sys.argv[sys.argv.index('--target-sha') + 1]

def finish(_signum, _frame):
    calls = [{'request_sha256': str(index), 'response_sha256': str(index), 'http_status': 200, 'successful': True} for index in range(15)]
    attestation.write_text(json.dumps({
        'schema': 'memory-engine/generation-061-provider-attestation/v1',
        'target_sha': target_sha,
        'provider_calls': 15,
        'successful_provider_calls': 15,
        'canonical_acceptance': 'provider-calls-observed',
        'calls': calls,
    }) + '\n')
    server_socket.close()
    remove_socket_path(socket_path)
    raise SystemExit(0)

signal.signal(signal.SIGTERM, finish)
while True:
    time.sleep(1)
";

#[test]
fn honest_build_proxy_remains_python_37_compatible() {
    assert!(
        HONEST_BUILD_PROXY.contains("except FileNotFoundError:"),
        "the honest-build proxy must clean up sockets without relying on Python 3.8+ missing_ok"
    );
    assert!(
        !HONEST_BUILD_PROXY.contains("missing_ok=True"),
        "the honest-build proxy must not require Path.unlink(missing_ok=True)"
    );
}

#[cfg(unix)]
fn spawn_real_trusted_proxy(
    label: &str,
    extra_args: &[&str],
) -> (
    std::process::Child,
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    use std::io::Write as _;
    // Unix socket paths are limited to ~104 bytes on macOS; the default
    // test temp dir under /var/folders is too long, so use a short root.
    let root = std::path::PathBuf::from(format!("/tmp/me098-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create short proxy fixture root");
    let socket = root.join("provider.sock");
    let attestation = root.join("provider-attestation.json");
    // The declared interpreter is whatever `python3` resolves to on the host
    // (3.7 on the oldest supported dev machine, newer on the hosted runner).
    // Running the real proxy through it is the executable portability
    // regression: any 3.8+-only API (Path.unlink(missing_ok=...),
    // socket-timeout aliasing) crashes before the socket appears.
    let mut child = std::process::Command::new("python3")
        .arg(repo_root().join("scripts/generation-061-trusted-provider-proxy.py"))
        .arg("--socket")
        .arg(&socket)
        .arg("--attestation")
        .arg(&attestation)
        .args(["--target-sha", "fixture-head", "--token", "one-run-token"])
        .args(["--provider-key-fd", "0"])
        .args(extra_args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn real trusted provider proxy");
    child
        .stdin
        .take()
        .expect("proxy stdin")
        .write_all(b"sk-fixture-provider-key\n")
        .expect("write fixture provider key");
    for _ in 0..200 {
        if socket.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(
        socket.exists(),
        "the real proxy must start under the host python3 interpreter; it crashed before binding its socket"
    );
    (child, root, socket, attestation)
}

#[cfg(unix)]
fn proxy_round_trip(socket: &std::path::Path, request: &str) -> String {
    use std::io::{BufRead as _, BufReader, Write as _};
    use std::os::unix::net::UnixStream;
    let mut stream = UnixStream::connect(socket).expect("connect trusted proxy socket");
    stream
        .write_all(request.as_bytes())
        .expect("write proxy request");
    stream.write_all(b"\n").expect("terminate proxy request");
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .expect("read proxy reply");
    line
}

#[cfg(unix)]
fn stop_real_trusted_proxy(mut child: std::process::Child) -> String {
    use std::io::Read as _;
    let _ = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status()
        .expect("signal trusted proxy");
    // Bounded shutdown is acceptance: SIGTERM must yield a clean exit within
    // ten seconds. An unbounded wait would hang the suite instead of failing
    // when signal/socket-loop cleanup regresses.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll trusted proxy") {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the trusted proxy did not shut down within ten seconds of SIGTERM");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        pipe.read_to_string(&mut stderr).expect("read proxy stderr");
    }
    assert!(
        status.success(),
        "the trusted proxy must exit cleanly on SIGTERM: {status:?}; stderr={stderr}"
    );
    stderr
}

#[cfg(unix)]
fn allowlisted_proxy_payload() -> String {
    r#"{"model":"google/gemini-3.5-flash","messages":[{"role":"user","content":"fixture prompt"}],"response_format":{"type":"json_schema","json_schema":{"name":"fixture","strict":true,"schema":{"type":"object"}}},"provider":{"require_parameters":true,"allow_fallbacks":true},"usage":{"include":true}}"#
        .to_owned()
}

#[cfg(unix)]
#[test]
fn real_trusted_proxy_runs_on_host_python3_and_fails_closed() {
    let (child, root, socket, attestation) =
        spawn_real_trusted_proxy("real-proxy-fail-closed", &[]);

    let unauthorized = proxy_round_trip(
        &socket,
        r#"{"token":"forged","payload":{"model":"google/gemini-3.5-flash","messages":[]}}"#,
    );
    assert!(
        unauthorized.contains("\"status\":401"),
        "a forged capability token must be rejected without any upstream call: {unauthorized}"
    );

    let wrong_model = proxy_round_trip(
        &socket,
        &format!(
            r#"{{"token":"one-run-token","payload":{}}}"#,
            allowlisted_proxy_payload().replace("google/gemini-3.5-flash", "attacker/exfil-model")
        ),
    );
    assert!(
        wrong_model.contains("\"status\":400"),
        "a non-allowlisted model must be rejected before any upstream call: {wrong_model}"
    );

    let malformed = proxy_round_trip(&socket, r#"{"token":"one-run-token","payload":[1,2]}"#);
    assert!(
        malformed.contains("\"status\":400"),
        "a non-object payload must be rejected: {malformed}"
    );

    stop_real_trusted_proxy(child);
    let written = fs::read_to_string(&attestation).expect("read proxy attestation");
    assert!(
        written.contains("\"provider_calls\": 0") && written.contains("\"rejected\""),
        "rejected traffic must never appear as provider proof: {written}"
    );
    assert!(
        !socket.exists(),
        "the proxy must remove its socket on shutdown under the host interpreter"
    );
    fs::remove_dir_all(root).expect("remove real proxy fixture");
}

#[cfg(unix)]
#[test]
fn real_trusted_proxy_enforces_a_global_call_budget() {
    let (child, root, socket, attestation) =
        spawn_real_trusted_proxy("real-proxy-budget", &["--max-calls", "0"]);
    let over_budget = proxy_round_trip(
        &socket,
        &format!(
            r#"{{"token":"one-run-token","payload":{}}}"#,
            allowlisted_proxy_payload()
        ),
    );
    assert!(
        over_budget.contains("\"status\":429"),
        "a schema-valid request beyond the global call budget must be refused without egress: {over_budget}"
    );
    stop_real_trusted_proxy(child);
    let written = fs::read_to_string(&attestation).expect("read proxy attestation");
    assert!(
        written.contains("\"provider_calls\": 0"),
        "budget-refused calls must not count as provider proof: {written}"
    );
    fs::remove_dir_all(root).expect("remove proxy budget fixture");
}

#[test]
fn real_trusted_proxy_source_declares_redirect_schema_and_budget_guards() {
    let proxy = read_repo_file("scripts/generation-061-trusted-provider-proxy.py");
    assert_contains_all(
        "scripts/generation-061-trusted-provider-proxy.py",
        &proxy,
        &[
            "HTTPRedirectHandler",
            "redirect_request",
            "return None",
            "validate_payload",
            "ALLOWED_ROLES",
            "max_calls",
            "max_total_bytes",
            "BoundedSemaphore",
            "rejected_provider_calls",
            "except FileNotFoundError:",
        ],
    );
    assert!(
        !proxy.contains("missing_ok"),
        "the trusted proxy must stay runnable on the oldest declared python3 (3.7)"
    );
    assert!(
        !proxy.contains("urllib.request.urlopen("),
        "upstream calls must go through the redirect-refusing opener, never default urlopen"
    );
}

#[cfg(unix)]
fn run_honest_wrapper(
    label: &str,
    before_run: impl FnOnce(&std::path::Path),
) -> (std::process::Output, std::path::PathBuf, std::path::PathBuf) {
    let (root, scripts, fake_bin, original_cache, _target_receipt, _stale) =
        prepare_stale_receipt_fixture();
    let cache = std::path::PathBuf::from(format!("/tmp/me098-{label}-{}", std::process::id()));
    fs::remove_dir_all(&original_cache).expect("remove long fixture cache");
    let _ = fs::remove_dir_all(&cache);
    fs::create_dir_all(&cache).expect("create short fixture cache");
    let proxy = root.join("honest-build-proxy.py");
    fs::write(&proxy, HONEST_BUILD_PROXY).expect("write honest build proxy");
    let attestation_validator = root.join("honest-attestation-validator.sh");
    fs::write(
        &attestation_validator,
        "#!/bin/sh\nset -eu\ntest -s \"$1\"\ntest \"$2\" = fixture-head\n",
    )
    .expect("write honest attestation validator");
    let docker = fake_bin.join("docker");
    fs::write(
        &docker,
        HONEST_BUILD_DOCKER.replace(
            "__FIXTURE_CACHE__",
            cache.to_str().expect("fixture cache path is UTF-8"),
        ),
    )
    .expect("write honest build docker");
    for path in [&proxy, &attestation_validator, &docker] {
        let mut permissions = fs::metadata(path)
            .expect("fixture helper metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("make fixture helper executable");
    }
    before_run(&cache);
    let original_path = std::env::var("PATH").expect("test PATH");
    let result = std::process::Command::new("bash")
        .arg(scripts.join("generation-061-live-comparison.sh"))
        .current_dir(&root)
        .env("PATH", format!("{}:{original_path}", fake_bin.display()))
        .env("FAKE_GIT_DIR", root.join("git-common"))
        .env("GENERATION_PROVIDER_KEY", "sk-test")
        .env("GENERATION_HEAD_SHA", "fixture-head")
        .env("GENERATION_CONTAINER_IMAGE", "fixture-image")
        .env("GENERATION_RUNTIME_IMAGE", "fixture-runtime-image")
        .env("GENERATION_CONTAINER_LABEL", "fixture-label")
        .env("GENERATION_BUILD_TIMEOUT_SECONDS", "30")
        .env("GENERATION_CONTAINER_TIMEOUT_SECONDS", "30")
        .env("GENERATION_CACHE_DIR", &cache)
        .env(
            "GENERATION_TRUSTED_HELPER",
            scripts.join("generation-061-live-comparison.sh"),
        )
        .env(
            "GENERATION_TRUSTED_VALIDATOR",
            scripts.join("generation-061-validate-receipt.sh"),
        )
        .env("GENERATION_TRUSTED_PROXY", &proxy)
        .env(
            "GENERATION_PROVIDER_ATTESTATION",
            cache.join("provider-attestation.json"),
        )
        .env(
            "GENERATION_RUNTIME_CLEANUP_EVIDENCE",
            cache.join("runtime-cleanup.json"),
        )
        .env(
            "GENERATION_TRUSTED_ATTESTATION_VALIDATOR",
            &attestation_validator,
        )
        .env("GENERATION_CONTAINER_NAME", "fixture-container")
        .output()
        .expect("run honest-path build canary");
    (result, cache, root)
}

#[cfg(unix)]
#[test]
fn trusted_live_honest_path_prebuilds_before_network_disabled_execution() {
    let (result, cache, root) = run_honest_wrapper("honest", |_| {});
    let log = cache.join("honest-build-network.log");
    assert!(
        result.status.success(),
        "an exact benchmark must build before network-disabled execution and then run: {result:?}; log={:?}; evidence={:?}",
        fs::read_to_string(&log).ok(),
        fs::read_to_string(cache.join("runtime-cleanup.json")).ok()
    );
    assert_eq!(
        fs::read_to_string(&log).expect("read honest path network log"),
        "bridge\nnone\n"
    );
    assert!(!cache.join("prepared").join("memory-engine-bench").exists());
    let evidence =
        fs::read_to_string(cache.join("runtime-cleanup.json")).expect("read cleanup evidence");
    assert!(
        evidence.contains("\"container_removed\":true"),
        "honest cleanup must prove container removal: {evidence}"
    );
    fs::remove_dir_all(&cache).expect("remove honest build cache fixture");
    fs::remove_dir_all(root).expect("remove honest build fixture");
}

#[cfg(unix)]
#[test]
fn trusted_live_wrapper_rejects_prepared_directory_with_extra_entries() {
    let (result, cache, root) = run_honest_wrapper("prepared-extra", |cache| {
        fs::write(
            cache.join("fixture-extra-entry"),
            "plant an extra artifact\n",
        )
        .expect("request an extra prepared entry");
    });
    assert!(
        !result.status.success(),
        "a build that leaves anything besides the exact benchmark binary in the prepared \
         directory must fail closed before the provider proxy exists: {result:?}"
    );
    assert!(
        !cache.join("provider-attestation.json").exists()
            || !fs::read_to_string(cache.join("provider-attestation.json"))
                .unwrap_or_default()
                .contains("provider-calls-observed"),
        "a poisoned prepared directory must never reach canonical acceptance"
    );
    fs::remove_dir_all(&cache).expect("remove prepared-extra cache fixture");
    fs::remove_dir_all(root).expect("remove prepared-extra fixture");
}

#[cfg(unix)]
#[test]
fn trusted_live_cleanup_proof_fails_closed_on_docker_errors() {
    let (result, cache, root) = run_honest_wrapper("broken-cleanup", |cache| {
        fs::write(
            cache.join("fixture-docker-broken-cleanup"),
            "docker daemon errors during cleanup\n",
        )
        .expect("break docker container commands");
    });
    assert!(
        !result.status.success(),
        "docker errors during container cleanup must fail the run, not pass as absent: {result:?}"
    );
    let evidence =
        fs::read_to_string(cache.join("runtime-cleanup.json")).expect("read cleanup evidence");
    assert!(
        evidence.contains("\"container_removed\":false"),
        "cleanup evidence must fail closed when docker cannot prove removal: {evidence}"
    );
    fs::remove_dir_all(&cache).expect("remove broken-cleanup cache fixture");
    fs::remove_dir_all(root).expect("remove broken-cleanup fixture");
}

#[test]
fn trusted_live_wrapper_rejects_preseeded_target_receipt_without_new_output() {
    let (root, scripts, fake_bin, cache, target_receipt, stale) = prepare_stale_receipt_fixture();
    let original_path = std::env::var("PATH").expect("test PATH");
    let result = std::process::Command::new("bash")
        .arg(scripts.join("generation-061-live-comparison.sh"))
        .current_dir(&root)
        .env("PATH", format!("{}:{original_path}", fake_bin.display()))
        .env("FAKE_BIN", &fake_bin)
        .env("DOCKER_OUTPUT_MOUNT_LOG", root.join("output-mount.txt"))
        .env("FAKE_GIT_DIR", root.join("git-common"))
        .env("GENERATION_PROVIDER_KEY", "sk-test")
        .env("GENERATION_HEAD_SHA", "fixture-head")
        .env("GENERATION_CONTAINER_IMAGE", "fixture-image")
        .env("GENERATION_RUNTIME_IMAGE", "fixture-runtime-image")
        .env("GENERATION_CONTAINER_LABEL", "fixture-label")
        .env("GENERATION_BUILD_TIMEOUT_SECONDS", "30")
        .env("GENERATION_CONTAINER_TIMEOUT_SECONDS", "30")
        .env("GENERATION_CACHE_DIR", &cache)
        .env(
            "GENERATION_TRUSTED_HELPER",
            scripts.join("generation-061-live-comparison.sh"),
        )
        .env(
            "GENERATION_TRUSTED_VALIDATOR",
            scripts.join("generation-061-validate-receipt.sh"),
        )
        .env(
            "GENERATION_TRUSTED_PROXY",
            scripts.join("generation-061-trusted-provider-proxy.py"),
        )
        .env(
            "GENERATION_PROVIDER_ATTESTATION",
            cache.join("provider-attestation.json"),
        )
        .env(
            "GENERATION_RUNTIME_CLEANUP_EVIDENCE",
            cache.join("runtime-cleanup.json"),
        )
        .env(
            "GENERATION_TRUSTED_ATTESTATION_VALIDATOR",
            scripts.join("generation-061-validate-provider-attestation.sh"),
        )
        .env("GENERATION_CONTAINER_NAME", "fixture-container")
        .output()
        .expect("run trusted wrapper against fixture docker");
    assert!(
        !result.status.success(),
        "success without a new receipt must fail closed: {result:?}"
    );
    assert_eq!(
        fs::read_to_string(&target_receipt).expect("read target receipt"),
        stale
    );
    let evidence =
        fs::read_to_string(cache.join("runtime-cleanup.json")).expect("read cleanup evidence");
    assert!(
        evidence.contains("\"output_host_mount\":false"),
        "the target report surface must be an in-container tmpfs, never a host mount: {evidence}"
    );
    fs::remove_dir_all(&cache).expect("remove stale receipt cache fixture");
    fs::remove_dir_all(root).expect("remove stale receipt fixture");
}

#[test]
fn trusted_live_wrapper_rejects_target_forged_receipt_and_stdout_without_provider_call() {
    let (root, scripts, fake_bin, cache) = prepare_forged_receipt_fixture();
    let original_path = std::env::var("PATH").expect("test PATH");
    let result = std::process::Command::new("bash")
        .arg(scripts.join("generation-061-live-comparison.sh"))
        .current_dir(&root)
        .env("PATH", format!("{}:{original_path}", fake_bin.display()))
        .env("FAKE_BIN", &fake_bin)
        .env("DOCKER_OUTPUT_MOUNT_LOG", root.join("output-mount.txt"))
        .env("FAKE_GIT_DIR", root.join("git-common"))
        .env("GENERATION_PROVIDER_KEY", "sk-test")
        .env("GENERATION_HEAD_SHA", "fixture-head")
        .env("GENERATION_CONTAINER_IMAGE", "fixture-image")
        .env("GENERATION_RUNTIME_IMAGE", "fixture-runtime-image")
        .env("GENERATION_CONTAINER_LABEL", "fixture-label")
        .env("GENERATION_BUILD_TIMEOUT_SECONDS", "30")
        .env("GENERATION_CONTAINER_TIMEOUT_SECONDS", "30")
        .env("GENERATION_CACHE_DIR", &cache)
        .env(
            "GENERATION_TRUSTED_HELPER",
            scripts.join("generation-061-live-comparison.sh"),
        )
        .env(
            "GENERATION_TRUSTED_VALIDATOR",
            scripts.join("generation-061-validate-receipt.sh"),
        )
        .env(
            "GENERATION_TRUSTED_PROXY",
            scripts.join("generation-061-trusted-provider-proxy.py"),
        )
        .env(
            "GENERATION_PROVIDER_ATTESTATION",
            cache.join("provider-attestation.json"),
        )
        .env(
            "GENERATION_RUNTIME_CLEANUP_EVIDENCE",
            cache.join("runtime-cleanup.json"),
        )
        .env(
            "GENERATION_TRUSTED_ATTESTATION_VALIDATOR",
            scripts.join("generation-061-validate-provider-attestation.sh"),
        )
        .env("GENERATION_CONTAINER_NAME", "fixture-container")
        .output()
        .expect("run trusted wrapper against forged target");
    assert!(
        !result.status.success(),
        "a target-controlled receipt/stdout with zero provider calls must not become canonical proof: {result:?}"
    );
    let evidence =
        fs::read_to_string(cache.join("runtime-cleanup.json")).expect("read cleanup evidence");
    assert!(
        evidence.contains("\"output_host_mount\":false"),
        "forged target output must stay inside the container tmpfs, never a host mount: {evidence}"
    );
    fs::remove_dir_all(&cache).expect("remove forged receipt cache fixture");
    fs::remove_dir_all(root).expect("remove forged receipt fixture");
}

#[test]
fn trusted_live_receipt_validator_rejects_failed_receipts() {
    let validator = repo_root().join("scripts/generation-061-validate-receipt.sh");
    let directory = test_temp_dir("failed-receipt");
    let failed = directory.join("failed.md");
    fs::write(
        &failed,
        "# Generation eval receipt\n\n- Corpus: 14 sources\n- Provider failures: 1/14\n| 001 | fixture | FAILED: provider unavailable |\n- Bridge fixture: quality · FAIL\n",
    )
    .expect("write failed receipt");
    let result = std::process::Command::new("bash")
        .arg(&validator)
        .arg(&failed)
        .output()
        .expect("run receipt validator");
    assert!(
        !result.status.success(),
        "a FAILED receipt must not be accepted as live proof"
    );
    fs::remove_dir_all(directory).expect("remove temporary receipt directory");
}

#[test]
fn trusted_live_receipt_validator_rejects_bridge_fixture_failures() {
    let validator = repo_root().join("scripts/generation-061-validate-receipt.sh");
    let directory = test_temp_dir("bridge-failed-receipt");
    let receipt = directory.join("bridge-failed.md");
    let sources = [
        "mitochondria",
        "nato-alphabet",
        "http-caching",
        "rubicon",
        "sourdough",
        "gdpr-basis",
        "hope-feathers",
        "pythagorean",
        "curie",
        "git-branching",
        "spacing-effect",
        "water-boiling",
        "apostles-creed",
        "us-presidents-ordinal",
    ];
    let mut rows = String::new();
    for source in sources {
        writeln!(&mut rows, "| {source} | fixture | pass |").expect("write receipt row");
    }
    fs::write(
        &receipt,
        format!(
            "# Generation eval receipt\n\n- Corpus: 14 sources\n{rows}- Provider failures: 0/14 sources\n- Bridge fixture: easier 100% · faithful 100% · duplicate 0% · FAIL\n"
        ),
    )
    .expect("write bridge-failed receipt");
    let result = std::process::Command::new("/bin/bash")
        .arg(&validator)
        .arg(&receipt)
        .output()
        .expect("run receipt validator");
    assert!(
        !result.status.success(),
        "a bridge FAIL receipt must not be accepted as live proof: {result:?}"
    );
    fs::remove_dir_all(directory).expect("remove bridge-failed receipt directory");
}

#[test]
fn trusted_live_receipt_validator_accepts_only_complete_receipts() {
    let validator = repo_root().join("scripts/generation-061-validate-receipt.sh");
    let directory = test_temp_dir("complete-receipt");
    let receipt = directory.join("complete.md");
    let sources = [
        "mitochondria",
        "nato-alphabet",
        "http-caching",
        "rubicon",
        "sourdough",
        "gdpr-basis",
        "hope-feathers",
        "pythagorean",
        "curie",
        "git-branching",
        "spacing-effect",
        "water-boiling",
        "apostles-creed",
        "us-presidents-ordinal",
    ];
    let mut rows = String::new();
    for source in sources {
        writeln!(&mut rows, "| {source} | fixture | pass |").expect("write receipt row");
    }
    fs::write(
        &receipt,
        format!(
            "# Generation eval receipt\n\n- Corpus: 14 sources\n{rows}- Provider failures: 0/14 sources\n- Bridge fixture: easier 100% · faithful 100% · duplicate 0% · pass\n"
        ),
    )
    .expect("write complete receipt");
    let result = std::process::Command::new("/bin/bash")
        .arg(&validator)
        .arg(&receipt)
        .output()
        .expect("run receipt validator");
    assert!(
        result.status.success(),
        "a complete receipt must be accepted: {result:?}"
    );
    fs::remove_dir_all(directory).expect("remove temporary complete receipt directory");
}

#[test]
fn trusted_live_scanner_rejects_missing_or_failing_scanner() {
    let scanner = repo_root().join("scripts/generation-061-scan-safe.sh");

    for fake_rg in ["missing", "failing"] {
        let source = test_temp_dir("evidence");
        let safe = test_temp_dir("safe");
        fs::write(source.join("evidence.txt"), "safe evidence\n").expect("write evidence");
        let bin = test_temp_dir(fake_rg);
        for utility in [
            "find",
            "rm",
            "mkdir",
            "cp",
            "basename",
            "dirname",
            "sha256sum",
            "python3",
            "test",
        ] {
            let system_path = [
                std::path::Path::new("/usr/bin").join(utility),
                std::path::Path::new("/bin").join(utility),
                std::path::Path::new("/sbin").join(utility),
                std::path::Path::new("/usr/local/bin").join(utility),
                std::path::Path::new("/opt/homebrew/bin").join(utility),
            ]
            .into_iter()
            .find(|path| path.exists())
            .expect("system utility for scanner regression");
            symlink(system_path, bin.join(utility)).expect("link scanner utility");
        }
        if fake_rg == "failing" {
            let path = bin.join("rg");
            fs::write(&path, "#!/bin/sh\nexit 2\n").expect("write failing scanner");
            let mut permissions = fs::metadata(&path).expect("scanner metadata").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&path, permissions).expect("make scanner executable");
        }
        let result = std::process::Command::new("/bin/bash")
            .arg(&scanner)
            .arg(&source)
            .arg(&safe)
            .env("PATH", bin.display().to_string())
            .output()
            .expect("run safe scanner");
        assert!(
            !result.status.success(),
            "{fake_rg} scanner must fail closed"
        );
        fs::remove_dir_all(bin).expect("remove temporary scanner directory");
        if safe.exists() {
            fs::remove_dir_all(safe).expect("remove temporary safe directory");
        }
        if source.exists() {
            fs::remove_dir_all(source).expect("remove temporary evidence directory");
        }
    }
}

#[test]
fn trusted_live_generation_helper_owns_the_exact_benchmark_and_redacts_secrets() {
    let helper = read_repo_file("scripts/generation-061-live-comparison.sh");
    assert_contains_all(
        "scripts/generation-061-live-comparison.sh",
        &helper,
        &[
            "GENERATION_PROVIDER_KEY:?",
            "GENERATION_HEAD_SHA:?",
            "git rev-parse HEAD",
            "git rev-parse --git-common-dir",
            "git config --file \"$config\" --get-regexp 'http\\..*extraheader' >/dev/null 2>&1",
            "--model google/gemini-3.5-flash",
            "--prompt principled",
            "--out \"$receipt\"",
            "literalize_glob",
            "redact_secret",
            "grep -Fq -- \"$OPENROUTER_PROXY_TOKEN\" \"$receipt\"",
            "--- GENERATION_061_RECEIPT_BEGIN ---",
        ],
    );
    assert!(
        !helper.contains("CERBERUS_OPENROUTER_PROVISIONING_KEY"),
        "the benchmark helper must not know the provisioning secret"
    );
    assert!(
        !helper.contains("cargo run \"$1\"") && !helper.contains("cargo run \"$@\""),
        "the exact benchmark must not be caller-configurable"
    );
}

#[test]
fn trusted_live_generation_documentation_names_the_security_oracle() {
    let docs = read_repo_file("docs/evals/generation-061-live-lane.md");
    assert_contains_all(
        "docs/evals/generation-061-live-lane.md",
        &docs,
        &[
            "manual workflow on `master`",
            "40-character commit SHA",
            "exact `refs/remotes/origin/master`",
            "b10bffb6ddb14ec553fbcf4f5e687aee13424717",
            "USD 2",
            "no `pull_request` trigger",
            "arbitrary fork",
            "same-repository target is still",
            "missing-secret",
            "prior-step snapshots",
            "## Trust anchor",
            "protected-master",
            "target binary remains untrusted",
            "debian:bookworm-slim@sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818",
            "exactly one regular executable",
            "noexec",
            "check run",
        ],
    );
}
