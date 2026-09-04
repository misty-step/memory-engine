use std::{fs, path::Path};

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
    assert!(
        agents.contains("GitHub Issues is authoritative"),
        "AGENTS.md must name GitHub Issues as the work ledger"
    );
    assert!(
        agents.contains("Refs #") && agents.contains("Closes #"),
        "AGENTS.md must document issue attribution without premature closure"
    );
    assert!(
        !agents.contains("Powder"),
        "AGENTS.md must not retain the retired Powder workflow"
    );
    for relative in ["README.md", "VISION.md", "docs/fleet-onboarding.md"] {
        let text = read_repo_file(relative);
        assert!(
            text.contains("GitHub Issues"),
            "{relative} must point at the active GitHub Issues ledger"
        );
        assert!(
            !text.contains("Powder"),
            "{relative} must not retain the retired Powder workflow"
        );
    }
}

#[test]
fn current_product_docs_keep_the_native_host_contract() {
    for relative in [
        "AGENTS.md",
        "README.md",
        "VISION.md",
        "docs/runbook.md",
        ".agents/skills/scry-qa/SKILL.md",
    ] {
        let text = read_repo_file(relative);
        assert!(
            text.contains("native Rust `memory-engine-api`"),
            "{relative} must name the native production runtime"
        );
        let lower = text.to_ascii_lowercase();
        for retired_surface in [
            "digitalocean app platform",
            "memory-engine-api-i2xcr.ondigitalocean.app",
            "5ab05b73-9265-43c9-a01c-fef53f5f46a4",
            "encrypted app platform",
            "digitalocean app spec",
            "baked\ninto the image",
        ] {
            assert!(
                !lower.contains(retired_surface),
                "{relative} retains retired runtime surface `{retired_surface}`"
            );
        }
    }

    let runbook = read_repo_file("docs/runbook.md");
    assert_contains_all(
        "docs/runbook.md",
        &runbook,
        &[
            "systemd unit `scry.service`",
            "/opt/public-apps/scry/releases/<git-commit>",
            "/opt/public-apps/scry/current",
            "/etc/public-apps/scry.env",
            "ln -sfn \"$root/current/bin/send-magic-link\" /usr/local/bin/send-magic-link",
            "## Deployed smoke",
            "base=\"https://scry.study\"",
            "curl -fsS --max-time 15 -o /tmp/memory-engine-healthz -w \"%{http_code}\" \"$base/healthz\"",
            "curl -fsS --max-time 15 -o /tmp/memory-engine-readyz -w \"%{http_code}\" \"$base/readyz\"",
            "curl -fsS --max-time 15 -o /tmp/memory-engine-home -w \"%{http_code}\" \"$base/\"",
            "curl -fsS --max-time 15 -o /tmp/memory-engine-auth-boundary -w \"%{http_code}\" -X POST \"$base/app/generate\"",
            "case \"$status\" in 4??)",
        ],
    );
    let fetch = runbook
        .find("git fetch --quiet origin master")
        .expect("runbook fetches the protected remote tip");
    let protected_tip_gate = runbook
        .find("test \"$release\" = \"$(git rev-parse refs/remotes/origin/master)\"")
        .expect("runbook rejects an unreviewed local master commit");
    let build = runbook
        .find("cargo build --release --locked")
        .expect("runbook builds a release");
    let upload = runbook
        .find("scp \"$archive\"")
        .expect("runbook uploads a release");
    assert!(
        fetch < protected_tip_gate && protected_tip_gate < build && build < upload,
        "protected-tip verification must run before build and upload"
    );
    assert_eq!(
        runbook.matches("./bin/smoke-production").count(),
        2,
        "deploy and rollback must invoke the same complete production smoke"
    );
    assert!(
        !runbook.contains("memory-engine-097"),
        "current runbook must not assign work to a retired legacy card"
    );
    assert!(
        !repo_root().join(".github/workflows/deploy.yml").exists(),
        "the retired provider deploy workflow must not be restored"
    );
    assert!(
        !repo_root().join("fly.toml").exists(),
        "the retired provider manifest must not remain a runnable rollback path"
    );
    for (workflow, text) in github_workflows() {
        for retired_surface in ["flyctl", "FLY_API_TOKEN", "memory-engine-api.fly.dev"] {
            assert!(
                !text.contains(retired_surface),
                ".github/workflows/{workflow} restores retired provider surface `{retired_surface}`"
            );
        }
    }
    let smoke = read_repo_file("bin/smoke-production");
    assert_contains_all(
        "bin/smoke-production",
        &smoke,
        &[
            "\"$base/healthz\"",
            "\"$base/readyz\"",
            "\"$base/\"",
            "\"$base/v1/accounts/smoke-anonymous/sources/smoke-source/generation-jobs\"",
            "test \"$status\" = 401",
        ],
    );
}

