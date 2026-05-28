//! Coverage floor gate for memory-engine.
//!
//! The gate intentionally keeps Bun as the TypeScript oracle runner while Rust
//! owns orchestration, summary parsing, and floor enforcement.

use std::{
    fmt,
    process::{Command, Stdio},
};

const COVERAGE_FLOOR: f64 = 80.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct CoverageSummary {
    function_percent: f64,
    line_percent: f64,
}

#[derive(Debug, Eq, PartialEq)]
enum CoverageError {
    SummaryMissing,
    NonNumericSummary,
}

impl fmt::Display for CoverageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SummaryMissing => {
                formatter.write_str("Coverage summary not found in bun test --coverage output.")
            }
            Self::NonNumericSummary => {
                formatter.write_str("Coverage summary contained non-numeric values.")
            }
        }
    }
}

fn main() {
    let output = match Command::new("bun")
        .args(["test", "--coverage"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            eprintln!("failed to run bun test --coverage: {error}");
            std::process::exit(127);
        }
    };

    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));

    if !output.status.success() {
        std::process::exit(output.status.code().unwrap_or(1));
    }

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let summary = match parse_summary(&combined) {
        Ok(summary) => summary,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };

    if summary.function_percent < COVERAGE_FLOOR || summary.line_percent < COVERAGE_FLOOR {
        eprintln!(
            "Coverage floor {COVERAGE_FLOOR:.0}% not met (funcs={:.2}%, lines={:.2}%).",
            summary.function_percent, summary.line_percent
        );
        std::process::exit(1);
    }

    println!(
        "Coverage floor {COVERAGE_FLOOR:.0}% met (funcs={:.2}%, lines={:.2}%).",
        summary.function_percent, summary.line_percent
    );
}

fn parse_summary(output: &str) -> Result<CoverageSummary, CoverageError> {
    let Some(line) = output
        .lines()
        .find(|line| line.trim_start().starts_with("All files"))
    else {
        return Err(CoverageError::SummaryMissing);
    };

    let mut columns = line.split('|').map(str::trim);
    let _label = columns.next();
    let Some(function_text) = columns.next() else {
        return Err(CoverageError::SummaryMissing);
    };
    let Some(line_text) = columns.next() else {
        return Err(CoverageError::SummaryMissing);
    };

    let function_percent = parse_percent(function_text)?;
    let line_percent = parse_percent(line_text)?;

    Ok(CoverageSummary {
        function_percent,
        line_percent,
    })
}

fn parse_percent(value: &str) -> Result<f64, CoverageError> {
    let percent = value
        .parse::<f64>()
        .map_err(|_| CoverageError::NonNumericSummary)?;

    if percent.is_finite() {
        Ok(percent)
    } else {
        Err(CoverageError::NonNumericSummary)
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_summary, CoverageError, CoverageSummary};

    #[test]
    fn parses_bun_coverage_summary() {
        let output = r"
-----------------------------------------|---------|---------|
File                                     | % Funcs | % Lines |
All files                                |   93.13 |   93.27 |
";

        assert_eq!(
            parse_summary(output),
            Ok(CoverageSummary {
                function_percent: 93.13,
                line_percent: 93.27,
            })
        );
    }

    #[test]
    fn rejects_missing_summary() {
        assert_eq!(
            parse_summary("tests passed without coverage table"),
            Err(CoverageError::SummaryMissing)
        );
    }

    #[test]
    fn rejects_non_numeric_summary_values() {
        assert_eq!(
            parse_summary("All files | funcs | 93.27 |"),
            Err(CoverageError::NonNumericSummary)
        );
    }
}
