//! Closed, versioned performance contract for boundary instrumentation.
//!
//! This crate intentionally contains no telemetry sender, runtime framework, or
//! persistence dependency. It defines only fixed, privacy-safe values and a
//! mergeable aggregate that downstream boundary crates can export later.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The only schema version currently accepted on the wire.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaVersion {
    V1,
}

/// The v1 contract identifier.
pub const SCHEMA_ID: &str = "memory_engine.performance";
/// Baseline that downstream contract consumers must pin until a new schema version is published.
pub const BASELINE_VERSION: SchemaVersion = SchemaVersion::V1;
/// Maximum request/UI duration accepted by v1, in milliseconds.
pub const REQUEST_UI_MAX_DURATION_MS: u64 = 60_000;
/// Maximum generation duration accepted by v1, in milliseconds.
pub const GENERATION_MAX_DURATION_MS: u64 = 600_000;
/// Strict maximum for one aggregate payload. Valid payloads must be smaller.
pub const MAX_PAYLOAD_BYTES: usize = 2 * 1024;
/// Fixed upper bound on the number of dimension series in one aggregate window.
pub const MAX_SERIES_CARDINALITY: usize = 32_768;
/// Aggregate export budget per minute.
pub const MAX_AGGREGATE_BATCHES_PER_MINUTE: u8 = 3;

/// Auth and account actions available to browser and service boundaries.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthAccountAction {
    CreateAccount,
    Login,
    LoginVerify,
    Logout,
    IssueServiceSession,
    AppHome,
    Analytics,
    SaveAccount,
    ReturnNotifications,
}

/// Material/source actions available to browser and service boundaries.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialAction {
    ListSources,
    CreateSource,
    CaptureSource,
    ArchiveSource,
    CreateProjectDeck,
    InvalidateProjectDeck,
}

/// Review actions available to browser and service boundaries.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAction {
    Start,
    Next,
    Reveal,
    Reference,
    Skip,
    Snooze,
    SnoozeConcept,
    Bridge,
    Submit,
    ContentFeedback,
    Edit,
    SaveEdit,
    Delete,
}

/// Generation actions available to browser and service boundaries.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationAction {
    Start,
    Enqueue,
    GetJob,
    Keep,
    EditDraft,
    RejectDraft,
    Retry,
    DurableTerminal,
}

/// Actions that can only be emitted by machine/API routes.
///
/// These names mirror the closed v1 route operation set rather than accepting
/// a path string. Raw paths and route templates are privacy exclusions.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineRouteAction {
    OpenApi,
    Accounts,
    ServiceSessions,
    Sources,
    Source,
    ProjectDecks,
    ProjectDeckInvalidate,
    Generate,
    GenerationJobs,
    GenerationJob,
    Keep,
    EditDraft,
    RejectDraft,
    SessionRevoke,
    SessionsRevokeAll,
    Next,
    Reveal,
    Reference,
    Skip,
    Snooze,
    SnoozeConcept,
    Bridge,
    Submit,
    ContentFeedback,
}

/// Closed action taxonomy. No free-form action labels are representable.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    AuthAccount(AuthAccountAction),
    Material(MaterialAction),
    Review(ReviewAction),
    Generation(GenerationAction),
    Machine(MachineRouteAction),
}

/// Broad action family used for fixed histogram selection.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionFamily {
    AuthAccount,
    Material,
    Review,
    Generation,
    Machine,
}

impl Action {
    /// Every v1 action, in deterministic order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::AuthAccount(AuthAccountAction::CreateAccount),
            Self::AuthAccount(AuthAccountAction::Login),
            Self::AuthAccount(AuthAccountAction::LoginVerify),
            Self::AuthAccount(AuthAccountAction::Logout),
            Self::AuthAccount(AuthAccountAction::IssueServiceSession),
            Self::AuthAccount(AuthAccountAction::AppHome),
            Self::AuthAccount(AuthAccountAction::Analytics),
            Self::AuthAccount(AuthAccountAction::SaveAccount),
            Self::AuthAccount(AuthAccountAction::ReturnNotifications),
            Self::Material(MaterialAction::ListSources),
            Self::Material(MaterialAction::CreateSource),
            Self::Material(MaterialAction::CaptureSource),
            Self::Material(MaterialAction::ArchiveSource),
            Self::Material(MaterialAction::CreateProjectDeck),
            Self::Material(MaterialAction::InvalidateProjectDeck),
            Self::Review(ReviewAction::Start),
            Self::Review(ReviewAction::Next),
            Self::Review(ReviewAction::Reveal),
            Self::Review(ReviewAction::Reference),
            Self::Review(ReviewAction::Skip),
            Self::Review(ReviewAction::Snooze),
            Self::Review(ReviewAction::SnoozeConcept),
            Self::Review(ReviewAction::Bridge),
            Self::Review(ReviewAction::Submit),
            Self::Review(ReviewAction::ContentFeedback),
            Self::Review(ReviewAction::Edit),
            Self::Review(ReviewAction::SaveEdit),
            Self::Review(ReviewAction::Delete),
            Self::Generation(GenerationAction::Start),
            Self::Generation(GenerationAction::Enqueue),
            Self::Generation(GenerationAction::GetJob),
            Self::Generation(GenerationAction::Keep),
            Self::Generation(GenerationAction::EditDraft),
            Self::Generation(GenerationAction::RejectDraft),
            Self::Generation(GenerationAction::Retry),
            Self::Generation(GenerationAction::DurableTerminal),
            Self::Machine(MachineRouteAction::OpenApi),
            Self::Machine(MachineRouteAction::Accounts),
            Self::Machine(MachineRouteAction::ServiceSessions),
            Self::Machine(MachineRouteAction::Sources),
            Self::Machine(MachineRouteAction::Source),
            Self::Machine(MachineRouteAction::ProjectDecks),
            Self::Machine(MachineRouteAction::ProjectDeckInvalidate),
            Self::Machine(MachineRouteAction::Generate),
            Self::Machine(MachineRouteAction::GenerationJobs),
            Self::Machine(MachineRouteAction::GenerationJob),
            Self::Machine(MachineRouteAction::Keep),
            Self::Machine(MachineRouteAction::EditDraft),
            Self::Machine(MachineRouteAction::RejectDraft),
            Self::Machine(MachineRouteAction::SessionRevoke),
            Self::Machine(MachineRouteAction::SessionsRevokeAll),
            Self::Machine(MachineRouteAction::Next),
            Self::Machine(MachineRouteAction::Reveal),
            Self::Machine(MachineRouteAction::Reference),
            Self::Machine(MachineRouteAction::Skip),
            Self::Machine(MachineRouteAction::Snooze),
            Self::Machine(MachineRouteAction::SnoozeConcept),
            Self::Machine(MachineRouteAction::Bridge),
            Self::Machine(MachineRouteAction::Submit),
            Self::Machine(MachineRouteAction::ContentFeedback),
        ]
    }

    #[must_use]
    pub const fn family(self) -> ActionFamily {
        match self {
            Self::AuthAccount(_) => ActionFamily::AuthAccount,
            Self::Material(_) => ActionFamily::Material,
            Self::Review(_) => ActionFamily::Review,
            Self::Generation(_) => ActionFamily::Generation,
            Self::Machine(_) => ActionFamily::Machine,
        }
    }

    /// Machine-only routes are distinguishable without retaining a route.
    #[must_use]
    pub const fn is_machine_route_only(self) -> bool {
        matches!(self, Self::Machine(_))
    }

    /// Generation routes use the longer fixed bucket set.
    #[must_use]
    pub const fn histogram_kind(self) -> HistogramKind {
        match self {
            Self::Generation(_)
            | Self::Machine(
                MachineRouteAction::Generate
                | MachineRouteAction::GenerationJobs
                | MachineRouteAction::GenerationJob,
            ) => HistogramKind::Generation,
            _ => HistogramKind::RequestUi,
        }
    }
}

/// Completion phase with fixed semantics. These are content-free markers.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionPhase {
    ImmediateAck,
    VisibleAfterTwoAnimationFrames,
    DurableGenerationTerminal,
    SseVisibleTerminal,
}

