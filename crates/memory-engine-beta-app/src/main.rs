use std::{env, path::PathBuf, process};

use memory_engine_beta_app::{serve, BetaAppConfig};
use memory_engine_study::BetaStudyOptions;

fn main() {
    let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_owned());
    let port = env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(4174);
    let store_path = env::var("BETA_STUDY_STORE").map_or_else(
        |_| PathBuf::from(".tmp/beta-study/store.json"),
        PathBuf::from,
    );

    let address = format!("{host}:{port}");
    println!("Beta study listening on http://{address}");

    if let Err(error) = serve(BetaAppConfig {
        address,
        study: BetaStudyOptions::new(store_path),
    }) {
        eprintln!("{error}");
        process::exit(1);
    }
}
