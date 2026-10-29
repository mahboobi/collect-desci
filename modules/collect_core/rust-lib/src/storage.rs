//! `collect_core_logic::store::DocumentStore` over the real `storage_module`.
//!
//! This is the half of publishing that cannot live in the logic crate: it
//! needs the Logos SDK, and anything that needs the SDK cannot be compiled by
//! `cargo test` (see this crate's `lib.rs`). So the logic names the capability
//! and this supplies it. It is deliberately thin — no document knowledge, no
//! canonicalization, no schema — because everything testable belongs on the
//! other side of the trait.
//!
//! Written against `docs/storage_module.lidl`, the contract the module itself
//! publishes (`nix build github:logos-co/logos-storage-module#lidl`), not
//! against the rendered API reference. Three things in it are easy to get
//! wrong from the prose alone:
//!
//! 1. **Every event payload is `tstr`** — a JSON *string*, so what arrives is
//!    text to parse, not an object to index. Reading it as an object yields
//!    null for every field, which looks exactly like a timed-out upload.
//! 2. **`downloadToUrl` is asynchronous too.** It returns a session id and
//!    completes on `storageDownloadDone`; the file is not there when the call
//!    returns.
//! 3. **The node has to be running.** `uploadUrl` on a node that was never
//!    `init()`-ed and `start()`-ed fails, and nothing else in Basecamp
//!    necessarily starts it.
//!
//! ## Why a temp file
//!
//! `uploadUrl` takes a path. The chunked alternative
//! (`uploadInit`/`uploadChunk`/`uploadFinalize`) would avoid the file and even
//! return the CID synchronously, but it moves each chunk as `tstr`, which is
//! wrong for the photo bytes this same store will carry later. One path for
//! documents and media is worth a file write. It goes in the host's
//! per-instance persistence directory — the only location we are promised is
//! writable — and is removed whatever happens.

use collect_core_logic::store::{DocumentStore, StoreError};
use logos_rust_sdk::{LogosModuleSDK, PluginProxy};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// The module we publish through.
const STORAGE_MODULE: &str = "storage_module";

/// Storage's chunk size, the contract's documented default.
const CHUNK_SIZE: i64 = 65536;

/// How long a single upload may wait before it is reported as a timeout.
///
/// **Under** the 20 s protocol default, deliberately. This runs inside a call
/// the UI is waiting on, so if it outlasts that default the UI gets no reply
/// at all — which is what "experiments.create: collect_core did not reply"
/// was. Failing first means the caller gets a real answer: a retryable
/// `storage_timeout`, with the document kept and `experiments.publish` able
/// to try again.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(12);

/// How often to look for the upload in `manifests()` while waiting.
const UPLOAD_POLL: Duration = Duration::from_millis(200);

/// How long a fetch may take. Longer than an upload on purpose: a cold fetch
/// crosses the DHT, and SPEC 3.3 asks for patience there.
const FETCH_TIMEOUT: Duration = Duration::from_secs(600);

/// How long to wait for the node to come up.
const START_TIMEOUT: Duration = Duration::from_secs(60);

/// How often to ask `isRunning` while waiting for the node to come up.
const START_POLL: Duration = Duration::from_millis(250);

/// Bound on the first call, which is only asking whether the module is there.
///
/// Well under the 20 s protocol default on purpose: this runs on the first UI
/// call, and when Storage is absent the whole connect has to fail fast enough
/// not to look like a hang. A present module answers `isRunning` immediately.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Everything the listener threads collect, under one lock.
#[derive(Default)]
struct Events {
    /// `storageUploadDone`: session id -> CID, or Storage's own message.
    uploads: BTreeMap<String, Result<String, String>>,
    /// `storageDownloadDone`: session id -> done, or Storage's own message.
    downloads: BTreeMap<String, Result<(), String>>,
    /// An upload completion carrying no session id. Kept because uploads are
    /// serialised here, so one that arrives mid-upload is almost certainly
    /// that upload's — see `claim_upload`.
    orphan_upload: Option<Result<String, String>>,
    /// `storageStart`, once.
    started: Option<Result<(), String>>,
    /// Set when a listener thread ends, so a waiter gives up immediately
    /// instead of sitting out the whole timeout.
    closed: bool,
}

type Shared = Arc<(Mutex<Events>, Condvar)>;

