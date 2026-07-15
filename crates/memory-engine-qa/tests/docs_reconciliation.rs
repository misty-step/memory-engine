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
            "refs/heads/*:refs/remotes/origin/*",
            "git for-each-ref --format='%(objectname)' refs/remotes/origin/",
            "--local-runtime-command \"$GITHUB_WORKSPACE/scripts/generation-061-live-comparison.sh\"",
            "--allow-env OPENROUTER_API_KEY",
            "CERBERUS_OPENROUTER_PROVISIONING_KEY is required; refusing to run live generation",
            "receipt=\"docs/evals/generation-061-live-comparison-${date_utc}.md\"",
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
        comparison.contains("cargo_bin=\"$(command -v cargo || true)\"")
            && comparison.contains("/usr/local/cargo/bin/cargo")
            && comparison.contains("if [[ -z \"$cargo_bin\" ]]")
            && comparison.contains("! -d \"$shared_git_dir\"")
            && comparison.contains("! -f \"$shared_git_dir/config\""),
        "the isolated benchmark must use explicit shared-Git and cargo invariants"
    );
    assert!(
        comparison.contains("cache_root=\"$(cd -P -- \"$GENERATION_CACHE_DIR\" && pwd)\"")
            && comparison.contains("output_dir=\"$cache_root/live-output\"")
            && comparison.contains("rm -rf -- \"$output_dir\"")
            && comparison.contains("--mount \"type=bind,src=$output_dir,dst=/output,rw\""),
        "the benchmark output mount must be freshly recreated outside the target tree"
    );
}

const STALE_RECEIPT_DOCKER: &str = r#"#!/bin/bash
set -euo pipefail
if [[ "${1:-}" == container ]]; then exit 1; fi
[[ "${1:-}" == run ]]
output_mount=''
helper_mount=''
validator_mount=''
env_file=''
while (($#)); do
  case "$1" in
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
      esac
      shift 2
      ;;
    *) shift ;;
  esac
done
printf '%s' "$output_mount" > "$DOCKER_OUTPUT_MOUNT_LOG"
set -a
. "$env_file"
set +a
GENERATION_LIVE_IN_CONTAINER=true \
GENERATION_OUTPUT_DIR="$output_mount" \
GENERATION_RECEIPT_VALIDATOR="$validator_mount" \
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

#[test]
fn trusted_live_wrapper_rejects_preseeded_target_receipt_without_new_output() {
    let (root, scripts, fake_bin, cache, target_receipt, stale) = prepare_stale_receipt_fixture();
    let mount_log = root.join("output-mount.txt");
    let original_path = std::env::var("PATH").expect("test PATH");
    let result = std::process::Command::new("bash")
        .arg(scripts.join("generation-061-live-comparison.sh"))
        .current_dir(&root)
        .env("PATH", format!("{}:{original_path}", fake_bin.display()))
        .env("FAKE_BIN", &fake_bin)
        .env("DOCKER_OUTPUT_MOUNT_LOG", &mount_log)
        .env("FAKE_GIT_DIR", root.join("git-common"))
        .env("OPENROUTER_API_KEY", "sk-test")
        .env("GENERATION_HEAD_SHA", "fixture-head")
        .env("GENERATION_CONTAINER_IMAGE", "fixture-image")
        .env("GENERATION_CONTAINER_LABEL", "fixture-label")
        .env("GENERATION_CACHE_DIR", &cache)
        .env(
            "GENERATION_TRUSTED_HELPER",
            scripts.join("generation-061-live-comparison.sh"),
        )
        .env(
            "GENERATION_TRUSTED_VALIDATOR",
            scripts.join("generation-061-validate-receipt.sh"),
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
    let output_mount = fs::read_to_string(&mount_log).expect("read output mount log");
    assert!(output_mount.starts_with(cache.to_str().expect("cache path")));
    assert!(!output_mount.starts_with(root.join("docs/evals").to_str().expect("target path")));
    assert!(!output_mount.contains("generation-061-live-comparison"));
    assert!(
        !cache.join("live-output").exists(),
        "failed benchmark cleanup must remove only its owned output directory"
    );
    fs::remove_dir_all(&cache).expect("remove stale receipt cache fixture");
    fs::remove_dir_all(root).expect("remove stale receipt fixture");
}

#[test]
fn trusted_live_receipt_validator_rejects_failed_receipts() {
    let validator = repo_root().join("scripts/generation-061-validate-receipt.sh");
    let directory = test_temp_dir("failed-receipt");
    let failed = directory.join("failed.md");
    fs::write(
        &failed,
        "# Generation eval receipt\n\n- Corpus: 14 sources\n- Provider failures: 1/14\n| 001 | fixture | FAILED: provider unavailable |\n",
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
            "# Generation eval receipt\n\n- Corpus: 14 sources\n{rows}- Provider failures: 0/14 sources\n"
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
            "OPENROUTER_API_KEY:?",
            "GENERATION_HEAD_SHA:?",
            "git rev-parse HEAD",
            "git rev-parse --git-common-dir",
            "git config --file \"$config\" --get-regexp 'http\\..*extraheader' >/dev/null 2>&1",
            "--model google/gemini-3.5-flash",
            "--prompt principled",
            "--out \"$receipt\"",
            "literalize_glob",
            "redact_secret",
            "grep -Fq -- \"$OPENROUTER_API_KEY\" \"$receipt\"",
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
            "exact head of a branch",
            "b10bffb6ddb14ec553fbcf4f5e687aee13424717",
            "USD 2",
            "no `pull_request` trigger",
            "arbitrary fork",
            "same-repository target is still treated as untrusted",
            "missing-secret",
        ],
    );
}
