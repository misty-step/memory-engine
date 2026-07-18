use std::{
    collections::BTreeMap,
    fmt,
    time::{SystemTime, UNIX_EPOCH},
};

use memory_engine_performance::{
    Action, Boot, CompletionMarker, CompletionPhase, Namespace, Navigation, Observation, Outcome,
    Snapshot, Viewport, Window, MAX_PAYLOAD_BYTES,
};
use serde_json::{json, Map, Value};

pub const PERFORMANCE_EVENT_NAME: &str = "memory_engine.performance.snapshot";
pub const PERFORMANCE_BATCH_SCHEMA: &str = "memory_engine.performance_batch.v1";
pub const DEBUG_AUTHORITY: &str = "non_authoritative_debug";
// Canary stores the event `attributes` object with a hard 8,192-byte cap.
// Leave headroom for the per-attempt delivery counters added at send time.
const MAX_BATCH_ATTRIBUTES_BYTES: usize = 8_000;
const MAX_BATCH_SERIES: usize = 40;
const MAX_TIMELINE_PAGES: usize = 1_000;
const NAMESPACE_COUNT: usize = Namespace::all().len();

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeliveryAccounting {
    batches_sent: u64,
    batches_retried: u64,
    batches_dropped: u64,
    observations_dropped: u64,
    observations_invalid: u64,
    series_dropped: u64,
}

impl DeliveryAccounting {
    #[must_use]
    pub const fn batches_sent(&self) -> u64 {
        self.batches_sent
    }

    #[must_use]
    pub const fn batches_retried(&self) -> u64 {
        self.batches_retried
    }

    #[must_use]
    pub const fn batches_dropped(&self) -> u64 {
        self.batches_dropped
    }

    #[must_use]
    pub const fn observations_dropped(&self) -> u64 {
        self.observations_dropped
    }

    #[must_use]
    pub const fn observations_invalid(&self) -> u64 {
        self.observations_invalid
    }

    #[must_use]
    pub const fn series_dropped(&self) -> u64 {
        self.series_dropped
    }

    fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    fn as_value(&self) -> Value {
        json!({
            "batches_sent": self.batches_sent,
            "batches_retried": self.batches_retried,
            "batches_dropped": self.batches_dropped,
            "observations_dropped": self.observations_dropped,
            "observations_invalid": self.observations_invalid,
            "series_dropped": self.series_dropped,
        })
    }

    fn with_batch_attempt(&self, attempts: usize, sent: bool) -> Self {
        let mut accounting = self.clone();
        accounting.batches_retried = accounting
            .batches_retried
            .saturating_add(attempts.saturating_sub(1) as u64);
        if sent {
            accounting.batches_sent = accounting.batches_sent.saturating_add(1);
        } else {
            accounting.batches_dropped = accounting.batches_dropped.saturating_add(1);
        }
        accounting
    }