/// Logos Storage, as a document store.
pub struct LogosStorage {
    proxy: PluginProxy,
    /// Where upload and fetch temp files go.
    scratch: PathBuf,
    events: Shared,
    /// One transfer at a time. Storage accepts concurrent sessions; we have no
    /// need of them, and serialising is what makes the orphan fallback sound.
    transfer_lock: Mutex<()>,
}

impl LogosStorage {
    /// Connect to Storage, make sure the node is up, and start listening.
    ///
    /// Fails rather than degrading: a store that cannot report a CID is worse
    /// than no store, because the caller would believe publishing works and
    /// every document would come back as a timeout.
    pub fn connect(scratch: &Path) -> Result<Self, StoreError> {
        let mut proxy = LogosModuleSDK::new().plugin(STORAGE_MODULE);
        let events: Shared = Arc::new((Mutex::new(Events::default()), Condvar::new()));

        // Subscribed before anything is asked of the node: an event that
        // arrives between a call returning and a subscription being set up is
        // gone, and the transfer would look like a timeout.
        for (name, kind) in [
            ("storageUploadDone", Kind::Upload),
            ("storageDownloadDone", Kind::Download),
            ("storageStart", Kind::Start),
        ] {
            let subscription = proxy.on(name).map_err(|e| {
                StoreError::Unavailable(format!("cannot watch {name}: {e}"))
            })?;
            let sink = events.clone();
            std::thread::Builder::new()
                .name(format!("collect-{name}"))
                .spawn(move || {
                    // Iterating keeps the subscription alive; dropping it
                    // unsubscribes. The loop ends when the channel closes,
                    // which is the module going away.
                    while let Ok(event) = subscription.recv() {
                        let (lock, waiters) = &*sink;
                        if let Ok(mut state) = lock.lock() {
                            record(&mut state, kind, &payload_of(&event.data));
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
                    StoreError::Unavailable(format!("cannot start the {name} listener: {e}"))
                })?;
        }

        std::fs::create_dir_all(scratch)
            .map_err(|e| StoreError::Unavailable(format!("no scratch directory: {e}")))?;

        let storage = LogosStorage {
            proxy,
            scratch: scratch.to_path_buf(),
            events,
            transfer_lock: Mutex::new(()),
        };
        storage.ensure_running()?;
        Ok(storage)
    }

    /// Make sure the storage node is up, starting it if nobody else has.
    ///
    /// `init` is documented as once-per-instance, so a second call failing is
    /// expected rather than fatal — it means something already configured the
    /// node, and `start` is still the right next step.
    fn ensure_running(&self) -> Result<(), StoreError> {
        // Bounded, because this is also the probe for "is the module there at
        // all" and it runs on a UI call.
        if self
            .proxy
            .call_json_with_timeout("isRunning", &Value::Array(vec![]), PROBE_TIMEOUT)
            .map_err(|e| StoreError::Unavailable(format!("storage_module not reachable: {e}")))?
            .as_bool()
            .unwrap_or(false)
        {
            return Ok(());
        }

        // The module's own saved configuration, or its defaults — never one we
        // invent. It carries the network, the data directory and the quota,
        // and getting any of those wrong by hand would be a worse failure
        // than not publishing at all.
        let cfg = self
            .proxy
            .call_json("loadConfigOrDefault", &Value::Array(vec![]))
            .ok()
            .and_then(|r| result_value(&r).ok())
            .map(|v| match v {
                Value::String(s) => s,
                other => other.to_string(),
            })
            .unwrap_or_else(|| "{}".to_string());
        let _ = self.proxy.call_json("init", &serde_json::json!([cfg]));

        self.proxy
            .call_json("start", &Value::Array(vec![]))
            .map_err(|e| StoreError::Unavailable(format!("cannot start the storage node: {e}")))?;

        // Wait by **asking**, not only by listening.
        //
        // `storageStart` is the documented completion, and the first run of
        // this code waited for it for 60 s while the module log said
        // `NTC Started Storage node` within one second of the call. So the
        // event did not reach this subscription — plausibly because it fired
        // before the deferred subscription armed, which is a race no amount
        // of waiting fixes. `isRunning()` is synchronous and authoritative,
        // so it is the primary signal and the event is a shortcut when it
        // does arrive.
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            {
                let mut state = self.lock()?;
                if let Some(result) = state.started.take() {
                    return result.map_err(StoreError::Unavailable);
                }
                if state.closed {
                    return Err(StoreError::Unavailable("storage_module stopped".into()));
                }
            }
            if self
                .proxy
                .call_json_with_timeout("isRunning", &Value::Array(vec![]), PROBE_TIMEOUT)
                .ok()
                .and_then(|r| r.as_bool())
                .unwrap_or(false)
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(StoreError::Timeout(format!(
                    "the storage node was not running within {}s, and sent no storageStart",
                    START_TIMEOUT.as_secs()
                )));
            }
            std::thread::sleep(START_POLL);
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, Events>, StoreError> {
        self.events
            .0
            .lock()
            .map_err(|_| StoreError::Rejected("a storage listener panicked".into()))
    }

    /// Wait for a notification, or `None` once `deadline` has passed.
    fn wait<'a>(
        &'a self,
        state: MutexGuard<'a, Events>,
        deadline: Instant,
    ) -> Result<Option<MutexGuard<'a, Events>>, StoreError> {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(None);
        }
        let (next, timed_out) = self
            .events
            .1
            .wait_timeout(state, left)
            .map_err(|_| StoreError::Rejected("a storage listener panicked".into()))?;
        if timed_out.timed_out() && Instant::now() >= deadline {
            return Ok(None);
        }
        Ok(Some(next))
    }

    /// Wait for an upload to complete and give up its CID.
    ///
    /// Two signals, because the event alone is not reliable: `storageStart`
    /// never reached this process either, and the same deferred-subscription
    /// race applies to `storageUploadDone`. So the authoritative check is
    /// `manifests()`, which lists what this node holds along with each
    /// entry's `filename` — and the staged filename is ours, so the upload
    /// can be recognised there without knowing its CID in advance.
    fn claim_upload(&self, session: Option<&str>, filename: &str) -> Result<String, StoreError> {
        let deadline = Instant::now() + UPLOAD_TIMEOUT;
        loop {
            {
                let mut state = self.lock()?;
                if let Some(id) = session {
                    if let Some(result) = state.uploads.remove(id) {
                        return result.map_err(StoreError::Rejected);
                    }
                }
                // Otherwise the completion that arrived while we held the
                // transfer lock. Sound only because transfers are serialised.
                if let Some(result) = state.orphan_upload.take() {
                    return result.map_err(StoreError::Rejected);
                }
                if state.closed {
                    return Err(StoreError::Unavailable(
                        "storage_module stopped before the upload completed".into(),
                    ));
                }
            }
            if let Some(cid) = self.cid_in_manifests(filename) {
                return Ok(cid);
            }
            if Instant::now() >= deadline {
                return Err(StoreError::Timeout(format!(
                    "the upload was not in manifests() within {}s, and sent no storageUploadDone",
                    UPLOAD_TIMEOUT.as_secs()
                )));
            }
            std::thread::sleep(UPLOAD_POLL);
        }
    }

    /// The CID this node holds under `filename`, if any.
    fn cid_in_manifests(&self, filename: &str) -> Option<String> {
        let answer = self
            .proxy
            .call_json_with_timeout("manifests", &Value::Array(vec![]), PROBE_TIMEOUT)
            .ok()?;
        let listed = result_value(&answer).ok()?;
        // The value may arrive as a JSON array or as text holding one.
        let items = match &listed {
            Value::String(text) => serde_json::from_str::<Value>(text).ok()?,
            other => other.clone(),
        };
        items
            .as_array()?
            .iter()
            .find(|m| m.get("filename").and_then(Value::as_str) == Some(filename))
            .and_then(|m| m.get("cid").and_then(Value::as_str))
            .map(str::to_string)
    }

    /// Wait for `session`'s download to finish writing its file.
    fn claim_download(&self, session: &str) -> Result<(), StoreError> {
        let deadline = Instant::now() + FETCH_TIMEOUT;
        let mut state = self.lock()?;
        loop {
            if let Some(result) = state.downloads.remove(session) {
                return result.map_err(StoreError::Rejected);
            }
            if state.closed {
                return Err(StoreError::Unavailable(
                    "storage_module stopped before the download completed".into(),
                ));
            }
            state = match self.wait(state, deadline)? {
                Some(next) => next,
                None => {
                    return Err(StoreError::Timeout(format!(
                        "no storageDownloadDone in {}s",
                        FETCH_TIMEOUT.as_secs()
                    )))
                }
            };
        }
    }
}

