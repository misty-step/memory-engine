use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::json;

const DEFAULT_BASE_URL: &str = "https://memory-engine-api-i2xcr.ondigitalocean.app";
const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_SOURCE_BODY: &str = "Concept: NATO letter A\nActivity: quiz\nStage: recognition-3\nQuestion: What is the NATO phonetic alphabet word for A?\nAnswer: ALFA\nDistractors: BRAVO, CHARLIE\nReference: The NATO phonetic alphabet word for A is ALFA.\n\nConcept: NATO CAT composition\nActivity: exercise\nStage: composition\nQuestion: Spell CAT over the phone using the NATO phonetic alphabet.\nAnswer: CHARLIE ALFA TANGO\nWorked Solution: C is CHARLIE, A is ALFA, and T is TANGO.\nReference: C is CHARLIE. A is ALFA. T is TANGO.";
const REQUIRED_CONTRACT_PATHS: &[&str] = &[
    "/v1/accounts/{account_id}/sources",
    "/v1/accounts/{account_id}/sources/{source_id}",
    "/v1/accounts/{account_id}/sources/{source_id}/generate",
    "/v1/accounts/{account_id}/drafts/{draft_id}/keep",
    "/v1/accounts/{account_id}/review/next",
    "/v1/accounts/{account_id}/review/{review_unit_id}/reveal",
    "/v1/accounts/{account_id}/review/{review_unit_id}/submit",
];

fn main() {
    match cli_action(env::args().skip(1), CliEnv::from_process()) {
        Ok(CliAction::Help) => print_usage(),
        Ok(CliAction::Run(config)) => match run_contract(&config) {
            Ok(receipt) => match serde_json::to_string_pretty(&receipt) {
                Ok(serialized) => println!("{serialized}"),
                Err(error) => {
                    eprintln!("contract receipt could not be serialized: {error}");
                    std::process::exit(1);
                }
            },
            Err(error) => {
                eprintln!("contract run failed: {error}");
                std::process::exit(1);
            }
        },
        Err(error) => {
            eprintln!("{error}");
            eprintln!();
            print_usage();
            std::process::exit(2);
        }
    }
}

fn print_usage() {
    println!(
        "usage: cargo run -p memory-engine-contract -- --account-id ID --session-token TOKEN [--base-url URL]\n\nCredentials must be pre-provisioned by the invite or operator service-session flow. Environment fallbacks: MEMORY_ENGINE_ACCOUNT_ID and MEMORY_ENGINE_SESSION_TOKEN."
    );
}

fn run_contract(config: &ContractConfig) -> Result<ContractReceipt, ContractFailure> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build()
        .into();
    let contract = fetch_openapi(&agent, &config.base_url)?;
    let Credentials::Existing {
        account_id,
        session_token,
    } = &config.credentials;
    let session = Session {
        account_id: account_id.clone(),
        token: session_token.clone(),
    };
    let client = ContractClient::new(agent, &config.base_url, session);
    let source = client.create_source(&config.source_title, &config.source_body)?;
    let source_id = source.source_id;

    let proof = drive_review_loop(&client, &source_id);
    let archive = client.archive_source(&source_id);
    let proof = match (proof, archive) {
        (Ok(proof), Ok(())) => proof,
        (Err(error), Ok(())) | (Ok(_), Err(error)) => return Err(error),
        (Err(error), Err(cleanup_error)) => {
            return Err(ContractFailure(format!(
                "{error}; cleanup also failed: {cleanup_error}"
            )));
        }
    };

    let active_source_present_after_archive = client
        .list_sources()?
        .sources
        .iter()
        .any(|source| source.source_id == source_id);

    if active_source_present_after_archive {
        return Err(ContractFailure(format!(
            "source {source_id} remained active after archive"
        )));
    }

    Ok(ContractReceipt {
        base_url: config.base_url.clone(),
        openapi_version: contract.openapi,
        contract_path_count: contract.paths.len(),
        account_id: client.account_id.clone(),
        source_id,
        draft_id: proof.draft_id,
        review_unit_id: proof.review_unit_id,
        revealed_answer: proof.revealed_answer,
        verdict: proof.verdict,
        attempt_count: proof.attempt_count,
        archived_source: true,
        active_source_present_after_archive,
    })
}