impl CompletionPhase {
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::ImmediateAck,
            Self::VisibleAfterTwoAnimationFrames,
            Self::DurableGenerationTerminal,
            Self::SseVisibleTerminal,
        ]
    }

    #[must_use]
    const fn requires_generation(self) -> bool {
        matches!(
            self,
            Self::DurableGenerationTerminal | Self::SseVisibleTerminal
        )
    }
}

/// Fixed outcome class; status strings are deliberately not accepted.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Succeeded,
    ClientRejected,
    ServerFailed,
    TimedOut,
    Cancelled,
}

impl Outcome {
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Succeeded,
            Self::ClientRejected,
            Self::ServerFailed,
            Self::TimedOut,
            Self::Cancelled,
        ]
    }
}

/// Namespace split prevents browser, server, and job observations from sharing
/// an accidental series.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Namespace {
    Browser,
    Server,
    Job,
}

impl Namespace {
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Browser, Self::Server, Self::Job]
    }
}

/// Navigation mode keeps full-page and progressive/browser-disabled behavior
/// distinguishable without storing a URL or browser payload.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Navigation {
    FullPage,
    JavascriptEnhanced,
    JavascriptDisabled,
    Machine,
}

impl Navigation {
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::FullPage,
            Self::JavascriptEnhanced,
            Self::JavascriptDisabled,
            Self::Machine,
        ]
    }
}

/// Coarse viewport class; exact dimensions are not allowed in the contract.
///
/// Browser code uses [`Self::Unknown`] when it cannot observe a viewport
/// reliably, including JavaScript-disabled navigation. Server and job markers
/// always use [`Self::NotApplicable`] rather than inferring viewport from
/// request metadata.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Viewport {
    Unknown,
    Mobile,
    Tablet,
    Desktop,
    NotApplicable,
}

impl Viewport {
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Unknown,
            Self::Mobile,
            Self::Tablet,
            Self::Desktop,
            Self::NotApplicable,
        ]
    }
}

/// Explicit route classes that are excluded from performance series.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExclusionClass {
    Included,
    Health,
    Readiness,
    StaticAsset,
    TelemetryIngest,
    SseStream,
}

impl ExclusionClass {
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Included,
            Self::Health,
            Self::Readiness,
            Self::StaticAsset,
            Self::TelemetryIngest,
            Self::SseStream,
        ]
    }

    #[must_use]
    pub const fn is_excluded(self) -> bool {
        !matches!(self, Self::Included)
    }
}

/// Inputs that must never enter a performance marker or aggregate.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExcludedField {
    RawPath,
    RawQuery,
    Referrer,
    UserAgent,
    IpAddress,
    LearnerContent,
    Email,
    AccountId,
    SessionId,
    SourceId,
    ReviewUnitId,
    JobId,
    Token,
    ModelName,
    ProviderName,
    FreeformError,
    ClientRoute,
    ClientStatus,
    ServerLabel,
}

impl ExcludedField {
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::RawPath,
            Self::RawQuery,
            Self::Referrer,
            Self::UserAgent,
            Self::IpAddress,
            Self::LearnerContent,
            Self::Email,
            Self::AccountId,
            Self::SessionId,
            Self::SourceId,
            Self::ReviewUnitId,
            Self::JobId,
            Self::Token,
            Self::ModelName,
            Self::ProviderName,
            Self::FreeformError,
            Self::ClientRoute,
            Self::ClientStatus,
            Self::ServerLabel,
        ]
    }
}

/// The two fixed histogram families in v1.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistogramKind {
    RequestUi,
    Generation,
}

impl HistogramKind {
    /// Inclusive upper bounds for the complete fixed bucket set. Durations
    /// above the final bound are rejected as invalid rather than placed in an
    /// implicit overflow bucket.
    #[must_use]
    pub const fn upper_bounds_ms(self) -> &'static [u64] {
        match self {
            Self::RequestUi => &[
                10, 25, 50, 100, 200, 300, 500, 750, 1_000, 1_500, 2_000, 3_000, 5_000, 10_000,
                30_000, 60_000,
            ],
            Self::Generation => &[
                100, 250, 500, 1_000, 2_000, 5_000, 10_000, 30_000, 60_000, 120_000, 300_000,
                600_000,
            ],
        }
    }

    #[must_use]
    pub const fn max_duration_ms(self) -> u64 {
        match self {
            Self::RequestUi => REQUEST_UI_MAX_DURATION_MS,
            Self::Generation => GENERATION_MAX_DURATION_MS,
        }
    }

    #[must_use]
    pub const fn bucket_count(self) -> usize {
        match self {
            Self::RequestUi => 16,
            Self::Generation => 12,
        }
    }
}

/// Fixed histogram counts. The count vector length is checked against kind and
/// cannot be changed to introduce arbitrary bucket labels.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "HistogramWire")]
pub struct Histogram {
    kind: HistogramKind,
    bucket_counts: Vec<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HistogramWire {
    kind: HistogramKind,
    bucket_counts: Vec<u64>,
}

impl TryFrom<HistogramWire> for Histogram {
    type Error = HistogramError;

    fn try_from(wire: HistogramWire) -> Result<Self, Self::Error> {
        let histogram = Self {
            kind: wire.kind,
            bucket_counts: wire.bucket_counts,
        };
        histogram.validate()?;
        Ok(histogram)
    }
}

impl Histogram {
    #[must_use]
    pub fn new(kind: HistogramKind) -> Self {
        Self {
            kind,
            bucket_counts: vec![0; kind.bucket_count()],
        }
    }

    /// Decode fixed counts from a trusted adapter, rejecting arbitrary shapes.
    ///
    /// # Errors
    ///
    /// Returns [`HistogramError`] when the fixed bucket shape is invalid.
    pub fn from_counts(
        kind: HistogramKind,
        bucket_counts: Vec<u64>,
    ) -> Result<Self, HistogramError> {
        let histogram = Self {
            kind,
            bucket_counts,
        };
        histogram.validate()?;
        Ok(histogram)
    }

    #[must_use]
    pub const fn kind(&self) -> HistogramKind {
        self.kind
    }

    #[must_use]
    pub fn bucket_counts(&self) -> &[u64] {
        &self.bucket_counts
    }

    #[must_use]
    pub fn total_count(&self) -> u64 {
        self.checked_total_count().unwrap_or(u64::MAX)
    }

    fn checked_total_count(&self) -> Option<u64> {
        self.bucket_counts
            .iter()
            .try_fold(0_u64, |total, value| total.checked_add(*value))
    }

    /// Record one duration into its fixed inclusive-upper-bound bucket.
    ///
    /// # Errors
    ///
    /// Returns [`HistogramError`] when the duration exceeds the fixed range or
    /// a bucket counter overflows.
    pub fn record(&mut self, duration_ms: u64) -> Result<usize, HistogramError> {
        if duration_ms > self.kind.max_duration_ms() {
            return Err(HistogramError::Duration(DurationError {
                duration_ms,
                max_duration_ms: self.kind.max_duration_ms(),
            }));
        }
        self.checked_total_count()
            .and_then(|count| count.checked_add(1))
            .ok_or(HistogramError::CounterOverflow)?;
        let index = self
            .kind
            .upper_bounds_ms()
            .iter()
            .position(|upper| duration_ms <= *upper)
            .unwrap_or(self.kind.bucket_count() - 1);
        self.bucket_counts[index] = self.bucket_counts[index]
            .checked_add(1)
            .ok_or(HistogramError::CounterOverflow)?;
        Ok(index)
    }

    fn validate(&self) -> Result<(), HistogramError> {
        if self.bucket_counts.len() != self.kind.bucket_count() {
            return Err(HistogramError::WrongBucketCount {
                expected: self.kind.bucket_count(),
                actual: self.bucket_counts.len(),
            });
        }
        if self.checked_total_count().is_none() {
            return Err(HistogramError::CounterOverflow);
        }
        Ok(())
    }

