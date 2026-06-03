//! QA lane runner for memory-engine.
//!
//! This binary owns QA orchestration and receipt formatting. The individual
//! lanes execute the Rust tools that prove each surface, with Dagger retained as
//! the canonical CI handoff.

use std::{
    env, fmt,
    process::{Command, Stdio},
    time::Instant,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QaMode {
    Local,
    Full,
}

impl fmt::Display for QaMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local => formatter.write_str("local"),
            Self::Full => formatter.write_str("full"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct QaLane {
    id: &'static str,
    title: &'static str,
    surface: &'static str,
    purpose: &'static str,
    command: &'static [&'static str],
    gating: bool,
    modes: &'static [QaMode],
}

#[derive(Clone, Debug, PartialEq)]
struct LaneReceipt<'a> {
    lane: &'a QaLane,
    exit_code: i32,
    elapsed_ms: u128,
}

fn main() {
    let mode = parse_mode(env::args().skip(1));
    let selected = selected_lanes(mode);
    let started_at = Instant::now();
    let mut receipts = Vec::new();
    let mut failed = false;

    print_header(mode, &selected);

    for lane in &selected {
        let receipt = run_lane(lane);
        print_receipt(&receipt);

        if receipt.exit_code != 0 && receipt.lane.gating {
            failed = true;
            receipts.push(receipt);
            break;
        }

        receipts.push(receipt);
    }

    print_summary(mode, &receipts, started_at.elapsed().as_millis());

    if failed {
        std::process::exit(1);
    }
}

fn lanes() -> Vec<QaLane> {
    let mut lanes = Vec::new();
    lanes.extend(static_lanes());
    lanes.extend(kernel_lanes());
    lanes.extend(boundary_lanes());
    lanes.extend(handoff_lanes());
    lanes
}