    fn decode(value: &Value) -> Result<Self, PerformanceError> {
        let object = exact_object(
            value,
            "delivery",
            &[
                "batches_sent",
                "batches_retried",
                "batches_dropped",
                "observations_dropped",
                "observations_invalid",
                "series_dropped",
            ],
        )?;
        Ok(Self {
            batches_sent: required_u64(object, "batches_sent")?,
            batches_retried: required_u64(object, "batches_retried")?,
            batches_dropped: required_u64(object, "batches_dropped")?,
            observations_dropped: required_u64(object, "observations_dropped")?,
            observations_invalid: required_u64(object, "observations_invalid")?,
            series_dropped: required_u64(object, "series_dropped")?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerformanceBatch {
    namespace: Namespace,
    window: Window,
    snapshots: Vec<Snapshot>,
    delivery: DeliveryAccounting,
}

impl PerformanceBatch {
    #[must_use]
    pub const fn namespace(&self) -> Namespace {
        self.namespace
    }

    #[must_use]
    pub const fn window(&self) -> Window {
        self.window
    }

    #[must_use]
    pub fn snapshots(&self) -> &[Snapshot] {
        &self.snapshots
    }

    #[must_use]
    pub const fn delivery(&self) -> &DeliveryAccounting {
        &self.delivery
    }

    pub(crate) fn attributes_for_attempt(&self, attempts: usize, sent: bool) -> Value {
        let delivery = self.delivery.with_batch_attempt(attempts, sent);
        self.attributes_with_delivery(&delivery)
    }

    fn attributes_with_delivery(&self, delivery: &DeliveryAccounting) -> Value {
        json!({
            "schema": PERFORMANCE_BATCH_SCHEMA,
            "authority": DEBUG_AUTHORITY,
            "namespace": namespace_label(self.namespace),
            "window": { "start_minute": self.window.start_minute() },
            "delivery": delivery.as_value(),
            "snapshots": self.snapshots,
        })
    }

    /// Decode and strictly validate one closed batch payload.
    ///
    /// # Errors
    ///
    /// Returns [`PerformanceError`] for unknown fields, unsupported schemas,
    /// metadata mismatches, invalid snapshots, or exceeded bounds.
    pub fn decode(value: &Value) -> Result<Self, PerformanceError> {
        if serde_json::to_vec(value)
            .map_err(|error| PerformanceError::Json(error.to_string()))?
            .len()
            > MAX_BATCH_ATTRIBUTES_BYTES
        {
            return Err(PerformanceError::Contract(
                "performance batch exceeds the fixed payload bound".to_owned(),
            ));
        }
        let object = exact_object(
            value,
            "performance batch",
            &[
                "schema",
                "authority",
                "namespace",
                "window",
                "delivery",
                "snapshots",
            ],
        )?;
        if required_str(object, "schema")? != PERFORMANCE_BATCH_SCHEMA {
            return Err(PerformanceError::Contract(
                "unsupported performance batch schema".to_owned(),
            ));
        }
        if required_str(object, "authority")? != DEBUG_AUTHORITY {
            return Err(PerformanceError::Contract(
                "performance batch authority marker is invalid".to_owned(),
            ));
        }
        let namespace = decode_namespace(required_str(object, "namespace")?)?;
        let window_object = exact_object(
            object
                .get("window")
                .ok_or_else(|| PerformanceError::Contract("missing window".to_owned()))?,
            "window",
            &["start_minute"],
        )?;
        let window = Window::new(required_u64(window_object, "start_minute")?);
        let delivery = DeliveryAccounting::decode(
            object
                .get("delivery")
                .ok_or_else(|| PerformanceError::Contract("missing delivery".to_owned()))?,
        )?;
        let snapshot_values = object
            .get("snapshots")
            .and_then(Value::as_array)
            .ok_or_else(|| PerformanceError::Contract("snapshots must be an array".to_owned()))?;
        if snapshot_values.len() > MAX_BATCH_SERIES {
            return Err(PerformanceError::Contract(
                "performance batch contains too many series".to_owned(),
            ));
        }
        let mut snapshots = Vec::with_capacity(snapshot_values.len());
        for value in snapshot_values {
            let encoded = serde_json::to_vec(value)
                .map_err(|error| PerformanceError::Json(error.to_string()))?;
            if encoded.len() >= MAX_PAYLOAD_BYTES {
                return Err(PerformanceError::Contract(
                    "embedded snapshot exceeds the v1 payload bound".to_owned(),
                ));
            }
            let snapshot = Snapshot::decode_json(&encoded)
                .map_err(|error| PerformanceError::Contract(error.to_string()))?;
            if snapshot.marker().namespace() != namespace || snapshot.window() != window {
                return Err(PerformanceError::Contract(
                    "snapshot metadata does not match its namespace batch".to_owned(),
                ));
            }
            snapshots.push(snapshot);
        }
        Ok(Self {
            namespace,
            window,
            snapshots,
            delivery,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ReadbackConfig {
    pub endpoint: String,
    pub api_key: String,
    pub service: String,
    pub window: String,
}

impl ReadbackConfig {
    /// Construct a service-scoped readback configuration.
    ///
    /// # Errors
    ///
    /// Returns [`PerformanceError::Config`] when any required value is empty.
    pub fn new(
        endpoint: impl Into<String>,
        api_key: impl Into<String>,
        service: impl Into<String>,
        window: impl Into<String>,
    ) -> Result<Self, PerformanceError> {
        let endpoint = endpoint.into().trim_end_matches('/').to_owned();
        let api_key = api_key.into();
        let service = service.into();
        let window = window.into();
        if endpoint.is_empty() || api_key.is_empty() || service.is_empty() || window.is_empty() {
            return Err(PerformanceError::Config(
                "readback endpoint, key, service, and window are required",
            ));
        }
        Ok(Self {
            endpoint,
            api_key,
            service,
            window,
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PerformanceReadback {
    snapshots: Vec<Snapshot>,
    batches: usize,
    pages: usize,
}

impl PerformanceReadback {
    #[must_use]
    pub fn snapshots(&self) -> &[Snapshot] {
        &self.snapshots
    }

    #[must_use]
    pub const fn batches(&self) -> usize {
        self.batches
    }

    #[must_use]
    pub const fn pages(&self) -> usize {
        self.pages
    }

    #[must_use]
    pub fn as_value(&self) -> Value {
        json!({
            "schema": "memory_engine.performance_readback.v1",
            "batches": self.batches,
            "pages": self.pages,
            "snapshots": self.snapshots,
        })
    }
}

#[derive(Debug)]
pub enum PerformanceError {
    Config(&'static str),
    Http(String),
    Json(String),
    Contract(String),
    Merge(String),
}

impl fmt::Display for PerformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(message) => formatter.write_str(message),
            Self::Http(message) => write!(formatter, "Canary readback failed: {message}"),
            Self::Json(message) => write!(formatter, "Canary JSON failed: {message}"),
            Self::Contract(message) => write!(formatter, "performance contract failed: {message}"),
            Self::Merge(message) => write!(formatter, "performance merge failed: {message}"),
        }
    }
}

impl std::error::Error for PerformanceError {}

/// Page service-filtered Canary timeline events and merge matching snapshots.
///
/// # Errors
///
/// Returns [`PerformanceError`] for transport failures, malformed pages,
/// invalid batches, exceeded page bounds, or incompatible aggregates.
pub fn read_performance_timeline(
    config: &ReadbackConfig,
) -> Result<PerformanceReadback, PerformanceError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(10)))
        .build()
        .into();
    let authorization = format!("Bearer {}", config.api_key);
    let url = format!("{}/api/v1/timeline", config.endpoint);
    let mut cursor: Option<String> = None;
    let mut batches = Vec::new();
    let mut pages = 0_usize;

    loop {
        if pages >= MAX_TIMELINE_PAGES {
            return Err(PerformanceError::Contract(
                "timeline pagination exceeded the fixed page bound".to_owned(),
            ));
        }
        let mut request = agent
            .get(&url)
            .header("Authorization", &authorization)
            .query("service", &config.service)
            .query("window", &config.window)
            .query("limit", "100")
            .query("event_type", "telemetry.event");
        if let Some(value) = cursor.as_deref() {
            request = request.query("cursor", value);
        }
        let mut response = request
            .call()
            .map_err(|error| PerformanceError::Http(error.to_string()))?;
        if !response.status().is_success() {
            return Err(PerformanceError::Http(format!(
                "timeline returned HTTP {}",
                response.status()
            )));
        }
        let body: Value = response
            .body_mut()
            .read_json()
            .map_err(|error| PerformanceError::Json(error.to_string()))?;
        let object = body.as_object().ok_or_else(|| {
            PerformanceError::Contract("timeline body must be an object".to_owned())
        })?;
        let events = object
            .get("events")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                PerformanceError::Contract("timeline events must be an array".to_owned())
            })?;
        for event in events {
            let Some(event_object) = event.as_object() else {
                return Err(PerformanceError::Contract(
                    "timeline event must be an object".to_owned(),
                ));
            };
            if event_object.get("signal_name").and_then(Value::as_str)
                != Some(PERFORMANCE_EVENT_NAME)
            {
                continue;
            }
            if event_object.get("service").and_then(Value::as_str) != Some(config.service.as_str())
            {
                return Err(PerformanceError::Contract(
                    "timeline returned an event outside the service authority".to_owned(),
                ));
            }
            let attributes = event_object.get("attributes").ok_or_else(|| {
                PerformanceError::Contract("performance event has no attributes".to_owned())
            })?;
            batches.push(PerformanceBatch::decode(attributes)?);
        }
        pages += 1;
        cursor = match object.get("cursor") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) if !value.is_empty() => Some(value.clone()),
            Some(_) => {
                return Err(PerformanceError::Contract(
                    "timeline cursor must be null or a non-empty string".to_owned(),
                ));
            }
        };
        if cursor.is_none() {
            break;
        }
    }

    merge_batches(batches, pages)
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SeriesKey {
    action: Action,
    phase: CompletionPhase,
    outcome: Outcome,
    namespace: Namespace,
    navigation: Navigation,
    viewport: Viewport,
}

impl From<CompletionMarker> for SeriesKey {
    fn from(marker: CompletionMarker) -> Self {
        Self {
            action: marker.action(),
            phase: marker.phase(),
            outcome: marker.outcome(),
            namespace: marker.namespace(),
            navigation: marker.navigation(),
            viewport: marker.viewport(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FleetKey {
    series: SeriesKey,
    window: Window,
}

pub(crate) struct Aggregator {
    active_minute: Option<u64>,
    closed_through: Option<u64>,
    boot: Boot,
    series: [BTreeMap<SeriesKey, Snapshot>; NAMESPACE_COUNT],
    accounting: [DeliveryAccounting; NAMESPACE_COUNT],
}

impl Aggregator {
    pub(crate) fn new() -> Self {
        Self {
            active_minute: None,
            closed_through: None,
            boot: Boot::new(boot_sequence()),
            series: std::array::from_fn(|_| BTreeMap::new()),
            accounting: std::array::from_fn(|_| DeliveryAccounting::default()),
        }
    }

    pub(crate) const fn active_minute(&self) -> Option<u64> {
        self.active_minute
    }

    pub(crate) fn record(&mut self, minute: u64, observation: Observation) {
        let namespace = observation.marker().namespace();
        let index = namespace_index(namespace);
        if self.closed_through.is_some_and(|closed| minute <= closed)
            || self.active_minute.is_some_and(|active| minute != active)
        {
            self.accounting[index].observations_invalid = self.accounting[index]
                .observations_invalid
                .saturating_add(1);
            return;
        }
        self.active_minute = Some(minute);
        let marker = observation.marker();
        let key = SeriesKey::from(marker);
        let snapshot = self.series[index].entry(key).or_insert_with(|| {
            Snapshot::new(marker, Window::new(minute), self.boot)
                .expect("validated observation must construct a snapshot")
        });
        if snapshot.record(observation).is_err() {
            self.accounting[index].observations_invalid = self.accounting[index]
                .observations_invalid
                .saturating_add(1);
        }
    }

    pub(crate) fn note_queue_drop(&mut self, namespace: Namespace, count: u64) {
        let accounting = &mut self.accounting[namespace_index(namespace)];
        accounting.observations_dropped = accounting.observations_dropped.saturating_add(count);
    }

    pub(crate) fn take_batches(&mut self) -> Vec<PerformanceBatch> {
        let Some(minute) = self.active_minute.take() else {
            return self.take_accounting_batches();
        };
        self.closed_through = Some(
            self.closed_through
                .map_or(minute, |closed| closed.max(minute)),
        );
        let mut batches = Vec::with_capacity(NAMESPACE_COUNT);
        for namespace in Namespace::all() {
            let index = namespace_index(*namespace);
            let all_snapshots = std::mem::take(&mut self.series[index]);
            let mut delivery = std::mem::take(&mut self.accounting[index]);
            let snapshots = all_snapshots
                .into_values()
                .filter_map(|snapshot| {
                    if snapshot.encoded_len().is_ok() {
                        Some(snapshot)
                    } else {
                        delivery.observations_invalid = delivery
                            .observations_invalid
                            .saturating_add(snapshot.count());
                        None
                    }
                })
                .collect();
            let mut batch = PerformanceBatch {
                namespace: *namespace,
                window: Window::new(minute),
                snapshots,
                delivery,
            };
            loop {
                let encoded_bytes =
                    serde_json::to_vec(&batch.attributes_with_delivery(&batch.delivery))
                        .map_or(usize::MAX, |encoded| encoded.len());
                if batch.snapshots.len() <= MAX_BATCH_SERIES
                    && encoded_bytes <= MAX_BATCH_ATTRIBUTES_BYTES
                {
                    break;
                }
                let Some(snapshot) = batch.snapshots.pop() else {
                    break;
                };
                batch.delivery.series_dropped = batch.delivery.series_dropped.saturating_add(1);
                batch.delivery.observations_dropped = batch
                    .delivery
                    .observations_dropped
                    .saturating_add(snapshot.count());
            }
            if !batch.snapshots.is_empty() || !batch.delivery.is_empty() {
                batches.push(batch);
            }
        }
        batches
    }

    fn take_accounting_batches(&mut self) -> Vec<PerformanceBatch> {
        let minute = current_minute();
        if self.closed_through.is_some_and(|closed| minute <= closed) {
            return Vec::new();
        }
        self.closed_through = Some(minute);
        let mut batches = Vec::new();
        for namespace in Namespace::all() {
            let index = namespace_index(*namespace);
            let delivery = std::mem::take(&mut self.accounting[index]);
            if !delivery.is_empty() {
                batches.push(PerformanceBatch {
                    namespace: *namespace,
                    window: Window::new(minute),
                    snapshots: Vec::new(),
                    delivery,
                });
            }
        }
        batches
    }

    pub(crate) fn note_batch_result(&mut self, namespace: Namespace, retries: u64, sent: bool) {
        let accounting = &mut self.accounting[namespace_index(namespace)];
        accounting.batches_retried = accounting.batches_retried.saturating_add(retries);
        if sent {
            accounting.batches_sent = accounting.batches_sent.saturating_add(1);
        } else {
            accounting.batches_dropped = accounting.batches_dropped.saturating_add(1);
        }
    }
}

fn merge_batches(
    batches: Vec<PerformanceBatch>,
    pages: usize,
) -> Result<PerformanceReadback, PerformanceError> {
    let batch_count = batches.len();
    let mut merged = BTreeMap::<FleetKey, Snapshot>::new();
    for batch in batches {
        for snapshot in batch.snapshots {
            let key = FleetKey {
                series: SeriesKey::from(snapshot.marker()),
                window: snapshot.window(),
            };
            if let Some(existing) = merged.get_mut(&key) {
                existing
                    .merge(&snapshot)
                    .map_err(|error| PerformanceError::Merge(error.to_string()))?;
            } else {
                merged.insert(key, snapshot);
            }
        }
    }
    Ok(PerformanceReadback {
        snapshots: merged.into_values().collect(),
        batches: batch_count,
        pages,
    })
}

pub(crate) fn current_minute() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() / 60)
}

fn boot_sequence() -> u64 {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    time.as_secs().rotate_left(32) ^ u64::from(time.subsec_nanos()) ^ u64::from(std::process::id())
}

pub(crate) const fn namespace_label(namespace: Namespace) -> &'static str {
    match namespace {
        Namespace::Browser => "browser",
        Namespace::Server => "server",
        Namespace::Job => "job",
    }
}

pub(crate) const fn namespace_index(namespace: Namespace) -> usize {
    match namespace {
        Namespace::Browser => 0,
        Namespace::Server => 1,
        Namespace::Job => 2,
    }
}

fn decode_namespace(value: &str) -> Result<Namespace, PerformanceError> {
    match value {
        "browser" => Ok(Namespace::Browser),
        "server" => Ok(Namespace::Server),
        "job" => Ok(Namespace::Job),
        _ => Err(PerformanceError::Contract(
            "unknown performance namespace".to_owned(),
        )),
    }
}

fn exact_object<'a>(
    value: &'a Value,
    label: &str,
    fields: &[&str],
) -> Result<&'a Map<String, Value>, PerformanceError> {
    let object = value
        .as_object()
        .ok_or_else(|| PerformanceError::Contract(format!("{label} must be an object")))?;
    if object.len() != fields.len() || object.keys().any(|key| !fields.contains(&key.as_str())) {
        return Err(PerformanceError::Contract(format!(
            "{label} fields do not match the closed schema"
        )));
    }
    Ok(object)
}

fn required_str<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a str, PerformanceError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| PerformanceError::Contract(format!("{field} must be a non-empty string")))
}

fn required_u64(object: &Map<String, Value>, field: &str) -> Result<u64, PerformanceError> {
    object
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| PerformanceError::Contract(format!("{field} must be an unsigned integer")))
}