    fn merge(&mut self, other: &Self) -> Result<(), MergeError> {
        self.validate().map_err(MergeError::InvalidHistogram)?;
        other.validate().map_err(MergeError::InvalidHistogram)?;
        if self.kind != other.kind {
            return Err(MergeError::HistogramKindMismatch);
        }
        for (left, right) in self.bucket_counts.iter().zip(&other.bucket_counts) {
            left.checked_add(*right)
                .ok_or(MergeError::CounterOverflow)?;
        }
        for (left, right) in self.bucket_counts.iter_mut().zip(&other.bucket_counts) {
            *left += *right;
        }
        Ok(())
    }
}

/// A content-free marker for one completion phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "CompletionMarkerWire")]
pub struct CompletionMarker {
    schema: SchemaVersion,
    action: Action,
    phase: CompletionPhase,
    outcome: Outcome,
    namespace: Namespace,
    navigation: Navigation,
    viewport: Viewport,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletionMarkerWire {
    schema: SchemaVersion,
    action: Action,
    phase: CompletionPhase,
    outcome: Outcome,
    namespace: Namespace,
    navigation: Navigation,
    viewport: Viewport,
}

impl TryFrom<CompletionMarkerWire> for CompletionMarker {
    type Error = MarkerError;

    fn try_from(wire: CompletionMarkerWire) -> Result<Self, Self::Error> {
        let marker = Self {
            schema: wire.schema,
            action: wire.action,
            phase: wire.phase,
            outcome: wire.outcome,
            namespace: wire.namespace,
            navigation: wire.navigation,
            viewport: wire.viewport,
        };
        marker.validate()?;
        Ok(marker)
    }
}

impl CompletionMarker {
    fn new(
        namespace: Namespace,
        action: Action,
        phase: CompletionPhase,
        outcome: Outcome,
        navigation: Navigation,
        viewport: Viewport,
    ) -> Result<Self, MarkerError> {
        let marker = Self {
            schema: SchemaVersion::V1,
            action,
            phase,
            outcome,
            namespace,
            navigation,
            viewport,
        };
        marker.validate()?;
        Ok(marker)
    }

    /// Construct a browser marker. Clients cannot provide route, status, or
    /// server labels; only the closed dimensions below are accepted.
    ///
    /// # Errors
    ///
    /// Returns [`MarkerError`] when the dimensions do not describe a valid
    /// browser observation.
    pub fn browser(
        action: Action,
        phase: CompletionPhase,
        outcome: Outcome,
        navigation: Navigation,
        viewport: Viewport,
    ) -> Result<Self, MarkerError> {
        Self::new(
            Namespace::Browser,
            action,
            phase,
            outcome,
            navigation,
            viewport,
        )
    }

    /// Construct a server marker with fixed server-only dimensions.
    ///
    /// # Errors
    ///
    /// Returns [`MarkerError`] when the dimensions do not describe a valid
    /// server observation.
    pub fn server(
        action: Action,
        phase: CompletionPhase,
        outcome: Outcome,
    ) -> Result<Self, MarkerError> {
        Self::new(
            Namespace::Server,
            action,
            phase,
            outcome,
            Navigation::Machine,
            Viewport::NotApplicable,
        )
    }

    /// Construct a generation-job marker with fixed job-only dimensions.
    ///
    /// # Errors
    ///
    /// Returns [`MarkerError`] when the dimensions do not describe a valid
    /// generation-job observation.
    pub fn job(
        action: Action,
        phase: CompletionPhase,
        outcome: Outcome,
    ) -> Result<Self, MarkerError> {
        Self::new(
            Namespace::Job,
            action,
            phase,
            outcome,
            Navigation::Machine,
            Viewport::NotApplicable,
        )
    }

    fn validate(self) -> Result<(), MarkerError> {
        if self.namespace == Namespace::Browser && self.navigation == Navigation::Machine {
            return Err(MarkerError::BrowserCannotUseMachineNavigation);
        }
        if self.namespace != Namespace::Browser && self.navigation != Navigation::Machine {
            return Err(MarkerError::NonBrowserRequiresMachineNavigation);
        }
        if self.namespace == Namespace::Browser && self.viewport == Viewport::NotApplicable {
            return Err(MarkerError::BrowserRequiresViewport);
        }
        if self.navigation == Navigation::JavascriptDisabled && self.viewport != Viewport::Unknown {
            return Err(MarkerError::JavascriptDisabledRequiresUnknownViewport);
        }
        if self.navigation == Navigation::JavascriptDisabled
            && matches!(
                self.phase,
                CompletionPhase::VisibleAfterTwoAnimationFrames
                    | CompletionPhase::SseVisibleTerminal
            )
        {
            return Err(MarkerError::JavascriptDisabledCannotUseVisiblePhase);
        }
        if self.namespace != Namespace::Browser && self.viewport != Viewport::NotApplicable {
            return Err(MarkerError::NonBrowserViewportNotApplicable);
        }
        if self.action.is_machine_route_only()
            && (self.namespace == Namespace::Browser || self.navigation != Navigation::Machine)
        {
            return Err(MarkerError::MachineActionRequiresMachineRoute);
        }
        if self.phase.requires_generation()
            && !matches!(self.action.family(), ActionFamily::Generation)
            && !matches!(
                self.action,
                Action::Machine(
                    MachineRouteAction::Generate
                        | MachineRouteAction::GenerationJobs
                        | MachineRouteAction::GenerationJob
                )
            )
        {
            return Err(MarkerError::GenerationPhaseForNonGenerationAction);
        }
        if matches!(self.phase, CompletionPhase::VisibleAfterTwoAnimationFrames)
            && self.namespace != Namespace::Browser
        {
            return Err(MarkerError::VisiblePhaseRequiresBrowser);
        }
        if matches!(self.phase, CompletionPhase::SseVisibleTerminal)
            && self.namespace != Namespace::Browser
        {
            return Err(MarkerError::SsePhaseRequiresBrowser);
        }
        Ok(())
    }

    #[must_use]
    pub const fn schema(&self) -> SchemaVersion {
        self.schema
    }

    #[must_use]
    pub const fn action(&self) -> Action {
        self.action
    }

    #[must_use]
    pub const fn phase(&self) -> CompletionPhase {
        self.phase
    }

    #[must_use]
    pub const fn outcome(&self) -> Outcome {
        self.outcome
    }

    #[must_use]
    pub const fn namespace(&self) -> Namespace {
        self.namespace
    }

    #[must_use]
    pub const fn navigation(&self) -> Navigation {
        self.navigation
    }

    #[must_use]
    pub const fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// Convert the marker and duration into a content-free observation.
    ///
    /// # Errors
    ///
    /// Returns [`ObservationError`] when the marker or duration is invalid.
    pub fn observation(&self, duration_ms: u64) -> Result<Observation, ObservationError> {
        Observation::new(*self, duration_ms)
    }
}

/// One marker plus bounded duration, with no free-form payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "ObservationWire")]
pub struct Observation {
    marker: CompletionMarker,
    duration_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservationWire {
    marker: CompletionMarker,
    duration_ms: u64,
}

impl TryFrom<ObservationWire> for Observation {
    type Error = ObservationError;

    fn try_from(wire: ObservationWire) -> Result<Self, Self::Error> {
        Self::new(wire.marker, wire.duration_ms)
    }
}

impl Observation {
    /// Construct a bounded, content-free observation.
    ///
    /// # Errors
    ///
    /// Returns [`ObservationError`] when the marker or duration is invalid.
    pub fn new(marker: CompletionMarker, duration_ms: u64) -> Result<Self, ObservationError> {
        marker.validate().map_err(ObservationError::Marker)?;
        let max_duration_ms = marker.action.histogram_kind().max_duration_ms();
        if duration_ms > max_duration_ms {
            return Err(ObservationError::Duration(DurationError {
                duration_ms,
                max_duration_ms,
            }));
        }
        Ok(Self {
            marker,
            duration_ms,
        })
    }

    #[must_use]
    pub const fn marker(&self) -> CompletionMarker {
        self.marker
    }

