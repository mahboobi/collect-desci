//! `collect_core_logic::delivery::Courier` over the real `delivery_module`.
//!
//! Companion to `storage.rs`, and shaped differently for one reason worth
//! stating: **Delivery's events are not Storage's.** Storage declares every
//! event as `payload: tstr` — one JSON string to parse. Delivery declares
//! typed fields:
//!
//! ```text
//! event messageReceived(messageHash: tstr, contentTopic: tstr,
//!                       payload: bstr, source: tstr, timestamp: int)
//! ```
//!
//! so what arrives is a positional list, and `payload` is `bstr` — bytes,
//! base64 across the FFI boundary, decoded with the SDK's `bytes` helper. A
//! reader who assumed the Storage shape here would find null everywhere.
//!
//! Written against `docs/delivery_module.lidl` (delivery_module 0.3.3).
//!
//! ## Why sends are not awaited
//!
//! `send` returns a request id and completes through `messageQueued` /
//! `messagePropagated` / `messageSent` / `messageError`. We do not wait for
//! any of them, because none of them is the confirmation that matters: a
//! submitter re-announces until the **creator** sends a receipt (SPEC 3.2),
//! so `messageSent` would only tell us the network accepted a message whose
//! real acknowledgement is a different message entirely. What we do check
//! first is `rlnState()`, because that is the failure a user can act on.

use collect_core_logic::delivery::{Courier, DeliveryError, RlnStatus};
use logos_rust_sdk::{LogosModuleSDK, PluginProxy};
use serde_json::{json, Value};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// The module we announce through.
const DELIVERY_MODULE: &str = "delivery_module";

/// How long to wait for the node to come up.
const START_TIMEOUT: Duration = Duration::from_secs(90);

/// Bound on the first call, which is only asking whether the module is there.
/// See `storage.rs` — same reason, same number.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// How often to ask whether the node exists yet, while waiting.
const START_POLL: Duration = Duration::from_millis(500);

/// The network Collect runs on. `logos.test` is the testnet the pinned
/// basecamp targets, and the one whose RLN membership SPEC Q9 is about.
const NETWORK: &str = "logos.test";

/// Inbound messages, waiting for the session to drain them.
#[derive(Default)]
struct Inbox {
    messages: Vec<Value>,
    /// `nodeStarted`, once.
    started: Option<Result<(), String>>,
    closed: bool,
}

/// The cap on queued inbound messages.
///
/// Store catch-up can replay a long outage in one burst. Dropping the oldest
/// beyond this is safe in a way that unbounded growth is not: an announcement
/// is only a pointer, the creator re-publishes `head` on a timer, and a
/// submitter re-announces until they get a receipt — so a dropped message
/// comes back. Running the module out of memory does not.
const INBOX_CAP: usize = 10_000;

pub struct LogosDelivery {
    proxy: PluginProxy,
    inbox: Arc<(Mutex<Inbox>, Condvar)>,
}

impl LogosDelivery {
    /// Connect to Delivery, make sure the node is up, and start listening.
    pub fn connect() -> Result<Self, DeliveryError> {
        let mut proxy = LogosModuleSDK::new().plugin(DELIVERY_MODULE);
        let inbox: Arc<(Mutex<Inbox>, Condvar)> = Arc::new((Mutex::new(Inbox::default()), Condvar::new()));

        for (name, kind) in [
            ("messageReceived", Event::Received),
            ("nodeStarted", Event::Started),
            // Sends are not awaited (see the module docs), so this is the
            // only way a refused announcement is ever heard about. Without
            // it a send that RLN or the relay rejects simply vanishes —
            // and on a node whose RLN subsystem is Ready but whose
            // membership is not, every send does exactly that.
            ("messageError", Event::SendFailed),
            // The other half: a send that got out. Without it a node that
            // failed once would read as failing forever.
            ("messageSent", Event::Sent),
        ] {
            let subscription = proxy
                .on(name)
                .map_err(|e| DeliveryError::Unavailable(format!("cannot watch {name}: {e}")))?;
            let sink = inbox.clone();
            std::thread::Builder::new()
                .name(format!("collect-{name}"))
                .spawn(move || {
                    while let Ok(event) = subscription.recv() {
                        let (lock, waiters) = &*sink;
                        if let Ok(mut state) = lock.lock() {
                            record(&mut state, kind, &event.data);
                        }
                        waiters.notify_all();
                    }
                    let (lock, waiters) = &*sink;
                    if let Ok(mut state) = lock.lock() {
                        state.closed = true;
                    }
                    waiters.notify_all();
                })
                .map_err(|e| {
                    DeliveryError::Unavailable(format!("cannot start the {name} listener: {e}"))
                })?;
        }

        let delivery = LogosDelivery { proxy, inbox };
        delivery.ensure_running()?;
        Ok(delivery)
    }

