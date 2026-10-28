//! The creator's accept rules and index paging (SPEC section 3.3).
//!
//! What the creator's node decides about a received submission, and how
//! accepted entries are laid out across index pages. Storage has no listing
//! operation and Delivery Store is temporary, so the index pages are the
//! durable record — if an entry never reaches a page, nothing can find the
//! submission again.
//!
//! Every decision here is deterministic given the submission, the clock and
//! the current index, so a creator who was offline and catches up later
//! reaches the same verdict.

use crate::fields::{self, Field};
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Entries per index page.
pub const PAGE_CAPACITY: usize = 1000;

/// Default submissions accepted per submitter key per day.
pub const DEFAULT_DAILY_LIMIT: usize = 200;

/// How long after `ends_at` a draft-allowing protocol still accepts arrivals.
pub const DRAFT_GRACE_DAYS: i64 = 7;

/// Why a submission was not indexed.
///
/// Carried into the creator's local log: invalid submissions are ignored and
/// never retried, so the reason has to be specific enough to act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reject {
    /// The submitter is hidden for this experiment.
    HiddenSubmitter,
    /// Arrived before the experiment opened.
    BeforeStart { starts_at: String },
    /// Arrived after the experiment closed, outside any draft grace.
    AfterEnd { ends_at: String },
    /// Allowed in late as a draft, but claims to have been filled in outside
    /// the window.
    SubmittedOutsideWindow { submitted_at: String },
    /// The protocol named is not part of this experiment, or is closed.
    ProtocolNotOpen { protocol: String },
    /// The protocol version named does not match the experiment's.
    ProtocolVersionMismatch { expected: u64, found: u64 },
    /// The submitter has already used up today's allowance.
    RateLimited { limit: usize },
    /// This protocol takes one submission per submitter and already has one.
    AlreadySubmitted,
    /// Field validation failed.
    Fields(fields::Report),
    /// The submission does not belong to this experiment.
    WrongExperiment,
}

impl std::fmt::Display for Reject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Reject::HiddenSubmitter => write!(f, "submitter is hidden"),
            Reject::BeforeStart { starts_at } => {
                write!(f, "arrived before the experiment opened at {starts_at}")
            }
            Reject::AfterEnd { ends_at } => {
                write!(f, "arrived after the experiment closed at {ends_at}")
            }
            Reject::SubmittedOutsideWindow { submitted_at } => {
                write!(f, "submitted_at {submitted_at} is outside the window")
            }
            Reject::ProtocolNotOpen { protocol } => {
                write!(f, "protocol `{protocol}` is not open on this experiment")
            }
            Reject::ProtocolVersionMismatch { expected, found } => {
                write!(
                    f,
                    "protocol version {found} is not the published {expected}"
                )
            }
            Reject::RateLimited { limit } => {
                write!(f, "submitter is over the {limit} per day limit")
            }
            Reject::AlreadySubmitted => {
                write!(f, "protocol takes one submission per submitter")
            }
            Reject::Fields(report) => {
                write!(f, "field validation failed: ")?;
                if !report.missing.is_empty() {
                    write!(f, "missing {}", report.missing.join(", "))?;
                }
                for (key, detail) in &report.errors {
                    write!(f, "; {key}: {detail}")?;
                }
                Ok(())
            }
            Reject::WrongExperiment => write!(f, "submission is for another experiment"),
        }
    }
}

