use std::process;

use memory_engine_import::run_import_probe;

fn main() {
    match run_import_probe() {
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