    /// Create and start the node if nothing else has.
    ///
    /// `createNode` takes the layered config from the contract: `mode: Edge`
    /// because Collect is a client and has no business relaying other
    /// people's traffic, and `anonymityLevel` left at its default — mix would
    /// be a change to what the spec says about privacy, and SPEC 2 forbids
    /// adding that in v1 without asking.
    fn ensure_running(&self) -> Result<(), DeliveryError> {
        // Probe first, and **fail on a failed probe** rather than pressing on
        // to createNode and start. Those are a minute of timeouts against a
        // module that has already said it cannot be reached, and the first
        // run through this code spent exactly that.
        let status = self
            .proxy
            .call_json_with_timeout("getConnectionStatus", &Value::Array(vec![]), PROBE_TIMEOUT)
            .map_err(|e| DeliveryError::Unavailable(format!("delivery_module not reachable: {e}")))?;
        let already = result_text(&status)
            .ok()
            .map(|s| s.contains("onnected") && !s.contains("isconnected"))
            .unwrap_or(false);
        if already {
            return Ok(());
        }

        let cfg = json!({ "mode": "Edge", "preset": NETWORK }).to_string();
        // Already created is not an error: something else in Basecamp may own
        // this node, and `start` is still the right next step.
        let _ = self.proxy.call_json("createNode", &json!([cfg]));
        self.proxy
            .call_json("start", &Value::Array(vec![]))
            .map_err(|e| DeliveryError::Unavailable(format!("cannot start the delivery node: {e}")))?;

        // Wait by **asking**, like `storage.rs`, and for the same reason:
        // `nodeStarted` did not arrive. The first run waited the full 90 s
        // for it, failed, and then succeeded on the retry — so the node was
        // up all along and the event had simply been missed, most likely
        // fired before the deferred subscription armed.
        //
        // What is waited for is the node *existing*, not the node being
        // connected to peers. Until `createNode` has run, the module answers
        // `getConnectionStatus` with an error ("context not initialized");
        // once it answers at all, there is a node. Peer connectivity is a
        // separate, reported concern — announcements can be made before the
        // mesh settles, and refusing to attach until it has would leave the
        // UI saying Delivery is missing when it is merely young.
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            {
                let (lock, _) = &*self.inbox;
                let mut state = lock
                    .lock()
                    .map_err(|_| DeliveryError::Rejected("a delivery listener panicked".into()))?;
                if let Some(result) = state.started.take() {
                    return result.map_err(DeliveryError::Unavailable);
                }
                if state.closed {
                    return Err(DeliveryError::Unavailable("delivery_module stopped".into()));
                }
            }
            let exists = self
                .proxy
                .call_json_with_timeout("getConnectionStatus", &Value::Array(vec![]), PROBE_TIMEOUT)
                .ok()
                .map(|answer| result_value(&answer).is_ok())
                .unwrap_or(false);
            if exists {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(DeliveryError::Timeout(format!(
                    "the delivery node was not up within {}s, and sent no nodeStarted",
                    START_TIMEOUT.as_secs()
                )));
            }
            std::thread::sleep(START_POLL);
        }
    }
}

