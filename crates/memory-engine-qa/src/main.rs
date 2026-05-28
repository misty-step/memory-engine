//! QA lane runner for memory-engine.
//!
//! This binary owns QA orchestration and receipt formatting. The individual
//! lanes still execute the tools that prove each surface: Bun for the remaining
//! TypeScript oracle tests, Cargo for Rust crates, and Dagger for canonical CI.

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
            id: "static.typecheck",
            title: "TypeScript strict contract",
            surface: "all package code included by tsconfig",
            purpose: "Catch type drift across public contracts and behavior tests.",
            command: &["bun", "run", "typecheck"],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
        QaLane {
            id: "static.biome",
            title: "Biome lint and format",
            surface: "repo source, tests, scripts, docs-adjacent code",
            purpose: "Keep strict style, unused imports, and unsafe patterns out of QA evidence.",
            command: &["bun", "run", "check"],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
    ]
}

fn kernel_lanes() -> Vec<QaLane> {
    vec![
        QaLane {
            id: "api.exports",
            title: "Public package exports",
            surface: "memory-engine, subpath exports, root compatibility",
            purpose: "Prove consumers can compose API surfaces without private src imports.",
            command: &[
                "bun",
                "test",
                "tests/api/module-exports.test.ts",
                "tests/api/compatibility.test.ts",
            ],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
        QaLane {
            id: "kernel.types-scheduler",
            title: "Types and scheduler behavior",
            surface: "ReviewUnitId, ScheduleState, FSRS next-state transition",
            purpose: "Protect JSON-safe schedule state and ts-fsrs round-trip semantics.",
            command: &[
                "bun",
                "test",
                "tests/types/",
                "tests/scheduler/roundtrip.test.ts",
                "tests/scheduler/serialize.test.ts",
            ],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
        QaLane {
            id: "kernel.grader",
            title: "Deterministic and rubric grading",
            surface: "Grader, AsyncGrader, rating policy, prompt exhaustiveness",
            purpose: "Protect one-envelope grade results and fixed verdict semantics.",
            command: &["bun", "test", "tests/grader/"],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
        QaLane {
            id: "kernel.progression-queue",
            title: "Progression and queue behavior",
            surface: "mastery, prerequisites, supersession, due ordering, anti-clumping",
            purpose: "Prove the actual learning flow selects eligible work in stable order.",
            command: &["bun", "test", "tests/progression/", "tests/queue/"],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
        QaLane {
            id: "contracts.testkit-adapters",
            title: "Testkit and adapter contracts",
            surface: "memory-engine/testkit, memory-engine/adapters",
            purpose: "Prove shared fixtures and adapter doubles remain usable consumer contracts.",
            command: &["bun", "test", "tests/testkit/", "tests/adapters/"],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
    ]
}

fn boundary_lanes() -> Vec<QaLane> {
    vec![
        QaLane {
            id: "service.prototype",
            title: "Service prototype behavior",
            surface: "repo-local service command boundary",
            purpose:
                "Prove injected persistence, command flow, and failure semantics stay explicit.",
            command: &["bun", "test", "tests/service/"],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
        QaLane {
            id: "evals.regression-corpus",
            title: "Learning behavior regression corpus",
            surface: "fixtures replayed through live public API surfaces",
            purpose:
                "Catch semantic drift across grading, scheduling, progression, and queue behavior.",
            command: &["bun", "test", "tests/evals/regression-corpus.test.ts"],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
        QaLane {
            id: "dogfood.rust-receipts",
            title: "Rust dogfood client receipts",
            surface: "Rust CLI review, import probe, web shell",
            purpose:
                "Exercise migrated dogfood clients through the Rust facade and service crates.",
            command: &["bun", "run", "rust:dogfood"],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
        QaLane {
            id: "dogfood.ts-oracles",
            title: "TypeScript dogfood parity oracles",
            surface: "CLI review, import probe, web shell",
            purpose:
                "Keep legacy dogfood behavior executable until the TypeScript runtime is deleted.",
            command: &[
                "bun",
                "test",
                "experiments/cli-review/cli-review.test.ts",
                "experiments/import-probe/import-probe.test.ts",
                "experiments/web-shell/web-shell.test.ts",
            ],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
    ]
}

fn handoff_lanes() -> Vec<QaLane> {
    vec![
        QaLane {
            id: "coverage.all",
            title: "Coverage-enforced full test sweep",
            surface: "all Bun tests included by the repo",
            purpose: "Preserve broad executable confidence and coverage floor evidence.",
            command: &["bun", "run", "coverage"],
            gating: true,
            modes: &[QaMode::Local, QaMode::Full],
        },
        QaLane {
            id: "performance.benchmarks",
            title: "Rust benchmark receipts",
            surface: "Rust facade, scheduler, queue, service composition",
            purpose: "Expose migrated-runtime performance drift without brittle local thresholds.",
            command: &["bun", "run", "bench"],
            gating: false,
            modes: &[QaMode::Local, QaMode::Full],
        },
        QaLane {
            id: "ci.canonical",
            title: "Canonical Dagger CI gate",
            surface: "install, typecheck, Biome, coverage, Gitleaks",
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

        assert_eq!(local.len(), 13);
        assert_eq!(full.len(), 14);
        assert_eq!(local.first().map(|lane| lane.id), Some("static.typecheck"));
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
        assert_eq!(quote_shell_arg("tests/grader/"), "tests/grader/");
        assert_eq!(quote_shell_arg("two words"), "'two words'");
        assert_eq!(quote_shell_arg("a'b"), "'a'\\''b'");
        assert_eq!(
            shell_command(&["bun", "test", "tests/api/module-exports.test.ts"]),
            "bun test tests/api/module-exports.test.ts"
        );
    }
}