/// Which event a listener thread is draining.
#[derive(Clone, Copy)]
enum Kind {
    Upload,
    Download,
    Start,
}

/// Unwrap the event's `payload`.
///
/// Every event in the contract is `event <name>(payload: tstr)`, so what
/// arrives is the argument array holding one JSON **string**. Indexing that as
/// an object gives null for every field, which is indistinguishable from a
/// transfer that never completed — so this parse is the difference between
/// working and silently timing out. An object arriving directly is still
/// accepted, in case a later contract stops wrapping it.
fn payload_of(data: &Value) -> Value {
    let first = match data {
        Value::Array(items) => items.first().cloned().unwrap_or(Value::Null),
        other => other.clone(),
    };
    match &first {
        Value::String(text) => serde_json::from_str(text).unwrap_or(Value::Null),
        _ => first,
    }
}

/// Read a `StdLogosResult` as JSON: its `value`, or its error.
fn result_value(value: &Value) -> Result<Value, String> {
    match value {
        Value::Object(map) => {
            if map.get("success").and_then(Value::as_bool) == Some(false) {
                let message = map
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("storage refused the call");
                return Err(message.to_string());
            }
            Ok(map.get("value").cloned().unwrap_or_else(|| value.clone()))
        }
        other => Ok(other.clone()),
    }
}