    #[must_use]
    pub const fn duration_ms(&self) -> u64 {
        self.duration_ms
    }
}

/// Nonzero window and boot metadata are bounded numeric values, never user IDs.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    start_minute: u64,
}

impl Window {
    #[must_use]
    pub const fn new(start_minute: u64) -> Self {
        Self { start_minute }
    }

    #[must_use]
    pub const fn start_minute(self) -> u64 {
        self.start_minute
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "BootWire")]
pub struct Boot {
    sequence: u64,
    merged: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BootWire {
    sequence: u64,
    merged: bool,
}

impl TryFrom<BootWire> for Boot {
    type Error = &'static str;

    fn try_from(wire: BootWire) -> Result<Self, Self::Error> {
        if wire.merged && wire.sequence != 0 {
            return Err("merged boot provenance requires sequence zero");
        }
        Ok(Self {
            sequence: wire.sequence,
            merged: wire.merged,
        })
    }
}

impl Boot {
    #[must_use]
    pub const fn new(sequence: u64) -> Self {
        Self {
            sequence,
            merged: false,
        }
    }

    const fn merged() -> Self {
        Self {
            sequence: 0,
            merged: true,
        }
    }

    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }

    /// True when this aggregate combines observations from more than one boot.
    #[must_use]
    pub const fn is_merged(self) -> bool {
        self.merged
    }
}

/// Mergeable fixed-series aggregate. Metadata and all counters are explicit so
/// adapters can merge two instances deterministically without labels.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "SnapshotWire")]
pub struct Snapshot {
    schema: SchemaVersion,
    marker: CompletionMarker,
    histogram: Histogram,
    count: u64,
    sum_ms: u64,
    max_ms: u64,
    sample: u64,
    #[serde(rename = "drop")]
    dropped: u64,
    invalid: u64,
    window: Window,
    boot: Boot,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotWire {
    schema: SchemaVersion,
    marker: CompletionMarker,
    histogram: Histogram,
    count: u64,
    sum_ms: u64,
    max_ms: u64,
    sample: u64,
    #[serde(rename = "drop")]
    dropped: u64,
    invalid: u64,
    window: Window,
    boot: Boot,
}

impl TryFrom<SnapshotWire> for Snapshot {
    type Error = SnapshotError;