/// Which event a listener thread is draining.
#[derive(Clone, Copy)]
enum Event {
    Received,
    Started,
    SendFailed,
    Sent,
}

/// Record one event.
///
/// Positional, because that is how the contract declares them. Reading
/// `messageReceived` by name would find nothing.
fn record(state: &mut Inbox, kind: Event, data: &Value) {
    let args = match data {
        Value::Array(items) => items.clone(),
        other => vec![other.clone()],
    };
    let at = |i: usize| args.get(i).cloned().unwrap_or(Value::Null);

    match kind {
        // messageReceived(messageHash, contentTopic, payload, source, timestamp)
        Event::Received => {
            let payload = logos_rust_sdk::bytes::decode(&at(2))
                .or_else(|| logos_rust_sdk::bytes::decode_lenient(&at(2)))
                .unwrap_or_default();
            // Announcements are UTF-8 canonical JSON. Anything else is not
            // ours and is dropped here rather than queued for the session to
            // fail on.
            let Ok(text) = String::from_utf8(payload) else {
                return;
            };
            state.messages.push(json!({
                "messageHash": at(0).as_str().unwrap_or_default(),
                "topic": at(1).as_str().unwrap_or_default(),
                "payload": text,
                "source": at(3).as_str().unwrap_or("live"),
            }));
            // Oldest first: see INBOX_CAP.
            let overflow = state.messages.len().saturating_sub(INBOX_CAP);
            if overflow > 0 {
                state.messages.drain(0..overflow);
            }
        }
        // messageError(requestId, messageHash, error, timestamp)
        //
        // Queued as an inbox item rather than a separate channel, so the
        // session hears about it through the same drain as everything else.
        // Shaped so `deliver` cannot mistake it for a message: no topic.
        Event::SendFailed => {
            state.messages.push(json!({
                "kind": "sendFailed",
                "requestId": at(0).as_str().unwrap_or_default(),
                "messageHash": at(1).as_str().unwrap_or_default(),
                "error": at(2).as_str().unwrap_or("the network refused the message"),
            }));
        }
        // messageSent(requestId, messageHash, timestamp)
        Event::Sent => {
            state.messages.push(json!({
                "kind": "sent",
                "requestId": at(0).as_str().unwrap_or_default(),
                "messageHash": at(1).as_str().unwrap_or_default(),
            }));
        }
        // nodeStarted(success, message, timestamp)
        Event::Started => {
            let success = at(0).as_bool().unwrap_or(true);
            state.started = Some(if success {
                Ok(())
            } else {
                Err(at(1).as_str().unwrap_or("the delivery node did not start").to_string())
            });
        }
    }
}

/// A `StdLogosResult`'s value, or its error.
fn result_value(value: &Value) -> Result<Value, String> {
    match value {
        Value::Object(map) => {
            if map.get("success").and_then(Value::as_bool) == Some(false) {
                return Err(map
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("delivery refused the call")
                    .to_string());
            }
            Ok(map.get("value").cloned().unwrap_or_else(|| value.clone()))
        }
        other => Ok(other.clone()),
    }
}

/// A `StdLogosResult`'s value as text.
fn result_text(value: &Value) -> Result<String, String> {
    result_value(value).map(|v| match v {
        Value::String(s) => s,
        other => other.to_string(),
    })
}

/// Read `rlnState()`'s answer.
///
/// The contract documents `{"state": …, "message": …}` plus `registryId`,
/// `rlnIdentifier` and `epochSizeSec` "once a preset has enabled RLN". The
/// presence of a registry is therefore what says RLN applies at all — more
/// reliable than matching on state names, which are upstream's to rename.
fn read_rln(value: &Value) -> RlnStatus {
    let body = match result_value(value) {
        Ok(Value::String(text)) => serde_json::from_str(&text).unwrap_or(Value::Null),
        Ok(other) => other,
        Err(message) => {
            return RlnStatus {
                state: "unknown".into(),
                message,
                // Unknown is treated as required-and-not-ready: claiming a
                // send will work and having it silently fail is worse than
                // saying we cannot tell.
                required: true,
                ready: false,
            }
        }
    };
    let state = body
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let message = body
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let required = body
        .get("registryId")
        .and_then(Value::as_str)
        .map(|r| !r.is_empty())
        .unwrap_or(false);
    if !required {
        return RlnStatus { state, message, required: false, ready: true };
    }
    // Only a registered membership may publish. Matched case-insensitively on
    // a prefix because the state name is upstream's, and "Registered" versus
    // "registered" should not decide whether a submission can be announced.
    let ready = {
        let lower = state.to_ascii_lowercase();
        lower.starts_with("registered") || lower.starts_with("ready") || lower.starts_with("active")
    };
    RlnStatus { state, message, required: true, ready }
}