/// Record one completion event.
///
/// Tolerant where the shape is another module's to change, strict where a
/// wrong reading would be invisible: a `success` that is missing is inferred
/// from whether a CID came with it, but a success with no CID is a failure,
/// because publishing a document to an empty address is worse than reporting
/// that it did not publish.
fn record(state: &mut Events, kind: Kind, body: &Value) {
    let session = body
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let error = || {
        body.get("error")
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty())
            .map(str::to_string)
    };

    match kind {
        Kind::Upload => {
            let cid = body.get("cid").and_then(Value::as_str).unwrap_or_default();
            let success = body
                .get("success")
                .and_then(Value::as_bool)
                .unwrap_or(!cid.is_empty());
            let result = if success && !cid.is_empty() {
                Ok(cid.to_string())
            } else {
                Err(error().unwrap_or_else(|| format!("storageUploadDone carried no cid: {body}")))
            };
            if session.is_empty() {
                state.orphan_upload = Some(result);
            } else {
                state.uploads.insert(session, result);
            }
        }
        Kind::Download => {
            let success = body.get("success").and_then(Value::as_bool).unwrap_or(true);
            let result = if success {
                Ok(())
            } else {
                Err(error().unwrap_or_else(|| "the download failed".to_string()))
            };
            if !session.is_empty() {
                state.downloads.insert(session, result);
            }
        }
        Kind::Start => {
            let success = body.get("success").and_then(Value::as_bool).unwrap_or(true);
            state.started = Some(if success {
                Ok(())
            } else {
                // `storageStart` reports its reason in `message`, not `error`.
                Err(body
                    .get("message")
                    .and_then(Value::as_str)
                    .filter(|m| !m.is_empty())
                    .unwrap_or("the storage node did not start")
                    .to_string())
            });
        }
    }
}

/// Pull the session id out of what an upload or download call returned.
///
/// The contract says `StdLogosResult::value` is the session id string; the
/// JSON mapping of `StdLogosResult` is the part not pinned down, so both the
/// bare string and the wrapped object are accepted. `None` is not fatal for an
/// upload — the completion is then matched through the orphan path.
fn session_id_of(value: &Value) -> Option<String> {
    let unwrapped = result_value(value).ok()?;
    match &unwrapped {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Object(_) => ["sessionId", "session_id", "value", "id"]
            .iter()
            .find_map(|k| unwrapped.get(*k).and_then(Value::as_str))
            .map(str::to_string),
        _ => None,
    }
}