#[test]
fn historical_planning_docs_do_not_assign_retired_work() {
    let generation = read_repo_file("docs/evals/generation-055-blocked-2026-06-13.md");
    assert!(
        generation.contains("historical negative evidence")
            && !generation.contains("Current status: blocked"),
        "retired generation receipt must not remain an active ticket"
    );

    let reminders = read_repo_file("docs/qa/097-scheduled-return-reminders-2026-07-15.md");
    assert!(
        reminders.contains("historical implementation evidence")
            && !reminders.contains("Card092 remains open"),
        "retired reminder receipt must not assign active legacy-card work"
    );

    let recovery = read_repo_file("docs/qa/092-return-recovery-2026-07-15.md");
    assert!(
        recovery.contains("historical local receipt")
            && recovery.contains("https://github.com/misty-step/scry/issues/98")
            && !recovery.contains("memory-engine-056"),
        "return-recovery receipt must route current proof to GitHub issue #98"
    );

    let brainstorm = read_repo_file("docs/research/ai-learning-design-brainstorm.md");
    assert!(
        brainstorm.contains("## Archived experiment sketches")
            && !brainstorm.contains("## Candidate GitHub Issues"),
        "historical research sketches must not masquerade as active GitHub issues"
    );
}

#[test]
fn shaped_issue_form_requires_exact_work_metadata_and_proof_fields() {
    let relative = ".github/ISSUE_TEMPLATE/work.yml";
    let template: serde_yaml::Value =
        serde_yaml::from_str(&read_repo_file(relative)).expect("parse shaped issue form");
    assert_eq!(template["name"].as_str(), Some("Shaped work"));
    assert_eq!(
        template["labels"].as_sequence(),
        Some(&vec![serde_yaml::Value::String(
            "status:backlog".to_owned()
        )])
    );

    let body = template["body"].as_sequence().expect("issue form body");
    for (id, field_type, label) in [
        ("outcome", "textarea", "Outcome"),
        ("why-now", "textarea", "Why now"),
        ("priority", "dropdown", "Priority"),
        ("work-type", "dropdown", "Type"),
        ("acceptance", "textarea", "Acceptance"),
        ("dependencies", "textarea", "Dependencies"),
        ("proof", "textarea", "Proof"),
        ("non-goals", "textarea", "Non-goals"),
    ] {
        let field = body
            .iter()
            .find(|field| field["id"].as_str() == Some(id))
            .unwrap_or_else(|| panic!("{relative} is missing body field `{id}`"));
        assert_eq!(field["type"].as_str(), Some(field_type), "{id} type");
        assert_eq!(
            field["attributes"]["label"].as_str(),
            Some(label),
            "{id} label"
        );
        assert_eq!(
            field["validations"]["required"].as_bool(),
            Some(true),
            "{id} must be required"
        );
    }

    for (id, expected) in [
        (
            "priority",
            ["priority:p0", "priority:p1", "priority:p2", "priority:p3"].as_slice(),
        ),
        (
            "work-type",
            [
                "type:bug",
                "type:feature",
                "type:infrastructure",
                "type:maintenance",
                "type:performance",
                "type:product-proof",
                "type:roadmap",
                "type:security",
            ]
            .as_slice(),
        ),
    ] {
        let field = body
            .iter()
            .find(|field| field["id"].as_str() == Some(id))
            .expect("metadata field");
        let options = field["attributes"]["options"]
            .as_sequence()
            .expect("metadata options")
            .iter()
            .map(|option| option.as_str().expect("string option"))
            .collect::<Vec<_>>();
        assert_eq!(options, expected, "{id} options");
    }
}