fn static_lanes() -> Vec<QaLane> {
    vec![
        QaLane {
            id: "static.rustfmt",
            title: "Rust formatting",
            surface: "all Rust crates",
            purpose: "Keep generated and hand-written Rust in the canonical format.",
            command: &["cargo", "fmt", "--all", "--check"],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
        QaLane {
            id: "static.clippy",
            title: "Rust lint",
            surface: "all Rust targets",
            purpose: "Catch correctness, maintainability, and API-shape warnings across crates.",
            command: &[
                "cargo",
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
    ]
}

fn kernel_lanes() -> Vec<QaLane> {
    vec![
        QaLane {
            id: "api.facade",
            title: "Rust facade exports",
            surface: "memory-engine facade crate",
            purpose: "Prove consumers can compose root, modular, testkit, and dogfood surfaces.",
            command: &["cargo", "test", "-p", "memory-engine"],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
        QaLane {
            id: "kernel.core",
            title: "Rust kernel behavior",
            surface: "memory-engine-core types, scheduler, grading, progression, queue",
            purpose: "Protect pure learning semantics and provider-neutral adapter contracts.",
            command: &["cargo", "test", "-p", "memory-engine-core"],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
    ]
}

fn boundary_lanes() -> Vec<QaLane> {
    vec![
        QaLane {
            id: "service.prototype",
            title: "Rust service boundary behavior",
            surface: "memory-engine-service command boundary",
            purpose:
                "Prove injected persistence, command flow, and failure semantics stay explicit.",
            command: &["cargo", "test", "-p", "memory-engine-service"],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
        QaLane {
            id: "persistence.beta-store",
            title: "Rust beta persistence behavior",
            surface: "memory-engine-persistence durable beta store",
            purpose: "Prove persisted snapshots, restart, conflict, and validation semantics.",
            command: &["cargo", "test", "-p", "memory-engine-persistence"],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
        QaLane {
            id: "generation.beta",
            title: "Rust beta generation behavior",
            surface: "memory-engine-generation deterministic generation probe",
            purpose: "Prove source parsing, provenance, draft validation, and promotion behavior.",
            command: &["cargo", "test", "-p", "memory-engine-generation"],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
        QaLane {
            id: "study.beta-session",
            title: "Rust beta study session",
            surface: "memory-engine-study session/API boundary",
            purpose: "Prove source, generation, approval, reveal, answer, queue, and resume flow.",
            command: &["cargo", "test", "-p", "memory-engine-study"],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
        QaLane {
            id: "app.beta-http",
            title: "Rust beta HTTP host",
            surface: "memory-engine-beta-app local HTTP routes",
            purpose: "Prove mobile app routes and validation run through the Rust study session.",
            command: &["cargo", "test", "-p", "memory-engine-beta-app"],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
        QaLane {
            id: "dogfood.rust-receipts",
            title: "Rust dogfood client receipts",
            surface: "Rust CLI review, import probe, web shell",
            purpose:
                "Exercise migrated dogfood clients through the Rust facade and service crates.",
            command: &[
                "bash",
                "-lc",
                "cargo run -p memory-engine-cli && cargo run -p memory-engine-import && cargo run -p memory-engine-web-shell -- --receipt",
            ],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
    ]
}

fn handoff_lanes() -> Vec<QaLane> {
    vec![
        QaLane {
            id: "docs.rustdoc",
            title: "Rust documentation build",
            surface: "all public Rust crates",
            purpose: "Prove public API documentation compiles after the cutover.",
            command: &["cargo", "doc", "--workspace", "--no-deps"],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
        QaLane {
            id: "performance.benchmarks",
            title: "Rust benchmark receipts",
            surface: "Rust facade, scheduler, queue, service composition",
            purpose: "Expose migrated-runtime performance drift without brittle local thresholds.",
            command: &["cargo", "run", "-p", "memory-engine-bench"],
            gating: false,
            modes: &[QaMode::Local, QaMode::Full],
        },
        QaLane {
            id: "ci.canonical",
            title: "Canonical Dagger CI gate",
            surface: "Rust fmt, test, clippy, doc, and Gitleaks",
            purpose: "Prove handoff quality with the repository gate, not adjacent evidence.",
            command: &["bun", "run", "ci"],
            gating: true,
            modes: &[QaMode::Full],
        },
    ]
}

fn parse_mode(args: impl IntoIterator<Item = String>) -> QaMode {
    let args = args.into_iter().collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--local") {
        QaMode::Local
    } else {
        QaMode::Full
    }
}

fn selected_lanes(mode: QaMode) -> Vec<QaLane> {
    lanes()
        .into_iter()
        .filter(|lane| lane.modes.contains(&mode))
        .collect()
}

fn print_header(mode: QaMode, selected: &[QaLane]) {
    println!("# memory-engine QA ({mode})");
    println!();
    println!("lanes: {}", selected.len());
    println!();
}

fn run_lane(lane: &QaLane) -> LaneReceipt<'_> {
    println!("## {}: {}", lane.id, lane.title);
    println!("surface: {}", lane.surface);
    println!("purpose: {}", lane.purpose);
    println!("command: {}", shell_command(lane.command));
    println!();

    let start = Instant::now();
    let exit_code = spawn_lane(lane.command);

    LaneReceipt {
        lane,
        exit_code,
        elapsed_ms: start.elapsed().as_millis(),
    }
}

fn spawn_lane(command: &[&str]) -> i32 {
    let Some((program, args)) = command.split_first() else {
        return 127;
    };

    match Command::new(program)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
    {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            eprintln!("failed to run {}: {error}", shell_command(command));
            127
        }
    }
}

fn print_receipt(receipt: &LaneReceipt<'_>) {
    let status = if receipt.exit_code == 0 {
        "PASS"
    } else if receipt.lane.gating {
        "FAIL"
    } else {
        "WARN"
    };

    println!();
    println!(
        "receipt: {status} {} exit={} elapsed_ms={}",
        receipt.lane.id, receipt.exit_code, receipt.elapsed_ms
    );
    println!();
}

fn print_summary(mode: QaMode, receipts: &[LaneReceipt<'_>], elapsed_ms: u128) {
    let failed = receipts
        .iter()
        .filter(|receipt| receipt.exit_code != 0 && receipt.lane.gating)
        .collect::<Vec<_>>();
    let warned = receipts
        .iter()
        .filter(|receipt| receipt.exit_code != 0 && !receipt.lane.gating)
        .count();

    println!("# QA summary");
    println!("mode: {mode}");
    println!("elapsed_ms: {elapsed_ms}");
    println!(
        "passed_lanes: {}",
        receipts
            .iter()
            .filter(|receipt| receipt.exit_code == 0)
            .count()
    );
    println!("warning_lanes: {warned}");
    println!("failed_lanes: {}", failed.len());

    if !failed.is_empty() {
        println!();
        println!("failed:");
        for receipt in failed {
            println!(
                "- {}: {}",
                receipt.lane.id,
                shell_command(receipt.lane.command)
            );
        }
    }
}

fn shell_command(command: &[&str]) -> String {
    command
        .iter()
        .map(|value| quote_shell_arg(value))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_shell_arg(value: &str) -> String {
    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "_./:@=-".contains(character))
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_mode, quote_shell_arg, selected_lanes, shell_command, QaMode};

    #[test]
    fn mode_selection_defaults_to_full_and_accepts_local() {
        assert_eq!(parse_mode(Vec::<String>::new()), QaMode::Full);
        assert_eq!(
            parse_mode(["--local".to_owned()].into_iter()),
            QaMode::Local
        );
        assert_eq!(parse_mode(["--full".to_owned()].into_iter()), QaMode::Full);
    }

    #[test]
    fn lane_selection_keeps_full_as_local_plus_canonical_ci() {
        let local = selected_lanes(QaMode::Local);
        let full = selected_lanes(QaMode::Full);

        assert_eq!(local.len(), 12);
        assert_eq!(full.len(), 13);
        assert_eq!(local.first().map(|lane| lane.id), Some("static.rustfmt"));
        assert_eq!(full.last().map(|lane| lane.id), Some("ci.canonical"));
        assert!(!local.iter().any(|lane| lane.id == "ci.canonical"));
        assert!(local
            .iter()
            .all(|lane| full.iter().any(|full_lane| full_lane.id == lane.id)));
    }

    #[test]
    fn lane_metadata_preserves_gating_contract() {
        let local = selected_lanes(QaMode::Local);
        let performance = local
            .iter()
            .find(|lane| lane.id == "performance.benchmarks")
            .expect("benchmark lane");

        assert!(!performance.gating);
        assert!(local
            .iter()
            .filter(|lane| lane.id != "performance.benchmarks")
            .all(|lane| lane.gating));
    }

    #[test]
    fn shell_quoting_matches_the_old_receipt_shape() {
        assert_eq!(
            quote_shell_arg("crates/memory-engine-core"),
            "crates/memory-engine-core"
        );
        assert_eq!(quote_shell_arg("two words"), "'two words'");
        assert_eq!(quote_shell_arg("a'b"), "'a'\\''b'");
        assert_eq!(
            shell_command(&["cargo", "test", "-p", "memory-engine-core"]),
            "cargo test -p memory-engine-core"
        );
    }
}