impl DocumentStore for LogosStorage {
    fn put(&self, filename: &str, bytes: &[u8]) -> Result<String, StoreError> {
        let _one_at_a_time = self
            .transfer_lock
            .lock()
            .map_err(|_| StoreError::Rejected("the transfer lock is poisoned".into()))?;

        let path = self.scratch.join(sanitize(filename));
        std::fs::write(&path, bytes)
            .map_err(|e| StoreError::Rejected(format!("cannot stage {}: {e}", path.display())))?;
        let staged = Staged(&path);

        // advertise = true: keeping a document available is exactly "the bytes
        // are in local storage, remove() is never called, and advertise stays
        // on" — there is no pin operation (SPEC 2).
        let args = serde_json::json!([path.to_string_lossy(), CHUNK_SIZE, true]);
        let accepted = self
            .proxy
            .call_json("uploadUrl", &args)
            .map_err(|e| StoreError::Unavailable(format!("uploadUrl failed: {e}")))?;
        if let Err(message) = result_value(&accepted) {
            return Err(StoreError::Rejected(format!("uploadUrl refused: {message}")));
        }

        let outcome = self.claim_upload(
            session_id_of(&accepted).as_deref(),
            &path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        );
        drop(staged);
        outcome
    }

    fn get(&self, cid: &str, local: bool) -> Result<Vec<u8>, StoreError> {
        let _one_at_a_time = self
            .transfer_lock
            .lock()
            .map_err(|_| StoreError::Rejected("the transfer lock is poisoned".into()))?;

        let path = self.scratch.join(format!("fetch-{}", sanitize(cid)));
        // downloadToUrl(cid, filePath, local, chunkSize, isPrivate, advertise)
        let args = serde_json::json!([
            cid,
            path.to_string_lossy(),
            local,
            CHUNK_SIZE,
            false,
            true
        ]);
        let accepted = self
            .proxy
            .call_json("downloadToUrl", &args)
            .map_err(|e| StoreError::Unavailable(format!("downloadToUrl failed: {e}")))?;
        if let Err(message) = result_value(&accepted) {
            return Err(StoreError::Rejected(format!("downloadToUrl refused: {message}")));
        }

        // The session id is the CID, per the contract — but take what the call
        // actually returned where it gave one.
        let session = session_id_of(&accepted).unwrap_or_else(|| cid.to_string());
        let staged = Staged(&path);
        let outcome = self.claim_download(&session).and_then(|()| {
            std::fs::read(&path)
                .map_err(|e| StoreError::Rejected(format!("nothing fetched for `{cid}`: {e}")))
        });
        drop(staged);
        outcome
    }

    fn describe(&self) -> String {
        format!("Logos Storage via `{STORAGE_MODULE}`")
    }
}

/// Removes its path when dropped, so a failed transfer leaves nothing behind.
struct Staged<'a>(&'a Path);

impl Drop for Staged<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0);
    }
}

