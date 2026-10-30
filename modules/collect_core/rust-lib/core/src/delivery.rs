//! Announcements over Logos Delivery, behind a trait.
//!
//! Same split as `store`: the logic names the capability, the module shim
//! supplies it (`rust-lib/src/courier.rs`). Written against
//! `docs/delivery_module.lidl` (delivery_module 0.3.3).
//!
//! ## What Delivery is for here, and what it is not
//!
//! Storage holds the documents; Delivery only says *that* a document exists.
//! Every announcement is a few hundred bytes naming a CID, which is why the
//! ~150 KiB message cap never comes near mattering. Nothing is ever only in
//! Delivery: its Store is temporary and operator-configured, so a submission
//! that was announced but never fetched is recoverable from the CID and a
//! submission that was never announced is still on Storage. The index pages
//! are the durable record (SPEC 2).
//!
//! ## Two things the contract is explicit about
//!
//! 1. **Deduplicate by `messageHash`.** `messageReceived` carries
//!    `source: "live" | "history"`, and Store catch-up replays what was missed
//!    while the node was down — on by default, so a restart re-delivers.
//!    Upstream suppresses duplicates "only for a few minutes and only in
//!    memory". A creator that indexed every delivery would record the same
//!    submission repeatedly after any restart.
//! 2. **RLN gates sending, not receiving.** `logos.test` requires a funded
//!    membership before a node will publish (SPEC Q9). `rlnState()` reports
//!    it, so this asks rather than assumes — a submitter whose announcement
//!    cannot go out needs to be told that, not left watching a spinner.
//!
//! Delivery events are shaped unlike Storage's: each is a list of typed
//! fields, not one JSON string. See `courier.rs`.

use serde_json::{json, Value};
use std::collections::{BTreeSet, VecDeque};
use std::fmt;

/// Why an announcement did not go out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryError {
    /// No Delivery module is attached. Nothing was attempted.
    Unavailable(String),
    /// Delivery answered, and the answer was no.
    Rejected(String),
    /// Accepted, but nothing came back in time.
    Timeout(String),
    /// The node has no RLN membership, so it cannot publish to this network.
    /// Separate from `Rejected` because no amount of retrying fixes it and
    /// the UI has to say something different.
    RlnNotReady { state: String, message: String },
}

impl DeliveryError {
    pub fn code(&self) -> &'static str {
        match self {
            DeliveryError::Unavailable(_) => "delivery_unavailable",
            DeliveryError::Rejected(_) => "delivery_rejected",
            DeliveryError::Timeout(_) => "delivery_timeout",
            DeliveryError::RlnNotReady { .. } => "rln_not_ready",
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(self, DeliveryError::Timeout(_) | DeliveryError::Rejected(_))
    }
}

impl fmt::Display for DeliveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeliveryError::Unavailable(m) => write!(f, "Logos Delivery is not available: {m}"),
            DeliveryError::Rejected(m) => write!(f, "Logos Delivery refused the message: {m}"),
            DeliveryError::Timeout(m) => write!(f, "Logos Delivery did not answer in time: {m}"),
            DeliveryError::RlnNotReady { state, message } => write!(
                f,
                "this node cannot publish to the network yet: RLN membership is `{state}` ({message})"
            ),
        }
    }
}

/// What `rlnState()` says about this node's ability to publish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RlnStatus {
    /// The state name Delivery reported, verbatim.
    pub state: String,
    pub message: String,
    /// Whether RLN applies on this network at all. False means there is
    /// nothing to be ready for.
    pub required: bool,
    /// Whether a send would be allowed now.
    pub ready: bool,
}

impl RlnStatus {
    /// A network with no RLN: nothing gates sending.
    pub fn not_required() -> Self {
        RlnStatus {
            state: "disabled".into(),
            message: "this network does not require RLN".into(),
            required: false,
            ready: true,
        }
    }

    pub fn as_json(&self) -> Value {
        json!({
            "state": self.state,
            "message": self.message,
            "required": self.required,
            "ready": self.ready,
        })
    }
}

/// Somewhere announcements can be sent and received.
pub trait Courier: Send {
    /// Publish `payload` on `topic`, returning Delivery's request id.
    fn send(&self, topic: &str, payload: &[u8]) -> Result<String, DeliveryError>;

    /// Start receiving messages on `topic`. Idempotent as far as callers are
    /// concerned: subscribing twice is not an error.
    fn subscribe(&self, topic: &str) -> Result<(), DeliveryError>;

    /// Whether this node may publish.
    fn rln(&self) -> RlnStatus;

    /// Delivery's own word for its connectivity — `Connected`,
    /// `PartiallyConnected` or `Disconnected` (`getConnectionStatus`). `None`
    /// when it could not be asked.
    fn connection(&self) -> Option<String> {
        None
    }

