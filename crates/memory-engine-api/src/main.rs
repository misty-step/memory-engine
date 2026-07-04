#![cfg_attr(not(test), deny(clippy::expect_used, clippy::unwrap_used))]

use std::{env, net::SocketAddr, process};

use memory_engine_api::{
    init_error_reporting, router, start_health_reporting_loop, AccountRegistry, ApiState,
    AuthConfig,
};

#[tokio::main]
async fn main() {
    init_error_reporting();
    start_health_reporting_loop();
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

    let auth_config = match auth_config_from_env() {
        Ok(auth_config) => auth_config,
        Err(error) => {
            eprintln!("{error}");
            process::exit(1);
        }
    };
    let state = if let Ok(database_url) = env::var("MEMORY_ENGINE_POSTGRES_URL") {
        ApiState::new(
            AccountRegistry::with_postgres_url(database_url).with_auth_config(auth_config),
        )
    } else if env::var("MEMORY_ENGINE_ENABLE_FILE_STORE").as_deref() == Ok("true") {
        let Ok(store_dir) = env::var("MEMORY_ENGINE_API_STORE_DIR") else {
            eprintln!(
                "MEMORY_ENGINE_API_STORE_DIR is required when MEMORY_ENGINE_ENABLE_FILE_STORE=true"
            );
            process::exit(1);
        };
        ApiState::new(AccountRegistry::with_store_root(store_dir).with_auth_config(auth_config))
    } else {
        eprintln!("MEMORY_ENGINE_POSTGRES_URL is required for memory-engine-api");
        process::exit(1);
    };

    // Start the background generation worker before serving so captures run
    // asynchronously instead of blocking the request thread.
    state.start_worker();

    if let Err(error) = axum::serve(listener, router(state)).await {
        eprintln!("{error}");
        process::exit(1);
    }
}

fn auth_config_from_env() -> Result<AuthConfig, String> {
    let allowed = env::var("MEMORY_ENGINE_AUTH_ALLOWED_EMAILS")
        .map_err(|_| "MEMORY_ENGINE_AUTH_ALLOWED_EMAILS is required for memory-engine-api")?;
    let allowed_emails = allowed
        .split(',')
        .map(str::trim)
        .filter(|email| !email.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if allowed_emails.is_empty() {
        return Err(
            "MEMORY_ENGINE_AUTH_ALLOWED_EMAILS must contain at least one allowed email".to_owned(),
        );
    }

    let mut auth_config = AuthConfig::allow_emails(allowed_emails);
    // Local/dev only: surface the magic link on the "check your email" page so
    // sign-in works without a real mailer. Never enable in production.
    if env::var("MEMORY_ENGINE_AUTH_EXPOSE_DEBUG_LINKS").as_deref() == Ok("true") {
        auth_config = auth_config.with_debug_links(true);
    }
    if let Ok(command) = env::var("MEMORY_ENGINE_AUTH_MAILER_COMMAND") {
        let command = command.trim();
        if !command.is_empty() {
            return Ok(auth_config.with_mailer_command(command.to_owned()));
        }
    }
    if let Ok(outbox_path) = env::var("MEMORY_ENGINE_AUTH_LINK_OUTBOX_PATH") {
        let outbox_path = outbox_path.trim();
        if !outbox_path.is_empty() {
            return Ok(auth_config.with_link_outbox(outbox_path));
        }
    }

    Err(
        "MEMORY_ENGINE_AUTH_MAILER_COMMAND or MEMORY_ENGINE_AUTH_LINK_OUTBOX_PATH is required for memory-engine-api"
            .to_owned(),
    )
}