    fn try_from(wire: SnapshotWire) -> Result<Self, Self::Error> {
        let snapshot = Self {
            schema: wire.schema,
            marker: wire.marker,
            histogram: wire.histogram,
            count: wire.count,
            sum_ms: wire.sum_ms,
            max_ms: wire.max_ms,
            sample: wire.sample,
            dropped: wire.dropped,
            invalid: wire.invalid,
            window: wire.window,
            boot: wire.boot,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }
}

impl Snapshot {
    /// Construct an empty aggregate for one marker, window, and boot.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotError`] when the marker cannot form a valid v1
    /// aggregate.
    pub fn new(
        marker: CompletionMarker,
        window: Window,
        boot: Boot,
    ) -> Result<Self, SnapshotError> {
        marker.validate().map_err(SnapshotError::Marker)?;
        let snapshot = Self {
            schema: SchemaVersion::V1,
            histogram: Histogram::new(marker.action.histogram_kind()),
            marker,
            count: 0,
            sum_ms: 0,
            max_ms: 0,
            sample: 0,
            dropped: 0,
            invalid: 0,
            window,
            boot,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    /// Decode and validate the closed JSON snapshot shape at an adapter edge.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotDecodeError`] when JSON is malformed or describes an
    /// unreachable v1 aggregate.
    pub fn decode_json(bytes: &[u8]) -> Result<Self, SnapshotDecodeError> {
        let wire: SnapshotWire =
            serde_json::from_slice(bytes).map_err(SnapshotDecodeError::Json)?;
        Self::try_from(wire).map_err(SnapshotDecodeError::Invalid)
    }

    /// Add one already-validated content-free observation.
    ///
    /// # Errors
    ///
    /// Returns [`RecordError`] when the observation belongs to another series
    /// or a counter would overflow.
    pub fn record(&mut self, observation: Observation) -> Result<(), RecordError> {
        if observation.marker != self.marker {
            return Err(RecordError::MarkerMismatch);
        }
        self.record_duration(observation.duration_ms)
    }

    /// Add one bounded duration and increment count/sum/max/sample.
    ///
    /// # Errors
    ///
    /// Returns [`RecordError`] when the duration exceeds the fixed range or a
    /// counter would overflow.
    pub fn record_duration(&mut self, duration_ms: u64) -> Result<(), RecordError> {
        let max_duration_ms = self.marker.action.histogram_kind().max_duration_ms();
        if duration_ms > max_duration_ms {
            return Err(RecordError::Duration(DurationError {
                duration_ms,
                max_duration_ms,
            }));
        }
        let bucket_index = self
            .histogram
            .kind
            .upper_bounds_ms()
            .iter()
            .position(|upper| duration_ms <= *upper)
            .unwrap_or(self.histogram.kind.bucket_count() - 1);
        if self.histogram.bucket_counts[bucket_index]
            .checked_add(1)
            .is_none()
        {
            return Err(RecordError::CounterOverflow);
        }
        let count = self
            .count
            .checked_add(1)
            .ok_or(RecordError::CounterOverflow)?;
        let sum_ms = self
            .sum_ms
            .checked_add(duration_ms)
            .ok_or(RecordError::CounterOverflow)?;
        let sample = self
            .sample
            .checked_add(1)
            .ok_or(RecordError::CounterOverflow)?;
        self.histogram.bucket_counts[bucket_index] += 1;
        self.count = count;
        self.sum_ms = sum_ms;
        self.max_ms = self.max_ms.max(duration_ms);
        self.sample = sample;
        Ok(())
    }

    /// Record a dropped observation without retaining any value.
    ///
    /// # Errors
    ///
    /// Returns [`RecordError`] when the drop counter would overflow.
    pub fn record_drop(&mut self) -> Result<(), RecordError> {
        self.dropped = self
            .dropped
            .checked_add(1)
            .ok_or(RecordError::CounterOverflow)?;
        Ok(())
    }

    /// Record an invalid observation after an adapter rejects it.
    ///
    /// # Errors
    ///
    /// Returns [`RecordError`] when the invalid counter would overflow.
    pub fn record_invalid(&mut self) -> Result<(), RecordError> {
        self.invalid = self
            .invalid
            .checked_add(1)
            .ok_or(RecordError::CounterOverflow)?;
        Ok(())
    }

    /// Merge an exact same-series snapshot. Window must match; distinct boot
    /// values are accepted and collapse to explicit merged boot provenance,
    /// while different rotations remain rejected.
    ///
    /// # Errors
    ///
    /// Returns [`MergeError`] when either snapshot is invalid, the series are
    /// incompatible, or a counter would overflow.
    pub fn merge(&mut self, other: &Self) -> Result<(), MergeError> {
        self.validate().map_err(MergeError::InvalidSnapshot)?;
        other.validate().map_err(MergeError::InvalidSnapshot)?;
        if self.schema != other.schema {
            return Err(MergeError::SchemaMismatch);
        }
        if self.marker != other.marker {
            return Err(MergeError::SeriesMismatch);
        }
        if self.window != other.window {
            return Err(MergeError::MetadataMismatch);
        }
        let count = self
            .count
            .checked_add(other.count)
            .ok_or(MergeError::CounterOverflow)?;
        let sum_ms = self
            .sum_ms
            .checked_add(other.sum_ms)
            .ok_or(MergeError::CounterOverflow)?;
        let sample = self
            .sample
            .checked_add(other.sample)
            .ok_or(MergeError::CounterOverflow)?;
        let dropped = self
            .dropped
            .checked_add(other.dropped)
            .ok_or(MergeError::CounterOverflow)?;
        let invalid = self
            .invalid
            .checked_add(other.invalid)
            .ok_or(MergeError::CounterOverflow)?;
        self.histogram.merge(&other.histogram)?;
        self.count = count;
        self.sum_ms = sum_ms;
        self.max_ms = self.max_ms.max(other.max_ms);
        self.sample = sample;
        self.dropped = dropped;
        self.invalid = invalid;
        if self.boot != other.boot {
            self.boot = Boot::merged();
        }
        Ok(())
    }

    /// Validate all structural and privacy invariants.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotError`] when the snapshot is not a reachable v1
    /// aggregate.
    pub fn validate(&self) -> Result<(), SnapshotError> {
        if self.schema != SchemaVersion::V1 {
            return Err(SnapshotError::UnsupportedSchema);
        }
        self.marker.validate().map_err(SnapshotError::Marker)?;
        self.histogram
            .validate()
            .map_err(SnapshotError::Histogram)?;
        if self.histogram.kind != self.marker.action.histogram_kind() {
            return Err(SnapshotError::HistogramKindMismatch);
        }
        if self.histogram.checked_total_count() != Some(self.count) {
            return Err(SnapshotError::CountMismatch);
        }
        if self.sample != self.count {
            return Err(SnapshotError::SampleCountMismatch);
        }
        let bounds = self.histogram.kind.upper_bounds_ms();
        let mut minimum_sum = 0_u128;
        let mut maximum_sum = 0_u128;
        let mut highest_nonempty_bucket = None;
        for (index, bucket_count) in self.histogram.bucket_counts.iter().copied().enumerate() {
            if bucket_count == 0 {
                continue;
            }
            let lower_ms = if index == 0 { 0 } else { bounds[index - 1] + 1 };
            let upper_ms = bounds[index];
            minimum_sum = minimum_sum
                .checked_add(u128::from(bucket_count) * u128::from(lower_ms))
                .ok_or(SnapshotError::DurationCountersInvalid)?;
            maximum_sum = maximum_sum
                .checked_add(u128::from(bucket_count) * u128::from(upper_ms))
                .ok_or(SnapshotError::DurationCountersInvalid)?;
            highest_nonempty_bucket = Some((lower_ms, upper_ms, bucket_count));
        }
        let max_duration_ms = self.marker.action.histogram_kind().max_duration_ms();
        if self.count == 0 {
            if self.sum_ms != 0 || self.max_ms != 0 {
                return Err(SnapshotError::EmptyCountersNonZero);
            }
        } else {
            let Some((maximum_lower_ms, maximum_upper_ms, maximum_bucket_count)) =
                highest_nonempty_bucket
            else {
                return Err(SnapshotError::DurationCountersInvalid);
            };
            let max_ms = u128::from(self.max_ms);
            let sum_ms = u128::from(self.sum_ms);
            let maximum_bucket_lower = u128::from(maximum_lower_ms);
            let maximum_bucket_upper = u128::from(maximum_upper_ms);
            let maximum_bucket_count = u128::from(maximum_bucket_count);
            let minimum_with_observed_max = minimum_sum
                .checked_add(max_ms.saturating_sub(maximum_bucket_lower))
                .ok_or(SnapshotError::DurationCountersInvalid)?;
            let maximum_with_observed_max = maximum_sum
                .checked_sub(maximum_bucket_count * maximum_bucket_upper)
                .and_then(|sum| sum.checked_add(maximum_bucket_count * max_ms))
                .ok_or(SnapshotError::DurationCountersInvalid)?;
            let maximum_from_observed_max = u128::from(self.count) * max_ms;
            if self.max_ms > max_duration_ms
                || self.max_ms > self.sum_ms
                || self.max_ms < maximum_lower_ms
                || self.max_ms > maximum_upper_ms
                || sum_ms < minimum_with_observed_max
                || sum_ms > maximum_with_observed_max
                || sum_ms > maximum_from_observed_max
            {
                return Err(SnapshotError::DurationCountersInvalid);
            }
        }
        Ok(())
    }

    #[must_use]
    pub const fn marker(&self) -> CompletionMarker {
        self.marker
    }

    #[must_use]
    pub const fn histogram_kind(&self) -> HistogramKind {
        self.histogram.kind
    }

    #[must_use]
    pub fn bucket_counts(&self) -> &[u64] {
        self.histogram.bucket_counts()
    }

    #[must_use]
    pub const fn count(&self) -> u64 {
        self.count
    }

    #[must_use]
    pub const fn sum_ms(&self) -> u64 {
        self.sum_ms
    }

    #[must_use]
    pub const fn max_ms(&self) -> u64 {
        self.max_ms
    }

    #[must_use]
    pub const fn sample(&self) -> u64 {
        self.sample
    }

    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    #[must_use]
    pub const fn invalid(&self) -> u64 {
        self.invalid
    }

    #[must_use]
    pub const fn window(&self) -> Window {
        self.window
    }

    #[must_use]
    pub const fn boot(&self) -> Boot {
        self.boot
    }

    /// JSON payload size after structural validation.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotError`] when the aggregate is invalid or cannot be
    /// serialized.
    pub fn encoded_len(&self) -> Result<usize, SnapshotError> {
        self.validate()?;
        serde_json::to_vec(self)
            .map(|encoded| encoded.len())
            .map_err(|_| SnapshotError::Serialization)
    }

    /// Return an inclusive duration range containing the requested percentile.
    ///
    /// # Errors
    ///
    /// Returns [`PercentileError`] when the percentile is outside 0 through
    /// 100 or the histogram is empty or inconsistent.
    pub fn percentile_bounds(&self, percentile: u8) -> Result<PercentileBounds, PercentileError> {
        self.histogram.percentile_bounds(self.count, percentile)
    }
}

impl Histogram {
    fn percentile_bounds(
        &self,
        count: u64,
        percentile: u8,
    ) -> Result<PercentileBounds, PercentileError> {
        self.validate().map_err(PercentileError::Histogram)?;
        if percentile > 100 {
            return Err(PercentileError::OutOfRange(percentile));
        }
        if count == 0 || self.checked_total_count() != Some(count) {
            return Err(PercentileError::EmptyOrInconsistent);
        }
        let rank = if percentile == 0 {
            1
        } else {
            (u128::from(percentile) * u128::from(count)).div_ceil(100)
        };
        let mut cumulative = 0_u128;
        let bounds = self.kind.upper_bounds_ms();
        for (index, bucket_count) in self.bucket_counts.iter().enumerate() {
            cumulative = cumulative
                .checked_add(u128::from(*bucket_count))
                .ok_or(PercentileError::CounterOverflow)?;
            if cumulative >= rank {
                let lower_ms = if index == 0 { 0 } else { bounds[index - 1] + 1 };
                let upper_ms = bounds
                    .get(index)
                    .copied()
                    .unwrap_or(self.kind.max_duration_ms());
                return Ok(PercentileBounds { lower_ms, upper_ms });
            }
        }
        Err(PercentileError::EmptyOrInconsistent)
    }
}

/// Inclusive range guaranteed to contain a percentile based on bucket data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PercentileBounds {
    pub lower_ms: u64,
    pub upper_ms: u64,
}

/// Calculated contract budget, suitable for deterministic CI assertions.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetReport {
    pub series_cardinality: usize,
    pub max_payload_bytes: usize,
    pub aggregate_batches_per_minute: u8,
    pub cardinality_within_limit: bool,
    pub payload_within_limit: bool,
    pub rate_within_limit: bool,
}

impl BudgetReport {
    #[must_use]
    pub const fn within_limits(self) -> bool {
        self.cardinality_within_limit && self.payload_within_limit && self.rate_within_limit
    }
}

/// Exact worst-case count of representable closed-series combinations.
/// Excluded classes are not dimensions, and invalid machine/browser or
/// terminal-phase combinations are not counted.
#[must_use]
pub fn worst_case_series_cardinality() -> usize {
    let mut count = 0;
    for &action in Action::all() {
        for &phase in CompletionPhase::all() {
            for &outcome in Outcome::all() {
                for &namespace in Namespace::all() {
                    for &navigation in Navigation::all() {
                        for &viewport in Viewport::all() {
                            let marker = CompletionMarker {
                                schema: SchemaVersion::V1,
                                action,
                                phase,
                                outcome,
                                namespace,
                                navigation,
                                viewport,
                            };
                            if marker.validate().is_ok() {
                                count += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    count
}

/// Serialize every fixed-shape family and closed enum spelling using
/// worst-case integer widths. This intentionally checks `RequestUi` as well as
/// `Generation` so adding a bucket cannot silently undercount the budget.
#[must_use]
pub fn worst_case_payload_bytes() -> usize {
    let mut maximum = 0;
    for &action in Action::all() {
        let histogram_kind = action.histogram_kind();
        for &phase in CompletionPhase::all() {
            for &outcome in Outcome::all() {
                for &namespace in Namespace::all() {
                    for &navigation in Navigation::all() {
                        for &viewport in Viewport::all() {
                            let marker = CompletionMarker {
                                schema: SchemaVersion::V1,
                                action,
                                phase,
                                outcome,
                                namespace,
                                navigation,
                                viewport,
                            };
                            let snapshot = Snapshot {
                                schema: SchemaVersion::V1,
                                marker,
                                histogram: Histogram {
                                    kind: histogram_kind,
                                    bucket_counts: vec![u64::MAX; histogram_kind.bucket_count()],
                                },
                                count: u64::MAX,
                                sum_ms: u64::MAX,
                                max_ms: u64::MAX,
                                sample: u64::MAX,
                                dropped: u64::MAX,
                                invalid: u64::MAX,
                                window: Window::new(u64::MAX),
                                boot: Boot::new(u64::MAX),
                            };
                            let length = serde_json::to_vec(&snapshot)
                                .map_or(usize::MAX, |encoded| encoded.len());
                            maximum = maximum.max(length);
                        }
                    }
                }
            }
        }
    }
    maximum
}

/// Compute all fixed v1 budgets without sending telemetry.
#[must_use]
pub fn budget_report() -> BudgetReport {
    let max_payload_bytes = worst_case_payload_bytes();
    let series_cardinality = worst_case_series_cardinality();
    BudgetReport {
        series_cardinality,
        max_payload_bytes,
        aggregate_batches_per_minute: MAX_AGGREGATE_BATCHES_PER_MINUTE,
        cardinality_within_limit: series_cardinality <= MAX_SERIES_CARDINALITY,
        payload_within_limit: max_payload_bytes < MAX_PAYLOAD_BYTES,
        rate_within_limit: MAX_AGGREGATE_BATCHES_PER_MINUTE <= 3,
    }
}

/// Marker validation failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarkerError {
    GenerationPhaseForNonGenerationAction,
    VisiblePhaseRequiresBrowser,
    SsePhaseRequiresBrowser,
    BrowserCannotUseMachineNavigation,
    NonBrowserRequiresMachineNavigation,
    MachineActionRequiresMachineRoute,
    BrowserRequiresViewport,
    NonBrowserViewportNotApplicable,
    JavascriptDisabledRequiresUnknownViewport,
    JavascriptDisabledCannotUseVisiblePhase,
}

impl fmt::Display for MarkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::GenerationPhaseForNonGenerationAction => {
                "generation terminal phase requires a generation action"
            }
            Self::VisiblePhaseRequiresBrowser => {
                "visible-after-two-frames requires browser namespace"
            }
            Self::SsePhaseRequiresBrowser => "SSE-visible terminal requires browser namespace",
            Self::BrowserCannotUseMachineNavigation => {
                "browser markers cannot use machine navigation"
            }
            Self::NonBrowserRequiresMachineNavigation => {
                "server and job markers require machine navigation"
            }
            Self::MachineActionRequiresMachineRoute => {
                "machine-only action requires a machine route"
            }
            Self::BrowserRequiresViewport => {
                "browser markers require an observed or unknown viewport"
            }
            Self::NonBrowserViewportNotApplicable => {
                "server and job markers require a not-applicable viewport"
            }
            Self::JavascriptDisabledRequiresUnknownViewport => {
                "JavaScript-disabled markers require an unknown viewport"
            }
            Self::JavascriptDisabledCannotUseVisiblePhase => {
                "JavaScript-disabled markers cannot use JavaScript-visible phases"
            }
        };
        f.write_str(message)
    }
}

impl std::error::Error for MarkerError {}

/// Duration bound failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DurationError {
    pub duration_ms: u64,
    pub max_duration_ms: u64,
}

impl fmt::Display for DurationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "duration {}ms exceeds {}ms",
            self.duration_ms, self.max_duration_ms
        )
    }
}

impl std::error::Error for DurationError {}

/// Fixed histogram shape failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistogramError {
    WrongBucketCount { expected: usize, actual: usize },
    Duration(DurationError),
    CounterOverflow,
}

impl fmt::Display for HistogramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongBucketCount { expected, actual } => {
                write!(f, "histogram needs {expected} buckets, got {actual}")
            }
            Self::Duration(error) => error.fmt(f),
            Self::CounterOverflow => f.write_str("histogram counter overflow"),
        }
    }
}