#[cfg(test)]
mod tests {
    use memory_engine_performance::{
        Action, CompletionMarker, CompletionPhase, MachineRouteAction, Outcome,
    };

    use super::{current_minute, Aggregator};

    #[test]
    fn late_invalid_observations_are_carried_into_the_next_window_once() {
        let marker = CompletionMarker::server(
            Action::Machine(MachineRouteAction::OpenApi),
            CompletionPhase::ImmediateAck,
            Outcome::Succeeded,
        )
        .expect("marker");
        let observation = marker.observation(12).expect("observation");
        let minute = current_minute();
        let mut aggregator = Aggregator::new();

        aggregator.record(minute, observation);
        assert_eq!(aggregator.take_batches().len(), 1);
        aggregator.record(minute, observation);
        aggregator.record(minute + 1, observation);
        let batches = aggregator.take_batches();

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].snapshots()[0].count(), 1);
        assert_eq!(batches[0].delivery().observations_invalid(), 1);
        assert!(aggregator.take_batches().is_empty());
    }

    #[test]
    fn packed_namespace_batch_stays_below_canary_attributes_limit() {
        let minute = current_minute();
        let mut aggregator = Aggregator::new();
        let mut observations = 0_u64;
        for action in Action::all() {
            for outcome in Outcome::all() {
                let Ok(marker) =
                    CompletionMarker::server(*action, CompletionPhase::ImmediateAck, *outcome)
                else {
                    continue;
                };
                aggregator.record(minute, marker.observation(25).expect("bounded observation"));
                observations += 1;
            }
        }

        let batches = aggregator.take_batches();
        assert_eq!(batches.len(), 1);
        let batch = &batches[0];
        let attributes = batch.attributes_for_attempt(2, true);
        let encoded = serde_json::to_vec(&attributes).expect("attributes");
        assert!(encoded.len() <= 8_192, "{} bytes", encoded.len());
        assert!(batch.delivery().series_dropped() > 0);
        let retained: u64 = batch
            .snapshots()
            .iter()
            .map(memory_engine_performance::Snapshot::count)
            .sum();
        assert_eq!(
            retained + batch.delivery().observations_dropped(),
            observations
        );
    }
}