fn fetch_openapi(agent: &ureq::Agent, base_url: &str) -> Result<OpenApiContract, ContractFailure> {
    let mut response = agent
        .get(&endpoint(base_url, "/v1/openapi.json"))
        .call()
        .map_err(|error| transport_failure("fetch OpenAPI contract", &error))?;
    let contract: OpenApiContract = read_json(&mut response, "fetch OpenAPI contract")?;
    if contract.openapi != "3.1.0" {
        return Err(ContractFailure(format!(
            "OpenAPI contract version {} is unsupported",
            contract.openapi
        )));
    }

    for path in REQUIRED_CONTRACT_PATHS {
        if !contract.paths.contains_key(*path) {
            return Err(ContractFailure(format!(
                "OpenAPI contract is missing required path {path}"
            )));
        }
    }

    Ok(contract)
}

fn drive_review_loop(
    client: &ContractClient,
    source_id: &str,
) -> Result<ReviewLoopProof, ContractFailure> {
    let generated: StudyView = client.post_empty(&format!(
        "/v1/accounts/{}/sources/{source_id}/generate",
        client.account_id
    ))?;
    let draft_id = generated
        .drafts
        .first()
        .ok_or_else(|| ContractFailure("source generation returned no drafts".to_owned()))?
        .id
        .clone();

    client.post_empty::<StudyView>(&format!(
        "/v1/accounts/{}/drafts/{draft_id}/keep",
        client.account_id
    ))?;

    let next: StudyView =
        client.post_empty(&format!("/v1/accounts/{}/review/next", client.account_id))?;
    let review_unit_id = next
        .current
        .ok_or_else(|| ContractFailure("queue/next returned no current review".to_owned()))?
        .review_unit_id;

    let revealed: StudyView = client.post_empty(&format!(
        "/v1/accounts/{}/review/{review_unit_id}/reveal",
        client.account_id
    ))?;
    let revealed_answer = revealed
        .current
        .and_then(|current| current.expected_answer)
        .ok_or_else(|| ContractFailure("reveal returned no expected answer".to_owned()))?;

    let submitted: StudyView = client.post_json(
        &format!(
            "/v1/accounts/{}/review/{review_unit_id}/submit",
            client.account_id
        ),
        &json!({
            "answer": revealed_answer,
            "responseTimeMs": 1200,
            "idempotencyKey": format!("scry-contract-{source_id}")
        }),
    )?;
    let current = submitted
        .current
        .ok_or_else(|| ContractFailure("submit returned no current review".to_owned()))?;
    let verdict = current
        .grade
        .ok_or_else(|| ContractFailure("submit returned no grade".to_owned()))?
        .verdict;

    Ok(ReviewLoopProof {
        draft_id,
        review_unit_id,
        revealed_answer,
        verdict,
        attempt_count: submitted.summary.attempt_count,
    })
}

fn read_json<T: DeserializeOwned>(
    response: &mut ureq::http::Response<ureq::Body>,
    action: &str,
) -> Result<T, ContractFailure> {
    response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_json()
        .map_err(|error| ContractFailure(format!("{action} returned unreadable JSON: {error}")))
}

fn transport_failure(action: &str, error: &ureq::Error) -> ContractFailure {
    match error {
        ureq::Error::StatusCode(status) => {
            ContractFailure(format!("{action} failed with HTTP {status}"))
        }
        _ => ContractFailure(format!("{action} transport failed: {error}")),
    }
}