/// What the creator knows about an experiment while judging a submission.
#[derive(Debug, Clone)]
pub struct Window {
    pub experiment_id: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

/// The protocol a submission names, as the experiment currently publishes it.
#[derive(Debug, Clone)]
pub struct OpenProtocol {
    pub id: String,
    pub version: u64,
    pub open: bool,
    pub allow_multiple: bool,
    pub allow_drafts: bool,
    pub fields: Vec<Field>,
}

/// What the current index already holds, as far as the accept rules care.
#[derive(Debug, Default, Clone)]
pub struct IndexState {
    pub hidden_keys: Vec<String>,
    /// Accepted submissions per submitter key, by UTC day (`YYYY-MM-DD`).
    pub daily_counts: BTreeMap<(String, String), usize>,
    /// Submitter keys that already have an accepted submission, per protocol.
    pub submitted: Vec<(String, String)>,
    /// Submissions accepted per day limit, if the creator changed it.
    pub daily_limit: Option<usize>,
}

impl IndexState {
    /// Rebuild the accept-relevant state from index pages, newest first.
    ///
    /// The creator may have restarted, so this has to come back from the
    /// pages on Storage rather than from memory.
    pub fn from_pages(pages: &[Value]) -> Self {
        let mut state = IndexState::default();
        for page in pages {
            for key in page
                .get("hidden_keys")
                .and_then(Value::as_array)
                .unwrap_or(&Vec::new())
            {
                if let Some(k) = key.as_str()
                    && !state.hidden_keys.iter().any(|h| h == k)
                {
                    state.hidden_keys.push(k.to_string());
                }
            }
            for entry in page
                .get("entries")
                .and_then(Value::as_array)
                .unwrap_or(&Vec::new())
            {
                let (Some(submitter), Some(protocol), Some(received_at)) = (
                    entry.get("submitter").and_then(Value::as_str),
                    entry.get("protocol").and_then(Value::as_str),
                    entry.get("received_at").and_then(Value::as_str),
                ) else {
                    continue;
                };
                state.record(submitter, protocol, received_at);
            }
        }
        state
    }

    fn record(&mut self, submitter: &str, protocol: &str, received_at: &str) {
        // The limit is "per day", counted on the receive day in UTC so the
        // creator's own time zone does not move the boundary.
        if let Some(day) = utc_day(received_at) {
            *self
                .daily_counts
                .entry((submitter.to_string(), day))
                .or_insert(0) += 1;
        }
        let pair = (protocol.to_string(), submitter.to_string());
        if !self.submitted.contains(&pair) {
            self.submitted.push(pair);
        }
    }

    fn limit(&self) -> usize {
        self.daily_limit.unwrap_or(DEFAULT_DAILY_LIMIT)
    }

    fn count_for(&self, submitter: &str, day: &str) -> usize {
        self.daily_counts
            .get(&(submitter.to_string(), day.to_string()))
            .copied()
            .unwrap_or(0)
    }

    fn has_submitted(&self, protocol: &str, submitter: &str) -> bool {
        self.submitted
            .iter()
            .any(|(p, s)| p == protocol && s == submitter)
    }
}

fn utc_day(ts: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|t| t.with_timezone(&Utc).format("%Y-%m-%d").to_string())
}

/// Decide whether a received submission may be indexed.
///
/// `received_at` is when the creator's node got it, which is the time the
/// window test uses — a submitter cannot extend the window by backdating
/// `submitted_at`.
pub fn accept(
    submission: &Value,
    window: &Window,
    protocol: &OpenProtocol,
    state: &IndexState,
    received_at: DateTime<Utc>,
) -> Result<(), Reject> {
    if submission.get("experiment").and_then(Value::as_str) != Some(window.experiment_id.as_str()) {
        return Err(Reject::WrongExperiment);
    }

    let submitter = submission
        .pointer("/submitter/key")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    if state.hidden_keys.contains(&submitter) {
        return Err(Reject::HiddenSubmitter);
    }

    let named = submission
        .pointer("/protocol/id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if named != protocol.id || !protocol.open {
        return Err(Reject::ProtocolNotOpen {
            protocol: named.to_string(),
        });
    }

    let version = submission
        .pointer("/protocol/version")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    if version != protocol.version {
        return Err(Reject::ProtocolVersionMismatch {
            expected: protocol.version,
            found: version,
        });
    }

    check_window(submission, window, protocol, received_at)?;

    if !protocol.allow_multiple && state.has_submitted(&protocol.id, &submitter) {
        return Err(Reject::AlreadySubmitted);
    }

    let day = received_at.format("%Y-%m-%d").to_string();
    if state.count_for(&submitter, &day) >= state.limit() {
        return Err(Reject::RateLimited {
            limit: state.limit(),
        });
    }

    let values = submission.get("values").unwrap_or(&Value::Null);
    let report = fields::validate_values(&protocol.fields, values, received_at);
    if !report.is_ok() {
        return Err(Reject::Fields(report));
    }

    Ok(())
}

