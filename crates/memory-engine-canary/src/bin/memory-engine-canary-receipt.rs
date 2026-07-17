use std::{
    process::ExitCode,
    time::{Duration, Instant},
};

use memory_engine_canary::{
    read_performance_timeline, CanaryConfig, CanaryReporter, ReadbackConfig,
};
use memory_engine_performance::{
    Action, CompletionMarker, CompletionPhase, MachineRouteAction, Outcome,
};

const READ_ENDPOINT_ENV: &str = "CANARY_READ_ENDPOINT";
const READ_API_KEY_ENV: &str = "CANARY_READ_API_KEY";
const OPENAPI_URL_ENV: &str = "MEMORY_ENGINE_RECEIPT_OPENAPI_URL";
const DEFAULT_SERVICE: &str = "memory-engine-api";
const OVERHEAD_SAMPLES: usize = 20_000;
const MAX_ADMISSION_P95_NANOS: u64 = 5_000_000;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("memory-engine-canary-receipt: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    match std::env::args().nth(1).as_deref() {
        Some("emit-openapi") => emit_openapi(),
        Some("readback") => readback(),
        Some("overhead") => overhead(),
        _ => Err(
            "usage: memory-engine-canary-receipt <emit-openapi|readback|overhead>\n\
             emit-openapi env: CANARY_ENDPOINT, CANARY_API_KEY, optional MEMORY_ENGINE_RECEIPT_OPENAPI_URL\n\
             readback env: CANARY_READ_ENDPOINT, CANARY_READ_API_KEY, optional CANARY_READ_SERVICE/CANARY_READ_WINDOW\n\
             overhead: measures bounded report_performance admission p95"
                .to_owned(),
        ),
    }
}

fn emit_openapi() -> Result<(), String> {
    let config = CanaryConfig::from_env()
        .ok_or_else(|| "CANARY_ENDPOINT and CANARY_API_KEY are required".to_owned())?;
    let url = std::env::var(OPENAPI_URL_ENV).unwrap_or_else(|_| {
        let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_owned());
        format!("http://127.0.0.1:{port}/v1/openapi.json")
    });
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    let started = Instant::now();
    let outcome = match agent.get(&url).call() {
        Ok(mut response) => {
            let status = response.status();
            if !status.is_success() {
                let elapsed = duration_ms(started);
                emit_observation(config, elapsed, Outcome::ServerFailed)?;
                return Err(format!("OpenAPI request failed with status {status}"));
            }
            response
                .body_mut()
                .read_to_vec()
                .map_err(|error| format!("OpenAPI response read failed: {error}"))?;
            Outcome::Succeeded
        }
        Err(error) => {
            let elapsed = duration_ms(started);
            emit_observation(config, elapsed, Outcome::ServerFailed)?;
            return Err(format!("OpenAPI request failed: {error}"));
        }
    };
    let elapsed = duration_ms(started);
    emit_observation(config, elapsed, outcome)?;
    println!(
        "{}",
        serde_json::json!({
            "schema": "memory_engine.performance_receipt.v1",
            "action": "open_api",
            "duration_ms": elapsed,
            "outcome": "succeeded",
            "drained": true,
        })
    );
    Ok(())
}

fn emit_observation(
    config: CanaryConfig,
    duration_ms: u64,
    outcome: Outcome,
) -> Result<(), String> {
    let marker = CompletionMarker::server(
        Action::Machine(MachineRouteAction::OpenApi),
        CompletionPhase::ImmediateAck,
        outcome,
    )
    .map_err(|error| error.to_string())?;
    let observation = marker
        .observation(duration_ms)
        .map_err(|error| error.to_string())?;
    let reporter = CanaryReporter::new(config);
    if !reporter.report_performance(observation) {
        return Err("bounded reporter queue rejected the receipt observation".to_owned());
    }
    if !reporter.shutdown(Duration::from_secs(6)) {
        return Err("Canary reporter did not drain before the shutdown deadline".to_owned());
    }
    Ok(())
}

fn readback() -> Result<(), String> {
    let endpoint =
        std::env::var(READ_ENDPOINT_ENV).map_err(|_| format!("{READ_ENDPOINT_ENV} is required"))?;
    let api_key =
        std::env::var(READ_API_KEY_ENV).map_err(|_| format!("{READ_API_KEY_ENV} is required"))?;
    let service =
        std::env::var("CANARY_READ_SERVICE").unwrap_or_else(|_| DEFAULT_SERVICE.to_owned());
    let window = std::env::var("CANARY_READ_WINDOW").unwrap_or_else(|_| "1h".to_owned());
    let config = ReadbackConfig::new(endpoint, api_key, service, window)
        .map_err(|error| error.to_string())?;
    let receipt = read_performance_timeline(&config).map_err(|error| error.to_string())?;
    println!("{}", receipt.as_value());
    Ok(())
}

fn overhead() -> Result<(), String> {
    let config = CanaryConfig::from_parts(
        Some("http://127.0.0.1:9".to_owned()),
        Some("overhead-receipt".to_owned()),
    )
    .ok_or_else(|| "failed to construct overhead reporter".to_owned())?;
    let reporter = CanaryReporter::new(config);
    let marker = CompletionMarker::server(
        Action::Machine(MachineRouteAction::OpenApi),
        CompletionPhase::ImmediateAck,
        Outcome::Succeeded,
    )
    .map_err(|error| error.to_string())?;
    let observation = marker.observation(1).map_err(|error| error.to_string())?;
    let mut durations = Vec::with_capacity(OVERHEAD_SAMPLES);
    let mut accepted = 0_usize;
    for _ in 0..OVERHEAD_SAMPLES {
        let started = Instant::now();
        if reporter.report_performance(observation) {
            accepted += 1;
        }
        durations.push(u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX));
    }
    durations.sort_unstable();
    let p95_index = (durations.len() * 95).div_ceil(100).saturating_sub(1);
    let p95_nanos = durations[p95_index];
    let _ = reporter.shutdown(Duration::from_secs(6));
    println!(
        "{}",
        serde_json::json!({
            "schema": "memory_engine.performance_overhead_receipt.v1",
            "samples": OVERHEAD_SAMPLES,
            "accepted": accepted,
            "p95_nanos": p95_nanos,
            "limit_nanos": MAX_ADMISSION_P95_NANOS,
            "passed": p95_nanos <= MAX_ADMISSION_P95_NANOS,
        })
    );
    if p95_nanos > MAX_ADMISSION_P95_NANOS {
        return Err(format!(
            "reporter admission p95 {p95_nanos}ns exceeds {MAX_ADMISSION_P95_NANOS}ns"
        ));
    }
    Ok(())
}

fn duration_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}
