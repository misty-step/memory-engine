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
        "AGENTS.md must point cold agents at the deployed Fly surface runbook"
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
fn runbook_contains_reproducible_deployed_smoke_commands() {
    let runbook = read_repo_file("docs/runbook.md");

    assert_contains_all(
        "docs/runbook.md",
        &runbook,
        &[
            "App: `memory-engine-api`",
            "primary region `ord`",
            "MEMORY_ENGINE_POSTGRES_URL",
            "MEMORY_ENGINE_ENABLE_FILE_STORE=true",
            "MEMORY_ENGINE_AUTH_ALLOWED_EMAILS",
            "## Deployed smoke",
            "base=\"https://memory-engine-api.fly.dev\"",
            "curl -fsS --max-time 15 -o /tmp/memory-engine-healthz -w \"%{http_code}\" \"$base/healthz\"",
            "curl -fsS --max-time 15 -o /tmp/memory-engine-home -w \"%{http_code}\" \"$base/\"",
            "curl -fsS --max-time 15 -o /tmp/memory-engine-auth-boundary -w \"%{http_code}\" -X POST \"$base/app/generate\"",
            "case \"$status\" in 4??)",
        ],
    );
}