    /// Messages that have arrived since the last call, and clear them.
    ///
    /// A queue the session drains rather than a callback into it. Delivery
    /// hands messages to a listener thread, and the session is `&mut`-owned by
    /// the module — so a callback would need the session behind a lock held
    /// across the whole of `deliver`, which does Storage fetches and
    /// publishes. Draining on a timer keeps every mutation on one thread and
    /// makes the receive path testable without one.
    ///
    /// Each item is a `deliver` request: `{topic, messageHash, payload,
    /// source}`.
    fn inbox(&self) -> Vec<Value> {
        Vec::new()
    }

    fn describe(&self) -> String {
        "a courier".to_string()
    }
}

/// The content topic an experiment's announcements travel on.
///
/// Waku's four-part shape, `/<app>/<version>/<name>/<encoding>`. The
/// experiment id is the name, so one subscription per experiment: a single
/// shared topic would make every creator receive every other experiment's
/// traffic, and an id is already a public, non-guessable 16 characters.
pub fn topic(experiment_id: &str) -> String {
    format!("/collect/1/{experiment_id}/json")
}

/// Which experiment a content topic belongs to, if it is one of ours.
pub fn experiment_of(topic: &str) -> Option<&str> {
    let rest = topic.strip_prefix("/collect/1/")?;
    let id = rest.strip_suffix("/json")?;
    if id.is_empty() || id.contains('/') {
        return None;
    }
    Some(id)
}

/// The creator saying where the current manifest and index are.
///
/// `sig` is a placeholder: the caller signs it, over the canonical form
/// without `sig`, exactly as for a document.
pub fn head(
    experiment: &str,
    from: &str,
    manifest: &str,
    manifest_version: u64,
    index: Option<&str>,
    index_page: u64,
    at: &str,
) -> Value {
    json!({
        "kind": "head",
        "experiment": experiment,
        "from": from,
        "at": at,
        "manifest": manifest,
        "manifest_version": manifest_version,
        "index": index.map(Value::from).unwrap_or(Value::Null),
        "index_page": index_page,
        "sig": "00".repeat(64),
    })
}

/// A submitter saying a submission exists.
pub fn submission(
    experiment: &str,
    from: &str,
    cid: &str,
    protocol: &str,
    version: u64,
    at: &str,
) -> Value {
    json!({
        "kind": "submission",
        "experiment": experiment,
        "from": from,
        "at": at,
        "cid": cid,
        "protocol": protocol,
        "version": version,
        "sig": "00".repeat(64),
    })
}

/// The creator confirming a submission was indexed.
pub fn receipt(experiment: &str, from: &str, cid: &str, index_page: u64, at: &str) -> Value {
    json!({
        "kind": "receipt",
        "experiment": experiment,
        "from": from,
        "at": at,
        "cid": cid,
        "index_page": index_page,
        "sig": "00".repeat(64),
    })
}

/// Message hashes already handled.
///
/// Bounded and insertion-ordered: unbounded would grow for as long as a
/// creator's node runs, and the only thing that matters is not re-handling a
/// replay, which arrives close to the original in practice. Dropping the
/// oldest hash risks re-handling something from very long ago, which is the
/// cheaper mistake — `index_accept` refuses a duplicate submission on its own
/// merits anyway, so this is an optimisation over that, not the only guard.
pub struct Seen {
    order: VecDeque<String>,
    set: BTreeSet<String>,
    cap: usize,
}

impl Default for Seen {
    fn default() -> Self {
        Seen::with_capacity(4096)
    }
}

impl Seen {
    pub fn with_capacity(cap: usize) -> Self {
        Seen { order: VecDeque::new(), set: BTreeSet::new(), cap: cap.max(1) }
    }

