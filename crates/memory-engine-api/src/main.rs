use std::{env, net::SocketAddr, process};

use memory_engine_api::{router, AccountRegistry, ApiState};

#[tokio::main]
async fn main() {
    let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_owned());
    let port = env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8080);
    let address = format!("{host}:{port}");
    let listener = match tokio::net::TcpListener::bind(&address).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("failed to bind {address}: {error}");
            process::exit(1);
        }
    };
    let local_addr = listener.local_addr().unwrap_or_else(|_| {
        address
            .parse::<SocketAddr>()
            .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], port)))
    });

    println!("Memory Engine API listening on http://{local_addr}");

    let state = if let Ok(database_url) = env::var("MEMORY_ENGINE_POSTGRES_URL") {
        ApiState::new(AccountRegistry::with_postgres_url(database_url))
    } else if env::var("MEMORY_ENGINE_ENABLE_FILE_STORE").as_deref() == Ok("true") {
        let Ok(store_dir) = env::var("MEMORY_ENGINE_API_STORE_DIR") else {
            eprintln!(
                "MEMORY_ENGINE_API_STORE_DIR is required when MEMORY_ENGINE_ENABLE_FILE_STORE=true"
            );
            process::exit(1);
        };
        ApiState::new(AccountRegistry::with_store_root(store_dir))
    } else {
        eprintln!("MEMORY_ENGINE_POSTGRES_URL is required for memory-engine-api");
        process::exit(1);
    };

    if let Err(error) = axum::serve(listener, router(state)).await {
        eprintln!("{error}");
        process::exit(1);
    }
}
