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
            "MEMORY_ENGINE_POSTGRES_URL",
            "MEMORY_ENGINE_ENABLE_FILE_STORE=true",
            "MEMORY_ENGINE_AUTH_ALLOWED_EMAILS",
            "do-connecting-ip",
            "## Deployed smoke",
            "base=\"https://memory-engine-api-i2xcr.ondigitalocean.app\"",
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
        ".agents/skills/memory-engine-qa/SKILL.md",
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
            "name: Memory Engine",
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

#[test]
fn trusted_live_lane_isolates_target_lifecycle_and_audits_transport() {
    let workflow = read_repo_file(".github/workflows/generation-061-live.yml");
    let key_helper = read_repo_file("scripts/generation-061-cerberus-key.sh");
    let comparison = read_repo_file("scripts/generation-061-live-comparison.sh");
    let scanner = read_repo_file("scripts/generation-061-scan-safe.sh");
    let validator = read_repo_file("scripts/generation-061-validate-receipt.sh");

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
        "the target worktree must be read-only inside the container"
    );
    assert!(
        comparison.contains("--mount") && comparison.contains("dst=/output,rw"),
        "only the bounded output directory may be writable"
    );
    assert!(
        workflow.contains("rust:1.88-bookworm@sha256:4727898c104ecd2e22d780925832502faee9fe4e70581b8572af081370b315a0"),
        "the target runtime must use a pinned image"
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
        workflow.contains("permissions:\n  contents: read\n\nconcurrency:"),
        "the workflow must have the exact top-level read-only permissions block"
    );
    assert!(
        workflow.contains(
            "if: github.repository == 'misty-step/memory-engine' && github.ref == 'refs/heads/master'"
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
            && comparison.contains("output_dir=\"$cache_root/live-output\"")
            && comparison.contains("rm -rf -- \"$output_dir\"")
            && comparison.contains("--mount \"type=bind,src=$output_dir,dst=/output,rw\"")
            && comparison.contains("prepared_dir=\"$cache_root/prepared\"")
            && comparison.contains("GENERATION_RUNTIME_CLEANUP_EVIDENCE"),
        "the benchmark output mount must be freshly recreated outside the target tree"
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
if [[ "${1:-}" == container ]]; then exit 1; fi
[[ "${1:-}" == run ]]
network=''
output_mount=''
target_mount=''
prepared_mount=''
helper_mount=''
validator_mount=''
env_file=''
while (($#)); do
  case "$1" in
    --network) network="$2"; shift 2 ;;
    --env-file) env_file="$2"; shift 2 ;;
    --mount)
      mount="$2"
      case "$mount" in
        *",dst=/output,rw")
          output_mount="${mount#type=bind,src=}"
          output_mount="${output_mount%,dst=/output,rw}"
          ;;
        *",dst=/workspace,readonly")
          target_mount="${mount#type=bind,src=}"
          target_mount="${target_mount%,dst=/workspace,readonly}"
          ;;
        *",dst=/prepared,rw")
          prepared_mount="${mount#type=bind,src=}"
          prepared_mount="${prepared_mount%,dst=/prepared,rw}"
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
  sha256sum "$prepared_mount/memory-engine-bench" > "$prepared_mount/memory-engine-bench.sha256"
  exit 0
