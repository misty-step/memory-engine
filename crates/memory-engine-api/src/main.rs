#![cfg_attr(not(test), deny(clippy::expect_used, clippy::unwrap_used))]

use std::{env, net::SocketAddr, process, time::Duration};

use memory_engine_api::{
    init_error_reporting, router, shutdown_error_reporting, start_health_reporting_loop,
    AccountRegistry, ApiState, AuthConfig, OpenRouterConfig,
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
            AccountRegistry::with_postgres_url(database_url)
                .with_auth_config(auth_config)
                .with_generation_provider_config(OpenRouterConfig::from_env().ok()),
        )
    } else if env::var("MEMORY_ENGINE_ENABLE_FILE_STORE").as_deref() == Ok("true") {
        let Ok(store_dir) = env::var("MEMORY_ENGINE_API_STORE_DIR") else {
            eprintln!(
                "MEMORY_ENGINE_API_STORE_DIR is required when MEMORY_ENGINE_ENABLE_FILE_STORE=true"
            );
            process::exit(1);
        };
        ApiState::new(
            AccountRegistry::with_store_root(store_dir)
                .with_auth_config(auth_config)
                .with_generation_provider_config(OpenRouterConfig::from_env().ok()),
        )
    } else {
        eprintln!("MEMORY_ENGINE_POSTGRES_URL is required for memory-engine-api");
        process::exit(1);
    };

    // Start the background generation worker before serving so captures run
    // asynchronously instead of blocking the request thread.
    state.start_worker();
    let scheduler = state.start_return_notification_scheduler();

    let serve_result = axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await;
    if let Err(error) = &serve_result {
        eprintln!("{error}");
    }
    scheduler.shutdown().await;
    if !shutdown_error_reporting(Duration::from_secs(6)) {
        eprintln!("Canary reporter did not drain before shutdown deadline");
    }
    if serve_result.is_err() {
        process::exit(1);
    }
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        eprintln!("failed to install shutdown signal: {error}");
    }
}

fn local_auth_environment(environment: Option<&str>) -> bool {
    matches!(environment.map(str::trim), Some("development" | "test"))
}

fn normalized_admin_token(token: Option<String>) -> Option<String> {
    token
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty())
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

    // Anonymous account and guest credential minting is deny-by-default. Only
    // explicit local environments may opt into that surface; missing, empty,
    // production, staging, and typoed labels remain production-safe.
    let local_environment =
        local_auth_environment(env::var("MEMORY_ENGINE_ENVIRONMENT").ok().as_deref());
    let production = !local_environment;
    let mut auth_config =
        AuthConfig::allow_emails(allowed_emails).with_anonymous_account_creation(local_environment);
    let unsubscribe_secret = env::var("MEMORY_ENGINE_RETURN_UNSUBSCRIBE_SECRET")
        .map_err(|_| "MEMORY_ENGINE_RETURN_UNSUBSCRIBE_SECRET is required for memory-engine-api")?;
    if unsubscribe_secret.trim().is_empty() {
        return Err("MEMORY_ENGINE_RETURN_UNSUBSCRIBE_SECRET must not be empty".to_owned());
    }
    auth_config = auth_config.with_unsubscribe_secret(unsubscribe_secret);
    if let Ok(token) = env::var("MEMORY_ENGINE_RETURN_NOTIFICATION_MANUAL_TOKEN") {
        let token = token.trim();
        if !token.is_empty() {
            auth_config = auth_config.with_scheduler_manual_token(token.to_owned());
        }
    }
    let admin_token = normalized_admin_token(env::var("MEMORY_ENGINE_ADMIN_TOKEN").ok());
    if let Some(token) = admin_token.as_deref() {
        auth_config = auth_config.with_admin_token(token.to_owned());
    }
    if production && admin_token.is_none() {
        return Err("MEMORY_ENGINE_ADMIN_TOKEN is required in production".to_owned());
    }
    // Local/dev only: surface the magic link on the "check your email" page.
    if env::var("MEMORY_ENGINE_AUTH_EXPOSE_DEBUG_LINKS").as_deref() == Ok("true") {
        if production {
            return Err(
                "MEMORY_ENGINE_AUTH_EXPOSE_DEBUG_LINKS is forbidden in production".to_owned(),
            );
        }
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

#[cfg(test)]
mod tests {
    use super::{local_auth_environment, normalized_admin_token};

    #[test]
    fn auth_bootstrap_environment_gate_is_deny_by_default() {
        for environment in [
            None,
            Some(""),
            Some("production"),
            Some("Production"),
            Some("staging"),
            Some("developement"),
            Some("Development"),
        ] {
            assert!(
                !local_auth_environment(environment),
                "unexpected local auth opt-in"
            );
        }
        assert!(local_auth_environment(Some("development")));
        assert!(local_auth_environment(Some("test")));
    }

    #[test]
    fn auth_bootstrap_admin_token_rejects_whitespace_only_values() {
        assert_eq!(normalized_admin_token(None), None);
        assert_eq!(normalized_admin_token(Some("   ".to_owned())), None);
        assert_eq!(
            normalized_admin_token(Some("  operator-secret  ".to_owned())),
            Some("operator-secret".to_owned())
        );
    }
}
