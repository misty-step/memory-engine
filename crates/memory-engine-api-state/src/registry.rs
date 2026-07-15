use std::{collections::BTreeMap, fmt::Write as _, fs, io::Write as _, process::Command};

use axum::http::HeaderMap;
use hmac::{KeyInit, Mac};
use memory_engine_persistence::GeneratedPromptValidationStatus;
use memory_engine_service::RecordContentFeedbackCommand;

use crate::storage::GenerationCommitFence;
use crate::{
    account_id_for, app_session_max_age_ms, new_browser_session_id, new_magic_link_token,
    new_session_token, normalize_email, normalize_required_text, project_deck_id_for,
    read_browser_session_id, require_account_session, secret_hash, session_csrf_token,
    source_id_for, AccountCreated, AccountRecord, AccountRegistry, ApiFailure, AppAccount,
    AuthConfig, AuthLinkDelivery, BrowserSessionRecord, ContentFeedbackRequest,
    CreateProjectDeckRequest, CreateSourceRequest, InvalidateProjectDeckRequest, MagicLinkRequest,
    ProjectDeckRecord, ReturnNotificationClaimRequest, ReturnNotificationSchedulerConfig,
    ScheduledReturnNotificationReport, SourceRecord, StudyStorage, StudyViewResponse,
    SubmitReviewRequest, APP_ACCOUNT_RATE_LIMIT_MAX_ATTEMPTS, APP_ACCOUNT_RATE_LIMIT_WINDOW_MS,
    AUTH_CHALLENGE_TTL_MS, RETURN_NOTIFICATION_INTERVAL_MS, RETURN_NOTIFICATION_UNSUBSCRIBE_TTL_MS,
};

const RETURN_NOTIFICATION_CLAIM_TTL_MS: i64 = 5 * 60 * 1_000;

impl AccountRegistry {
    /// Create a local account record for the production shell.
    ///
    /// The first slice keeps this registry in-memory while the Postgres adapter
    /// is shaped behind the same account-scoped route contract.
    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn create_account(&self, email: &str) -> Result<AccountCreated, ApiFailure> {
        // Found live during ticket-42 QA: this route issued a session token to
        // any email, bypassing the allowlist the magic-link flow enforces.
        let allowed = {
            let data = self.lock_data();
            data.auth_config.email_allowed(email)
        };
        if !allowed {
            return Err(ApiFailure::forbidden(
                "This email is not allowed to register.",
            ));
        }
        let account_id = account_id_for(email);
        if self.account_exists(&account_id)? {
            return Err(ApiFailure::conflict("Account already exists."));
        }
        let account = AccountCreated {
            account_id: account_id.clone(),
            session_token: new_session_token(),
        };
        let storage = self.storage();
        storage.save_account_session(&account_id, &account.session_token)?;
        let mut data = self.lock_data();
        let record = data
            .accounts
            .entry(account.account_id.clone())
            .or_insert_with(|| AccountRecord {
                session_token: String::new(),
                store_path: storage.account_store_path(&account_id),
                sources: BTreeMap::new(),
                submitted_reviews: BTreeMap::new(),
            });
        record.session_token.clone_from(&account.session_token);
        drop(data);

        Ok(account)
    }

    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn save_account(
        &self,
        source_account_id: &str,
        source_session_token: &str,
        email: &str,
    ) -> Result<AccountCreated, ApiFailure> {
        let allowed = {
            let data = self.lock_data();
            data.auth_config.email_allowed(email)
        };
        if !allowed {
            return Err(ApiFailure::forbidden(
                "This email is not allowed to register.",
            ));
        }
        let target_account_id = account_id_for(email);
        let target = AccountCreated {
            account_id: target_account_id.clone(),
            session_token: new_session_token(),
        };
        let source = self.require_account(source_account_id, source_session_token)?;
        let storage = self.storage();
        if target_account_id != source_account_id && self.account_exists(&target_account_id)? {
            return Err(ApiFailure::conflict("Account already exists."));
        }
        storage.save_account_session(&target_account_id, &target.session_token)?;
        let target_store_path = storage.account_store_path(&target_account_id);
        storage.copy_account(source_account_id, &target_account_id, &source.store_path)?;

        let mut data = self.lock_data();
        let record = data
            .accounts
            .entry(target.account_id.clone())
            .or_insert_with(|| AccountRecord {
                session_token: String::new(),
                store_path: target_store_path,
                sources: source.sources.clone(),
                submitted_reviews: BTreeMap::new(),
            });
        record.session_token.clone_from(&target.session_token);

        Ok(target)
    }

    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn request_magic_link(
        &self,
        email: &str,
        client_rate_limit_key: &str,
    ) -> Result<MagicLinkRequest, ApiFailure> {
        let Some(email) = normalize_email(email) else {
            self.record_app_account_request(None, client_rate_limit_key)?;
            return Err(ApiFailure::bad_request(
                "Account email must contain one @ and a domain.",
            ));
        };
        self.record_app_account_request(Some(&email), client_rate_limit_key)?;
        let auth_config = {
            let data = self.lock_data();
            data.auth_config.clone()
        };
        if !auth_config.email_allowed(&email) {
            return Ok(MagicLinkRequest { debug_link: None });
        }

        let token = new_magic_link_token();
        let token_hash = secret_hash(&token);
        self.storage().save_auth_challenge(
            &token_hash,
            &email,
            self.now().saturating_add(AUTH_CHALLENGE_TTL_MS),
        )?;
        let link = format!("/app/login/verify?token={token}");
        auth_config.deliver_magic_link(&email, &link)?;

        Ok(MagicLinkRequest {
            debug_link: auth_config.expose_debug_links.then_some(link),
        })
    }

    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn verify_magic_link(&self, token: &str) -> Result<AppAccount, ApiFailure> {
        let token_hash = secret_hash(token.trim());
        let email = self
            .storage()
            .consume_auth_challenge(&token_hash, self.now())?
            .ok_or_else(|| ApiFailure::forbidden("Magic link is invalid or expired."))?;
        let account = self.login_account(&email)?;

        self.create_browser_session(&account)
    }