fn check_window(
    submission: &Value,
    window: &Window,
    protocol: &OpenProtocol,
    received_at: DateTime<Utc>,
) -> Result<(), Reject> {
    if received_at < window.starts_at {
        return Err(Reject::BeforeStart {
            starts_at: window.starts_at.to_rfc3339(),
        });
    }

    if received_at <= window.ends_at {
        return Ok(());
    }

    // Past the end. A protocol that allows drafts still accepts arrivals for
    // DRAFT_GRACE_DAYS, because a submitter who filled the form in time may
    // have been offline when it closed — but only if the submission itself
    // claims a time inside the window.
    if !protocol.allow_drafts {
        return Err(Reject::AfterEnd {
            ends_at: window.ends_at.to_rfc3339(),
        });
    }
    if received_at > window.ends_at + Duration::days(DRAFT_GRACE_DAYS) {
        return Err(Reject::AfterEnd {
            ends_at: window.ends_at.to_rfc3339(),
        });
    }

    let raw = submission
        .get("submitted_at")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let submitted_at = DateTime::parse_from_rfc3339(raw)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|_| Reject::SubmittedOutsideWindow {
            submitted_at: raw.to_string(),
        })?;

    if submitted_at < window.starts_at || submitted_at > window.ends_at {
        return Err(Reject::SubmittedOutsideWindow {
            submitted_at: raw.to_string(),
        });
    }
    Ok(())
}

/// One accepted submission, as it appears in an index page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub cid: String,
    pub protocol: String,
    pub version: u64,
    pub submitter: String,
    pub submitted_at: String,
    pub received_at: String,
}

impl Entry {
    fn to_value(&self) -> Value {
        json!({
            "cid": self.cid,
            "protocol": self.protocol,
            "version": self.version,
            "submitter": self.submitter,
            "submitted_at": self.submitted_at,
            "received_at": self.received_at,
        })
    }
}

/// Lay accepted entries out across index pages.
///
/// `existing` is the newest page, if there is one; entries fill it to
/// `PAGE_CAPACITY` before a new page opens. Returns the pages to upload,
/// oldest first, each linked to its predecessor by `prev`.
///
/// Pages are immutable once uploaded, so a page that was already full comes
/// back unchanged and only its successors are new.
pub fn page_entries(
    experiment_id: &str,
    existing: Option<&Value>,
    existing_cid: Option<&str>,
    new_entries: &[Entry],
    hidden_keys: &[String],
    created_at: DateTime<Utc>,
) -> Vec<Value> {
    let (mut page_number, mut current, mut prev) = match existing {
        Some(page) => {
            let number = page.get("page").and_then(Value::as_u64).unwrap_or(1);
            let entries = page
                .get("entries")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if entries.len() >= PAGE_CAPACITY {
                // Full and immutable — start the next page after it.
                (number + 1, Vec::new(), existing_cid.map(str::to_string))
            } else {
                (
                    number,
                    entries,
                    page.get("prev").and_then(Value::as_str).map(str::to_string),
                )
            }
        }
        None => (1, Vec::new(), None),
    };

    let mut pages = Vec::new();
    let mut queue = new_entries.iter();
    let mut pending = queue.next();

    loop {
        while current.len() < PAGE_CAPACITY {
            match pending {
                Some(entry) => {
                    current.push(entry.to_value());
                    pending = queue.next();
                }
                None => break,
            }
        }

        pages.push(build_page(
            experiment_id,
            page_number,
            &current,
            hidden_keys,
            prev.as_deref(),
            created_at,
        ));

        if pending.is_none() {
            break;
        }

        // The page just built is full. Its CID is not known until it is
        // uploaded, so the caller stitches `prev` as it goes — see
        // `link_pages`.
        page_number += 1;
        current = Vec::new();
        prev = None;
    }

    pages
}

fn build_page(
    experiment_id: &str,
    number: u64,
    entries: &[Value],
    hidden_keys: &[String],
    prev: Option<&str>,
    created_at: DateTime<Utc>,
) -> Value {
    json!({
        "schema": "collect/index@1",
        "experiment": experiment_id,
        "page": number,
        "entries": entries,
        "hidden_keys": hidden_keys,
        "prev": prev.map(Value::from).unwrap_or(Value::Null),
        "created_at": created_at.to_rfc3339(),
    })
}