#[test]
fn hosted_ci_passes_commit_sha_to_every_latency_receipt() {
    let workflow = read_repo_file(".github/workflows/ci.yml");
    assert_contains_all(
        ".github/workflows/ci.yml",
        &workflow,
        &[
            "run: bun run ci:full",
            "dagger call action-latency-postgres --source=. --git-sha=\"$GITHUB_SHA\" export",
        ],
    );

    let workflow_yaml: serde_yaml::Value =
        serde_yaml::from_str(&workflow).expect("parse hosted CI workflow");
    assert_eq!(
        workflow_yaml["permissions"]["contents"].as_str(),
        Some("read"),
        "hosted CI must grant only explicit read access to repository contents"
    );
    let checkout = workflow_yaml["jobs"]["ci"]["steps"]
        .as_sequence()
        .expect("hosted CI steps")
        .iter()
        .find(|step| {
            step["uses"]
                .as_str()
                .is_some_and(|uses| uses.starts_with("actions/checkout@"))
        })
        .expect("checkout step");
    assert_eq!(
        checkout["with"]["persist-credentials"].as_bool(),
        Some(false),
        "PR-controlled commands must not receive the checkout credential"
    );
}

#[test]
fn action_latency_pr_code_cannot_read_checkout_git_metadata() {
    let dagger = read_repo_file(".dagger/src/index.ts");
    assert!(
        !dagger.contains("SOURCE_EXCLUDES_WITH_GIT")
            && !dagger.contains("rustContainer(source, true)")
            && !dagger.contains("includeGit"),
        "PR-controlled latency code must not receive checkout Git metadata"
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

    let workflows = github_workflows();
    assert_eq!(workflows.len(), 1, "CI is the only active GitHub workflow");
    assert_eq!(workflows[0].0, "ci.yml");

    let onboarding = read_repo_file("docs/fleet-onboarding.md");
    assert_contains_all(
        "docs/fleet-onboarding.md",
        &onboarding,
        &[
            "memory-engine.map.json",
            "CANARY_ENDPOINT",
            "memory-engine-api",
            "GitHub Issues",
            "Landmark",
        ],
    );
}

#[test]
fn architecture_map_has_one_exact_github_issues_ledger_node() {
    let map: serde_json::Value =
        serde_json::from_str(&read_repo_file("docs/architecture/memory-engine.map.json"))
            .expect("parse architecture map");
    let nodes = map["nodes"].as_array().expect("architecture map nodes");
    for id in [
        "node.fleet.landmark",
        "node.fleet.canary",
        "node.fleet.github-issues",
    ] {
        assert!(
            nodes.iter().any(|node| node["id"].as_str() == Some(id)),
            "architecture map is missing `{id}`"
        );
    }
    assert!(
        nodes.iter().all(|node| {
            node["id"].as_str() != Some("node.fleet.powder")
                && node["kind"].as_str() != Some("powder")
        }),
        "the architecture map must not retain the retired Powder work ledger"
    );

    let github_issues = nodes
        .iter()
        .find(|node| node["id"].as_str() == Some("node.fleet.github-issues"))
        .expect("GitHub Issues node");
    assert_eq!(github_issues["kind"].as_str(), Some("issue"));
    assert!(github_issues["viewTags"]
        .as_array()
        .expect("GitHub Issues view tags")
        .iter()
        .any(|tag| tag.as_str() == Some("fleet-integration")));
    assert!(
        github_issues["refs"]
            .as_array()
            .expect("GitHub Issues refs")
            .iter()
            .any(|reference| {
                reference["kind"].as_str() == Some("issue")
                    && reference["path"].as_str()
                        == Some("https://github.com/misty-step/scry/issues")
                    && reference["label"].as_str() == Some("active issue queue")
            }),
        "GitHub Issues node must link the active issue collection"
    );
}