fi
printf '%s' "$output_mount" > "$DOCKER_OUTPUT_MOUNT_LOG"
set -a
. "$env_file"
set +a
GENERATION_LIVE_IN_CONTAINER=true \
GENERATION_OUTPUT_DIR="$output_mount" \
GENERATION_RECEIPT_VALIDATOR="$validator_mount" \
GENERATION_PREPARED_BINARY="$prepared_mount/memory-engine-bench" \
GENERATION_PREPARED_BINARY_SHA256="$(cut -d ' ' -f1 "$prepared_mount/memory-engine-bench.sha256")" \
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
network=''
env_file=''
output_mount=''
helper_mount=''
validator_mount=''
prepared_mount=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    --network) network="$2"; shift 2 ;;
    --env-file) env_file="$2"; shift 2 ;;
    --mount)
      mount="$2"
      case "$mount" in
        *",dst=/output,rw")
          output_mount="${mount#type=bind,src=}"
          output_mount="${output_mount%,dst=/output,rw}"
          ;;
        *",dst=/trusted/generation-061-live-comparison.sh,readonly")
          helper_mount="${mount#type=bind,src=}"
          helper_mount="${helper_mount%,dst=/trusted/generation-061-live-comparison.sh,readonly}"
          ;;
        *",dst=/trusted/validate-receipt.sh,readonly")
          validator_mount="${mount#type=bind,src=}"
          validator_mount="${validator_mount%,dst=/trusted/validate-receipt.sh,readonly}"
          ;;
        *",dst=/prepared,readonly")
          prepared_mount="${mount#type=bind,src=}"
          prepared_mount="${prepared_mount%,dst=/prepared,readonly}"
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
if [ -n "$prepared_mount" ]; then
  printf '%s\n' "$network" >> "$(dirname "$prepared_mount")/honest-build-network.log"
fi
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
  sha256sum "$prepared_mount/memory-engine-bench" > "$prepared_mount/memory-engine-bench.sha256"
  exit 0
fi
test "$network" = none
test -s "$env_file"
if grep -Eq 'GENERATION_PROVIDER_KEY|CARGO_HOME|CARGO_TARGET_DIR' "$env_file"; then
  exit 1
fi
test -n "$prepared_mount"
test -x "$prepared_mount/memory-engine-bench"
test -s "$prepared_mount/memory-engine-bench.sha256"
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
socket_path.unlink(missing_ok=True)
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
    socket_path.unlink(missing_ok=True)
    raise SystemExit(0)

signal.signal(signal.SIGTERM, finish)
while True:
    time.sleep(1)
";

#[test]
fn trusted_live_honest_path_prebuilds_before_network_disabled_execution() {
    let (root, scripts, fake_bin, original_cache, _target_receipt, stale) =
        prepare_stale_receipt_fixture();
    let cache = std::path::PathBuf::from(format!("/tmp/me098-honest-{}", std::process::id()));
    fs::remove_dir_all(&original_cache).expect("remove long fixture cache");
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
    fs::write(&docker, HONEST_BUILD_DOCKER).expect("write honest build docker");
    let log = cache.join("honest-build-network.log");
    let prepared = cache.join("prepared");
    let mut permissions = fs::metadata(&proxy).expect("proxy metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&proxy, permissions).expect("make proxy executable");
    let mut permissions = fs::metadata(&attestation_validator)
        .expect("attestation validator metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&attestation_validator, permissions)
        .expect("make attestation validator executable");
    let mut permissions = fs::metadata(&docker)
        .expect("docker metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&docker, permissions).expect("make docker executable");
    let original_path = std::env::var("PATH").expect("test PATH");
    let result = std::process::Command::new("bash")
        .arg(scripts.join("generation-061-live-comparison.sh"))
        .current_dir(&root)
        .env("PATH", format!("{}:{original_path}", fake_bin.display()))
        .env("FAKE_GIT_DIR", root.join("git-common"))
        .env("GENERATION_PROVIDER_KEY", "sk-test")
        .env("GENERATION_HEAD_SHA", "fixture-head")
        .env("GENERATION_CONTAINER_IMAGE", "fixture-image")
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
    assert!(!prepared.join("memory-engine-bench").exists());
    assert!(cache.join("runtime-cleanup.json").exists());
    let _ = stale;
    fs::remove_dir_all(&cache).expect("remove honest build cache fixture");
    fs::remove_dir_all(root).expect("remove honest build fixture");
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
    assert!(
        !cache.join("live-output").exists(),
        "failed benchmark cleanup must remove only its owned output directory"
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
    assert!(
        !cache.join("live-output").exists(),
        "forged target output must be cleaned from the isolated mount"
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
        for utility in ["find", "rm", "mkdir", "cp", "basename", "sha256sum"] {
            let system_path = [
                std::path::Path::new("/usr/bin").join(utility),
                std::path::Path::new("/bin").join(utility),
                std::path::Path::new("/sbin").join(utility),
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
        ],
    );
}