impl Courier for LogosDelivery {
    fn send(&self, topic: &str, payload: &[u8]) -> Result<String, DeliveryError> {
        // Checked first: RLN is the one send failure a user can do something
        // about, and it would otherwise only surface as an async messageError.
        let rln = self.rln();
        if !rln.ready {
            return Err(DeliveryError::RlnNotReady { state: rln.state, message: rln.message });
        }
        let args = json!([topic, logos_rust_sdk::bytes::encode(payload)]);
        let accepted = self
            .proxy
            .call_json("send", &args)
            .map_err(|e| DeliveryError::Unavailable(format!("send failed: {e}")))?;
        result_text(&accepted).map_err(DeliveryError::Rejected)
    }

    fn subscribe(&self, topic: &str) -> Result<(), DeliveryError> {
        let accepted = self
            .proxy
            .call_json("subscribe", &json!([topic]))
            .map_err(|e| DeliveryError::Unavailable(format!("subscribe failed: {e}")))?;
        result_value(&accepted)
            .map(|_| ())
            .map_err(DeliveryError::Rejected)
    }

    fn rln(&self) -> RlnStatus {
        match self.proxy.call_json("rlnState", &Value::Array(vec![])) {
            Ok(answer) => read_rln(&answer),
            Err(e) => RlnStatus {
                state: "unknown".into(),
                message: format!("rlnState failed: {e}"),
                required: true,
                ready: false,
            },
        }
    }

    fn connection(&self) -> Option<String> {
        let answer = self
            .proxy
            .call_json("getConnectionStatus", &Value::Array(vec![]))
            .ok()?;
        result_text(&answer).ok()
    }

    fn inbox(&self) -> Vec<Value> {
        let (lock, _) = &*self.inbox;
        lock.lock()
            .map(|mut state| std::mem::take(&mut state.messages))
            .unwrap_or_default()
    }

