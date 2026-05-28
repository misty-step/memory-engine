use std::process;

use memory_engine_web_shell::{serve, WebShellConfig};

fn main() {
    let host = std::env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_owned());
    let port = std::env::var("PORT").unwrap_or_else(|_| "4173".to_owned());
    let config = WebShellConfig {
        address: format!("{host}:{port}"),
    };

    if let Err(error) = serve(&config) {
        eprintln!("{error}");
        process::exit(1);
    }
}