fn endpoint(base_url: &str, path: &str) -> String {
    format!("{}{}", base_url.trim_end_matches('/'), path)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CliEnv {
    account_id: Option<String>,
    session_token: Option<String>,
}

impl CliEnv {
    fn from_process() -> Self {
        Self {
            account_id: non_empty_env("MEMORY_ENGINE_ACCOUNT_ID"),
            session_token: non_empty_env("MEMORY_ENGINE_SESSION_TOKEN"),
        }
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CliAction {
    Run(ContractConfig),
    Help,
}

fn cli_action(
    args: impl IntoIterator<Item = String>,
    process_env: CliEnv,
) -> Result<CliAction, ContractFailure> {
    let args = args.into_iter().collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Ok(CliAction::Help);
    }

    let mut base_url = DEFAULT_BASE_URL.to_owned();
    let mut account_id = process_env.account_id;
    let mut session_token = process_env.session_token;
    let mut source_title = unique_source_title();
    let mut source_body = DEFAULT_SOURCE_BODY.to_owned();
    let mut index = 0;

    while index < args.len() {
        let flag = &args[index];
        let value = args
            .get(index + 1)
            .ok_or_else(|| ContractFailure(format!("{flag} requires a value")))?;
        match flag.as_str() {
            "--base-url" => base_url.clone_from(value),
            "--account-id" => account_id = Some(value.clone()),
            "--session-token" => session_token = Some(value.clone()),
            "--source-title" => source_title.clone_from(value),
            "--source-body" => source_body.clone_from(value),
            other => return Err(ContractFailure(format!("unknown argument {other}"))),
        }
        index += 2;
    }

    let credentials = match (account_id, session_token) {
        (Some(account_id), Some(session_token)) => Credentials::Existing {
            account_id,
            session_token,
        },
        (None, None) => {
            return Err(ContractFailure(
                "--account-id and --session-token are required; credentials must be pre-provisioned"
                    .to_owned(),
            ));
        }
        _ => {
            return Err(ContractFailure(
                "--account-id and --session-token must be provided together".to_owned(),
            ));
        }
    };

    Ok(CliAction::Run(ContractConfig {
        base_url,
        credentials,
        source_title,
        source_body,
    }))
}

fn unique_source_title() -> String {
    format!("Scry v1 contract fixture {}", unique_suffix())
}

fn unique_suffix() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    format!("{}-{millis}", std::process::id())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ContractConfig {
    base_url: String,
    credentials: Credentials,
    source_title: String,
    source_body: String,
}

#[derive(Clone, Eq, PartialEq)]
enum Credentials {
    Existing {
        account_id: String,
        session_token: String,
    },
}

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Existing { account_id, .. } => formatter
                .debug_struct("Existing")
                .field("account_id", account_id)
                .field("session_token", &"<redacted>")
                .finish(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Session {
    account_id: String,
    token: String,
}

#[derive(Clone, Debug)]
struct ContractClient {
    agent: ureq::Agent,
    base_url: String,
    account_id: String,
    session_token: String,
}

impl ContractClient {
    fn new(agent: ureq::Agent, base_url: &str, session: Session) -> Self {
        Self {
            agent,
            base_url: base_url.to_owned(),
            account_id: session.account_id,
            session_token: session.token,
        }
    }

    fn create_source(&self, title: &str, body: &str) -> Result<SourceRecord, ContractFailure> {
        self.post_json(
            &format!("/v1/accounts/{}/sources", self.account_id),
            &json!({
                "title": title,
                "body": body,
            }),
        )
    }

    fn list_sources(&self) -> Result<SourceList, ContractFailure> {
        let mut response = self
            .agent
            .get(&endpoint(
                &self.base_url,
                &format!("/v1/accounts/{}/sources", self.account_id),
            ))
            .header("Authorization", &self.authorization())
            .call()
            .map_err(|error| transport_failure("list sources", &error))?;
        read_json(&mut response, "list sources")
    }

    fn archive_source(&self, source_id: &str) -> Result<(), ContractFailure> {
        let response = self
            .agent
            .delete(&endpoint(
                &self.base_url,
                &format!("/v1/accounts/{}/sources/{source_id}", self.account_id),
            ))
            .header("Authorization", &self.authorization())
            .call()
            .map_err(|error| transport_failure("archive source", &error))?;

        if response.status().as_u16() == 204 {
            Ok(())
        } else {
            Err(ContractFailure(format!(
                "archive source returned HTTP {}",
                response.status()
            )))
        }
    }

    fn post_empty<T: DeserializeOwned>(&self, path: &str) -> Result<T, ContractFailure> {
        let mut response = self
            .agent
            .post(&endpoint(&self.base_url, path))
            .header("Authorization", &self.authorization())
            .send_empty()
            .map_err(|error| transport_failure(path, &error))?;
        read_json(&mut response, path)
    }

    fn post_json<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<T, ContractFailure> {
        let mut response = self
            .agent
            .post(&endpoint(&self.base_url, path))
            .header("Authorization", &self.authorization())
            .send_json(body)
            .map_err(|error| transport_failure(path, &error))?;
        read_json(&mut response, path)
    }

    fn authorization(&self) -> String {
        format!("Bearer {}", self.session_token)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct OpenApiContract {
    openapi: String,
    paths: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct SourceRecord {
    source_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct SourceList {
    sources: Vec<SourceRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct StudyView {
    drafts: Vec<Draft>,
    current: Option<CurrentReview>,
    summary: StudySummary,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct Draft {
    id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct CurrentReview {
    review_unit_id: String,
    expected_answer: Option<String>,
    grade: Option<Grade>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct Grade {
    verdict: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct StudySummary {
    attempt_count: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReviewLoopProof {
    draft_id: String,
    review_unit_id: String,
    revealed_answer: String,
    verdict: String,
    attempt_count: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ContractReceipt {
    base_url: String,
    openapi_version: String,
    contract_path_count: usize,
    account_id: String,
    source_id: String,
    draft_id: String,
    review_unit_id: String,
    revealed_answer: String,
    verdict: String,
    attempt_count: u32,
    archived_source: bool,
    active_source_present_after_archive: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ContractFailure(String);

impl fmt::Display for ContractFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ContractFailure {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_uses_existing_credentials_from_flags() {
        let action = cli_action(
            [
                "--base-url",
                "http://127.0.0.1:18080",
                "--account-id",
                "acct_demo",
                "--session-token",
                "secret",
            ]
            .into_iter()
            .map(str::to_owned),
            CliEnv {
                account_id: None,
                session_token: None,
            },
        )
        .expect("cli action");

        let CliAction::Run(config) = action else {
            panic!("expected run action");
        };
        assert_eq!(config.base_url, "http://127.0.0.1:18080");
        assert_eq!(
            config.credentials,
            Credentials::Existing {
                account_id: "acct_demo".to_owned(),
                session_token: "secret".to_owned(),
            }
        );
    }

    #[test]
    fn parser_rejects_missing_credentials_instead_of_minting_an_account() {
        let error = cli_action(
            std::iter::empty::<String>(),
            CliEnv {
                account_id: None,
                session_token: None,
            },
        )
        .expect_err("missing credentials should fail closed");

        assert_eq!(
            error.to_string(),
            "--account-id and --session-token are required; credentials must be pre-provisioned"
        );
    }

    #[test]
    fn parser_rejects_partial_existing_credentials() {
        let error = cli_action(
            ["--account-id", "acct_demo"].into_iter().map(str::to_owned),
            CliEnv {
                account_id: None,
                session_token: None,
            },
        )
        .expect_err("partial credentials should fail");

        assert_eq!(
            error.to_string(),
            "--account-id and --session-token must be provided together"
        );
    }

    #[test]
    fn default_source_fixture_is_suitable_for_answer_submit() {
        assert!(DEFAULT_SOURCE_BODY
            .contains("Question: What is the NATO phonetic alphabet word for A?"));
        assert!(DEFAULT_SOURCE_BODY.contains("Answer: ALFA"));
    }

    #[test]
    fn receipt_serialization_does_not_include_session_token() {
        let receipt = ContractReceipt {
            base_url: "https://memory-engine-api-i2xcr.ondigitalocean.app".to_owned(),
            openapi_version: "3.1.0".to_owned(),
            contract_path_count: 9,
            account_id: "acct_demo".to_owned(),
            source_id: "src_demo".to_owned(),
            draft_id: "draft_demo".to_owned(),
            review_unit_id: "ru_demo".to_owned(),
            revealed_answer: "ALFA".to_owned(),
            verdict: "correct".to_owned(),
            attempt_count: 1,
            archived_source: true,
            active_source_present_after_archive: false,
        };

        let serialized = serde_json::to_string(&receipt).expect("receipt JSON");
        assert!(!serialized.contains("session"));
        assert!(!serialized.contains("token"));
    }

    #[test]
    fn endpoint_trims_duplicate_slashes() {
        assert_eq!(
            endpoint(
                "https://memory-engine-api-i2xcr.ondigitalocean.app/",
                "/v1/openapi.json"
            ),
            "https://memory-engine-api-i2xcr.ondigitalocean.app/v1/openapi.json"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn runner_drives_real_api_over_http() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local API listener");
        let address = listener.local_addr().expect("local address");
        let store_root =
            std::env::temp_dir().join(format!("memory-engine-contract-{}", unique_suffix()));
        let state = memory_engine_api::ApiState::new(
            memory_engine_api::AccountRegistry::with_store_root(&store_root).with_auth_config(
                memory_engine_api::AuthConfig::allow_emails([
                    "scry-contract-local@example.com".to_owned()
                ])
                .with_anonymous_account_creation(true),
            ),
        );
        let created = state
            .create_account("scry-contract-local@example.com")
            .expect("pre-provision local account");
        let server = tokio::spawn(async move {
            axum::serve(listener, memory_engine_api::router(state))
                .await
                .expect("serve local API");
        });

        let receipt = run_contract(&ContractConfig {
            base_url: format!("http://{address}"),
            credentials: Credentials::Existing {
                account_id: created.account_id,
                session_token: created.session_token,
            },
            source_title: "Scry v1 contract test".to_owned(),
            source_body: DEFAULT_SOURCE_BODY.to_owned(),
        })
        .expect("contract run");

        server.abort();

        assert_eq!(receipt.openapi_version, "3.1.0");
        assert_eq!(receipt.verdict, "correct");
        assert_eq!(receipt.attempt_count, 1);
        assert!(receipt.archived_source);
        assert!(!receipt.active_source_present_after_archive);
    }
}