    /// Record `hash`, and say whether it is new.
    pub fn accept(&mut self, hash: &str) -> bool {
        if hash.is_empty() {
            // Nothing to deduplicate on. Treated as new, because dropping a
            // message for want of a hash would lose a real submission.
            return true;
        }
        if !self.set.insert(hash.to_string()) {
            return false;
        }
        self.order.push_back(hash.to_string());
        while self.order.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
        true
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Every remembered hash, oldest first, so the set can be saved and
    /// replayed through `accept` on the next start.
    pub fn hashes(&self) -> impl Iterator<Item = &String> {
        self.order.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

/// An in-process courier, for tests.
///
/// Records what was sent so a test can assert on it, and can be configured to
/// fail or to report RLN as not ready.
pub struct MemoryCourier {
    sent: std::sync::Mutex<Vec<(String, Vec<u8>)>>,
    subscribed: std::sync::Mutex<Vec<String>>,
    arrived: std::sync::Mutex<Vec<Value>>,
    fail_with: Option<DeliveryError>,
    rln: RlnStatus,
    connection: Option<String>,
}

impl Default for MemoryCourier {
    fn default() -> Self {
        MemoryCourier::new()
    }
}

impl MemoryCourier {
    pub fn new() -> Self {
        MemoryCourier {
            sent: std::sync::Mutex::new(Vec::new()),
            subscribed: std::sync::Mutex::new(Vec::new()),
            arrived: std::sync::Mutex::new(Vec::new()),
            fail_with: None,
            rln: RlnStatus::not_required(),
            connection: Some("Connected".into()),
        }
    }

    /// A courier that reports `status` from `getConnectionStatus`.
    pub fn connected(status: Option<&str>) -> Self {
        MemoryCourier { connection: status.map(str::to_string), ..MemoryCourier::new() }
    }

    pub fn failing(error: DeliveryError) -> Self {
        MemoryCourier { fail_with: Some(error), ..MemoryCourier::new() }
    }

    /// A courier on a network whose RLN membership this node does not have.
    pub fn without_rln() -> Self {
        MemoryCourier {
            rln: RlnStatus {
                state: "unregistered".into(),
                message: "no funded membership for logos.test".into(),
                required: true,
                ready: false,
            },
            ..MemoryCourier::new()
        }
    }

    /// Everything sent, as parsed JSON.
    pub fn messages(&self) -> Vec<Value> {
        self.sent
            .lock()
            .map(|sent| {
                sent.iter()
                    .filter_map(|(_, bytes)| serde_json::from_slice(bytes).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn topics(&self) -> Vec<String> {
        self.sent
            .lock()
            .map(|sent| sent.iter().map(|(t, _)| t.clone()).collect())
            .unwrap_or_default()
    }

    pub fn subscriptions(&self) -> Vec<String> {
        self.subscribed.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Put any inbox item in, as the courier's listener would — for the
    /// items that are not messages, such as a refused send.
    pub fn arrive_raw(&self, item: Value) {
        if let Ok(mut arrived) = self.arrived.lock() {
            arrived.push(item);
        }
    }

    /// Put any inbox item in, as the real client's event threads do —
    /// `sendFailed` and `sent` included.
    pub fn push_raw(&self, item: Value) {
        if let Ok(mut arrived) = self.arrived.lock() {
            arrived.push(item);
        }
    }

    /// Put a message in the inbox, as Delivery would.
    pub fn arrive(&self, topic: &str, message_hash: &str, payload: &Value) {
        if let Ok(mut arrived) = self.arrived.lock() {
            arrived.push(json!({
                "topic": topic,
                "messageHash": message_hash,
                "source": "live",
                "payload": payload.to_string(),
            }));
        }
    }
}

impl Courier for MemoryCourier {
    fn send(&self, topic: &str, payload: &[u8]) -> Result<String, DeliveryError> {
        if let Some(e) = &self.fail_with {
            return Err(e.clone());
        }
        if !self.rln.ready {
            return Err(DeliveryError::RlnNotReady {
                state: self.rln.state.clone(),
                message: self.rln.message.clone(),
            });
        }
        match self.sent.lock() {
            Ok(mut sent) => {
                sent.push((topic.to_string(), payload.to_vec()));
                Ok(format!("req-{}", sent.len()))
            }
            Err(_) => Err(DeliveryError::Rejected("the in-memory courier is poisoned".into())),
        }
    }

    fn subscribe(&self, topic: &str) -> Result<(), DeliveryError> {
        if let Some(e) = &self.fail_with {
            return Err(e.clone());
        }
        if let Ok(mut subscribed) = self.subscribed.lock() {
            if !subscribed.iter().any(|t| t == topic) {
                subscribed.push(topic.to_string());
            }
        }
        Ok(())
    }

    fn rln(&self) -> RlnStatus {
        self.rln.clone()
    }

    fn connection(&self) -> Option<String> {
        self.connection.clone()
    }

    fn inbox(&self) -> Vec<Value> {
        self.arrived
            .lock()
            .map(|mut arrived| std::mem::take(&mut *arrived))
            .unwrap_or_default()
    }

    fn describe(&self) -> String {
        "an in-process courier (nothing leaves this process)".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema;

    #[test]
    fn a_topic_names_one_experiment_and_round_trips() {
        assert_eq!(topic("q7k2m3x4w3b5n6tz"), "/collect/1/q7k2m3x4w3b5n6tz/json");
        assert_eq!(experiment_of(&topic("q7k2m3x4w3b5n6tz")), Some("q7k2m3x4w3b5n6tz"));
    }

    #[test]
    fn a_topic_that_is_not_ours_is_not_claimed() {
        assert_eq!(experiment_of("/waku/2/default-waku/proto"), None);
        assert_eq!(experiment_of("/collect/1//json"), None);
        assert_eq!(experiment_of("/collect/1/a/b/json"), None);
        assert_eq!(experiment_of("/collect/2/abc/json"), None);
        assert_eq!(experiment_of(""), None);
    }

    #[test]
    fn each_announcement_kind_validates_against_the_contract() {
        // The sig is a placeholder here; the schema only checks its shape,
        // and the caller signs before sending.
        let h = head("q7k2m3x4w3b5n6tz", &"a1".repeat(32), "logos:zMan", 3, Some("logos:zIdx"), 2, "2026-10-29T08:00:00Z");
        assert_eq!(schema::errors_as(&h, schema::Kind::HeadAnnouncement), Vec::<String>::new());

        let s = submission("q7k2m3x4w3b5n6tz", &"a1".repeat(32), "logos:zSub", "wildlife-sighting", 2, "2026-10-29T08:00:00Z");
        assert_eq!(schema::errors_as(&s, schema::Kind::SubmissionAnnouncement), Vec::<String>::new());

        let r = receipt("q7k2m3x4w3b5n6tz", &"a1".repeat(32), "logos:zSub", 2, "2026-10-29T08:00:00Z");
        assert_eq!(schema::errors_as(&r, schema::Kind::ReceiptAnnouncement), Vec::<String>::new());
    }

    #[test]
    fn a_head_with_no_index_yet_is_still_valid() {
        // An experiment that has received nothing has no index page, and that
        // is a state the creator announces rather than withholds.
        let h = head("q7k2m3x4w3b5n6tz", &"a1".repeat(32), "logos:zMan", 1, None, 0, "2026-10-29T08:00:00Z");
        assert_eq!(h["index"], Value::Null);
        assert_eq!(schema::errors_as(&h, schema::Kind::HeadAnnouncement), Vec::<String>::new());
    }

    #[test]
    fn a_repeat_message_hash_is_not_accepted_twice() {
        // Store catch-up replays what was missed while the node was down, so
        // a creator that indexed every delivery would record the same
        // submission again after any restart.
        let mut seen = Seen::default();
        assert!(seen.accept("0xabc"));
        assert!(!seen.accept("0xabc"));
        assert!(seen.accept("0xdef"));
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn a_message_with_no_hash_is_handled_rather_than_dropped() {
        let mut seen = Seen::default();
        assert!(seen.accept(""));
        assert!(seen.accept(""));
    }

    #[test]
    fn the_seen_set_stays_bounded() {
        let mut seen = Seen::with_capacity(2);
        seen.accept("a");
        seen.accept("b");
        seen.accept("c");
        assert_eq!(seen.len(), 2);
        // "a" was evicted, so it looks new again — the cheaper mistake, and
        // index_accept still refuses a duplicate submission on its own.
        assert!(seen.accept("a"));
        assert!(!seen.accept("c"));
    }

    #[test]
    fn a_courier_without_rln_refuses_to_send_and_says_why() {
        let courier = MemoryCourier::without_rln();
        let error = courier.send("/collect/1/x/json", b"{}").expect_err("should refuse");
        assert_eq!(error.code(), "rln_not_ready");
        // Not retryable: waiting does not fund a membership.
        assert!(!error.retryable());
        assert!(error.to_string().contains("unregistered"), "{error}");
        assert!(!courier.rln().ready);
        assert!(courier.rln().required);
    }

    #[test]
    fn a_working_courier_records_what_it_sent() {
        let courier = MemoryCourier::new();
        courier.subscribe("/collect/1/x/json").unwrap();
        courier.subscribe("/collect/1/x/json").unwrap();
        assert_eq!(courier.subscriptions(), vec!["/collect/1/x/json".to_string()]);

        let id = courier.send("/collect/1/x/json", br#"{"kind":"head"}"#).unwrap();
        assert_eq!(id, "req-1");
        assert_eq!(courier.messages()[0]["kind"], json!("head"));
        assert_eq!(courier.topics(), vec!["/collect/1/x/json".to_string()]);
    }

    #[test]
    fn the_inbox_is_drained_by_reading_it() {
        // Read once: a second read must not replay what the session already
        // handled, or every drain would re-deliver the whole backlog.
        let courier = MemoryCourier::new();
        courier.arrive("/collect/1/x/json", "0xaaa", &json!({ "kind": "head" }));
        let first = courier.inbox();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0]["messageHash"], json!("0xaaa"));
        assert_eq!(first[0]["source"], json!("live"));
        // The payload is text, which is what `deliver` parses.
        assert_eq!(first[0]["payload"], json!("{\"kind\":\"head\"}"));
        assert!(courier.inbox().is_empty());
    }

    #[test]
    fn a_failing_courier_reports_a_code_and_whether_to_retry() {
        let courier = MemoryCourier::failing(DeliveryError::Timeout("no messageSent".into()));
        let error = courier.send("/collect/1/x/json", b"{}").expect_err("should fail");
        assert_eq!(error.code(), "delivery_timeout");
        assert!(error.retryable());
    }
}