impl std::error::Error for HistogramError {}

/// Observation construction failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationError {
    Marker(MarkerError),
    Duration(DurationError),
}

impl fmt::Display for ObservationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Marker(error) => error.fmt(f),
            Self::Duration(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ObservationError {}

/// Snapshot structural failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotError {
    UnsupportedSchema,
    Marker(MarkerError),
    Histogram(HistogramError),
    HistogramKindMismatch,
    CountMismatch,
    SampleCountMismatch,
    EmptyCountersNonZero,
    DurationCountersInvalid,
    Serialization,
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema => f.write_str("unsupported schema version"),
            Self::Marker(error) => error.fmt(f),
            Self::Histogram(error) => error.fmt(f),
            Self::HistogramKindMismatch => f.write_str("histogram kind does not match action"),
            Self::CountMismatch => f.write_str("histogram count does not match snapshot count"),
            Self::SampleCountMismatch => f.write_str("sample does not match count"),
            Self::EmptyCountersNonZero => {
                f.write_str("empty snapshot has nonzero duration counters")
            }
            Self::DurationCountersInvalid => f.write_str("duration counters violate bounds"),
            Self::Serialization => f.write_str("snapshot serialization failed"),
        }
    }
}

impl std::error::Error for SnapshotError {}

/// Snapshot JSON decoding failure.
#[derive(Debug)]
pub enum SnapshotDecodeError {
    Json(serde_json::Error),
    Invalid(SnapshotError),
}

impl fmt::Display for SnapshotDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => error.fmt(f),
            Self::Invalid(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for SnapshotDecodeError {}

/// Record failure while preserving aggregate invariants.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordError {
    MarkerMismatch,
    Duration(DurationError),
    CounterOverflow,
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MarkerMismatch => {
                f.write_str("observation marker does not match snapshot series")
            }
            Self::Duration(error) => error.fmt(f),
            Self::CounterOverflow => f.write_str("snapshot counter overflow"),
        }
    }
}

impl std::error::Error for RecordError {}

/// Merge failure for unlike series or overflowing counters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MergeError {
    InvalidSnapshot(SnapshotError),
    InvalidHistogram(HistogramError),
    SchemaMismatch,
    SeriesMismatch,
    MetadataMismatch,
    HistogramKindMismatch,
    CounterOverflow,
}

impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSnapshot(error) => error.fmt(f),
            Self::InvalidHistogram(error) => error.fmt(f),
            Self::SchemaMismatch => f.write_str("snapshot schema versions differ"),
            Self::SeriesMismatch => f.write_str("snapshot series dimensions differ"),
            Self::MetadataMismatch => f.write_str("snapshot window differs"),
            Self::HistogramKindMismatch => f.write_str("snapshot histogram kinds differ"),
            Self::CounterOverflow => f.write_str("snapshot counter overflow"),
        }
    }
}

impl std::error::Error for MergeError {}

/// Percentile request failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PercentileError {
    OutOfRange(u8),
    EmptyOrInconsistent,
    Histogram(HistogramError),
    CounterOverflow,
}

impl fmt::Display for PercentileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange(value) => write!(f, "percentile {value} is outside 0..=100"),
            Self::EmptyOrInconsistent => f.write_str("histogram is empty or inconsistent"),
            Self::Histogram(error) => error.fmt(f),
            Self::CounterOverflow => f.write_str("percentile counter overflow"),
        }
    }
}