    pub(crate) fn set_return_notification(
        &self,
        account_id: &str,
        session_token: &str,
        email: Option<&str>,
        enabled: bool,
    ) -> Result<(), ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        let storage = self.storage();
        let existing = storage.load_return_notification_preference(account_id)?;
        let email = match (enabled, email, existing.as_ref()) {
            (true, Some(email), _) => normalize_email(email).ok_or_else(|| {
                ApiFailure::bad_request("Reminder email must contain one @ and a domain.")
            })?,
            (true, None, _) => {
                return Err(ApiFailure::bad_request(
                    "Reminder email must contain one @ and a domain.",
                ));
            }
            (false, _, Some(existing)) => existing.email.clone(),
            (false, _, None) => return Ok(()),
        };
        if enabled {
            let allowed = {
                let data = self.lock_data();
                data.auth_config.email_allowed(&email)
            };
            if !allowed {
                return Err(ApiFailure::forbidden(
                    "That reminder email is not allowed for this account.",
                ));
            }
            if account_id_for(&email) != account_id {
                return Err(ApiFailure::forbidden(
                    "That reminder email must belong to the authenticated account.",
                ));
            }
        }
        let unsubscribe_nonce = existing
            .as_ref()
            .filter(|preference| {
                preference.enabled == enabled
                    && preference.email == email
                    && !preference.unsubscribe_nonce.is_empty()
            })
            .map_or_else(new_unsubscribe_nonce, |preference| {
                preference.unsubscribe_nonce.clone()
            });
        let last_sent_at_ms = if enabled {
            None
        } else {
            existing.and_then(|preference| preference.last_sent_at_ms)
        };
        storage.save_return_notification_preference(
            account_id,
            &email,
            enabled,
            last_sent_at_ms,
            &unsubscribe_nonce,
        )?;
        let mut data = self.lock_data();
        data.accounts
            .entry(account_id.to_owned())
            .or_insert(account);
        Ok(())
    }

    pub(crate) fn maybe_send_due_count_notification(
        &self,
        account_id: &str,
        session_token: &str,
        due_count: usize,
        force_confirmation: bool,
    ) -> Result<bool, ApiFailure> {
        self.require_account(account_id, session_token)?;
        self.send_due_count_notification(account_id, due_count, force_confirmation)
    }

    pub(crate) fn run_scheduled_return_notifications(
        &self,
        config: ReturnNotificationSchedulerConfig,
    ) -> Result<ScheduledReturnNotificationReport, ApiFailure> {
        let started_at_ms = self.now();
        let storage = self.storage();
        let mut account_ids = storage.enabled_return_notification_accounts(
            config.batch_size.saturating_add(1),
            started_at_ms,
            RETURN_NOTIFICATION_INTERVAL_MS,
        )?;
        let truncated = account_ids.len() > config.batch_size;
        account_ids.truncate(config.batch_size);
        let mut report = ScheduledReturnNotificationReport {
            started_at_ms,
            ..ScheduledReturnNotificationReport::default()
        };
        report.examined = account_ids.len();
        for account_id in account_ids {
            let preference = match storage.load_return_notification_preference(&account_id) {
                Ok(Some(preference)) => preference,
                Ok(None) => {
                    report.skipped = report.skipped.saturating_add(1);
                    continue;
                }
                Err(error) => {
                    report.failed = report.failed.saturating_add(1);
                    eprintln!(
                        "return notification scheduler preference failed account={account_id}: {error:?}"
                    );
                    continue;
                }
            };
            let view = match storage
                .study_view(&account_id, &storage.account_store_path(&account_id))
            {
                Ok(view) => view,
                Err(error) => {
                    report.failed = report.failed.saturating_add(1);
                    eprintln!("return notification scheduler due-count failed account={account_id}: {error:?}");
                    continue;
                }
            };
            if view.due_count == 0 {
                if preference.pending_delivery_key.is_none() {
                    report.skipped = report.skipped.saturating_add(1);
                    continue;
                }
            } else {
                report.due = report.due.saturating_add(1);
            }
            match self.send_due_count_notification(&account_id, view.due_count, false) {
                Ok(true) => report.sent = report.sent.saturating_add(1),
                Ok(false) => report.skipped = report.skipped.saturating_add(1),
                Err(error) => {
                    report.failed = report.failed.saturating_add(1);
                    eprintln!(
                        "return notification scheduler send failed account={account_id}: {error:?}"
                    );
                }
            }
        }
        report.truncated = truncated;
        report.finished_at_ms = self.now();
        eprintln!(
            "return notification scheduler examined={} due={} sent={} skipped={} failed={} truncated={}",
            report.examined, report.due, report.sent, report.skipped, report.failed, report.truncated
        );
        Ok(report)
    }

    fn send_due_count_notification(
        &self,
        account_id: &str,
        due_count: usize,
        force_confirmation: bool,
    ) -> Result<bool, ApiFailure> {
        let now = self.now();
        let auth_config = {
            let data = self.lock_data();
            data.auth_config.clone()
        };
        let claim_id = format!("return_claim_{:032x}", rand::random::<u128>());
        let delivery_key = format!(
            "return-notification:{account_id}:{:032x}",
            rand::random::<u128>()
        );
        let storage = self.storage();
        let claim_request = ReturnNotificationClaimRequest {
            account_id: account_id.to_owned(),
            now_ms: now,
            due_count,
            force_confirmation,
            interval_ms: RETURN_NOTIFICATION_INTERVAL_MS,
            claim_id,
            delivery_key,
            claim_expires_at_ms: now.saturating_add(RETURN_NOTIFICATION_CLAIM_TTL_MS),
            unsubscribe_nonce: new_unsubscribe_nonce(),
            unsubscribe_expires_at_ms: now.saturating_add(RETURN_NOTIFICATION_UNSUBSCRIBE_TTL_MS),
        };
        let Some(claim) = storage.claim_return_notification(&claim_request)? else {
            return Ok(false);
        };
        let token = signed_unsubscribe_token(
            &auth_config.unsubscribe_secret,
            account_id,
            &claim.email,
            &claim.unsubscribe_nonce,
            claim.unsubscribe_expires_at_ms,
        );
        let unsubscribe_link = format!("/app/return-notifications?token={token}");
        if let Err(error) = auth_config.deliver_due_count_notification(
            &claim.email,
            claim.due_count,
            &unsubscribe_link,
            &claim.delivery_key,
        ) {
            let release_at_ms = self.now();
            storage.release_return_notification(account_id, &claim.claim_id, release_at_ms)?;
            return Err(error);
        }
        let completed_at_ms = self.now();
        if !storage.complete_return_notification(account_id, &claim.claim_id, completed_at_ms)? {
            // A contended completion or an expired lease is a fenced send,
            // not a scheduler failure. The durable delivery key makes the
            // next reclaim idempotent while the persisted claim is recovered.
            return Ok(false);
        }
        Ok(true)
    }

    pub(crate) fn validate_return_notification_token(&self, token: &str) -> Result<(), ApiFailure> {
        let (account_id, email, unsubscribe_nonce) = self.verify_unsubscribe_token(token)?;
        let preference = self
            .storage()
            .load_return_notification_preference(&account_id)?
            .ok_or_else(|| ApiFailure::forbidden("That unsubscribe link is no longer valid."))?;
        if preference.email != email || preference.unsubscribe_nonce != unsubscribe_nonce {
            return Err(ApiFailure::forbidden(
                "That unsubscribe link is not for this reminder account.",
            ));
        }
        Ok(())
    }

    pub(crate) fn disable_return_notification(&self, token: &str) -> Result<(), ApiFailure> {
        let (account_id, email, unsubscribe_nonce) = self.verify_unsubscribe_token(token)?;
        let changed = self.storage().disable_return_notification(
            &account_id,
            &email,
            &unsubscribe_nonce,
            &new_unsubscribe_nonce(),
            self.now(),
        )?;
        if changed {
            Ok(())
        } else {
            Err(ApiFailure::forbidden(
                "That unsubscribe link is not for this reminder account.",
            ))
        }
    }

    fn verify_unsubscribe_token(
        &self,
        token: &str,
    ) -> Result<(String, String, String), ApiFailure> {
        let (payload_hex, signature_hex) = token
            .trim()
            .split_once('.')
            .ok_or_else(|| ApiFailure::forbidden("That unsubscribe link is invalid or expired."))?;
        let payload = decode_hex(payload_hex)
            .ok_or_else(|| ApiFailure::forbidden("That unsubscribe link is invalid or expired."))?;
        let signature = decode_hex(signature_hex)
            .ok_or_else(|| ApiFailure::forbidden("That unsubscribe link is invalid or expired."))?;
        let auth_config = {
            let data = self.lock_data();
            data.auth_config.clone()
        };
        let mut mac =
            crate::UnsubscribeHmac::new_from_slice(auth_config.unsubscribe_secret.as_bytes())
                .map_err(|_| {
                    ApiFailure::internal("unsubscribe signing secret is invalid".to_owned())
                })?;
        mac.update(&payload);
        mac.verify_slice(&signature)
            .map_err(|_| ApiFailure::forbidden("That unsubscribe link is invalid or expired."))?;
        let mut fields = payload.split(|byte| *byte == b'\n');
        if fields.next() != Some(b"v2") {
            return Err(ApiFailure::forbidden(
                "That unsubscribe link is invalid or expired.",
            ));
        }
        let account_id = fields
            .next()
            .and_then(|value| String::from_utf8(value.to_owned()).ok())
            .filter(|value| !value.is_empty());
        let email = fields
            .next()
            .and_then(|value| String::from_utf8(value.to_owned()).ok())
            .filter(|value| !value.is_empty());
        let unsubscribe_nonce = fields
            .next()
            .and_then(|value| String::from_utf8(value.to_owned()).ok())
            .filter(|value| !value.is_empty());
        let expires_at_ms = fields
            .next()
            .and_then(|value| String::from_utf8(value.to_owned()).ok())
            .and_then(|value| value.parse::<i64>().ok());
        if fields.next().is_some() {
            return Err(ApiFailure::forbidden(
                "That unsubscribe link is invalid or expired.",
            ));
        }
        let (Some(account_id), Some(email), Some(unsubscribe_nonce), Some(expires_at_ms)) =
            (account_id, email, unsubscribe_nonce, expires_at_ms)
        else {
            return Err(ApiFailure::forbidden(
                "That unsubscribe link is invalid or expired.",
            ));
        };
        if expires_at_ms <= self.now() {
            return Err(ApiFailure::forbidden(
                "That unsubscribe link is invalid or expired.",
            ));
        }
        Ok((account_id, email, unsubscribe_nonce))
    }

    fn record_app_account_request(
        &self,
        email: Option<&str>,
        client_rate_limit_key: &str,
    ) -> Result<(), ApiFailure> {
        let storage = self.storage();
        let now_ms = self.now();
        let mut keys = Vec::with_capacity(2);
        if let Some(email) = email {
            keys.push(format!("app-account-email:{email}"));
        }
        keys.push(format!("app-account-ip:{client_rate_limit_key}"));

        if !storage.record_rate_limit_attempts(
            &keys,
            now_ms,
            APP_ACCOUNT_RATE_LIMIT_WINDOW_MS,
            APP_ACCOUNT_RATE_LIMIT_MAX_ATTEMPTS,
        )? {
            return Err(ApiFailure::too_many_requests(
                "Too many sign-in attempts. Try again later.",
            ));
        }

        Ok(())
    }

    fn login_account(&self, email: &str) -> Result<AccountCreated, ApiFailure> {
        let account_id = account_id_for(email);
        let account = AccountCreated {
            account_id: account_id.clone(),
            session_token: new_session_token(),
        };
        let storage = self.storage();
        storage.save_account_session(&account_id, &account.session_token)?;
        let mut data = self.lock_data();
        let record = data
            .accounts
            .entry(account.account_id.clone())
            .or_insert_with(|| AccountRecord {
                session_token: String::new(),
                store_path: storage.account_store_path(&account_id),
                sources: BTreeMap::new(),
                submitted_reviews: BTreeMap::new(),
            });
        record.session_token.clone_from(&account.session_token);

        Ok(account)
    }

    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn save_source(
        &self,
        account_id: &str,
        session_token: &str,
        request: &CreateSourceRequest,
    ) -> Result<SourceRecord, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        let title = normalize_required_text(&request.title, "Source title")?;
        let body = normalize_required_text(&request.body, "Source body")?;
        let source = SourceRecord {
            source_id: source_id_for(account_id, &title, &body),
            title,
            body,
            project_key: None,
            ttl_expires_at: None,
        };

        let store_lock = self.store_lock(account_id);
        let _guard = store_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let storage = self.storage();
        storage.save_source(account_id, &account.store_path, &source)?;
        let mut data = self.lock_data();
        let record = data
            .accounts
            .entry(account_id.to_owned())
            .or_insert_with(|| account.clone());
        record
            .sources
            .entry(source.source_id.clone())
            .or_insert_with(|| source.clone());

        Ok(source)
    }

    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, validation, or storage rejects the deck.
    pub(crate) fn create_project_deck(
        &self,
        account_id: &str,
        session_token: &str,
        request: &CreateProjectDeckRequest,
    ) -> Result<ProjectDeckRecord, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        let project_key = normalize_required_text(&request.project_key, "Project key")?;
        let title = normalize_required_text(&request.title, "Project deck title")?;
        let body = normalize_required_text(&request.body, "Project deck body")?;
        let source = SourceRecord {
            source_id: project_deck_id_for(account_id, &project_key, &title, &body),
            title,
            body,
            project_key: Some(project_key.clone()),
            ttl_expires_at: request.ttl_expires_at,
        };

        let store_lock = self.store_lock(account_id);
        let _guard = store_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let storage = self.storage();
        storage.save_source(account_id, &account.store_path, &source)?;
        let mut data = self.lock_data();
        let record = data
            .accounts
            .entry(account_id.to_owned())
            .or_insert_with(|| account.clone());
        record
            .sources
            .entry(source.source_id.clone())
            .or_insert_with(|| source.clone());

        Ok(ProjectDeckRecord {
            deck_id: source.source_id.clone(),
            project_key,
            source,
        })
    }

    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, deck lookup, or storage rejects the event.
    pub(crate) fn invalidate_project_deck(
        &self,
        account_id: &str,
        session_token: &str,
        deck_id: &str,
        request: &InvalidateProjectDeckRequest,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        normalize_required_text(&request.event, "Invalidation event")?;
        let store_lock = self.store_lock(account_id);
        let _guard = store_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.storage()
            .invalidate_project_deck(account_id, &account.store_path, deck_id, self.now())
    }

    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn list_sources(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<Vec<SourceRecord>, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        let storage = self.storage();

        storage.list_sources(account_id, &account.store_path)
    }

    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn generate_source(
        &self,
        account_id: &str,
        session_token: &str,
        source_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        if self.postgres_url().is_some() {
            return Err(ApiFailure::conflict(
                "Direct synchronous generation is disabled in production. Use the queued generation workflow.",
            ));
        }
        let account = self.require_account(account_id, session_token)?;
        self.storage()
            .generate_source(account_id, &account.store_path, source_id)
    }

    /// Run a queued generation job end to end on a worker thread: generate from
    /// the already-saved source, then optimistically approve (schedule) every
    /// accepted card. Returns how many cards were scheduled.
    ///
    /// Session-free by design — enqueueing was already authorized in the request
    /// that created the job, and the background worker is trusted, so it keys off
    /// the account id alone rather than carrying a credential.
    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn run_generation_job(
        &self,
        account_id: &str,
        source_id: &str,
        run_id: &str,
        generation_attempt: i32,
        lease_token: &str,
        lease_valid: impl Fn() -> bool,
    ) -> Result<usize, ApiFailure> {
        // Serialize generation per account: two captures otherwise read-modify-
        // write the whole study store concurrently and clobber each other's
        // cards (059). Held across the whole run; different accounts never block.
        let store_lock = self.store_lock(account_id);
        let _guard = store_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let storage = self.storage();
        let store_path = storage.account_store_path(account_id);
        storage.generate_source_with_run_id(account_id, &store_path, source_id, run_id)?;
        if !lease_valid() {
            return Err(ApiFailure::conflict(
                "Generation lease lost before cards could be committed.",
            ));
        }
        let view = storage.study_view(account_id, &store_path)?;
        let pending = view
            .drafts
            .iter()
            .filter(|draft| {
                draft.validation_status == GeneratedPromptValidationStatus::Accepted
                    && !draft.approved
            })
            .map(|draft| draft.id.clone())
            .collect::<Vec<_>>();
        let card_count = pending.len();
        for draft_id in pending {
            if !lease_valid() {
                return Err(ApiFailure::conflict(
                    "Generation lease lost before cards could be committed.",
                ));
            }
            storage.approve_draft(
                account_id,
                &store_path,
                &draft_id,
                Some(GenerationCommitFence {
                    generation_run_id: run_id,
                    generation_attempt,
                    lease_token,
                }),
            )?;
        }
        Ok(card_count)
    }

    /// Runs the typed content-feedback command for one authenticated account.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, validation, or persistence rejects the
    /// append.
    pub(crate) fn record_content_feedback(
        &self,
        account_id: &str,
        session_token: &str,
        review_unit_id: &str,
        request: &ContentFeedbackRequest,
    ) -> Result<memory_engine_service::ContentFeedback, ApiFailure> {
        let feedback_id = normalize_required_text(&request.idempotency_key, "Idempotency key")?;
        let account = self.require_account(account_id, session_token)?;
        self.storage().record_content_feedback(
            account_id,
            &account.store_path,
            RecordContentFeedbackCommand {
                feedback_id,
                review_unit_id: memory_engine_core::ReviewUnitId::new(review_unit_id),
                verdict: request.verdict,
                rationale: request.rationale.clone(),
                account_id: account_id.to_owned(),
                occurred_at: self.now(),
                supersedes_id: request.supersedes_id.clone(),
            },
        )
    }

    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn archive_source(
        &self,
        account_id: &str,
        session_token: &str,
        source_id: &str,
    ) -> Result<(StudyViewResponse, usize), ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        let store_lock = self.store_lock(account_id);
        let _guard = store_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.storage()
            .archive_source(account_id, &account.store_path, source_id)
    }

    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn approve_draft(
        &self,
        account_id: &str,
        session_token: &str,
        draft_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        let store_lock = self.store_lock(account_id);
        let _guard = store_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.storage()
            .approve_draft(account_id, &account.store_path, draft_id, None)
    }

    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn next_review(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        self.storage().next_review(account_id, &account.store_path)
    }

    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn study_view(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        self.storage().study_view(account_id, &account.store_path)
    }

    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn reveal_review(
        &self,
        account_id: &str,
        session_token: &str,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        self.storage()
            .reveal_review(account_id, &account.store_path, review_unit_id)
    }

    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn learn_more_review(
        &self,
        account_id: &str,
        session_token: &str,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        self.storage()
            .learn_more_review(account_id, &account.store_path, review_unit_id)
    }

    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn skip_review(
        &self,
        account_id: &str,
        session_token: &str,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        let store_lock = self.store_lock(account_id);
        let _guard = store_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.storage()
            .skip_review(account_id, &account.store_path, review_unit_id)
    }

    /// Permanently remove a review card from the learner's queue. Backed by
    /// archival (`archived_at`), so the card never resurfaces in review while
    /// the underlying record stays recoverable in storage.
    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn delete_review(
        &self,
        account_id: &str,
        session_token: &str,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        let store_lock = self.store_lock(account_id);
        let _guard = store_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.storage()
            .delete_review(account_id, &account.store_path, review_unit_id)
    }

    /// Runs an API registry operation for an authenticated review edit.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, review lookup, validation, or storage
    /// rejects the edit.
    pub(crate) fn edit_review(
        &self,
        account_id: &str,
        session_token: &str,
        review_unit_id: &str,
        prompt: &str,
        expected_answer: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        let prompt = normalize_required_text(prompt, "Review unit prompt")?;
        let expected_answer =
            normalize_required_text(expected_answer, "Review unit expected answer")?;
        self.storage().edit_review(
            account_id,
            &account.store_path,
            review_unit_id,
            &prompt,
            &expected_answer,
        )
    }

    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn snooze_review(
        &self,
        account_id: &str,
        session_token: &str,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        let store_lock = self.store_lock(account_id);
        let _guard = store_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.storage()
            .snooze_review(account_id, &account.store_path, review_unit_id)
    }

    /// Runs an API registry operation for the active review's whole concept.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn snooze_concept_review(
        &self,
        account_id: &str,
        session_token: &str,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        let store_lock = self.store_lock(account_id);
        let _guard = store_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.storage()
            .snooze_concept_review(account_id, &account.store_path, review_unit_id)
    }

    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn bridge_review(
        &self,
        account_id: &str,
        session_token: &str,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        let store_lock = self.store_lock(account_id);
        let _guard = store_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.storage()
            .bridge_review(account_id, &account.store_path, review_unit_id)
    }

    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn submit_review(
        &self,
        account_id: &str,
        session_token: &str,
        review_unit_id: &str,
        request: &SubmitReviewRequest,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let idempotency_key = normalize_required_text(&request.idempotency_key, "Idempotency key")?;
        let answer = normalize_required_text(&request.answer, "Review answer")?;
        if request.response_time_ms == 0 {
            return Err(ApiFailure::bad_request(
                "Review response time must be a positive integer.",
            ));
        }
        let storage = self.storage();
        let account = self.require_account(account_id, session_token)?;
        let store_lock = self.store_lock(account_id);
        let _guard = store_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut data = self.lock_data();
        if let Some(response) = data
            .accounts
            .get(account_id)
            .and_then(|account| account.submitted_reviews.get(&idempotency_key))
        {
            return Ok(response.clone());
        }
        let response = storage.submit_review(
            account_id,
            &account.store_path,
            review_unit_id,
            answer,
            request.response_time_ms,
            idempotency_key.clone(),
        )?;
        let record = data
            .accounts
            .entry(account_id.to_owned())
            .or_insert_with(|| account.clone());
        require_account_session(record, session_token)?;
        record
            .submitted_reviews
            .insert(idempotency_key, response.clone());

        Ok(response)
    }

    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn create_browser_session(
        &self,
        account: &AccountCreated,
    ) -> Result<AppAccount, ApiFailure> {
        let browser_session_id = new_browser_session_id();
        let csrf_token = session_csrf_token(&account.session_token);
        let session = BrowserSessionRecord {
            account_id: account.account_id.clone(),
            session_token: account.session_token.clone(),
            csrf_token_hash: secret_hash(&csrf_token),
            expires_at_ms: self.now().saturating_add(app_session_max_age_ms()),
        };
        self.storage()
            .save_browser_session(&browser_session_id, &session)?;
        let mut data = self.lock_data();
        data.browser_sessions
            .insert(browser_session_id.clone(), session);

        Ok(AppAccount {
            browser_session_id,
            account_id: account.account_id.clone(),
            session_token: account.session_token.clone(),
            csrf_token,
        })
    }

    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn require_browser_session(
        &self,
        headers: &HeaderMap,
        csrf_token: &str,
    ) -> Result<AppAccount, ApiFailure> {
        let session_id = read_browser_session_id(headers)?;
        let mut session = {
            let data = self.lock_data();
            data.browser_sessions.get(session_id).cloned()
        };
        if session.is_none() {
            session = self.storage().load_browser_session(session_id)?;
            if let Some(session) = &session {
                let mut data = self.lock_data();
                data.browser_sessions
                    .insert(session_id.to_owned(), session.clone());
            }
        }
        let session = session.ok_or_else(ApiFailure::missing_session)?;
        if session.expires_at_ms <= self.now() {
            let mut data = self.lock_data();
            data.browser_sessions.remove(session_id);
            drop(data);
            return Err(ApiFailure::missing_session());
        }
        if session.csrf_token_hash != secret_hash(csrf_token) {
            return Err(ApiFailure::forbidden("CSRF token does not match session."));
        }
        self.require_account(&session.account_id, &session.session_token)?;

        Ok(AppAccount {
            browser_session_id: session_id.to_owned(),
            account_id: session.account_id,
            session_token: session.session_token,
            csrf_token: csrf_token.to_owned(),
        })
    }

    /// Session-only auth for GET requests, which carry the session cookie but no
    /// CSRF token in the request: the SSE job stream and the signed-in home
    /// render. A GET submits nothing, so there is no token to validate; the
    /// returned account still carries the session's derived CSRF token so a
    /// rendered home can emit valid forms (the actual CSRF guard runs when those
    /// forms POST back through [`AccountRegistry::require_browser_session`]).
    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn require_browser_session_readonly(
        &self,
        headers: &HeaderMap,
    ) -> Result<AppAccount, ApiFailure> {
        let session_id = read_browser_session_id(headers)?;
        let mut session = {
            let data = self.lock_data();
            data.browser_sessions.get(session_id).cloned()
        };
        if session.is_none() {
            session = self.storage().load_browser_session(session_id)?;
            if let Some(session) = &session {
                let mut data = self.lock_data();
                data.browser_sessions
                    .insert(session_id.to_owned(), session.clone());
            }
        }
        let session = session.ok_or_else(ApiFailure::missing_session)?;
        if session.expires_at_ms <= self.now() {
            let mut data = self.lock_data();
            data.browser_sessions.remove(session_id);
            drop(data);
            return Err(ApiFailure::missing_session());
        }
        self.require_account(&session.account_id, &session.session_token)?;

        let csrf_token = session_csrf_token(&session.session_token);
        Ok(AppAccount {
            browser_session_id: session_id.to_owned(),
            account_id: session.account_id,
            session_token: session.session_token,
            csrf_token,
        })
    }

    /// Runs an API registry operation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, storage, or study state rejects the operation.
    pub(crate) fn revoke_browser_session(
        &self,
        headers: &HeaderMap,
        csrf_token: &str,
    ) -> Result<(), ApiFailure> {
        let account = self.require_browser_session(headers, csrf_token)?;
        self.storage()
            .revoke_browser_session(&account.browser_session_id, self.now())?;
        let mut data = self.lock_data();
        data.browser_sessions.remove(&account.browser_session_id);

        Ok(())
    }

    pub(crate) fn storage(&self) -> StudyStorage {
        let data = self.lock_data();
        data.storage.storage(data.now_fn)
    }

    fn account_exists(&self, account_id: &str) -> Result<bool, ApiFailure> {
        let storage = self.storage();
        {
            let data = self.lock_data();
            if data.accounts.contains_key(account_id) {
                return Ok(true);
            }
        }

        storage.account_exists(account_id)
    }

    fn require_account(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<AccountRecord, ApiFailure> {
        let storage = self.storage();
        {
            let data = self.lock_data();
            if let Some(account) = data.accounts.get(account_id) {
                require_account_session(account, session_token)?;

                return Ok(account.clone());
            }
        }

        if storage.account_session_matches(account_id, session_token)? {
            return Ok(AccountRecord {
                session_token: session_token.to_owned(),
                store_path: storage.account_store_path(account_id),
                sources: BTreeMap::new(),
                submitted_reviews: BTreeMap::new(),
            });
        }

        if storage.account_exists(account_id)? {
            return Err(ApiFailure::forbidden_account());
        }

        Err(ApiFailure::unknown_account())
    }
}