    fn describe(&self) -> String {
        format!("Logos Delivery via `{DELIVERY_MODULE}` on {NETWORK}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How the contract delivers `messageReceived`: positional, with `payload`
    /// base64-encoded across the FFI boundary.
    fn received(hash: &str, topic: &str, body: &str, source: &str) -> Value {
        json!([hash, topic, logos_rust_sdk::bytes::encode(body.as_bytes()), source, 1_762_000_000])
    }

    #[test]
    fn a_received_message_is_read_positionally_and_its_payload_decoded() {
        // Reading these by name — as Storage's single-string payload would
        // need — finds nothing at all.
        let mut inbox = Inbox::default();
        record(
            &mut inbox,
            Event::Received,
            &received("0xabc", "/collect/1/q7k2m3x4w3b5n6tz/json", r#"{"kind":"head"}"#, "history"),
        );
        assert_eq!(inbox.messages.len(), 1);
        let m = &inbox.messages[0];
        assert_eq!(m["messageHash"], json!("0xabc"));
        assert_eq!(m["topic"], json!("/collect/1/q7k2m3x4w3b5n6tz/json"));
        assert_eq!(m["payload"], json!(r#"{"kind":"head"}"#));
        assert_eq!(m["source"], json!("history"));
    }

    #[test]
    fn a_payload_that_is_not_text_is_dropped_rather_than_queued() {
        let mut inbox = Inbox::default();
        record(
            &mut inbox,
            Event::Received,
            &json!(["0xabc", "/collect/1/x/json", logos_rust_sdk::bytes::encode(&[0xff, 0xfe]), "live", 0]),
        );
        assert!(inbox.messages.is_empty());
    }

    #[test]
    fn the_inbox_stays_bounded() {
        let mut inbox = Inbox::default();
        for n in 0..(INBOX_CAP + 5) {
            record(
                &mut inbox,
                Event::Received,
                &received(&format!("0x{n}"), "/collect/1/x/json", "{}", "live"),
            );
        }
        assert_eq!(inbox.messages.len(), INBOX_CAP);
        // The oldest went, not the newest: the newest is the one most likely
        // to still matter.
        assert_eq!(inbox.messages[0]["messageHash"], json!("0x5"));
    }

    #[test]
    fn a_refused_send_lands_in_the_inbox_with_its_reason() {
        // The only way a refused announcement is ever heard about.
        let mut inbox = Inbox::default();
        record(
            &mut inbox,
            Event::SendFailed,
            &json!(["req-7", "0xabc", "rln: no usable membership", 1_762_000_000]),
        );
        assert_eq!(inbox.messages.len(), 1);
        let m = &inbox.messages[0];
        assert_eq!(m["kind"], json!("sendFailed"));
        assert_eq!(m["requestId"], json!("req-7"));
        assert_eq!(m["error"], json!("rln: no usable membership"));
        // Not mistakable for a received message.
        assert!(m.get("topic").is_none());
    }

    #[test]
    fn a_node_start_failure_reports_its_message() {
        let mut inbox = Inbox::default();
        record(&mut inbox, Event::Started, &json!([false, "no peers", 0]));
        assert_eq!(inbox.started.take().unwrap(), Err("no peers".to_string()));

        let mut inbox = Inbox::default();
        record(&mut inbox, Event::Started, &json!([true, "", 0]));
        assert_eq!(inbox.started.take().unwrap(), Ok(()));
    }

    #[test]
    fn rln_is_not_required_when_no_registry_is_reported() {
        let status = read_rln(&json!({ "success": true, "value": { "state": "disabled", "message": "" } }));
        assert!(!status.required);
        assert!(status.ready);
    }

    #[test]
    fn rln_with_a_registry_gates_sending_on_the_state() {
        let registered = read_rln(&json!({
            "state": "Registered", "message": "ok",
            "registryId": "logos.test", "rlnIdentifier": "1", "epochSizeSec": 600
        }));
        assert!(registered.required);
        assert!(registered.ready, "{registered:?}");

        let unregistered = read_rln(&json!({
            "state": "unregistered", "message": "no funded membership",
            "registryId": "logos.test", "rlnIdentifier": "1", "epochSizeSec": 600
        }));
        assert!(unregistered.required);
        assert!(!unregistered.ready);
        assert_eq!(unregistered.message, "no funded membership");
    }

    #[test]
    fn an_rln_answer_delivered_as_a_json_string_is_still_read() {
        let text = json!({
            "state": "Registered", "message": "", "registryId": "logos.test"
        })
        .to_string();
        let status = read_rln(&json!({ "success": true, "value": text }));
        assert!(status.required);
        assert!(status.ready);
    }

    #[test]
    fn an_unreadable_rln_answer_is_not_ready_rather_than_assumed_fine() {
        // Claiming a send will work and having it silently fail is worse than
        // saying we cannot tell.
        let status = read_rln(&json!({ "success": false, "error": "no node" }));
        assert!(status.required);
        assert!(!status.ready);
        assert_eq!(status.message, "no node");
    }

    #[test]
    fn a_result_gives_up_its_value_or_its_error() {
        assert_eq!(result_text(&json!({ "success": true, "value": "req-1" })).unwrap(), "req-1");
        assert_eq!(result_text(&json!("req-1")).unwrap(), "req-1");
        assert_eq!(
            result_value(&json!({ "success": false, "error": "nope" })),
            Err("nope".to_string())
        );
    }
}