/// Keep a caller-supplied label from escaping the scratch directory. The
/// filename is only a manifest label, so flattening it costs nothing.
fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let trimmed = cleaned.trim_matches('.');
    if trimmed.is_empty() {
        "document.json".to_string()
    } else {
        trimmed.chars().take(120).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// How the contract actually delivers an event: the argument array holds
    /// one JSON *string*.
    fn event(payload: Value) -> Value {
        json!([payload.to_string()])
    }

    #[test]
    fn an_event_payload_is_parsed_out_of_its_json_string() {
        // The bug this guards is silent: reading the wrapper as an object
        // gives null for every field, which reads as a timeout.
        let data = event(json!({ "success": true, "sessionId": "s1", "cid": "zAbc123" }));
        assert_eq!(payload_of(&data)["cid"], json!("zAbc123"));
    }

    #[test]
    fn an_unwrapped_object_payload_is_still_accepted() {
        let data = json!([{ "success": true, "sessionId": "s1", "cid": "zAbc123" }]);
        assert_eq!(payload_of(&data)["cid"], json!("zAbc123"));
    }

    #[test]
    fn an_unparseable_payload_becomes_null_rather_than_panicking() {
        assert_eq!(payload_of(&json!(["not json at all"])), Value::Null);
        assert_eq!(payload_of(&json!([])), Value::Null);
    }

    #[test]
    fn a_successful_upload_is_recorded_under_its_session() {
        let mut state = Events::default();
        let data = event(json!({ "success": true, "sessionId": "s1", "cid": "zAbc123" }));
        record(&mut state, Kind::Upload, &payload_of(&data));
        assert_eq!(state.uploads.remove("s1").unwrap(), Ok("zAbc123".to_string()));
        assert!(state.orphan_upload.is_none());
    }

    #[test]
    fn a_failed_upload_keeps_storages_own_message() {
        let mut state = Events::default();
        let data = event(json!({ "success": false, "sessionId": "s1", "error": "disk full" }));
        record(&mut state, Kind::Upload, &payload_of(&data));
        assert_eq!(state.uploads.remove("s1").unwrap(), Err("disk full".to_string()));
    }

    #[test]
    fn an_upload_success_without_a_cid_is_a_failure() {
        // Publishing a document to an empty address would be worse than
        // reporting that it did not publish.
        let mut state = Events::default();
        let data = event(json!({ "success": true, "sessionId": "s1", "cid": "" }));
        record(&mut state, Kind::Upload, &payload_of(&data));
        assert!(state.uploads.remove("s1").unwrap().is_err());
    }

    #[test]
    fn an_upload_completion_without_a_session_becomes_the_orphan() {
        let mut state = Events::default();
        record(&mut state, Kind::Upload, &payload_of(&event(json!({ "cid": "zAbc123" }))));
        assert!(state.uploads.is_empty());
        assert_eq!(state.orphan_upload.take().unwrap(), Ok("zAbc123".to_string()));
    }

    #[test]
    fn a_download_completion_carries_no_cid_only_its_session() {
        // storageDownloadDone has no `cid` field at all, so keying it on one
        // would never match.
        let mut state = Events::default();
        let data = event(json!({ "success": true, "sessionId": "zAbc123" }));
        record(&mut state, Kind::Download, &payload_of(&data));
        assert_eq!(state.downloads.remove("zAbc123").unwrap(), Ok(()));
    }

    #[test]
    fn a_failed_download_keeps_its_reason() {
        let mut state = Events::default();
        let data = event(json!({ "success": false, "sessionId": "z1", "error": "not found" }));
        record(&mut state, Kind::Download, &payload_of(&data));
        assert_eq!(state.downloads.remove("z1").unwrap(), Err("not found".to_string()));
    }

    #[test]
    fn a_start_failure_reports_message_not_error() {
        // storageStart's payload is { success, message } — reading `error`
        // here would throw away the only explanation there is.
        let mut state = Events::default();
        let data = event(json!({ "success": false, "message": "port in use" }));
        record(&mut state, Kind::Start, &payload_of(&data));
        assert_eq!(state.started.take().unwrap(), Err("port in use".to_string()));
    }

    #[test]
    fn a_start_success_is_recorded() {
        let mut state = Events::default();
        record(&mut state, Kind::Start, &payload_of(&event(json!({ "success": true }))));
        assert_eq!(state.started.take().unwrap(), Ok(()));
    }

    #[test]
    fn a_std_logos_result_gives_up_its_value_or_its_error() {
        assert_eq!(result_value(&json!({ "success": true, "value": "s1" })).unwrap(), json!("s1"));
        assert_eq!(
            result_value(&json!({ "success": false, "error": "nope" })),
            Err("nope".to_string())
        );
        // A bare value, for a method the mapping does not wrap.
        assert_eq!(result_value(&json!("s1")).unwrap(), json!("s1"));
        assert_eq!(result_value(&json!(true)).unwrap(), json!(true));
    }

    #[test]
    fn a_session_id_is_found_in_every_shape_the_mapping_might_use() {
        assert_eq!(session_id_of(&json!("s1")).as_deref(), Some("s1"));
        assert_eq!(session_id_of(&json!({ "value": "s1" })).as_deref(), Some("s1"));
        assert_eq!(session_id_of(&json!({ "success": true, "value": "s1" })).as_deref(), Some("s1"));
        assert_eq!(session_id_of(&json!({ "sessionId": "s1" })).as_deref(), Some("s1"));
        assert_eq!(session_id_of(&json!({ "nothing": 1 })), None);
        assert_eq!(session_id_of(&json!(null)), None);
        assert_eq!(session_id_of(&json!({ "success": false, "error": "no" })), None);
    }

    #[test]
    fn a_filename_cannot_escape_the_scratch_directory() {
        // Every separator is flattened, so the result cannot leave the
        // directory; the leading dots are then trimmed as well.
        assert_eq!(sanitize("../../etc/passwd"), "_.._etc_passwd");
        assert!(!sanitize("../../etc/passwd").contains('/'));
        assert_eq!(sanitize("experiment-q7k2.json"), "experiment-q7k2.json");
        assert_eq!(sanitize(""), "document.json");
        assert_eq!(sanitize("..."), "document.json");
        assert!(sanitize(&"x".repeat(500)).len() <= 120);
    }
}