fn signed_unsubscribe_token(
    secret: &str,
    account_id: &str,
    email: &str,
    unsubscribe_nonce: &str,
    expires_at_ms: i64,
) -> String {
    let payload = format!("v2\n{account_id}\n{email}\n{unsubscribe_nonce}\n{expires_at_ms}");
    let Ok(mut mac) = crate::UnsubscribeHmac::new_from_slice(secret.as_bytes()) else {
        return String::new();
    };
    mac.update(payload.as_bytes());
    format!(
        "{}.{}",
        encode_hex(payload.as_bytes()),
        encode_hex(&mac.finalize().into_bytes())
    )
}

fn new_unsubscribe_nonce() -> String {
    format!("unsubscribe_nonce_{:032x}", rand::random::<u128>())
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if !value.is_ascii() || !value.len().is_multiple_of(2) {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
        .collect()
}

impl AuthConfig {
    fn deliver_magic_link(&self, email: &str, link: &str) -> Result<(), ApiFailure> {
        match &self.link_delivery {
            AuthLinkDelivery::None => Ok(()),
            AuthLinkDelivery::OutboxFile(path) => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|error| ApiFailure::internal(error.to_string()))?;
                }
                let existing = fs::read_to_string(path).unwrap_or_default();
                fs::write(path, format!("{existing}{email}\t{link}\n"))
                    .map_err(|error| ApiFailure::internal(error.to_string()))
            }
            AuthLinkDelivery::Command(command) => {
                let status = Command::new(command)
                    .env("MEMORY_ENGINE_AUTH_EMAIL", email)
                    .env("MEMORY_ENGINE_AUTH_LINK", link)
                    .status()
                    .map_err(|error| ApiFailure::internal(error.to_string()))?;
                if status.success() {
                    Ok(())
                } else {
                    Err(ApiFailure::internal(format!(
                        "auth mailer command exited with {status}"
                    )))
                }
            }
        }
    }

    fn deliver_due_count_notification(
        &self,
        email: &str,
        due_count: usize,
        unsubscribe_link: &str,
        delivery_key: &str,
    ) -> Result<(), ApiFailure> {
        match &self.link_delivery {
            AuthLinkDelivery::None => Ok(()),
            AuthLinkDelivery::OutboxFile(path) => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|error| ApiFailure::internal(error.to_string()))?;
                }
                let lock_path = path.with_extension("lock");
                let _lock = crate::file_lock::acquire(&lock_path)?;
                let already_recorded = fs::read_to_string(path)
                    .unwrap_or_default()
                    .lines()
                    .filter(|line| line.starts_with("due-count\t"))
                    .any(|line| line.split('\t').nth(3) == Some(delivery_key));
                if already_recorded {
                    return Ok(());
                }
                let mut outbox = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .map_err(|error| ApiFailure::internal(error.to_string()))?;
                writeln!(
                    outbox,
                    "due-count\t{email}\t{due_count}\t{delivery_key}\t{unsubscribe_link}"
                )
                .map_err(|error| ApiFailure::internal(error.to_string()))
            }
            AuthLinkDelivery::Command(command) => {
                let status = Command::new(command)
                    .env("MEMORY_ENGINE_RETURN_NOTIFICATION_EMAIL", email)
                    .env(
                        "MEMORY_ENGINE_RETURN_NOTIFICATION_DUE_COUNT",
                        due_count.to_string(),
                    )
                    .env(
                        "MEMORY_ENGINE_RETURN_NOTIFICATION_UNSUBSCRIBE",
                        unsubscribe_link,
                    )
                    .env(
                        "MEMORY_ENGINE_RETURN_NOTIFICATION_IDEMPOTENCY_KEY",
                        delivery_key,
                    )
                    .status()
                    .map_err(|error| ApiFailure::internal(error.to_string()))?;
                if status.success() {
                    Ok(())
                } else {
                    Err(ApiFailure::internal(format!(
                        "return notification mailer command exited with {status}"
                    )))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Arc, Barrier},
        thread,
        time::Duration,
    };

    fn test_now() -> i64 {
        1_700_000_000_000
    }

    #[test]
    fn file_outbox_deduplicates_a_reclaimed_slow_send_by_delivery_key() {
        let root = std::env::temp_dir().join(format!(
            "memory-engine-outbox-reclaim-{}-{}",
            std::process::id(),
            rand::random::<u128>()
        ));
        let outbox = root.join("reminders.tsv");
        let storage = StudyStorage::file(&root, test_now);
        storage
            .save_return_notification_preference(
                "account-slow-send",
                "slow@example.com",
                true,
                None,
                "slow-nonce",
            )
            .expect("preference");
        let first = storage
            .claim_return_notification(&ReturnNotificationClaimRequest {
                account_id: "account-slow-send".to_owned(),
                now_ms: test_now(),
                due_count: 3,
                force_confirmation: true,
                interval_ms: RETURN_NOTIFICATION_INTERVAL_MS,
                claim_id: "slow-claim-1".to_owned(),
                delivery_key: "slow-delivery-key".to_owned(),
                claim_expires_at_ms: test_now() + 50,
                unsubscribe_nonce: "slow-nonce".to_owned(),
                unsubscribe_expires_at_ms: test_now() + 604_800_000,
            })
            .expect("first claim")
            .expect("first claim available");
        let second_storage = storage.clone();
        let first_auth = AuthConfig::default().with_link_outbox(&outbox);
        let second_auth = first_auth.clone();
        let first = Arc::new(first);
        let first_for_thread = Arc::clone(&first);
        let slow_send_started = Arc::new(Barrier::new(2));
        let reclaimed_send_finished = Arc::new(Barrier::new(2));
        let slow_send_started_for_thread = Arc::clone(&slow_send_started);
        let reclaimed_send_finished_for_thread = Arc::clone(&reclaimed_send_finished);
        let slow_sender = thread::spawn(move || {
            // The provider accepted the request only after the lease expired;
            // the durable outbox must still collapse the reclaim to one key.
            slow_send_started_for_thread.wait();
            reclaimed_send_finished_for_thread.wait();
            first_auth
                .deliver_due_count_notification(
                    &first_for_thread.email,
                    first_for_thread.due_count,
                    "/unsubscribe/slow",
                    &first_for_thread.delivery_key,
                )
                .expect("slow sender outbox");
        });

        slow_send_started.wait();
        thread::sleep(Duration::from_millis(75));
        let second = second_storage
            .claim_return_notification(&ReturnNotificationClaimRequest {
                account_id: "account-slow-send".to_owned(),
                now_ms: test_now() + 100,
                due_count: 4,
                force_confirmation: false,
                interval_ms: RETURN_NOTIFICATION_INTERVAL_MS,
                claim_id: "slow-claim-2".to_owned(),
                delivery_key: "new-delivery-key-must-not-replace-pending".to_owned(),
                claim_expires_at_ms: test_now() + 1_000,
                unsubscribe_nonce: "new-nonce-must-not-replace-pending".to_owned(),
                unsubscribe_expires_at_ms: test_now() + 604_800_100,
            })
            .expect("reclaim")
            .expect("expired claim is reclaimable");
        assert_eq!(second.delivery_key, first.delivery_key);
        second_auth
            .deliver_due_count_notification(
                &second.email,
                second.due_count,
                "/unsubscribe/slow",
                &second.delivery_key,
            )
            .expect("reclaimed sender outbox");
        reclaimed_send_finished.wait();
        slow_sender.join().expect("slow sender");

        let lines = fs::read_to_string(&outbox)
            .expect("outbox")
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 1, "one delivery key produces one durable send");
        assert!(lines[0].contains("slow-delivery-key"));
        let _ = fs::remove_dir_all(root);
    }
}