/// Set a page's `prev` to the CID of the page before it.
///
/// Pages are uploaded oldest first, so the predecessor's CID only exists once
/// it has been stored. Callers upload a page, then call this on the next one
/// before signing it.
pub fn set_prev(page: &mut Value, prev_cid: &str) {
    if let Some(obj) = page.as_object_mut() {
        obj.insert("prev".into(), Value::String(prev_cid.to_string()));
    }
}

/// Add a submitter key to a page's `hidden_keys`.
///
/// Hiding drops the submitter from results and exports from the next page on.
/// Their documents stay on Storage, because nothing there can be deleted.
pub fn hide_submitter(page: &mut Value, key: &str) {
    let Some(obj) = page.as_object_mut() else {
        return;
    };
    let list = obj
        .entry("hidden_keys")
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Some(arr) = list.as_array_mut()
        && !arr.iter().any(|k| k.as_str() == Some(key))
    {
        arr.push(Value::String(key.to_string()));
    }
}

/// Entries from index pages with hidden submitters dropped.
///
/// Results, the table and every export read through this, so one rule covers
/// all three.
pub fn visible_entries(pages: &[Value]) -> Vec<Value> {
    let hidden: Vec<String> = pages
        .iter()
        .filter_map(|p| p.get("hidden_keys").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();

    pages
        .iter()
        .filter_map(|p| p.get("entries").and_then(Value::as_array))
        .flatten()
        .filter(|e| {
            e.get("submitter")
                .and_then(Value::as_str)
                .map(|s| !hidden.iter().any(|h| h == s))
                .unwrap_or(false)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::parse_fields;

    const PROTOCOL: &str = include_str!("../../../examples/protocol.json");
    const SUBMISSION: &str = include_str!("../../../examples/submission.json");
    const INDEX: &str = include_str!("../../../examples/index.json");

    const EXPERIMENT_ID: &str = "q7k2m3x4w3b5n6tz";
    const ALICE: &str = "a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4";
    const BOB: &str = "b2c3d4e5b2c3d4e5b2c3d4e5b2c3d4e5b2c3d4e5b2c3d4e5b2c3d4e5b2c3d4e5";

    fn at(ts: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(ts)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn window() -> Window {
        Window {
            experiment_id: EXPERIMENT_ID.into(),
            starts_at: at("2026-10-20T00:00:00Z"),
            ends_at: at("2026-11-20T00:00:00Z"),
        }
    }

    fn protocol(allow_multiple: bool, allow_drafts: bool) -> OpenProtocol {
        let raw: Value = serde_json::from_str(PROTOCOL).unwrap();
        OpenProtocol {
            id: "wildlife-sighting".into(),
            version: 2,
            open: true,
            allow_multiple,
            allow_drafts,
            fields: parse_fields(&raw).unwrap(),
        }
    }

    fn submission() -> Value {
        serde_json::from_str(SUBMISSION).unwrap()
    }

    /// Inside the window, after the example's submitted_at.
    fn received() -> DateTime<Utc> {
        at("2026-10-28T08:00:00Z")
    }

    fn entry(n: usize, submitter: &str) -> Entry {
        Entry {
            cid: format!("logos:zb2rhSub{n}"),
            protocol: "wildlife-sighting".into(),
            version: 2,
            submitter: submitter.into(),
            submitted_at: "2026-10-28T07:50:12+05:30".into(),
            received_at: "2026-10-28T07:50:31+05:30".into(),
        }
    }

    // ---- accept ----

    #[test]
    fn accepts_a_good_submission() {
        let r = accept(
            &submission(),
            &window(),
            &protocol(true, true),
            &IndexState::default(),
            received(),
        );
        assert_eq!(r, Ok(()));
    }

    #[test]
    fn rejects_another_experiments_submission() {
        let mut s = submission();
        s["experiment"] = json!("zzzzzzzzzzzzzzzz");
        assert_eq!(
            accept(
                &s,
                &window(),
                &protocol(true, true),
                &IndexState::default(),
                received()
            ),
            Err(Reject::WrongExperiment)
        );
    }

    #[test]
    fn rejects_a_hidden_submitter() {
        let state = IndexState {
            hidden_keys: vec![ALICE.into()],
            ..Default::default()
        };
        assert_eq!(
            accept(
                &submission(),
                &window(),
                &protocol(true, true),
                &state,
                received()
            ),
            Err(Reject::HiddenSubmitter)
        );
    }

    #[test]
    fn rejects_a_closed_protocol() {
        let mut p = protocol(true, true);
        p.open = false;
        assert!(matches!(
            accept(
                &submission(),
                &window(),
                &p,
                &IndexState::default(),
                received()
            ),
            Err(Reject::ProtocolNotOpen { .. })
        ));
    }

    #[test]
    fn rejects_an_unknown_protocol() {
        let mut s = submission();
        s["protocol"]["id"] = json!("nest-monitoring");
        assert!(matches!(
            accept(
                &s,
                &window(),
                &protocol(true, true),
                &IndexState::default(),
                received()
            ),
            Err(Reject::ProtocolNotOpen { .. })
        ));
    }

    #[test]
    fn rejects_a_stale_protocol_version() {
        // Editing a protocol publishes version 2; a submission naming version
        // 1 was filled in against different fields.
        let mut s = submission();
        s["protocol"]["version"] = json!(1);
        assert_eq!(
            accept(
                &s,
                &window(),
                &protocol(true, true),
                &IndexState::default(),
                received()
            ),
            Err(Reject::ProtocolVersionMismatch {
                expected: 2,
                found: 1
            })
        );
    }

    // ---- window ----

    #[test]
    fn rejects_an_arrival_before_the_start() {
        assert!(matches!(
            accept(
                &submission(),
                &window(),
                &protocol(true, true),
                &IndexState::default(),
                at("2026-10-19T23:59:59Z")
            ),
            Err(Reject::BeforeStart { .. })
        ));
    }

    #[test]
    fn accepts_an_arrival_exactly_at_the_boundaries() {
        let w = window();

        // The `observed` field is no_future, and the creator checks it
        // against its own receive time, so it has to sit at or before each
        // boundary being tested.
        let mut s = submission();
        s["submitted_at"] = json!("2026-10-20T00:00:00Z");
        s["values"]["observed"] = json!("2026-10-20T00:00:00Z");
        assert_eq!(
            accept(
                &s,
                &w,
                &protocol(true, true),
                &IndexState::default(),
                w.starts_at
            ),
            Ok(())
        );

        let mut s = submission();
        s["submitted_at"] = json!("2026-11-20T00:00:00Z");
        s["values"]["observed"] = json!("2026-11-20T00:00:00Z");
        assert_eq!(
            accept(
                &s,
                &w,
                &protocol(true, true),
                &IndexState::default(),
                w.ends_at
            ),
            Ok(())
        );
    }

    #[test]
    fn rejects_a_late_arrival_when_drafts_are_off() {
        let w = window();
        assert!(matches!(
            accept(
                &submission(),
                &w,
                &protocol(true, false),
                &IndexState::default(),
                w.ends_at + Duration::seconds(1)
            ),
            Err(Reject::AfterEnd { .. })
        ));
    }

    #[test]
    fn accepts_a_late_draft_filled_in_during_the_window() {
        // SPEC 3.3: with drafts allowed, a receive time up to 7 days past the
        // end is fine as long as submitted_at is inside the window.
        let w = window();
        let r = accept(
            &submission(),
            &w,
            &protocol(true, true),
            &IndexState::default(),
            w.ends_at + Duration::days(6),
        );
        assert_eq!(r, Ok(()));
    }

    #[test]
    fn rejects_a_late_draft_past_the_grace_period() {
        let w = window();
        assert!(matches!(
            accept(
                &submission(),
                &w,
                &protocol(true, true),
                &IndexState::default(),
                w.ends_at + Duration::days(DRAFT_GRACE_DAYS) + Duration::seconds(1)
            ),
            Err(Reject::AfterEnd { .. })
        ));
    }

    #[test]
    fn rejects_a_late_draft_backdated_outside_the_window() {
        // The attack the submitted_at check exists for: arrive late, then
        // claim to have filled the form in before the experiment opened.
        let w = window();
        let mut s = submission();
        s["submitted_at"] = json!("2026-10-01T00:00:00Z");
        assert!(matches!(
            accept(
                &s,
                &w,
                &protocol(true, true),
                &IndexState::default(),
                w.ends_at + Duration::days(1)
            ),
            Err(Reject::SubmittedOutsideWindow { .. })
        ));
    }

    #[test]
    fn a_backdated_submitted_at_cannot_extend_the_window() {
        // The window test uses the receive time, so backdating does not help
        // a protocol with drafts off.
        let w = window();
        let mut s = submission();
        s["submitted_at"] = json!("2026-10-25T00:00:00Z");
        assert!(matches!(
            accept(
                &s,
                &w,
                &protocol(true, false),
                &IndexState::default(),
                w.ends_at + Duration::days(1)
            ),
            Err(Reject::AfterEnd { .. })
        ));
    }

    // ---- rate limit ----

    #[test]
    fn rejects_a_submitter_over_the_daily_limit() {
        let day = received().format("%Y-%m-%d").to_string();
        let mut state = IndexState::default();
        state
            .daily_counts
            .insert((ALICE.into(), day), DEFAULT_DAILY_LIMIT);
        assert_eq!(
            accept(
                &submission(),
                &window(),
                &protocol(true, true),
                &state,
                received()
            ),
            Err(Reject::RateLimited {
                limit: DEFAULT_DAILY_LIMIT
            })
        );
    }

    #[test]
    fn accepts_one_below_the_daily_limit() {
        let day = received().format("%Y-%m-%d").to_string();
        let mut state = IndexState::default();
        state
            .daily_counts
            .insert((ALICE.into(), day), DEFAULT_DAILY_LIMIT - 1);
        assert_eq!(
            accept(
                &submission(),
                &window(),
                &protocol(true, true),
                &state,
                received()
            ),
            Ok(())
        );
    }

    #[test]
    fn the_limit_is_configurable() {
        let day = received().format("%Y-%m-%d").to_string();
        let mut state = IndexState {
            daily_limit: Some(2),
            ..Default::default()
        };
        state.daily_counts.insert((ALICE.into(), day), 2);
        assert_eq!(
            accept(
                &submission(),
                &window(),
                &protocol(true, true),
                &state,
                received()
            ),
            Err(Reject::RateLimited { limit: 2 })
        );
    }

    #[test]
    fn the_limit_is_per_day_and_per_submitter() {
        let mut state = IndexState::default();
        // Alice used up yesterday, not today.
        state
            .daily_counts
            .insert((ALICE.into(), "2026-10-27".into()), DEFAULT_DAILY_LIMIT);
        // Bob used up today, which must not affect Alice.
        state
            .daily_counts
            .insert((BOB.into(), "2026-10-28".into()), DEFAULT_DAILY_LIMIT);
        assert_eq!(
            accept(
                &submission(),
                &window(),
                &protocol(true, true),
                &state,
                received()
            ),
            Ok(())
        );
    }

    #[test]
    fn rejects_a_second_submission_when_multiple_are_not_allowed() {
        let state = IndexState {
            submitted: vec![("wildlife-sighting".into(), ALICE.into())],
            ..Default::default()
        };
        assert_eq!(
            accept(
                &submission(),
                &window(),
                &protocol(false, true),
                &state,
                received()
            ),
            Err(Reject::AlreadySubmitted)
        );
        // The same state is fine when the protocol allows repeats.
        assert_eq!(
            accept(
                &submission(),
                &window(),
                &protocol(true, true),
                &state,
                received()
            ),
            Ok(())
        );
    }

    // ---- field validation reaches the verdict ----

    #[test]
    fn rejects_a_submission_that_fails_field_validation() {
        let mut s = submission();
        s["values"]["category"] = json!("not-an-option");
        let err = accept(
            &s,
            &window(),
            &protocol(true, true),
            &IndexState::default(),
            received(),
        )
        .unwrap_err();
        match err {
            Reject::Fields(report) => assert!(report.errors.contains_key("category")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn reject_reasons_say_something_specific() {
        // The creator's log is the only place a rejection is explained.
        let mut s = submission();
        s["values"]["category"] = json!("not-an-option");
        let err = accept(
            &s,
            &window(),
            &protocol(true, true),
            &IndexState::default(),
            received(),
        )
        .unwrap_err();
        let text = err.to_string();
        assert!(text.contains("category"), "{text}");
        assert!(text.contains("not-an-option"), "{text}");
    }

    // ---- rebuilding state from pages ----

    #[test]
    fn rebuilds_state_from_the_example_index() {
        let page: Value = serde_json::from_str(INDEX).unwrap();
        let state = IndexState::from_pages(&[page]);
        assert_eq!(
            state.hidden_keys,
            ["9b104c2d9b104c2d9b104c2d9b104c2d9b104c2d9b104c2d9b104c2d9b104c2d"]
        );
        // Both entries were received on 2026-10-27/28 in +05:30, which is the
        // 27th in UTC for one of them.
        assert_eq!(state.count_for(ALICE, "2026-10-28"), 1);
        assert_eq!(state.count_for(BOB, "2026-10-27"), 1);
        assert!(state.has_submitted("wildlife-sighting", ALICE));
        assert!(state.has_submitted("nest-monitoring", BOB));
        assert!(!state.has_submitted("wildlife-sighting", BOB));
    }

    #[test]
    fn counts_the_receive_day_in_utc() {
        // 2026-10-28T02:00+05:30 is 2026-10-27 in UTC. Counting in local time
        // would put it on the wrong day and let a submitter over the limit.
        let page = json!({
            "entries": [{
                "cid": "logos:a", "protocol": "p", "version": 1,
                "submitter": ALICE,
                "submitted_at": "2026-10-28T02:00:00+05:30",
                "received_at": "2026-10-28T02:00:00+05:30"
            }],
            "hidden_keys": []
        });
        let state = IndexState::from_pages(&[page]);
        assert_eq!(state.count_for(ALICE, "2026-10-27"), 1);
        assert_eq!(state.count_for(ALICE, "2026-10-28"), 0);
    }

    #[test]
    fn hidden_keys_accumulate_across_pages() {
        let a = json!({"entries": [], "hidden_keys": [ALICE]});
        let b = json!({"entries": [], "hidden_keys": [BOB, ALICE]});
        let state = IndexState::from_pages(&[a, b]);
        assert_eq!(state.hidden_keys.len(), 2);
    }

    // ---- paging ----

    #[test]
    fn first_page_starts_at_one_with_no_prev() {
        let pages = page_entries(
            EXPERIMENT_ID,
            None,
            None,
            &[entry(1, ALICE)],
            &[],
            received(),
        );
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0]["page"], json!(1));
        assert_eq!(pages[0]["prev"], Value::Null);
        assert_eq!(pages[0]["entries"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn appends_to_a_page_that_has_room() {
        let existing = json!({
            "schema": "collect/index@1", "experiment": EXPERIMENT_ID, "page": 3,
            "entries": [entry(1, ALICE).to_value()],
            "hidden_keys": [], "prev": "logos:zb2rhPage2",
            "created_at": "2026-10-28T07:51:00+05:30"
        });
        let pages = page_entries(
            EXPERIMENT_ID,
            Some(&existing),
            Some("logos:zb2rhPage3"),
            &[entry(2, BOB)],
            &[],
            received(),
        );
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0]["page"], json!(3));
        // Re-issuing page 3 keeps its own prev, not the CID of the version it
        // replaces.
        assert_eq!(pages[0]["prev"], json!("logos:zb2rhPage2"));
        assert_eq!(pages[0]["entries"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn opens_a_new_page_when_the_last_one_is_full() {
        let full: Vec<Value> = (0..PAGE_CAPACITY)
            .map(|n| entry(n, ALICE).to_value())
            .collect();
        let existing = json!({
            "schema": "collect/index@1", "experiment": EXPERIMENT_ID, "page": 1,
            "entries": full, "hidden_keys": [], "prev": null,
            "created_at": "2026-10-28T07:51:00+05:30"
        });
        let pages = page_entries(
            EXPERIMENT_ID,
            Some(&existing),
            Some("logos:zb2rhPage1"),
            &[entry(9001, BOB)],
            &[],
            received(),
        );
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0]["page"], json!(2));
        // The full page is immutable, so the new one links back to it.
        assert_eq!(pages[0]["prev"], json!("logos:zb2rhPage1"));
        assert_eq!(pages[0]["entries"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn fills_a_page_to_capacity_before_opening_the_next() {
        let entries: Vec<Entry> = (0..PAGE_CAPACITY + 5).map(|n| entry(n, ALICE)).collect();
        let pages = page_entries(EXPERIMENT_ID, None, None, &entries, &[], received());
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0]["entries"].as_array().unwrap().len(), PAGE_CAPACITY);
        assert_eq!(pages[1]["entries"].as_array().unwrap().len(), 5);
        assert_eq!(pages[0]["page"], json!(1));
        assert_eq!(pages[1]["page"], json!(2));
    }

    #[test]
    fn no_page_ever_exceeds_capacity() {
        // The contract caps entries at 1000; going over makes the page
        // unuploadable.
        let entries: Vec<Entry> = (0..PAGE_CAPACITY * 3 + 7)
            .map(|n| entry(n, ALICE))
            .collect();
        let pages = page_entries(EXPERIMENT_ID, None, None, &entries, &[], received());
        assert_eq!(pages.len(), 4);
        for p in &pages {
            assert!(p["entries"].as_array().unwrap().len() <= PAGE_CAPACITY);
        }
        let total: usize = pages
            .iter()
            .map(|p| p["entries"].as_array().unwrap().len())
            .sum();
        assert_eq!(total, entries.len());
    }

    #[test]
    fn every_emitted_page_validates_against_the_contract() {
        let entries: Vec<Entry> = (0..PAGE_CAPACITY + 1).map(|n| entry(n, ALICE)).collect();
        let mut pages = page_entries(
            EXPERIMENT_ID,
            None,
            None,
            &entries,
            &[ALICE.to_string()],
            received(),
        );
        // `prev` and `sig` are filled in as pages are uploaded; stub the
        // signature so the shape can be checked here.
        for (i, page) in pages.iter_mut().enumerate() {
            if i > 0 {
                set_prev(page, "logos:zb2rhPrev");
            }
            page["sig"] = json!("ab".repeat(64));
            assert!(
                crate::schema::is_valid(page),
                "page {i}: {:?}",
                crate::schema::errors(page)
            );
        }
    }

    #[test]
    fn emitting_no_new_entries_still_republishes_the_page() {
        // The creator republishes head every 10 minutes; the page it points
        // at has to exist even when nothing arrived.
        let pages = page_entries(EXPERIMENT_ID, None, None, &[], &[], received());
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0]["entries"], json!([]));
    }

    #[test]
    fn set_prev_links_a_page() {
        let mut page = json!({"prev": null});
        set_prev(&mut page, "logos:zb2rhPage1");
        assert_eq!(page["prev"], json!("logos:zb2rhPage1"));
    }

    // ---- hiding ----

    #[test]
    fn hide_submitter_adds_a_key_once() {
        let mut page = json!({"hidden_keys": []});
        hide_submitter(&mut page, BOB);
        hide_submitter(&mut page, BOB);
        assert_eq!(page["hidden_keys"], json!([BOB]));
    }

    #[test]
    fn hiding_drops_rows_from_results_and_exports() {
        // SPEC 3.3: hiding removes the submitter from results and export, and
        // from the next index page.
        let page = json!({
            "entries": [entry(1, ALICE).to_value(), entry(2, BOB).to_value()],
            "hidden_keys": [BOB]
        });
        let visible = visible_entries(&[page]);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0]["submitter"], json!(ALICE));
    }

    #[test]
    fn hiding_applies_across_earlier_pages() {
        // The key appears on the newest page; rows on older pages must go too.
        let old = json!({"entries": [entry(1, BOB).to_value()], "hidden_keys": []});
        let new = json!({"entries": [entry(2, ALICE).to_value()], "hidden_keys": [BOB]});
        let visible = visible_entries(&[old, new]);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0]["submitter"], json!(ALICE));
    }
}
