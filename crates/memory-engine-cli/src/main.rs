use std::process;

use memory_engine_cli::run_cli_review;

fn main() {
    match run_cli_review() {
        Ok(receipt) => println!(
            "{}",
            serde_json::to_string_pretty(&receipt).expect("receipt serializes")
        ),
        Err(error) => {
            eprintln!("{error}");
            process::exit(1);
        }
    }
}