impl std::error::Error for PercentileError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn browser_marker(action: Action, phase: CompletionPhase) -> CompletionMarker {
        CompletionMarker::browser(
            action,
            phase,
            Outcome::Succeeded,
            Navigation::JavascriptEnhanced,
            Viewport::Desktop,
        )
        .expect("marker should be valid")
    }

    fn snapshot(action: Action, phase: CompletionPhase) -> Snapshot {
        Snapshot::new(browser_marker(action, phase), Window::new(42), Boot::new(7))
            .expect("snapshot should be valid")
    }

    #[test]
    fn closed_values_reject_unknown_actions_versions_and_labels() {
        assert!(serde_json::from_str::<Action>(r#"{"review":"future"}"#).is_err());
        assert!(serde_json::from_str::<SchemaVersion>(r#""v2""#).is_err());
        let marker = browser_marker(
            Action::Review(ReviewAction::Submit),
            CompletionPhase::ImmediateAck,
        );
        let mut json = serde_json::to_value(marker).expect("marker JSON");
        json.as_object_mut()
            .expect("object")
            .insert("route".to_owned(), serde_json::json!("/secret/path"));
        assert!(serde_json::from_value::<CompletionMarker>(json).is_err());
    }

    #[test]
    fn duration_bounds_are_fixed_per_histogram_family() {
        let request = browser_marker(
            Action::Review(ReviewAction::Submit),
            CompletionPhase::ImmediateAck,
        );
        assert!(request.observation(REQUEST_UI_MAX_DURATION_MS).is_ok());
        assert!(request.observation(REQUEST_UI_MAX_DURATION_MS + 1).is_err());

        let generation = CompletionMarker::job(
            Action::Generation(GenerationAction::DurableTerminal),
            CompletionPhase::DurableGenerationTerminal,
            Outcome::Succeeded,
        )
        .expect("generation marker");
        assert!(generation.observation(GENERATION_MAX_DURATION_MS).is_ok());
        assert!(generation
            .observation(GENERATION_MAX_DURATION_MS + 1)
            .is_err());
    }

    #[test]
    fn privacy_exclusions_are_structural_and_markers_are_content_free() {
        assert_eq!(ExcludedField::all().len(), 19);
        let marker = browser_marker(
            Action::AuthAccount(AuthAccountAction::Login),
            CompletionPhase::ImmediateAck,
        );
        let serialized = serde_json::to_string(&marker).expect("marker JSON");
        for forbidden in [
            "path",
            "query",
            "referrer",
            "user_agent",
            "ip_address",
            "email",
            "account_id",
            "session_id",
            "source_id",
            "review_unit_id",
            "job_id",
            "token",
            "model",
            "provider",
            "error",
            "route",
            "status",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "serialized marker leaked {forbidden}"
            );
        }
    }

    #[test]
    fn two_instance_merge_is_exact_for_all_counters_and_buckets() {
        let action = Action::Review(ReviewAction::Submit);
        let phase = CompletionPhase::ImmediateAck;
        let mut left = snapshot(action, phase);
        let mut right = Snapshot::new(browser_marker(action, phase), Window::new(42), Boot::new(8))
            .expect("snapshot should be valid");
        left.record_duration(10).expect("left duration");
        left.record_duration(300).expect("left duration");
        left.record_drop().expect("left drop");
        right.record_duration(11).expect("right duration");
        right.record_duration(60_000).expect("right duration");
        right.record_invalid().expect("right invalid");
        left.merge(&right).expect("same series merge");

        assert_eq!(left.count(), 4);
        assert_eq!(left.sum_ms(), 60_321);
        assert_eq!(left.max_ms(), 60_000);
        assert_eq!(left.sample(), 4);
        assert_eq!(left.dropped(), 1);
        assert_eq!(left.invalid(), 1);
        assert!(left.boot().is_merged());
        assert_eq!(left.bucket_counts().iter().sum::<u64>(), 4);
        assert_eq!(left.bucket_counts()[0], 1);
        assert_eq!(left.bucket_counts()[1], 1);
        assert_eq!(left.bucket_counts()[5], 1);
        assert_eq!(left.bucket_counts()[15], 1);
    }

    #[test]
    fn percentile_bounds_contain_observed_bucket_values() {
        let mut aggregate = snapshot(
            Action::Generation(GenerationAction::GetJob),
            CompletionPhase::DurableGenerationTerminal,
        );
        aggregate.record_duration(100).expect("p10");
        aggregate.record_duration(250).expect("p25");
        aggregate.record_duration(600_000).expect("max");
        assert_eq!(
            aggregate.percentile_bounds(0),
            Ok(PercentileBounds {
                lower_ms: 0,
                upper_ms: 100,
            })
        );
        assert_eq!(
            aggregate.percentile_bounds(50),
            Ok(PercentileBounds {
                lower_ms: 101,
                upper_ms: 250,
            })
        );
        for percentile in [90, 100] {
            assert_eq!(
                aggregate.percentile_bounds(percentile),
                Ok(PercentileBounds {
                    lower_ms: 300_001,
                    upper_ms: 600_000,
                })
            );
        }
        assert_eq!(
            aggregate.percentile_bounds(101),
            Err(PercentileError::OutOfRange(101))
        );
    }

    #[test]
    fn cardinality_payload_and_rate_budgets_are_calculated_and_bounded() {
        let report = budget_report();
        // Pinned tripwire, not a hard version gate: no live sender emits
        // `GenerationAction`/`MachineRouteAction` yet (grep the workspace —
        // only this crate's own `Action::all()` and tests reference the
        // full taxonomy), so v1's shape is still stabilizing pre-ship and
        // may move in place when it drifts from the real route set. Moved
        // 4,030 -> 4,450 here to match PR83's keep/edit/reject decision
        // split plus the new session-revoke machine routes, keeping this
        // taxonomy mirroring `V1Route` per the `MachineRouteAction` doc
        // comment. Once a real wire sender/consumer depends on v1's exact
        // shape, widen it only via a new `SchemaVersion` instead of editing
        // this baseline in place.
        assert_eq!(
            report.series_cardinality, 4_450,
            "schema v1 cardinality baseline moved: confirm this is a deliberate, \
             reviewed taxonomy change (see comment above), not an accidental budget regression"
        );
        assert!(report.series_cardinality <= MAX_SERIES_CARDINALITY);
        assert!(report.cardinality_within_limit);
        assert!(report.max_payload_bytes < MAX_PAYLOAD_BYTES);
        assert_eq!(report.aggregate_batches_per_minute, 3);
        assert!(report.within_limits());
    }

    #[test]
    fn health_readiness_static_ingest_and_sse_are_explicitly_excluded() {
        assert!(!ExclusionClass::Included.is_excluded());
        for class in [
            ExclusionClass::Health,
            ExclusionClass::Readiness,
            ExclusionClass::StaticAsset,
            ExclusionClass::TelemetryIngest,
            ExclusionClass::SseStream,
        ] {
            assert!(class.is_excluded());
        }
    }

    #[test]
    fn machine_routes_are_closed_and_generation_uses_generation_buckets() {
        let machine = Action::Machine(MachineRouteAction::GenerationJob);
        assert!(machine.is_machine_route_only());
        assert_eq!(machine.histogram_kind(), HistogramKind::Generation);
        assert_eq!(
            Action::Machine(MachineRouteAction::Submit).histogram_kind(),
            HistogramKind::RequestUi
        );
        assert!(CompletionMarker::browser(
            Action::Machine(MachineRouteAction::Accounts),
            CompletionPhase::ImmediateAck,
            Outcome::Succeeded,
            Navigation::JavascriptEnhanced,
            Viewport::Desktop,
        )
        .is_err());
        assert!(CompletionMarker::server(
            Action::Machine(MachineRouteAction::Accounts),
            CompletionPhase::ImmediateAck,
            Outcome::Succeeded,
        )
        .is_ok());
    }

    #[test]
    fn viewport_provenance_is_explicit_without_request_metadata_inference() {
        let server = CompletionMarker::server(
            Action::Review(ReviewAction::Submit),
            CompletionPhase::ImmediateAck,
            Outcome::Succeeded,
        )
        .expect("server marker");
        assert_eq!(server.viewport(), Viewport::NotApplicable);
        assert!(CompletionMarker::browser(
            Action::Review(ReviewAction::Submit),
            CompletionPhase::ImmediateAck,
            Outcome::Succeeded,
            Navigation::JavascriptEnhanced,
            Viewport::NotApplicable,
        )
        .is_err());

        let mut forged = serde_json::to_value(server).expect("server marker JSON");
        forged
            .as_object_mut()
            .expect("object")
            .insert("viewport".to_owned(), serde_json::json!("desktop"));
        assert!(serde_json::from_value::<CompletionMarker>(forged).is_err());
    }

    #[test]
    fn terminal_phase_namespace_and_action_rules_preserve_js_off_marker() {
        let js_off = CompletionMarker::browser(
            Action::Review(ReviewAction::Submit),
            CompletionPhase::ImmediateAck,
            Outcome::Succeeded,
            Navigation::JavascriptDisabled,
            Viewport::Unknown,
        )
        .expect("JS-off immediate ack remains representable");
        assert_eq!(js_off.navigation(), Navigation::JavascriptDisabled);
        assert!(CompletionMarker::browser(
            Action::Review(ReviewAction::Submit),
            CompletionPhase::ImmediateAck,
            Outcome::Succeeded,
            Navigation::JavascriptDisabled,
            Viewport::Desktop,
        )
        .is_err());
        assert!(CompletionMarker::browser(
            Action::Review(ReviewAction::Submit),
            CompletionPhase::VisibleAfterTwoAnimationFrames,
            Outcome::Succeeded,
            Navigation::JavascriptDisabled,
            Viewport::Unknown,
        )
        .is_err());
        assert!(CompletionMarker::browser(
            Action::Generation(GenerationAction::Start),
            CompletionPhase::SseVisibleTerminal,
            Outcome::Succeeded,
            Navigation::JavascriptDisabled,
            Viewport::Unknown,
        )
        .is_err());
        assert!(CompletionMarker::server(
            Action::Review(ReviewAction::Submit),
            CompletionPhase::VisibleAfterTwoAnimationFrames,
            Outcome::Succeeded,
        )
        .is_err());
        assert!(CompletionMarker::browser(
            Action::Review(ReviewAction::Submit),
            CompletionPhase::DurableGenerationTerminal,
            Outcome::Succeeded,
            Navigation::JavascriptEnhanced,
            Viewport::Desktop,
        )
        .is_err());
    }

    #[test]
    fn histogram_shape_and_total_overflow_are_rejected() {
        let histogram_json = r#"{"kind":"request_ui","bucket_counts":[0,0]}"#;
        assert!(serde_json::from_str::<Histogram>(histogram_json).is_err());

        let mut overflowing_counts = vec![0; HistogramKind::RequestUi.bucket_count()];
        overflowing_counts[0] = u64::MAX;
        overflowing_counts[1] = 1;
        assert_eq!(
            Histogram::from_counts(HistogramKind::RequestUi, overflowing_counts.clone()),
            Err(HistogramError::CounterOverflow)
        );
        assert!(serde_json::from_value::<Histogram>(serde_json::json!({
            "kind": "request_ui",
            "bucket_counts": overflowing_counts,
        }))
        .is_err());

        let mut saturated_counts = vec![0; HistogramKind::RequestUi.bucket_count()];
        saturated_counts[0] = u64::MAX;
        let mut saturated = Histogram::from_counts(HistogramKind::RequestUi, saturated_counts)
            .expect("maximum total is valid");
        assert_eq!(saturated.record(11), Err(HistogramError::CounterOverflow));
        assert_eq!(saturated.total_count(), u64::MAX);
    }

    #[test]
    fn malformed_snapshot_shape_is_rejected_at_decode_boundary() {
        let marker = browser_marker(
            Action::Review(ReviewAction::Submit),
            CompletionPhase::ImmediateAck,
        );
        let snapshot = Snapshot::new(marker, Window::new(1), Boot::new(1)).expect("snapshot");
        let mut value = serde_json::to_value(&snapshot).expect("snapshot JSON");
        value
            .as_object_mut()
            .expect("object")
            .insert("learner_content".to_owned(), serde_json::json!("secret"));
        assert!(Snapshot::decode_json(&serde_json::to_vec(&value).expect("JSON")).is_err());

        let marker_json = serde_json::to_string(&marker).expect("marker JSON");
        let oversized_observation = format!(
            r#"{{"marker":{marker_json},"duration_ms":{}}}"#,
            REQUEST_UI_MAX_DURATION_MS + 1
        );
        assert!(serde_json::from_str::<Observation>(&oversized_observation).is_err());

        let mut contradictory =
            Snapshot::new(marker, Window::new(1), Boot::new(1)).expect("snapshot");
        contradictory.record_duration(5).expect("duration");
        let mut contradictory_json = serde_json::to_value(contradictory).expect("snapshot JSON");
        contradictory_json
            .as_object_mut()
            .expect("object")
            .insert("max_ms".to_owned(), serde_json::json!(60_000));
        contradictory_json
            .as_object_mut()
            .expect("object")
            .insert("sum_ms".to_owned(), serde_json::json!(60_000));
        assert!(matches!(
            Snapshot::decode_json(
                &serde_json::to_vec(&contradictory_json).expect("contradictory JSON")
            ),
            Err(SnapshotDecodeError::Invalid(
                SnapshotError::DurationCountersInvalid
            ))
        ));

        let mut impossible_lower =
            Snapshot::new(marker, Window::new(1), Boot::new(1)).expect("snapshot");
        impossible_lower.record_duration(60_000).expect("duration");
        impossible_lower.record_duration(60_000).expect("duration");
        let mut impossible_lower_json =
            serde_json::to_value(impossible_lower).expect("snapshot JSON");
        impossible_lower_json
            .as_object_mut()
            .expect("object")
            .insert("sum_ms".to_owned(), serde_json::json!(60_002));
        assert!(Snapshot::decode_json(
            &serde_json::to_vec(&impossible_lower_json).expect("impossible lower JSON")
        )
        .is_err());

        let mut impossible_upper =
            Snapshot::new(marker, Window::new(1), Boot::new(1)).expect("snapshot");
        impossible_upper.record_duration(60_000).expect("duration");
        impossible_upper.record_duration(60_000).expect("duration");
        let mut impossible_upper_json =
            serde_json::to_value(impossible_upper).expect("snapshot JSON");
        impossible_upper_json
            .as_object_mut()
            .expect("object")
            .insert("max_ms".to_owned(), serde_json::json!(30_001));
        impossible_upper_json
            .as_object_mut()
            .expect("object")
            .insert("sum_ms".to_owned(), serde_json::json!(120_000));
        assert!(Snapshot::decode_json(
            &serde_json::to_vec(&impossible_upper_json).expect("impossible upper JSON")
        )
        .is_err());

        let mut sample_mismatch =
            Snapshot::new(marker, Window::new(1), Boot::new(1)).expect("snapshot");
        sample_mismatch.record_duration(5).expect("duration");
        let mut sample_mismatch_json =
            serde_json::to_value(sample_mismatch).expect("snapshot JSON");
        sample_mismatch_json
            .as_object_mut()
            .expect("object")
            .insert("sample".to_owned(), serde_json::json!(0));
        assert!(Snapshot::decode_json(
            &serde_json::to_vec(&sample_mismatch_json).expect("sample mismatch JSON")
        )
        .is_err());

        let mut forged_boot = serde_json::to_value(&snapshot).expect("snapshot JSON");
        forged_boot.as_object_mut().expect("object").insert(
            "boot".to_owned(),
            serde_json::json!({"sequence": 7, "merged": true}),
        );
        assert!(Snapshot::decode_json(&serde_json::to_vec(&forged_boot).expect("JSON")).is_err());
    }
}
