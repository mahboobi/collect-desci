//! The module's live state: which experiments this node knows about.
//!
//! Separate from `api` because everything there is a pure `&str -> String`.
//! Experiments accumulate as a creator makes them, so that needs somewhere to
//! live — and it lives here rather than in the module shim so it can be
//! tested without a node.
//!
//! In fixtures mode the embedded example is seeded as experiment one, and
//! anything created joins it in memory. Nothing is uploaded: publishing needs
//! Logos Storage (SPEC Q9/Q12), so `publish` is deliberately absent rather
//! than faked.

use crate::delivery::{self, Courier};
use crate::store::{self, DocumentStore};
use crate::social::{Received, Social};
use crate::{api, export, fields, identity::Identity, ids, schema};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

/// How an experiment reads on the dashboard.
///
/// Derived from the dates rather than stored, so a card cannot disagree with
/// the document it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    Draft,
    Open,
    Ended,
}

impl Standing {
    pub fn as_str(self) -> &'static str {
        match self {
            Standing::Draft => "draft",
            Standing::Open => "open",
            Standing::Ended => "ended",
        }
    }
}

/// What became of a document's upload.
///
/// Absent from the map means never attempted — a different thing from failed,
/// and the UI says so differently ("saved on this node" versus "could not
/// publish"). Keyed by document id rather than stored inside the document,
/// because a CID is taken over the document's own bytes and so cannot be one
/// of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Publication {
    /// On Logos Storage, at this CID.
    Published(String),
    /// Tried, and did not get there.
    Failed { code: String, message: String },
}

impl Publication {
    pub fn cid(&self) -> Option<&str> {
        match self {
            Publication::Published(cid) => Some(cid),
            Publication::Failed { .. } => None,
        }
    }

    /// How it reads in a response: `published`, plus the CID or the reason.
    fn as_json(&self) -> Value {
        match self {
            Publication::Published(cid) => json!({ "published": true, "cid": cid }),
            Publication::Failed { code, message } => json!({
                "published": false,
                "publish_error": { "code": code, "message": message },
            }),
        }
    }
}

/// What this node knows.
pub struct Session {
    /// Experiment manifests, newest last.
    experiments: Vec<Value>,
    /// Protocol documents, by id.
    protocols: Vec<Value>,
    /// Submissions this node holds, newest last.
    submissions: Vec<Value>,
    /// Index pages, newest last. The durable record of what was received.
    index_pages: Vec<Value>,
    /// This node's identity. Fixed in fixtures mode so the embedded example
    /// reads as "created by you"; otherwise the real signing key.
    creator_key: String,
    /// The signing key, once the host has given us somewhere to keep it.
    /// `None` means documents cannot be signed yet and are not pretended to
    /// be — `create` refuses rather than emitting a placeholder.
    identity: Option<Identity>,
    /// Logos Storage, if the host gave us a working client. `None` means
    /// documents stay on this node; they are still signed and still valid,
    /// they just have no address anyone else could fetch.
    store: Option<Box<dyn DocumentStore>>,
    /// Upload outcome per document id. See `Publication`.
    publications: BTreeMap<String, Publication>,
    /// How long the current call may go on spending on uploads. See
    /// `start_publish_budget`.
    publish_until: Option<std::time::Instant>,
    /// The host's per-instance directory, once we have been given one. Where
    /// exports are written, because the QML side cannot write files at all
    /// and this is the only place we are promised is writable.
    state_dir: Option<std::path::PathBuf>,
    /// Logos Delivery, if the host gave us a working client.
    courier: Option<Box<dyn Courier>>,
    /// Message hashes already handled, so Store catch-up replaying a
    /// restart's worth of traffic does not index everything twice.
    seen: delivery::Seen,
    /// Receipts we have been sent for our own submissions, by document CID.
    receipts: BTreeMap<String, Value>,
    /// The latest `head` each experiment's creator announced, by experiment.
    /// How a submitter learns there is a newer manifest or index to fetch.
    heads: BTreeMap<String, Value>,
    /// Profile, project members and discussion. Never touches Storage.
    social: Social,
    /// The state file's text as last written or read, so a call that changed
    /// nothing does not rewrite it. See `save_state`.
    saved_state: Option<String>,
    /// Thumbnails ready to show, as data URIs by sha256. Filled from the
    /// cache directory on first use, so a list does not re-read the files.
    thumbs: std::cell::RefCell<BTreeMap<String, String>>,
    /// Thumbnails already looked for on the network this run, by sha256, so
    /// one that cannot be found is not asked for on every drain.
    thumbs_tried: std::collections::BTreeSet<String>,
    /// Why the network last refused one of our messages, if nothing has got
    /// out since. Delivery's `rlnState` says `Ready` once its RLN bridge
    /// answers, which is not the same as holding a membership — and nothing
    /// in its API reports the membership — so whether sends get through is
    /// learned from how they end.
    send_failure: Option<String>,
    /// The connection level last reported, so a change is logged once.
    last_level: std::cell::RefCell<String>,
    /// The creator's verdict on each received submission, by CID:
    /// `approved` or `rejected`; absent is pending. Private to this node —
    /// it decides who is paid, and is nobody else's business until then.
    approvals: BTreeMap<String, String>,
    /// Every reward payment ever started, oldest first. Written to disk
    /// *before* each transfer is sent, so a crash mid-send leaves a record
    /// that stops the same submissions being paid twice.
    payouts: Vec<Value>,
    /// The `lez_core` wallet, once a rewards call has needed it.
    ledger: Option<Box<dyn crate::rewards::Ledger>>,
    /// This node's submissions to other people's experiments, until each
    /// one's creator sends a receipt (SPEC 3.2). Re-announced from the drain
    /// with growing gaps, and saved, so a restart does not forget one.
    outbox: Vec<Value>,
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}

impl Session {
    pub fn new() -> Self {
        let mut session = Session {
            experiments: Vec::new(),
            protocols: Vec::new(),
            submissions: Vec::new(),
            index_pages: Vec::new(),
            // The example experiment's creator, so the embedded fixture reads
            // as "created by you" and the creator screens have something to
            // open.
            creator_key: "3a9d0f213a9d0f213a9d0f213a9d0f213a9d0f213a9d0f213a9d0f213a9d0f21".into(),
            identity: None,
            store: None,
            publications: BTreeMap::new(),
            publish_until: None,
            state_dir: None,
            saved_state: None,
            thumbs: Default::default(),
            thumbs_tried: Default::default(),
            send_failure: None,
            last_level: Default::default(),
            approvals: BTreeMap::new(),
            payouts: Vec::new(),
            ledger: None,
            outbox: Vec::new(),
            courier: None,
            seen: delivery::Seen::default(),
            receipts: BTreeMap::new(),
            heads: BTreeMap::new(),
            social: Social::default(),
        };
        if api::fixtures_enabled() {
            session.seed_fixtures();
        }
        session
    }

    fn seed_fixtures(&mut self) {
        if let Ok(experiment) = serde_json::from_str::<Value>(api::fixture_experiment()) {
            self.experiments.push(experiment);
        }
        if let Ok(protocol) = serde_json::from_str::<Value>(api::fixture_protocol()) {
            self.protocols.push(protocol);
        }
        if let Ok(submission) = serde_json::from_str::<Value>(api::fixture_submission()) {
            self.submissions.push(submission);
        }
        if let Ok(page) = serde_json::from_str::<Value>(api::fixture_index()) {
            self.index_pages.push(page);
        }
    }

    /// `results.list` — the creator's table for one protocol.
    ///
    /// Columns follow the protocol's fields, in protocol order, because the
    /// table is of that form's answers and nothing else can define its shape
    /// (SPEC 3.3). Hidden submitters are dropped here, so results, the table
    /// and any export all read through one rule.
    pub fn results(&self, raw: &str) -> String {
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let Some(experiment_id) = request.get("experimentId").and_then(Value::as_str) else {
            return api::envelope_err(api::code::BAD_REQUEST, "`experimentId` is required");
        };
        let Some(protocol_id) = request.get("protocolId").and_then(Value::as_str) else {
            return api::envelope_err(api::code::BAD_REQUEST, "`protocolId` is required");
        };

        let Some(protocol) = self
            .protocols
            .iter()
            .find(|p| p.get("id").and_then(Value::as_str) == Some(protocol_id))
        else {
            return api::envelope_err(
                api::code::BAD_REQUEST,
                format!("no protocol `{protocol_id}` on this node"),
            );
        };

        let columns: Vec<Value> = match fields::parse_fields(protocol) {
            Ok(parsed) => parsed
                .iter()
                .map(|f| {
                    // The options travel with the column so a cell can show a
                    // label rather than the stored value. A value with no
                    // option left means it was removed in a later version;
                    // the cell falls back to the raw value, because the
                    // submission still holds it.
                    let options = match &f.config {
                        fields::Config::Choice { options }
                        | fields::Config::Multi { options, .. } => Value::Array(
                            options
                                .iter()
                                .map(|o| json!({ "value": o.value, "label": o.label }))
                                .collect(),
                        ),
                        _ => Value::Null,
                    };
                    json!({
                        "key": f.key,
                        "label": f.label,
                        "type": f.ty.as_str(),
                        "options": options
                    })
                })
                .collect(),
            Err(e) => return api::envelope_err(api::code::INVALID_PROTOCOL, e.to_string()),
        };

        // Who the creator currently hides. From the newest page for this
        // experiment, not the union of all of them — see
        // `indexer::hidden_keys`.
        let hidden = self.hidden_for(experiment_id);

        // The index entry for a submission.
        //
        // Joined on submitter plus submitted_at, because a submission
        // document does not carry its own CID — it is content-addressed, so
        // the CID only exists once Storage has it, and the index is where the
        // creator recorded it. That also makes the index the only place
        // received_at lives; the submission only knows when it was filled in.
        let entry_for = |submitter: &str, submitted_at: &str| -> Option<&Value> {
            self.index_pages
                .iter()
                .filter_map(|p| p.get("entries").and_then(Value::as_array))
                .flatten()
                .find(|e| {
                    e.get("submitter").and_then(Value::as_str) == Some(submitter)
                        && e.get("submitted_at").and_then(Value::as_str) == Some(submitted_at)
                })
        };

        let mut rows: Vec<Value> = Vec::new();
        let mut submitters: Vec<String> = Vec::new();
        for submission in &self.submissions {
            if submission.get("experiment").and_then(Value::as_str) != Some(experiment_id) {
                continue;
            }
            if submission.pointer("/protocol/id").and_then(Value::as_str) != Some(protocol_id) {
                continue;
            }
            let submitter = submission
                .pointer("/submitter/key")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if hidden.contains(&submitter) {
                continue;
            }
            if !submitters.contains(&submitter) {
                submitters.push(submitter.clone());
            }

            let submitted_at = submission
                .get("submitted_at")
                .and_then(Value::as_str)
                .unwrap_or("");
            let entry = entry_for(&submitter, submitted_at);
            let cid = entry
                .and_then(|e| e.get("cid").and_then(Value::as_str))
                .unwrap_or("")
                .to_string();
            let received = entry
                .and_then(|e| e.get("received_at").and_then(Value::as_str))
                .unwrap_or("")
                .to_string();
            rows.push(json!({
                "id": submission.get("id").and_then(Value::as_str).unwrap_or(""),
                "cid": cid,
                "submitter": submitter,
                "submitter_name": submission.pointer("/submitter/name").and_then(Value::as_str).unwrap_or(""),
                // Empty unless the submitter chose to include it.
                "submitter_wallet": submission.get("wallet").and_then(Value::as_str).unwrap_or(""),
                "version": submission.pointer("/protocol/version").and_then(Value::as_u64).unwrap_or(0),
                "submitted_at": submitted_at,
                // Empty when the index has no entry yet: received but not
                // indexed is a real state, and blanking it says so.
                "received_at": received,
                "values": submission.get("values").cloned().unwrap_or_else(|| json!({})),
                // pending until the creator decides; a row with no CID yet
                // cannot be decided on, since the CID is what is approved.
                "approval": self.approvals.get(&cid).map(String::as_str).unwrap_or("pending"),
                "paid": !cid.is_empty() && self.paid_cids(experiment_id).contains(&cid),
            }));
        }

        // What the index says was received, which may exceed what this node
        // has fetched — the difference is worth showing rather than hiding.
        let indexed = self
            .index_pages
            .iter()
            .filter_map(|p| p.get("entries").and_then(Value::as_array))
            .flatten()
            .filter(|e| e.get("protocol").and_then(Value::as_str) == Some(protocol_id))
            .filter(|e| {
                e.get("submitter")
                    .and_then(Value::as_str)
                    .map(|k| !hidden.contains(&k.to_string()))
                    .unwrap_or(false)
            })
            .count();

        api::envelope_ok(json!({
            "protocol": { "id": protocol_id, "name": protocol.get("name").and_then(Value::as_str).unwrap_or("") },
            "columns": columns,
            "rows": rows,
            // The keys themselves, not just the count: without them the UI
            // can say someone is hidden but offers no way to undo it.
            "hidden_keys": hidden,
            "summary": {
                "shown": rows.len(),
                "indexed": indexed,
                "submitters": submitters.len(),
                "hidden": hidden.len(),
                // Held locally vs named in the index: a creator who has not
                // fetched everything should know.
                "not_fetched": indexed.saturating_sub(rows.len()),
            }
        }))
    }

    pub fn creator_key(&self) -> &str {
        &self.creator_key
    }

    /// Open the signing key from the host's per-instance persistence path.
    ///
    /// Called from `on_context_ready`, which is the first moment the path
    /// exists. Outside fixtures mode the real key also becomes the creator
    /// identity, so experiments are attributed to whoever actually signed
    /// them rather than to a constant.
    pub fn attach_identity(&mut self, persistence_path: &str) -> Result<String, String> {
        let identity = Identity::load_or_create(Path::new(persistence_path))
            .map_err(|e| e.to_string())?;
        let public = identity.public_hex();
        self.state_dir = Some(Path::new(persistence_path).to_path_buf());
        self.identity = Some(identity);
        // Logged, not fatal: a profile that will not load must not stop the
        // node signing, and it is never overwritten.
        if let Err(e) = self.social.attach(Path::new(persistence_path)) {
            eprintln!("collect_core: profile and discussion unavailable: {e}");
        }
        self.load_state();
        self.creator_key = public.clone();
        if api::fixtures_enabled() {
            self.reauthor_fixtures();
        }
        Ok(public)
    }

    /// Read back what an earlier run saved.
    ///
    /// Experiments used to live only in memory, so every relaunch — and every
    /// module update — started the dashboard empty although the documents
    /// were still on Storage. Each document is re-checked against the schema
    /// on the way in, so a damaged or hand-edited file drops that document
    /// rather than putting an invalid one on screen. A file that will not
    /// parse is moved aside, never overwritten, and the run starts empty.
    fn load_state(&mut self) {
        if api::fixtures_enabled() {
            return;
        }
        let Some(dir) = self.state_dir.clone() else {
            return;
        };
        let path = dir.join(STATE_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                eprintln!("collect_core: cannot read {}: {e}", path.display());
                return;
            }
        };
        let state: Value = match serde_json::from_str(&text) {
            Ok(state) => state,
            Err(e) => {
                let aside = dir.join(format!("{STATE_FILE}.unreadable"));
                eprintln!(
                    "collect_core: {} is not JSON ({e}); moved to {}",
                    path.display(),
                    aside.display()
                );
                let _ = std::fs::rename(&path, &aside);
                return;
            }
        };
        let docs = |key: &str, kind: schema::Kind| -> Vec<Value> {
            let mut kept = Vec::new();
            for doc in state[key].as_array().into_iter().flatten() {
                let errors = schema::errors_as(doc, kind);
                if errors.is_empty() {
                    kept.push(doc.clone());
                } else {
                    eprintln!("collect_core: dropping a saved {key} entry: {}", errors.join("; "));
                }
            }
            kept
        };
        self.experiments = docs("experiments", schema::Kind::Experiment);
        self.protocols = docs("protocols", schema::Kind::Protocol);
        self.submissions = docs("submissions", schema::Kind::Submission);
        self.index_pages = docs("index_pages", schema::Kind::Index);
        // Only what reached Storage is kept: a failure is reported once, and
        // after a restart the document is offered for publishing again.
        if let Some(map) = state["published"].as_object() {
            for (id, cid) in map {
                if let Some(cid) = cid.as_str() {
                    self.publications.insert(id.clone(), Publication::Published(cid.to_string()));
                }
            }
        }
        for (field, key) in [(&mut self.receipts, "receipts"), (&mut self.heads, "heads")] {
            if let Some(map) = state[key].as_object() {
                field.extend(map.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
        }
        if let Some(map) = state["approvals"].as_object() {
            for (cid, verdict) in map {
                if let Some(v) = verdict.as_str().filter(|v| *v == "approved" || *v == "rejected") {
                    self.approvals.insert(cid.clone(), v.to_string());
                }
            }
        }
        self.payouts = state["payouts"].as_array().cloned().unwrap_or_default();
        self.outbox = state["outbox"].as_array().cloned().unwrap_or_default();
        for hash in state["seen"].as_array().into_iter().flatten() {
            if let Some(hash) = hash.as_str() {
                self.seen.accept(hash);
            }
        }
        self.saved_state = Some(text);
        eprintln!(
            "collect_core: restored {} experiment(s), {} form(s), {} submission(s)",
            self.experiments.len(),
            self.protocols.len(),
            self.submissions.len()
        );
    }

    /// Write what this node knows, if it changed since the last write.
    ///
    /// Called after every call into the module, so no method has to remember
    /// to. The documents are all signed and public already; the file is kept
    /// owner-only anyway, like every file here. Written to a temporary name
    /// and renamed, so a crash mid-write leaves the previous state intact.
    pub fn save_state(&mut self) {
        if api::fixtures_enabled() {
            return;
        }
        let Some(dir) = self.state_dir.clone() else {
            return;
        };
        let published: serde_json::Map<String, Value> = self
            .publications
            .iter()
            .filter_map(|(id, p)| p.cid().map(|cid| (id.clone(), json!(cid))))
            .collect();
        let state = json!({
            "version": 1,
            "experiments": self.experiments,
            "protocols": self.protocols,
            "submissions": self.submissions,
            "index_pages": self.index_pages,
            "published": published,
            "receipts": self.receipts,
            "heads": self.heads,
            "seen": self.seen.hashes().collect::<Vec<_>>(),
            "approvals": self.approvals,
            "payouts": self.payouts,
            "outbox": self.outbox,
        });
        let Ok(text) = serde_json::to_string(&state) else {
            return;
        };
        if self.saved_state.as_deref() == Some(text.as_str()) {
            return;
        }
        let path = dir.join(STATE_FILE);
        let temp = dir.join(format!("{STATE_FILE}.tmp"));
        let written = crate::profile::write_private(&temp, text.as_bytes())
            .and_then(|_| std::fs::rename(&temp, &path));
        match written {
            Ok(()) => self.saved_state = Some(text),
            Err(e) => eprintln!("collect_core: cannot save {}: {e}", path.display()),
        }
    }

    /// Re-sign the seeded example under this node's key.
    ///
    /// Fixtures mode used to keep the example's own creator key so it read as
    /// "created by you" on the dashboard. It read that way and was not: the
    /// example is signed by `3a9d0f21…`, a key nobody holds — and which is
    /// not even a point on the curve — so every creator action on it failed
    /// at the signature. Adding a form to the example reported
    ///
    ///   this node cannot sign for `3a9d0f21…`
    ///
    /// which is accurate and useless, because there was nothing the person
    /// could do about it.
    ///
    /// So the example becomes genuinely ours instead of nominally ours. The
    /// submissions are left alone: they are other people's, and re-signing
    /// them would turn every contributor into this node.
    fn reauthor_fixtures(&mut self) {
        let key = self.creator_key.clone();
        for i in 0..self.experiments.len() {
            self.experiments[i]["creator"]["key"] = json!(key);
            // And drop the example's author name. Leaving it would put a
            // stranger's name against this node's key — the screen said
            // "Nisha Kumar" over a key that is ours, which is worse than
            // saying nothing. Blank reads as "Unnamed creator", exactly like
            // a new experiment before a name is set.
            self.experiments[i]["creator"]["name"] = json!("");
            if let Ok(signed) = self.signed(&self.experiments[i].clone()) {
                self.experiments[i] = signed;
            }
        }
        // Protocols and index pages carry no author key of their own — they
        // are checked against the manifest's creator — so they only need the
        // new signature.
        for i in 0..self.protocols.len() {
            if let Ok(signed) = self.signed(&self.protocols[i].clone()) {
                self.protocols[i] = signed;
            }
        }
        for i in 0..self.index_pages.len() {
            if let Ok(signed) = self.signed(&self.index_pages[i].clone()) {
                self.index_pages[i] = signed;
            }
        }
    }

    pub fn can_sign(&self) -> bool {
        self.identity.is_some()
    }

    /// Sign a document, or say why not.
    ///
    /// Never falls back to an unsigned document: an unsigned one would be
    /// rejected by every reader anyway, and emitting it would make a failure
    /// look like a success.
    fn signed(&self, doc: &Value) -> Result<Value, String> {
        match &self.identity {
            Some(identity) => identity.sign(doc).map_err(|e| e.to_string()),
            None => Err("no signing key yet: the module context is not ready".to_string()),
        }
    }

    /// Hand the session a Logos Storage client.
    ///
    /// Called from `on_context_ready` once the dependency proxy exists. Until
    /// then — and in fixtures mode, and in every test that does not ask for
    /// one — there is no store, and documents are kept locally without
    /// pretending otherwise.
    pub fn attach_store(&mut self, store: Box<dyn DocumentStore>) {
        self.store = Some(store);
    }

    /// Whether documents created from here on can get a real address.
    pub fn can_publish(&self) -> bool {
        self.store.is_some()
    }

    /// What this node is publishing to, for `session.capabilities`.
    pub fn store_description(&self) -> Option<String> {
        self.store.as_ref().map(|s| s.describe())
    }

    /// The upload outcome for a document id, if one was ever attempted.
    pub fn publication(&self, id: &str) -> Option<&Publication> {
        self.publications.get(id)
    }

    /// The CID a document sits at, if it got one.
    fn cid_of(&self, id: &str) -> Option<&str> {
        self.publications.get(id).and_then(Publication::cid)
    }

    /// Give this call a budget for uploading.
    ///
    /// One method can touch several documents — saving a form publishes the
    /// form *and* the re-signed manifest, hiding a submitter publishes every
    /// changed index page *and* the manifest — and each upload is bounded
    /// separately. Two eight-second bounds exceed the twenty-second protocol
    /// timeout on the call the UI is waiting on, and when that passes the UI
    /// gets no reply at all: no result, no error, a button stuck on
    /// "Saving…". That is what "forms are not saving, no error" was.
    ///
    /// So the whole call gets one budget. Documents that do not fit are kept,
    /// signed and valid, and reported as retryable — `experiments.publish`
    /// picks them up — which is the contract a Storage timeout already had.
    fn start_publish_budget(&mut self) {
        self.publish_until = Some(std::time::Instant::now() + PUBLISH_BUDGET);
    }

    /// Whether there is still time to upload within this call.
    fn may_publish(&self) -> bool {
        match self.publish_until {
            Some(until) => std::time::Instant::now() < until,
            // No budget means a caller that publishes a single document, so
            // that document's own bound is the whole story.
            None => true,
        }
    }

    /// Publish a signed document and remember what happened.
    ///
    /// `None` means no Storage is attached, so nothing was attempted — which
    /// is reported differently from a failure, because a creator can do
    /// nothing about the first and may want to retry the second.
    /// `key` is how the outcome is looked up later. Passed in rather than
    /// read off the document because an index page has no `id` — it is
    /// identified by its experiment and page number — and keying those all on
    /// the empty string would have them overwrite each other.
    fn publish_doc(&mut self, kind: &str, key: &str, doc: &Value) -> Option<Publication> {
        let id = key.to_string();
        if !self.may_publish() {
            let outcome = Publication::Failed {
                code: "storage_timeout".to_string(),
                message: "not uploaded yet: this call ran out of time. It is saved, and publishing can be retried.".to_string(),
            };
            self.publications.insert(id, outcome.clone());
            return Some(outcome);
        }
        // Scoped so the immutable borrow of `store` is over before the map is
        // written.
        let result = {
            let store = self.store.as_deref()?;
            store::publish(store, kind, doc)
        };
        let outcome = match result {
            Ok(cid) => Publication::Published(cid),
            Err(e) => Publication::Failed { code: e.code().to_string(), message: e.to_string() },
        };
        self.publications.insert(id, outcome.clone());
        Some(outcome)
    }

    /// How a publish attempt reads in a response, including the never-tried
    /// case. One shape for every method that writes a document, so the UI has
    /// a single thing to render.
    fn publish_report(&self, id: &str) -> Value {
        match self.publications.get(id) {
            Some(p) => p.as_json(),
            None => json!({
                "published": false,
                "note": "Saved on this node only — no Logos Storage is attached to this build.",
            }),
        }
    }

    /// Re-sign and re-publish a manifest whose `protocols` list changed.
    ///
    /// A signed document cannot be edited: changing one byte invalidates the
    /// signature, so an amended manifest is a *new version* of it rather than
    /// the same one patched. `version` goes up, `prev` points at the CID the
    /// previous version was published to — which is what makes the chain in
    /// SPEC 4 walkable — and the whole document is signed again. Without
    /// this, attaching a form would upload a manifest whose signature no
    /// longer verifies, and every reader would reject it.
    fn reseal(&mut self, experiment_id: &str) -> Result<Value, String> {
        self.reseal_with(experiment_id, |_| {})
    }

    /// `reseal`, with an edit applied to the document first.
    fn reseal_with<F>(&mut self, experiment_id: &str, edit: F) -> Result<Value, String>
    where
        F: FnOnce(&mut Value),
    {
        let Some(at) = self
            .experiments
            .iter()
            .position(|e| e.get("id").and_then(Value::as_str) == Some(experiment_id))
        else {
            return Err(format!("no experiment `{experiment_id}` on this node"));
        };

        let prev = match self.cid_of(experiment_id) {
            Some(cid) => Value::String(cid.to_string()),
            None => Value::Null,
        };
        let mut doc = self.experiments[at].clone();
        edit(&mut doc);
        let version = doc.get("version").and_then(Value::as_u64).unwrap_or(1) + 1;
        doc["version"] = json!(version);
        doc["prev"] = prev;
        // Replaced by the signature over the new content.
        doc["sig"] = json!("00".repeat(64));

        let doc = self.signed(&doc)?;
        let errors = schema::errors_as(&doc, schema::Kind::Experiment);
        if !errors.is_empty() {
            return Err(format!(
                "the updated experiment does not validate: {}",
                errors.join("; ")
            ));
        }

        // The signature must verify against the key the document itself
        // names. It does not follow from signing succeeding: we sign with
        // whatever identity this node has, and an experiment authored by
        // someone else — the embedded fixture, or one fetched from another
        // creator — names a key we do not hold. Resealing that would produce
        // a document no reader could verify, and nothing downstream would
        // notice, because `sig` is just hex to the contract.
        if let Err(e) = crate::sign::verify(&doc) {
            return Err(format!(
                "this node cannot sign for `{}`: {e}",
                doc.pointer("/creator/key").and_then(Value::as_str).unwrap_or("?")
            ));
        }

        self.experiments[at] = doc.clone();
        self.publish_doc("experiment", experiment_id, &doc);
        Ok(doc)
    }

    fn standing(experiment: &Value, now: DateTime<Utc>) -> Standing {
        let parse = |key: &str| {
            experiment
                .get(key)
                .and_then(Value::as_str)
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.with_timezone(&Utc))
        };
        match (parse("starts_at"), parse("ends_at")) {
            (Some(from), Some(to)) => {
                if now < from {
                    // Not open yet. The dashboard calls this a draft, which is
                    // what a creator who has not published sees too.
                    Standing::Draft
                } else if now > to {
                    Standing::Ended
                } else {
                    Standing::Open
                }
            }
            _ => Standing::Draft,
        }
    }

    /// One dashboard card.
    fn card(&self, experiment: &Value, now: DateTime<Utc>) -> Value {
        let id = experiment.get("id").and_then(Value::as_str).unwrap_or("");
        let creator = experiment
            .pointer("/creator/key")
            .and_then(Value::as_str)
            .unwrap_or("");
        let protocols = experiment
            .get("protocols")
            .and_then(Value::as_array)
            .map(|a| a.len())
            .unwrap_or(0);
        let open_protocols = experiment
            .get("protocols")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter(|p| p.get("status").and_then(Value::as_str) == Some("open"))
                    .count()
            })
            .unwrap_or(0);

        json!({
            "id": id,
            "title": experiment.get("title").and_then(Value::as_str).unwrap_or(""),
            "description": experiment.get("description").and_then(Value::as_str).unwrap_or(""),
            "starts_at": experiment.get("starts_at").and_then(Value::as_str).unwrap_or(""),
            "ends_at": experiment.get("ends_at").and_then(Value::as_str).unwrap_or(""),
            "timezone": experiment.get("timezone").and_then(Value::as_str).unwrap_or(""),
            "creator": experiment.get("creator").cloned().unwrap_or(Value::Null),
            "standing": Self::standing(experiment, now).as_str(),
            "protocol_count": protocols,
            "open_protocol_count": open_protocols,
            // Which side of the app this card belongs to. The dashboard shows
            // both, in separate sections.
            "mine": creator == self.creator_key,
            // A link is only shareable once the manifest has an address; with
            // no Storage it still carries `logos:pending`, which the detail
            // screen shows as not yet publishable rather than hiding.
            "link": ids::link(id, self.cid_of(id).unwrap_or("logos:pending")),
            "published": self.cid_of(id).is_some(),
            "cid": self.cid_of(id).map(Value::from).unwrap_or(Value::Null),
            "thumbnail": self.thumbnail_uri(experiment).map(Value::from).unwrap_or(Value::Null),
        })
    }

    /// `experiments.list` — every experiment this node knows, as cards.
    pub fn list(&self, now: DateTime<Utc>) -> String {
        let cards: Vec<Value> = self
            .experiments
            .iter()
            .map(|e| self.card(e, now))
            .collect();
        let mine = cards.iter().filter(|c| c["mine"] == json!(true)).count();
        api::envelope_ok(json!({
            "experiments": cards,
            "mine": mine,
            "joined": cards.len() - mine,
            "creator_key": self.creator_key,
        }))
    }

    /// `experiments.get` — one manifest plus the protocol documents it names.
    pub fn get(&self, raw: &str) -> String {
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let wanted = match request.get("experimentId").and_then(Value::as_str) {
            Some(i) => i,
            None => {
                return api::envelope_err(api::code::BAD_REQUEST, "`experimentId` is required")
            }
        };
        let Some(experiment) = self
            .experiments
            .iter()
            .find(|e| e.get("id").and_then(Value::as_str) == Some(wanted))
        else {
            return api::envelope_err(
                api::code::BAD_REQUEST,
                format!("no experiment `{wanted}` on this node"),
            );
        };

        // Only the protocols we hold a document for. A manifest may name more
        // than this node has fetched, and the UI has to be able to tell.
        let named: Vec<&str> = experiment
            .get("protocols")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|p| p.get("id").and_then(Value::as_str))
                    .collect()
            })
            .unwrap_or_default();
        let held: Vec<Value> = self
            .protocols
            .iter()
            .filter(|p| {
                p.get("id")
                    .and_then(Value::as_str)
                    .map(|id| named.contains(&id))
                    .unwrap_or(false)
            })
            .cloned()
            .collect();

        let mut data = json!({
            "experiment": experiment,
            "protocols": held,
            "missing_protocols": named.len() - held.len(),
            // Ready to show, or null: none, or not on this node yet.
            "thumbnail": self.thumbnail_uri(experiment).map(Value::from).unwrap_or(Value::Null),
        });
        merge(&mut data, self.publish_report(wanted));
        api::envelope_ok(data)
    }

    // --------------------------------------------------------- joining, outbox

    /// `experiments.open {link}` — join an experiment from its share link.
    ///
    /// Fetches the manifest the link names from Logos Storage, and every form
    /// it lists, and keeps them only if each one is valid and signed by the
    /// creator the manifest names. Then this node watches the experiment's
    /// topic, so receipts and newer heads reach it. A form that cannot be
    /// fetched yet is reported, not fatal: the experiment is still kept, and
    /// opening the link again tries the rest.
    pub fn open_link(&mut self, raw: &str) -> String {
        if api::fixtures_enabled() {
            return api::experiments_open(raw);
        }
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let link = request.get("link").and_then(Value::as_str).unwrap_or("").trim().to_string();
        let Some((id, manifest_cid)) = ids::parse_link(&link) else {
            return api::envelope_err(
                api::code::BAD_REQUEST,
                "that is not a Collect link; it starts with logos-collect://exp/",
            );
        };
        let manifest_cid = match store::as_document_cid(&manifest_cid) {
            Ok(c) => c,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, e.to_string()),
        };
        // Already ours: nothing to fetch.
        if let Some(ours) = self.experiments.iter().find(|e| e.get("id").and_then(Value::as_str) == Some(&id)) {
            if ours.pointer("/creator/key").and_then(Value::as_str) == Some(&self.creator_key) {
                return self.get(&json!({ "experimentId": id }).to_string());
            }
        }
        let Some(store) = self.store.as_deref() else {
            return api::envelope_err(
                api::code::NOT_WIRED,
                "Logos Storage is not reachable, and the experiment has to be fetched from it",
            );
        };

        let fetch = |cid: &str| -> Result<Value, String> {
            let bytes = store
                .get(store::bare_cid(cid), false)
                .map_err(|e| format!("could not fetch {cid}: {e}"))?;
            serde_json::from_slice(&bytes).map_err(|e| format!("{cid} is not JSON: {e}"))
        };

        let manifest = match fetch(&manifest_cid) {
            Ok(m) => m,
            Err(e) => {
                return api::envelope_err(
                    api::code::REJECTED,
                    format!("{e}. The creator's node, or another that holds it, has to be online."),
                )
            }
        };
        let errors = schema::errors_as(&manifest, schema::Kind::Experiment);
        if !errors.is_empty() {
            return api::envelope_err(
                api::code::INVALID_DOCUMENT,
                format!("the experiment at that link is not valid: {}", errors.join("; ")),
            );
        }
        if manifest.get("id").and_then(Value::as_str) != Some(&id) {
            return api::envelope_err(
                api::code::INVALID_DOCUMENT,
                "the document at that link is a different experiment than the link names",
            );
        }
        let creator_key = match crate::sign::verify(&manifest) {
            Ok(k) => k,
            Err(e) => {
                return api::envelope_err(
                    api::code::INVALID_DOCUMENT,
                    format!("the experiment's signature does not check out: {e}"),
                )
            }
        };

        // Each form, checked the same way and against the same creator.
        let mut forms: Vec<Value> = Vec::new();
        let mut missing: Vec<Value> = Vec::new();
        for entry in manifest.get("protocols").and_then(Value::as_array).cloned().unwrap_or_default() {
            let pid = entry.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let cid = entry.get("cid").and_then(Value::as_str).unwrap_or("").to_string();
            let checked = fetch(&cid).and_then(|doc| {
                let errors = schema::errors_as(&doc, schema::Kind::Protocol);
                if !errors.is_empty() {
                    return Err(format!("not a valid form: {}", errors.join("; ")));
                }
                if doc.get("id").and_then(Value::as_str) != Some(&pid)
                    || doc.get("experiment").and_then(Value::as_str) != Some(&id)
                {
                    return Err("a different form than the experiment lists".into());
                }
                crate::sign::verify_with(&doc, &creator_key)
                    .map_err(|e| format!("not signed by the experiment's creator: {e}"))?;
                Ok(doc)
            });
            match checked {
                Ok(doc) => forms.push(doc),
                Err(e) => missing.push(json!({ "id": pid, "reason": e })),
            }
        }

        // Keep the newer of what we had and what we fetched.
        let version = |d: &Value| d.get("version").and_then(Value::as_u64).unwrap_or(0);
        match self.experiments.iter_mut().find(|e| e.get("id").and_then(Value::as_str) == Some(&id)) {
            Some(existing) if version(existing) >= version(&manifest) => {}
            Some(existing) => *existing = manifest.clone(),
            None => self.experiments.push(manifest.clone()),
        }
        self.publications.insert(id.clone(), Publication::Published(manifest_cid.clone()));
        for form in forms {
            let pid = form.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            match self.protocols.iter_mut().find(|p| p.get("id").and_then(Value::as_str) == Some(&pid)) {
                Some(existing) if version(existing) >= version(&form) => {}
                Some(existing) => *existing = form,
                None => self.protocols.push(form),
            }
        }
        if let Some(courier) = self.courier.as_deref() {
            if let Err(e) = courier.subscribe(&delivery::topic(&id)) {
                eprintln!("collect_core: cannot watch {id}: {e}");
            }
        }

        let mut data: Value = serde_json::from_str(&self.get(&json!({ "experimentId": id }).to_string()))
            .ok()
            .and_then(|v: Value| v.get("data").cloned())
            .unwrap_or(Value::Null);
        data["opened"] = json!(true);
        data["missing_forms"] = json!(missing);
        api::envelope_ok(data)
    }

    /// Announce outbox entry `at` once, and record how that went.
    fn announce_outbox(&mut self, at: usize, now: DateTime<Utc>) -> Value {
        let entry = self.outbox[at].clone();
        let text = |k: &str| entry.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let experiment = text("experiment");
        let announcement = delivery::submission(
            &experiment,
            &self.creator_key.clone(),
            &text("cid"),
            &text("protocol"),
            entry.get("version").and_then(Value::as_u64).unwrap_or(1),
            &now.to_rfc3339(),
        );
        let outcome = self.announce(&experiment, &announcement, schema::Kind::SubmissionAnnouncement);
        let record = &mut self.outbox[at];
        record["attempts"] = json!(record["attempts"].as_u64().unwrap_or(0) + 1);
        record["last_attempt"] = json!(now.to_rfc3339());
        match outcome {
            Ok(request_id) => {
                record["last_error"] = Value::Null;
                record["last_request"] = json!(request_id);
                json!({ "sent": true })
            }
            Err(envelope) => {
                let message = serde_json::from_str::<Value>(&envelope)
                    .ok()
                    .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string))
                    .unwrap_or_else(|| "the announcement could not be sent".into());
                record["last_error"] = json!(message);
                json!({ "sent": false, "error": message })
            }
        }
    }

    /// Re-announce whatever is still waiting and due. The gap doubles after
    /// each try, from `OUTBOX_FIRST_GAP` up to `OUTBOX_MAX_GAP`, so a creator
    /// who is away for days is not flooded and one who is back soon hears
    /// soon. Returns how many were sent.
    fn resend_outbox(&mut self, now: DateTime<Utc>) -> usize {
        let mut sent = 0;
        for at in 0..self.outbox.len() {
            if self.outbox[at]["state"] != json!("waiting") {
                continue;
            }
            let attempts = self.outbox[at]["attempts"].as_u64().unwrap_or(0) as u32;
            let gap = OUTBOX_FIRST_GAP
                .checked_mul(1 << attempts.saturating_sub(1).min(8))
                .unwrap_or(OUTBOX_MAX_GAP)
                .min(OUTBOX_MAX_GAP);
            let due = self.outbox[at]["last_attempt"]
                .as_str()
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                .map(|last| now >= last.with_timezone(&Utc) + gap)
                .unwrap_or(true);
            if due && self.announce_outbox(at, now)["sent"] == json!(true) {
                sent += 1;
            }
        }
        sent
    }

    /// `submissions.mine {experimentId?}` — this node's submissions to other
    /// people's experiments and whether each creator has confirmed it.
    pub fn submissions_mine(&self, raw: &str) -> String {
        let request: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
        let only = request.get("experimentId").and_then(Value::as_str);
        let rows: Vec<Value> = self
            .outbox
            .iter()
            .filter(|e| only.is_none_or(|id| e["experiment"] == json!(id)))
            .cloned()
            .collect();
        api::envelope_ok(json!({ "submissions": rows }))
    }

    /// `experiments.setThumbnail {experimentId, path}` — give an experiment a
    /// cover image.
    ///
    /// The photo is cleaned and cropped like any other (`media::thumbnail`),
    /// put on Storage, and named in a new version of the manifest, so everyone
    /// with the link sees it. Storage is required rather than optional here:
    /// a manifest may only name media by CID, and a thumbnail nobody else
    /// can fetch would be one only this screen shows.
    pub fn set_thumbnail(&mut self, raw: &str) -> String {
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let id = request.get("experimentId").and_then(Value::as_str).unwrap_or("").to_string();
        if let Err(e) = self.may_edit(&id) {
            return e;
        }
        let path = local_path(request.get("path").and_then(Value::as_str).unwrap_or(""));
        if path.is_empty() {
            return api::envelope_err(api::code::BAD_REQUEST, "`path` is required");
        }
        let original = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => return api::envelope_err(api::code::MEDIA, format!("cannot read the image: {e}")),
        };
        let thumb = match crate::media::thumbnail(&original) {
            Ok(t) => t,
            Err(e) => return api::envelope_err(api::code::MEDIA, e.to_string()),
        };
        let Some(store) = self.store.as_deref() else {
            return api::envelope_err(
                api::code::NOT_WIRED,
                "Logos Storage is not reachable, and a thumbnail has to be on Storage for anyone else to see it",
            );
        };
        let cid = match store
            .put(&format!("thumbnail-{id}.jpg"), &thumb.bytes)
            .and_then(|c| store::as_document_cid(&c))
        {
            Ok(cid) => cid,
            Err(e) => return api::envelope_err(e.code(), format!("the thumbnail was not stored: {e}")),
        };
        self.cache_thumbnail(&thumb.sha256, &thumb.bytes);
        let entry = thumb.media_entry(&cid);
        if let Err(e) = self.reseal_with(&id, |doc| doc["thumbnail"] = entry) {
            return api::envelope_err(api::code::REJECTED, e);
        }
        self.get(&json!({ "experimentId": id }).to_string())
    }

    /// `media.addPhoto {path, maxMbEach}` — one photo for a form's photo field.
    ///
    /// SPEC section 5 steps 1 to 4: read the location and time, remove every
    /// piece of metadata, put the cleaned file on Storage, and hand back the
    /// media entry the submission will carry, plus what was read so the form
    /// can prefill `location` and `datetime` where the protocol allows. The
    /// original never leaves this function.
    pub fn add_photo(&mut self, raw: &str) -> String {
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let path = local_path(request.get("path").and_then(Value::as_str).unwrap_or(""));
        if path.is_empty() {
            return api::envelope_err(api::code::BAD_REQUEST, "`path` is required");
        }
        let max_mb = request
            .get("maxMbEach")
            .and_then(Value::as_f64)
            .unwrap_or(fields::PHOTO_DEFAULT_MAX_MB);
        let original = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => return api::envelope_err(api::code::MEDIA, format!("cannot read the photo: {e}")),
        };
        let cleaned = match crate::media::process(&original, max_mb) {
            Ok(c) => c,
            Err(e) => return api::envelope_err(api::code::MEDIA, e.to_string()),
        };
        let Some(store) = self.store.as_deref() else {
            return api::envelope_err(
                api::code::NOT_WIRED,
                "Logos Storage is not reachable, and a photo has to be on Storage for the submission to name it",
            );
        };
        let ext = match cleaned.mime {
            "image/png" => "png",
            "image/webp" => "webp",
            _ => "jpg",
        };
        let cid = match store
            .put(&format!("photo-{}.{ext}", &cleaned.sha256[..16]), &cleaned.bytes)
            .and_then(|c| store::as_document_cid(&c))
        {
            Ok(cid) => cid,
            Err(e) => return api::envelope_err(e.code(), format!("the photo was not stored: {e}")),
        };
        let (location, datetime) = crate::media::prefills(&cleaned.exif);
        let name = Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        api::envelope_ok(json!({
            "entry": cleaned.media_entry(&cid),
            "exif": { "location": location, "datetime": datetime },
            "name": name,
            "reencoded": cleaned.reencoded,
        }))
    }

    /// `experiments.clearThumbnail {experimentId}` — back to no cover image.
    ///
    /// A new manifest version without the field. The image itself stays on
    /// Storage, like everything published: nothing there can be withdrawn.
    pub fn clear_thumbnail(&mut self, raw: &str) -> String {
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let id = request.get("experimentId").and_then(Value::as_str).unwrap_or("").to_string();
        if let Err(e) = self.may_edit(&id) {
            return e;
        }
        let has = self
            .experiments
            .iter()
            .any(|e| e.get("id").and_then(Value::as_str) == Some(&id) && e.get("thumbnail").is_some());
        if has {
            if let Err(e) = self.reseal_with(&id, |doc| {
                if let Some(map) = doc.as_object_mut() {
                    map.remove("thumbnail");
                }
            }) {
                return api::envelope_err(api::code::REJECTED, e);
            }
        }
        self.get(&json!({ "experimentId": id }).to_string())
    }

    /// Whether this node may make a new version of `id`, as an error envelope
    /// if not. Checked before any work, so a refused thumbnail is not
    /// uploaded first.
    fn may_edit(&self, id: &str) -> Result<(), String> {
        let Some(experiment) = self
            .experiments
            .iter()
            .find(|e| e.get("id").and_then(Value::as_str) == Some(id))
        else {
            return Err(api::envelope_err(
                api::code::BAD_REQUEST,
                format!("no experiment `{id}` on this node"),
            ));
        };
        if experiment.pointer("/creator/key").and_then(Value::as_str) != Some(self.creator_key.as_str()) {
            return Err(api::envelope_err(
                api::code::REJECTED,
                "only the experiment's creator can change it",
            ));
        }
        Ok(())
    }

    /// The experiment's thumbnail as a data URI the UI can show, if this node
    /// holds it.
    ///
    /// Never touches the network: this runs for every card in a list. A
    /// thumbnail not held yet is fetched by `fetch_thumbnails` on the drain.
    /// Whatever is shown must hash to the `sha256` the signed manifest names,
    /// and be the image type it says.
    fn thumbnail_uri(&self, experiment: &Value) -> Option<String> {
        let entry = experiment.get("thumbnail")?;
        let sha = entry.get("sha256").and_then(Value::as_str)?;
        if let Some(uri) = self.thumbs.borrow().get(sha) {
            return Some(uri.clone());
        }
        let path = self.state_dir.as_ref()?.join(THUMB_DIR).join(format!("{sha}.img"));
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => {
                // Held by the local Storage node, as our own uploads are.
                let cid = entry.get("cid").and_then(Value::as_str)?;
                let bytes = self.store.as_deref()?.get(store::bare_cid(cid), true).ok()?;
                if !thumbnail_matches(entry, &bytes) {
                    return None;
                }
                self.cache_thumbnail(sha, &bytes);
                bytes
            }
        };
        if !thumbnail_matches(entry, &bytes) {
            return None;
        }
        let mime = entry.get("mime").and_then(Value::as_str)?;
        let uri = format!("data:{mime};base64,{}", data_encoding::BASE64.encode(&bytes));
        self.thumbs.borrow_mut().insert(sha.to_string(), uri.clone());
        Some(uri)
    }

    /// Keep a thumbnail's bytes beside the state, by sha256.
    fn cache_thumbnail(&self, sha: &str, bytes: &[u8]) {
        let Some(dir) = self.state_dir.as_ref().map(|d| d.join(THUMB_DIR)) else {
            return;
        };
        let written = std::fs::create_dir_all(&dir)
            .and_then(|_| std::fs::write(dir.join(format!("{sha}.img")), bytes));
        if let Err(e) = written {
            eprintln!("collect_core: cannot cache a thumbnail: {e}");
        }
    }

    /// Fetch, from the network, thumbnails named by experiments this node
    /// holds but has not got the image for — a submitter's case. Run from the
    /// drain, which already waits on the network, and at most a couple per
    /// run so one slow fetch cannot hold the drain up for long. Returns how
    /// many arrived, so the UI knows to reload the list.
    fn fetch_thumbnails(&mut self) -> usize {
        let Some(store) = self.store.as_deref() else {
            return 0;
        };
        let wanted: Vec<Value> = self
            .experiments
            .iter()
            .filter_map(|e| e.get("thumbnail").cloned())
            .filter(|t| {
                t.get("sha256")
                    .and_then(Value::as_str)
                    .map(|sha| !self.thumbs_tried.contains(sha) && !self.thumbs.borrow().contains_key(sha))
                    .unwrap_or(false)
            })
            .take(THUMB_FETCHES_PER_DRAIN)
            .collect();
        let mut fetched = 0;
        for entry in wanted {
            let (Some(sha), Some(cid)) = (
                entry.get("sha256").and_then(Value::as_str),
                entry.get("cid").and_then(Value::as_str),
            ) else {
                continue;
            };
            self.thumbs_tried.insert(sha.to_string());
            let cached = self
                .state_dir
                .as_ref()
                .map(|d| d.join(THUMB_DIR).join(format!("{sha}.img")).exists())
                .unwrap_or(false);
            if cached {
                continue;
            }
            match store.get(store::bare_cid(cid), false) {
                Ok(bytes) if thumbnail_matches(&entry, &bytes) => {
                    self.cache_thumbnail(sha, &bytes);
                    fetched += 1;
                }
                Ok(_) => eprintln!("collect_core: thumbnail {cid} does not match its manifest; not shown"),
                Err(e) => eprintln!("collect_core: thumbnail {cid} not fetched: {e}"),
            }
        }
        fetched
    }

    // ------------------------------------------------------------- rewards

    pub fn attach_ledger(&mut self, ledger: Box<dyn crate::rewards::Ledger>) {
        self.ledger = Some(ledger);
    }

    pub fn has_ledger(&self) -> bool {
        self.ledger.is_some()
    }

    /// `experiments.setReward {experimentId, amount, maxPerSubmitter}` — offer
    /// (or with no amount, withdraw) a reward per approved submission.
    ///
    /// A new manifest version, so it is public: submitters see the offer
    /// before they take part. Refused once any reward has been paid, since
    /// changing the terms after paying some people would be unfair to them
    /// or to everyone else.
    pub fn set_reward(&mut self, raw: &str) -> String {
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let id = request.get("experimentId").and_then(Value::as_str).unwrap_or("").to_string();
        if let Err(e) = self.may_edit(&id) {
            return e;
        }
        if !self.paid_cids(&id).is_empty() {
            return api::envelope_err(
                api::code::REJECTED,
                "rewards have already been paid for this experiment, so its reward can no longer change",
            );
        }
        let amount = request.get("amount").and_then(Value::as_str).unwrap_or("").trim().to_string();
        let reward = if amount.is_empty() {
            None
        } else {
            let lepta = match crate::rewards::parse_tokens(&amount) {
                Ok(l) => l,
                Err(problem) => {
                    return api::envelope_err_data(
                        api::code::BAD_REQUEST,
                        problem.clone(),
                        json!({ "problems": { "amount": problem } }),
                    )
                }
            };
            let cap = request.get("maxPerSubmitter").and_then(Value::as_u64).unwrap_or(1);
            if cap == 0 || cap > crate::rewards::MAX_PER_SUBMITTER_CEILING {
                let problem = format!("Between 1 and {}", crate::rewards::MAX_PER_SUBMITTER_CEILING);
                return api::envelope_err_data(
                    api::code::BAD_REQUEST,
                    problem.clone(),
                    json!({ "problems": { "maxPerSubmitter": problem } }),
                );
            }
            Some(crate::rewards::manifest_reward(lepta, cap))
        };
        let current = self
            .experiments
            .iter()
            .find(|e| e.get("id").and_then(Value::as_str) == Some(&id))
            .and_then(|e| e.get("reward").cloned());
        if current == reward {
            return self.get(&json!({ "experimentId": id }).to_string());
        }
        if let Err(e) = self.reseal_with(&id, |doc| match reward {
            Some(r) => doc["reward"] = r,
            None => {
                if let Some(map) = doc.as_object_mut() {
                    map.remove("reward");
                }
            }
        }) {
            return api::envelope_err(api::code::REJECTED, e);
        }
        self.get(&json!({ "experimentId": id }).to_string())
    }

    /// `results.setApproval {experimentId, cid, status}` — approve, reject,
    /// or back to pending. Only received submissions (with a CID) from
    /// submitters not hidden, and never one already paid for.
    pub fn set_approval(&mut self, raw: &str) -> String {
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let id = request.get("experimentId").and_then(Value::as_str).unwrap_or("").to_string();
        if let Err(e) = self.may_edit(&id) {
            return e;
        }
        let cid = request.get("cid").and_then(Value::as_str).unwrap_or("").to_string();
        let status = request.get("status").and_then(Value::as_str).unwrap_or("");
        if !["approved", "rejected", "pending"].contains(&status) {
            return api::envelope_err(
                api::code::BAD_REQUEST,
                "`status` is approved, rejected or pending",
            );
        }
        if !self.visible_submissions(&id).iter().any(|(_, c)| c == &cid) {
            return api::envelope_err(
                api::code::BAD_REQUEST,
                "that submission is not one this experiment has received",
            );
        }
        if self.paid_cids(&id).contains(&cid) {
            return api::envelope_err(
                api::code::REJECTED,
                "that submission has already been rewarded",
            );
        }
        if status == "pending" {
            self.approvals.remove(&cid);
        } else {
            self.approvals.insert(cid.clone(), status.to_string());
        }
        api::envelope_ok(json!({ "cid": cid, "approval": status }))
    }

    /// Every received submission of `experiment_id` from a submitter who is
    /// not hidden, with its CID — the same join the results table makes.
    /// Submissions not indexed yet have no CID and are left out: the CID is
    /// what gets approved and paid for.
    fn visible_submissions(&self, experiment_id: &str) -> Vec<(&Value, String)> {
        let hidden = self.hidden_for(experiment_id);
        let mut out = Vec::new();
        for submission in &self.submissions {
            if submission.get("experiment").and_then(Value::as_str) != Some(experiment_id) {
                continue;
            }
            let submitter = submission.pointer("/submitter/key").and_then(Value::as_str).unwrap_or("");
            if hidden.iter().any(|h| h == submitter) {
                continue;
            }
            let submitted_at = submission.get("submitted_at").and_then(Value::as_str).unwrap_or("");
            let cid = self
                .index_pages
                .iter()
                .filter_map(|p| p.get("entries").and_then(Value::as_array))
                .flatten()
                .find(|e| {
                    e.get("submitter").and_then(Value::as_str) == Some(submitter)
                        && e.get("submitted_at").and_then(Value::as_str) == Some(submitted_at)
                })
                .and_then(|e| e.get("cid").and_then(Value::as_str))
                .unwrap_or("");
            if !cid.is_empty() {
                out.push((submission, cid.to_string()));
            }
        }
        out
    }

    /// Submissions of this experiment already paid for, or being paid for.
    /// A payment whose outcome is unknown counts as paid: paying twice is
    /// the failure to rule out.
    fn paid_cids(&self, experiment_id: &str) -> std::collections::BTreeSet<String> {
        self.payouts
            .iter()
            .filter(|p| p.get("experiment").and_then(Value::as_str) == Some(experiment_id))
            .filter(|p| p.get("state").and_then(Value::as_str) != Some("failed"))
            .filter_map(|p| p.get("submissions").and_then(Value::as_array))
            .flatten()
            .filter_map(|c| c.as_str().map(str::to_string))
            .collect()
    }

    /// Who would be paid now, and who would not and why.
    fn build_plan(
        &self,
        experiment_id: &str,
    ) -> Result<(Vec<crate::rewards::Payee>, Vec<Value>, usize, u128, u64), String> {
        let experiment = self
            .experiments
            .iter()
            .find(|e| e.get("id").and_then(Value::as_str) == Some(experiment_id))
            .ok_or_else(|| format!("no experiment `{experiment_id}` on this node"))?;
        let (each, cap) = crate::rewards::read_reward(experiment)
            .ok_or_else(|| "this experiment offers no reward".to_string())?;
        let paid = self.paid_cids(experiment_id);

        // Per submitter, in the order they first submitted.
        struct Acc<'a> {
            name: String,
            wallet: String,
            approved: Vec<String>,
            already: usize,
            latest: &'a str,
        }
        let mut order: Vec<String> = Vec::new();
        let mut by: BTreeMap<String, Acc> = BTreeMap::new();
        let mut pending = 0usize;
        for (submission, cid) in self.visible_submissions(experiment_id) {
            let key = submission.pointer("/submitter/key").and_then(Value::as_str).unwrap_or("").to_string();
            let at = submission.get("submitted_at").and_then(Value::as_str).unwrap_or("");
            let acc = by.entry(key.clone()).or_insert_with(|| {
                order.push(key.clone());
                Acc { name: String::new(), wallet: String::new(), approved: Vec::new(), already: 0, latest: "" }
            });
            // The newest submission's name and wallet are the ones that count.
            if at >= acc.latest {
                acc.latest = at;
                if let Some(n) = submission.pointer("/submitter/name").and_then(Value::as_str) {
                    if !n.is_empty() {
                        acc.name = n.to_string();
                    }
                }
                if let Some(w) = submission.get("wallet").and_then(Value::as_str) {
                    acc.wallet = w.to_string();
                }
            }
            if paid.contains(&cid) {
                acc.already += 1;
                continue;
            }
            match self.approvals.get(&cid).map(String::as_str) {
                Some("approved") => acc.approved.push(cid),
                Some(_) => {}
                None => pending += 1,
            }
        }

        let mut payees = Vec::new();
        let mut skipped = Vec::new();
        for key in order {
            let acc = &by[&key];
            if acc.approved.is_empty() {
                continue;
            }
            let room = (cap as usize).saturating_sub(acc.already);
            let skip = |reason: &str, count: usize| {
                json!({ "submitter": key, "name": acc.name, "wallet": acc.wallet,
                        "count": count, "reason": reason })
            };
            if room == 0 {
                skipped.push(skip(&format!("already rewarded for {cap}, the most per person"), acc.approved.len()));
                continue;
            }
            if acc.wallet.is_empty() {
                skipped.push(skip("did not include a wallet address", acc.approved.len()));
                continue;
            }
            let wallet_hex = match crate::rewards::account_hex(&acc.wallet) {
                Ok(h) => h,
                Err(_) => {
                    skipped.push(skip("the wallet address is not a LEZ public account", acc.approved.len()));
                    continue;
                }
            };
            let cids: Vec<String> = acc.approved.iter().take(room).cloned().collect();
            if acc.approved.len() > cids.len() {
                skipped.push(skip(
                    &format!("over the limit of {cap} per person"),
                    acc.approved.len() - cids.len(),
                ));
            }
            payees.push(crate::rewards::Payee {
                submitter: key.clone(),
                name: acc.name.clone(),
                wallet: acc.wallet.clone(),
                wallet_hex,
                lepta: each * cids.len() as u128,
                cids,
            });
        }
        Ok((payees, skipped, pending, each, cap))
    }

    /// `rewards.plan {experimentId, from?}` — what paying now would do.
    ///
    /// With `from`, also that account's balance, so the creator sees whether
    /// it covers the total before confirming anything.
    pub fn rewards_plan(&self, raw: &str, now: DateTime<Utc>) -> String {
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let id = request.get("experimentId").and_then(Value::as_str).unwrap_or("");
        if let Err(e) = self.may_edit(id) {
            return e;
        }
        let (payees, skipped, pending, each, cap) = match self.build_plan(id) {
            Ok(p) => p,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, e),
        };
        let total: u128 = payees.iter().map(|p| p.lepta).sum();
        let ends_at = self
            .experiments
            .iter()
            .find(|e| e.get("id").and_then(Value::as_str) == Some(id))
            .and_then(|e| e.get("ends_at").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();
        let ended = DateTime::parse_from_rfc3339(&ends_at)
            .map(|end| now >= end.with_timezone(&Utc))
            .unwrap_or(false);

        let mut from = json!(null);
        if let Some(address) = request.get("from").and_then(Value::as_str).filter(|a| !a.trim().is_empty()) {
            from = match crate::rewards::account_hex(address) {
                Err(problem) => json!({ "address": address, "problem": problem }),
                Ok(hex) => match self.ledger.as_deref() {
                    None => json!({ "address": address, "hex": hex,
                                    "problem": "LEZ Wallet is not reachable from Collect" }),
                    Some(ledger) => match ledger.balance(&hex) {
                        Ok(b) => json!({ "address": address, "hex": hex,
                                         "balance_lepta": b.to_string(),
                                         "balance": crate::rewards::format_tokens(b),
                                         "covers": b >= total }),
                        Err(e) => json!({ "address": address, "hex": hex,
                                          "problem": format!("could not read the balance: {e}") }),
                    },
                },
            };
        }

        api::envelope_ok(json!({
            "reward": {
                "amount": crate::rewards::format_tokens(each),
                "amount_lepta": each.to_string(),
                "max_per_submitter": cap,
                "symbol": crate::rewards::SYMBOL,
            },
            "ends_at": ends_at,
            "ended": ended,
            "payees": payees.iter().map(|p| p.as_json()).collect::<Vec<_>>(),
            "skipped": skipped,
            "pending": pending,
            "total": crate::rewards::format_tokens(total),
            "total_lepta": total.to_string(),
            "plan_id": crate::rewards::plan_id(&payees),
            "from": from,
            "ledger": self.ledger.as_deref().map(|l| l.describe()),
            "payouts": self.payouts_of(id),
        }))
    }

    fn payouts_of(&self, experiment_id: &str) -> Vec<Value> {
        self.payouts
            .iter()
            .filter(|p| p.get("experiment").and_then(Value::as_str) == Some(experiment_id))
            .cloned()
            .collect()
    }

    /// `rewards.pay {experimentId, from, submitter, cids, amountLepta}` — pay
    /// one submitter, exactly as the creator was shown.
    ///
    /// The plan is rebuilt and this submitter's row must match what came in —
    /// same submissions, same amount — or nothing is sent: an approval
    /// changed, or a payment landed, since the creator looked. The payment is
    /// recorded and saved *before* the transfer, so a crash in between can
    /// never lead to paying the same submissions again.
    pub fn rewards_pay(&mut self, raw: &str, now: DateTime<Utc>) -> String {
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let id = request.get("experimentId").and_then(Value::as_str).unwrap_or("").to_string();
        if let Err(e) = self.may_edit(&id) {
            return e;
        }
        let plan = self.rewards_plan(&json!({ "experimentId": id }).to_string(), now);
        let plan: Value = serde_json::from_str(&plan).unwrap_or(Value::Null);
        if plan["ok"] != json!(true) {
            return api::envelope_err(api::code::BAD_REQUEST, plan["error"]["message"].as_str().unwrap_or("no plan"));
        }
        if plan["data"]["ended"] != json!(true) {
            return api::envelope_err(api::code::REJECTED, "rewards are paid once the experiment has ended");
        }
        if self.payouts.iter().any(|p| {
            p.get("experiment").and_then(Value::as_str) == Some(&id)
                && p.get("state").and_then(Value::as_str) == Some("sending")
        }) {
            return api::envelope_err(
                api::code::REJECTED,
                "an earlier payment's outcome is unknown; check it in LEZ Wallet and resolve it before paying more",
            );
        }
        if self.ledger.is_none() {
            return api::envelope_err(api::code::NOT_WIRED, "LEZ Wallet is not reachable from Collect");
        }
        let from = match crate::rewards::account_hex(request.get("from").and_then(Value::as_str).unwrap_or("")) {
            Ok(h) => h,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("paying account: {e}")),
        };
        let submitter = request.get("submitter").and_then(Value::as_str).unwrap_or("");
        let Some(row) = plan["data"]["payees"]
            .as_array()
            .and_then(|a| a.iter().find(|p| p["submitter"] == json!(submitter)))
            .cloned()
        else {
            return api::envelope_err(api::code::REJECTED, "that submitter is not owed anything now");
        };
        if row["submissions"] != request["cids"] || row["amount_lepta"] != request["amountLepta"] {
            return api::envelope_err(
                api::code::REJECTED,
                "the plan changed since you reviewed it; look at it again before paying",
            );
        }
        let lepta: u128 = row["amount_lepta"].as_str().and_then(|a| a.parse().ok()).unwrap_or(0);
        let to = row["wallet_hex"].as_str().unwrap_or("").to_string();
        if lepta == 0 || to.is_empty() {
            return api::envelope_err(api::code::REJECTED, "nothing to pay");
        }
        if to == from {
            return api::envelope_err(api::code::REJECTED, "the paying account and the submitter's are the same");
        }

        let payout_id = uuid::Uuid::now_v7().to_string();
        self.payouts.push(json!({
            "id": payout_id,
            "experiment": id,
            "submitter": submitter,
            "name": row["name"],
            "wallet": row["wallet"],
            "wallet_hex": to,
            "from_hex": from,
            "submissions": row["submissions"],
            "amount_lepta": lepta.to_string(),
            "amount": crate::rewards::format_tokens(lepta),
            "state": "sending",
            "started_at": now.to_rfc3339(),
        }));
        // On disk before a single token moves.
        self.save_state();

        let outcome = match self.ledger.as_deref() {
            Some(ledger) => ledger.transfer_public(&from, &to, lepta),
            None => Err("LEZ Wallet stopped being reachable".to_string()),
        };
        let Some(record) = self.payouts.iter_mut().find(|p| p["id"] == json!(payout_id)) else {
            return api::envelope_err(api::code::REJECTED, "the payment record vanished");
        };
        match outcome {
            Ok(tx) => {
                record["state"] = json!("sent");
                record["tx"] = json!(tx.trim().trim_matches('"'));
            }
            Err(e) => {
                record["state"] = json!("failed");
                record["error"] = json!(e);
            }
        }
        record["finished_at"] = json!(now.to_rfc3339());
        let record = record.clone();
        self.save_state();
        if record["state"] == json!("failed") {
            return api::envelope_err_data(
                api::code::REJECTED,
                format!("the transfer was refused: {}", record["error"].as_str().unwrap_or("")),
                json!({ "payout": record }),
            );
        }
        api::envelope_ok(json!({ "payout": record }))
    }

    /// `rewards.refresh {experimentId}` — ask the chain whether sent payments
    /// are in a block yet.
    pub fn rewards_refresh(&mut self, raw: &str) -> String {
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let id = request.get("experimentId").and_then(Value::as_str).unwrap_or("").to_string();
        if let Err(e) = self.may_edit(&id) {
            return e;
        }
        if let Some(ledger) = self.ledger.as_deref() {
            for p in self.payouts.iter_mut() {
                if p["experiment"] != json!(id) || p["state"] != json!("sent") {
                    continue;
                }
                if let Some(tx) = p["tx"].as_str() {
                    if let Ok(true) = ledger.confirmed(tx) {
                        p["state"] = json!("confirmed");
                    }
                }
            }
        }
        api::envelope_ok(json!({ "payouts": self.payouts_of(&id) }))
    }

    /// `rewards.resolve {experimentId, payoutId, outcome}` — settle a payment
    /// whose outcome is unknown (a crash between recording and sending), once
    /// the creator has looked in LEZ Wallet: `failed` frees its submissions
    /// to be paid again, `sent` keeps them paid.
    pub fn rewards_resolve(&mut self, raw: &str) -> String {
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let id = request.get("experimentId").and_then(Value::as_str).unwrap_or("").to_string();
        if let Err(e) = self.may_edit(&id) {
            return e;
        }
        let outcome = request.get("outcome").and_then(Value::as_str).unwrap_or("");
        if outcome != "failed" && outcome != "sent" {
            return api::envelope_err(api::code::BAD_REQUEST, "`outcome` is failed or sent");
        }
        let payout_id = request.get("payoutId").cloned().unwrap_or(Value::Null);
        let Some(p) = self
            .payouts
            .iter_mut()
            .find(|p| p["id"] == payout_id && p["experiment"] == json!(id))
        else {
            return api::envelope_err(api::code::BAD_REQUEST, "no such payment");
        };
        if p["state"] != json!("sending") {
            return api::envelope_err(api::code::REJECTED, "only a payment with an unknown outcome can be resolved");
        }
        p["state"] = json!(outcome);
        p["resolved_by_creator"] = json!(true);
        api::envelope_ok(json!({ "payouts": self.payouts_of(&id) }))
    }

    // ------------------------------------------------------- published results

    /// `results.publish {experimentId, protocolId}` — the creator's "Push to
    /// Logos Storage": one signed `collect/results@1` document holding the
    /// experiment and the form as signed, and every **approved** submission
    /// to that form as signed, each with its CID and when it arrived.
    ///
    /// Every cost lands on the creator. Before publishing, each photo the
    /// approved submissions name is made local to the creator's node —
    /// fetched from the network if it is not here already — so the creator
    /// holds and serves the whole dataset, not just the table. Then the
    /// snapshot is uploaded, and a new manifest version lists it so anyone
    /// with the link can find it. Pushing again publishes a new snapshot
    /// whose `prev` names the last one: nothing published can be withdrawn.
    pub fn publish_results(&mut self, raw: &str, now: DateTime<Utc>) -> String {
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let id = request.get("experimentId").and_then(Value::as_str).unwrap_or("").to_string();
        let protocol_id = request.get("protocolId").and_then(Value::as_str).unwrap_or("").to_string();
        if let Err(e) = self.may_edit(&id) {
            return e;
        }
        if self.store.is_none() {
            return api::envelope_err(
                api::code::NOT_WIRED,
                "Logos Storage is not reachable, so there is nowhere to push the results to",
            );
        }
        let Some(experiment) = self
            .experiments
            .iter()
            .find(|e| e.get("id").and_then(Value::as_str) == Some(&id))
            .cloned()
        else {
            return api::envelope_err(api::code::BAD_REQUEST, format!("no experiment `{id}` on this node"));
        };
        let named = experiment
            .get("protocols")
            .and_then(Value::as_array)
            .map(|a| a.iter().any(|p| p.get("id").and_then(Value::as_str) == Some(&protocol_id)))
            .unwrap_or(false);
        let Some(protocol) = self
            .protocols
            .iter()
            .find(|p| p.get("id").and_then(Value::as_str) == Some(&protocol_id))
            .cloned()
            .filter(|_| named)
        else {
            return api::envelope_err(
                api::code::BAD_REQUEST,
                format!("this experiment has no form `{protocol_id}` on this node"),
            );
        };

        // Approved submissions to this form, with when each arrived.
        let mut rows: Vec<Value> = Vec::new();
        for (submission, cid) in self.visible_submissions(&id) {
            if submission.pointer("/protocol/id").and_then(Value::as_str) != Some(&protocol_id) {
                continue;
            }
            if self.approvals.get(&cid).map(String::as_str) != Some("approved") {
                continue;
            }
            let submitted_at = submission.get("submitted_at").and_then(Value::as_str).unwrap_or("");
            let received_at = self
                .index_pages
                .iter()
                .filter_map(|p| p.get("entries").and_then(Value::as_array))
                .flatten()
                .find(|e| e.get("cid").and_then(Value::as_str) == Some(&cid))
                .and_then(|e| e.get("received_at").and_then(Value::as_str))
                .unwrap_or(submitted_at)
                .to_string();
            rows.push(json!({ "cid": cid, "received_at": received_at, "submission": submission }));
        }
        if rows.is_empty() {
            return api::envelope_err(
                api::code::REJECTED,
                "no submissions to this form are approved yet; approve the ones to publish in the Review column",
            );
        }

        // Every photo the published rows name, held on this node.
        let mut wanted: Vec<String> = Vec::new();
        for row in &rows {
            collect_media_cids(&row["submission"]["values"], &mut wanted);
        }
        wanted.sort();
        wanted.dedup();
        let store = self.store.as_deref().expect("checked above");
        let deadline = std::time::Instant::now() + RESULTS_MEDIA_BUDGET;
        let mut missing: Vec<String> = Vec::new();
        for cid in &wanted {
            let bare = store::bare_cid(cid);
            let held = store.get(bare, true).is_ok()
                || (std::time::Instant::now() < deadline && store.get(bare, false).is_ok());
            if !held {
                missing.push(cid.clone());
            }
        }

        let prev = experiment
            .get("results")
            .and_then(Value::as_array)
            .and_then(|a| a.iter().find(|r| r.get("protocol").and_then(Value::as_str) == Some(&protocol_id)))
            .and_then(|r| r.get("cid").cloned())
            .unwrap_or(Value::Null);
        let mut media = json!({ "named": wanted.len(), "held": wanted.len() - missing.len() });
        if !missing.is_empty() {
            media["missing"] = json!(missing);
        }
        let doc = json!({
            "schema": "collect/results@1",
            "id": uuid::Uuid::now_v7().to_string(),
            "experiment": experiment,
            "protocol": protocol,
            "rows": rows,
            "media": media,
            "prev": prev,
            "published_at": now.to_rfc3339(),
            "sig": "00".repeat(64),
        });
        let doc = match self.signed(&doc) {
            Ok(d) => d,
            Err(e) => return api::envelope_err(api::code::NOT_WIRED, e),
        };
        let errors = schema::errors_as(&doc, schema::Kind::Results);
        if !errors.is_empty() {
            return api::envelope_err(
                api::code::INVALID_DOCUMENT,
                format!("the results document does not validate: {}", errors.join("; ")),
            );
        }

        // A fresh time budget for the upload itself: fetching photos may
        // have used the call's.
        self.start_publish_budget();
        let key = format!("results:{id}:{protocol_id}");
        self.publish_doc("results", &key, &doc);
        let Some(cid) = self.cid_of(&key).map(str::to_string) else {
            let report = self.publish_report(&key);
            return api::envelope_err_data(
                api::code::REJECTED,
                "the results could not be pushed to Logos Storage; try again",
                json!({ "publish": report }),
            );
        };

        // Listed in the manifest, newest per form, so the link leads to it.
        let entry = json!({
            "protocol": protocol_id,
            "cid": cid,
            "rows": doc["rows"].as_array().map(Vec::len).unwrap_or(0),
            "published_at": now.to_rfc3339(),
        });
        let listed = self.reseal_with(&id, |manifest| {
            let mut list: Vec<Value> = manifest
                .get("results")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|r| r.get("protocol").and_then(Value::as_str) != Some(&protocol_id))
                .collect();
            list.push(entry.clone());
            manifest["results"] = json!(list);
        });
        api::envelope_ok(json!({
            "cid": cid,
            "rows": entry["rows"],
            "media": doc["media"],
            "prev": doc["prev"],
            "published_at": entry["published_at"],
            // The snapshot stands even if the manifest could not be updated;
            // say so rather than fail a push that happened.
            "listed_in_manifest": listed.is_ok(),
        }))
    }

    /// `experiments.create` — a new manifest from the C2 form.
    ///
    /// Validated against the contract before it is kept, so a draft that
    /// could never be published cannot enter the list. It is **not** uploaded:
    /// `version` stays 1 and `prev`/`index` null until Storage exists.
    pub fn create(&mut self, raw: &str, now: DateTime<Utc>) -> String {
        self.start_publish_budget();
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };

        let details = match Details::read(&request) {
            Ok(d) => d,
            Err(problems) => {
                return api::envelope_ok(json!({ "created": false, "problems": problems }))
            }
        };

        let id = ids::experiment_id(&[0u8; 32], &ids::new_nonce());
        let experiment = json!({
            "schema": "collect/experiment@1",
            "id": id,
            "version": 1,
            "creator": { "key": self.creator_key, "name": details.creator_name },
            "title": details.title,
            "description": details.description,
            "starts_at": details.starts_at,
            "ends_at": details.ends_at,
            "timezone": details.timezone,
            "protocols": [],
            "index": Value::Null,
            "prev": Value::Null,
            "created_at": now.to_rfc3339(),
            // Replaced by the real signature below. Present so the shape is
            // checkable before signing.
            "sig": "00".repeat(64),
        });

        let experiment = match self.signed(&experiment) {
            Ok(signed) => signed,
            Err(e) => return api::envelope_err(api::code::NOT_WIRED, e),
        };

        // Refuse anything the contract would refuse, rather than discovering
        // it at publish time.
        let errors = schema::errors_as(&experiment, schema::Kind::Experiment);
        if !errors.is_empty() {
            return api::envelope_err(
                api::code::INVALID_DOCUMENT,
                format!("the new experiment does not validate: {}", errors.join("; ")),
            );
        }

        self.experiments.push(experiment.clone());
        // Publish after the document is complete and validated, never before:
        // a CID is the address of these exact bytes, so uploading a draft
        // would publish an address nothing else ever refers to.
        self.publish_doc("experiment", &id, &experiment);
        let mut data = json!({
            "created": true,
            "problems": [],
            "experiment": experiment,
        });
        merge(&mut data, self.publish_report(&id));
        api::envelope_ok(data)
    }

    /// `experiments.update` — edit an experiment's details.
    ///
    /// Only the details: the forms are `protocols.saveDraft`'s business and
    /// the creator cannot be changed at all, because the signature is what
    /// makes the attribution mean anything. Like every other amendment this
    /// produces a new signed version rather than patching the old one.
    ///
    /// Narrowing the window is allowed but reported: submissions already
    /// accepted outside the new dates stay accepted, because they were valid
    /// when they were made and the index that recorded them is published.
    /// Silently dropping them would rewrite history the submitter can see.
    pub fn update(&mut self, raw: &str, now: DateTime<Utc>) -> String {
        self.start_publish_budget();
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let Some(wanted) = request.get("experimentId").and_then(Value::as_str) else {
            return api::envelope_err(api::code::BAD_REQUEST, "`experimentId` is required");
        };
        let wanted = wanted.to_string();
        let Some(existing) = self
            .experiments
            .iter()
            .find(|e| e.get("id").and_then(Value::as_str) == Some(wanted.as_str()))
        else {
            return api::envelope_err(
                api::code::BAD_REQUEST,
                format!("no experiment `{wanted}` on this node"),
            );
        };
        if existing.pointer("/creator/key").and_then(Value::as_str) != Some(&self.creator_key) {
            return api::envelope_err(
                api::code::NOT_YOURS,
                "only the creator of an experiment can change it",
            );
        }

        let details = match Details::read(&request) {
            Ok(d) => d,
            Err(problems) => {
                return api::envelope_ok(json!({ "updated": false, "problems": problems }))
            }
        };

        // Which already-accepted submissions the new window would exclude.
        // Computed before the edit so it describes the change, not the result.
        let outside = self.submissions_outside(&wanted, &details.starts_at, &details.ends_at);

        let updated = {
            let d = details.clone();
            self.reseal_with(&wanted, move |doc| {
                doc["title"] = json!(d.title);
                doc["description"] = json!(d.description);
                doc["starts_at"] = json!(d.starts_at);
                doc["ends_at"] = json!(d.ends_at);
                doc["timezone"] = json!(d.timezone);
                // The creator's display name is theirs to change; the key is
                // not, and is left exactly as signed.
                if let Some(name) = doc.pointer_mut("/creator/name") {
                    *name = json!(d.creator_name);
                }
            })
        };
        let updated = match updated {
            Ok(doc) => doc,
            Err(e) => return api::envelope_err(api::code::INVALID_DOCUMENT, e),
        };

        let mut warnings: Vec<Value> = Vec::new();
        if outside > 0 {
            warnings.push(json!({
                "code": "submissions_outside_window",
                "message": format!(
                    "{outside} submission(s) already accepted fall outside the new dates. They stay accepted."
                ),
            }));
        }
        let _ = now;

        let mut data = json!({
            "updated": true,
            "problems": [],
            "experiment": updated,
            "warnings": warnings,
        });
        merge(&mut data, self.publish_report(&wanted));
        api::envelope_ok(data)
    }

    /// How many submissions this node holds for `experiment` fall outside the
    /// given window.
    fn submissions_outside(&self, experiment: &str, starts_at: &str, ends_at: &str) -> usize {
        let from = DateTime::parse_from_rfc3339(starts_at).ok();
        let to = DateTime::parse_from_rfc3339(ends_at).ok();
        let (Some(from), Some(to)) = (from, to) else { return 0 };
        self.submissions
            .iter()
            .filter(|s| s.get("experiment").and_then(Value::as_str) == Some(experiment))
            .filter(|s| {
                match s
                    .get("submitted_at")
                    .and_then(Value::as_str)
                    .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                {
                    Some(at) => at < from || at > to,
                    None => false,
                }
            })
            .count()
    }

    /// Who the creator currently hides for one experiment.
    fn hidden_for(&self, experiment_id: &str) -> Vec<String> {
        let pages: Vec<Value> = self
            .index_pages
            .iter()
            .filter(|p| p.get("experiment").and_then(Value::as_str) == Some(experiment_id))
            .cloned()
            .collect();
        crate::indexer::hidden_keys(&pages)
    }

    // ---------------------------------------------------------- Delivery

    /// Hand the session a Logos Delivery client.
    pub fn attach_courier(&mut self, courier: Box<dyn Courier>) {
        self.courier = Some(courier);
    }

    pub fn can_announce(&self) -> bool {
        self.courier.as_ref().map(|c| c.rln().ready).unwrap_or(false)
    }

    pub fn courier_description(&self) -> Option<String> {
        self.courier.as_ref().map(|c| c.describe())
    }

    /// What RLN membership this node has, if Delivery is attached at all.
    ///
    /// Reported rather than assumed: on `logos.test` a node needs a funded
    /// membership before it may publish, and a submitter whose announcement
    /// cannot go out needs to be told that rather than left watching a
    /// spinner (SPEC Q9).
    pub fn rln_status(&self) -> Option<Value> {
        self.courier.as_ref().map(|c| c.rln().as_json())
    }

    /// One reading of how well this node is connected, for the dot on the
    /// dashboard: `online`, `degraded` or `offline`, with the reason in words.
    ///
    /// Delivery's own `getConnectionStatus` decides most of it, because it is
    /// the only part that knows about peers. Offline means nothing can reach
    /// or leave this node. Degraded means something works and something does
    /// not — few peers, no Storage, or no RLN membership to send with — which
    /// is a different thing to fix, so it is a different colour.
    pub fn connection(&self) -> Value {
        let storage = self.store.is_some();
        let Some(courier) = self.courier.as_deref() else {
            return if storage {
                json!({ "level": "degraded", "delivery": null, "storage": true,
                        "reason": "Logos Delivery is not reachable: nothing can be announced or received." })
            } else {
                json!({ "level": "offline", "delivery": null, "storage": false,
                        "reason": "Neither Logos Storage nor Logos Delivery is reachable." })
            };
        };
        let status = courier.connection();
        let level = |level: &str, reason: String| {
            json!({ "level": level, "delivery": status, "storage": storage, "reason": reason })
        };
        match status.as_deref() {
            Some("Disconnected") => {
                return level("offline", "Logos Delivery has no peers.".into());
            }
            Some("PartiallyConnected") => {
                return level("degraded", "Logos Delivery is only partly connected: messages may be slow.".into());
            }
            Some("Connected") => {}
            Some(other) => {
                return level("degraded", format!("Logos Delivery reports \u{201c}{other}\u{201d}."));
            }
            None => {
                return level("degraded", "Logos Delivery did not say whether it is connected.".into());
            }
        }
        if !storage {
            return level("degraded", "Logos Storage is not reachable: new documents stay on this device.".into());
        }
        let rln = courier.rln();
        if rln.required && !rln.ready {
            return level(
                "degraded",
                format!("Connected, but this node cannot send yet: RLN is {}.", rln.state),
            );
        }
        if let Some(why) = &self.send_failure {
            return level(
                "degraded",
                format!(
                    "Connected, but the network refused this node's last message ({why}). \
                     On logos.test that almost always means no RLN membership is registered \
                     yet: register one in the RLN Membership app."
                ),
            );
        }
        level("online", "Connected to Logos Storage and Logos Delivery.".into())
    }

    /// `connection`, with a log line whenever the level changes — the dot on
    /// screen is otherwise the only record of why it was red.
    pub fn connection_logged(&self) -> Value {
        let reading = self.connection();
        let now = format!("{} ({})", reading["level"].as_str().unwrap_or("?"),
                          reading["delivery"].as_str().unwrap_or("no reading"));
        if *self.last_level.borrow() != now {
            eprintln!(
                "collect_core: connection {now}: {}",
                reading["reason"].as_str().unwrap_or("")
            );
            *self.last_level.borrow_mut() = now;
        }
        reading
    }

    /// Subscribe to the topics of every experiment this node cares about.
    ///
    /// Both roles need this and for the same reason: a creator receives
    /// submission announcements, a submitter receives the receipts and heads
    /// for experiments it has joined. Returns the topics now watched.
    pub fn watch_topics(&mut self) -> Vec<String> {
        let Some(courier) = self.courier.as_deref() else {
            return Vec::new();
        };
        let mut watched = Vec::new();
        for experiment in &self.experiments {
            let Some(id) = experiment.get("id").and_then(Value::as_str) else {
                continue;
            };
            let topic = delivery::topic(id);
            match courier.subscribe(&topic) {
                Ok(()) => watched.push(topic),
                // Logged, not fatal: one unsubscribable experiment should not
                // stop the others being watched.
                Err(e) => eprintln!("collect_core: cannot watch {topic}: {e}"),
            }
        }
        watched
    }

    /// Sign and send one announcement.
    ///
    /// `Ok` is Delivery's request id; `Err` is a ready-made failure envelope,
    /// so each caller can return it unchanged rather than restating a reason
    /// that four code paths would otherwise word four ways.
    fn announce(
        &mut self,
        experiment_id: &str,
        announcement: &Value,
        kind: schema::Kind,
    ) -> Result<String, String> {
        let signed = match self.signed(announcement) {
            Ok(s) => s,
            Err(e) => return Err(api::envelope_err(api::code::NOT_WIRED, e)),
        };
        let errors = schema::errors_as(&signed, kind);
        if !errors.is_empty() {
            return Err(api::envelope_err(
                api::code::INVALID_DOCUMENT,
                format!("the announcement does not validate: {}", errors.join("; ")),
            ));
        }
        let Some(courier) = self.courier.as_deref() else {
            return Err(api::envelope_err(
                api::code::NOT_WIRED,
                "no Logos Delivery is attached to this build",
            ));
        };
        // Canonical bytes, so the signature a receiver checks covers exactly
        // what went over the wire.
        let bytes = match crate::canonical::to_bytes(&signed) {
            Ok(b) => b,
            Err(e) => {
                return Err(api::envelope_err(
                    api::code::INVALID_DOCUMENT,
                    format!("the announcement is not canonicalizable: {e}"),
                ))
            }
        };
        match courier.send(&delivery::topic(experiment_id), &bytes) {
            Ok(request_id) => Ok(request_id),
            Err(e) => Err(api::envelope_err_data(
                e.code(),
                e.to_string(),
                json!({ "retryable": e.retryable(), "rln": courier.rln().as_json() }),
            )),
        }
    }

    /// `delivery.head` — the creator saying where the current documents are.
    ///
    /// Re-sent on a timer by the UI (SPEC 3.3: every 10 minutes), because
    /// Delivery's Store is temporary and a submitter who joins later has no
    /// other way to find the newest index.
    pub fn publish_head(&mut self, raw: &str, now: DateTime<Utc>) -> String {
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let Some(wanted) = request.get("experimentId").and_then(Value::as_str) else {
            return api::envelope_err(api::code::BAD_REQUEST, "`experimentId` is required");
        };
        let wanted = wanted.to_string();
        let Some(experiment) = self
            .experiments
            .iter()
            .find(|e| e.get("id").and_then(Value::as_str) == Some(wanted.as_str()))
            .cloned()
        else {
            return api::envelope_err(
                api::code::BAD_REQUEST,
                format!("no experiment `{wanted}` on this node"),
            );
        };
        if experiment.pointer("/creator/key").and_then(Value::as_str) != Some(&self.creator_key) {
            return api::envelope_err(
                api::code::NOT_YOURS,
                "only the creator of an experiment announces its head",
            );
        }
        // A head names the manifest's CID, so there is nothing to announce
        // until the manifest has one. Saying that is more useful than sending
        // a head pointing at `logos:pending`.
        let Some(manifest_cid) = self.cid_of(&wanted).map(str::to_string) else {
            return api::envelope_err(
                api::code::NOT_WIRED,
                "the manifest has no CID yet: publish it to Logos Storage first",
            );
        };

        let (index_cid, index_page) = self.newest_index(&wanted);
        let announcement = delivery::head(
            &wanted,
            &self.creator_key.clone(),
            &manifest_cid,
            experiment.get("version").and_then(Value::as_u64).unwrap_or(1),
            index_cid.as_deref(),
            index_page,
            &now.to_rfc3339(),
        );
        match self.announce(&wanted, &announcement, schema::Kind::HeadAnnouncement) {
            Ok(request_id) => api::envelope_ok(json!({
                "announced": true,
                "request_id": request_id,
                "topic": delivery::topic(&wanted),
                "head": announcement,
            })),
            Err(e) => e,
        }
    }

    /// The newest index page's CID and number, or `(None, 0)` when this
    /// experiment has received nothing yet.
    fn newest_index(&self, experiment_id: &str) -> (Option<String>, u64) {
        let Some(page) = self
            .index_pages
            .iter()
            .filter(|p| p.get("experiment").and_then(Value::as_str) == Some(experiment_id))
            .max_by_key(|p| p.get("page").and_then(Value::as_u64).unwrap_or(0))
        else {
            return (None, 0);
        };
        let number = page.get("page").and_then(Value::as_u64).unwrap_or(0);
        let key = self.page_key(experiment_id, number);
        (self.cid_of(&key).map(str::to_string), number)
    }

    /// `delivery.announceSubmission` — a submitter saying a submission exists.
    ///
    /// The last step of SPEC 3.2 step 3, after the submission document is on
    /// Storage: the CID is the whole message, so this cannot be sent before
    /// the upload has one.
    // ---------------------------------------------------- profile and discussion
    //
    // None of this reaches Logos Storage. The profile stays in the instance
    // directory; messages and sealed profiles go over the experiment's
    // Delivery topic and are kept locally. See `social`.

    /// `profile.get`
    pub fn profile_get(&self) -> String {
        api::envelope_ok(self.social.profile_json())
    }

    /// `profile.save` — `{ "fields": { name: { "value", "share" } } }`.
    pub fn profile_save(&mut self, raw: &str) -> String {
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        match self.social.save_profile(&request) {
            Ok(profile) => api::envelope_ok(profile),
            Err(detail) if detail.get("problems").is_some() => api::envelope_err_data(
                api::code::INVALID_VALUES,
                "Some profile fields need fixing.",
                detail,
            ),
            Err(detail) => api::envelope_err(
                api::code::REJECTED,
                detail["message"].as_str().unwrap_or("the profile could not be saved"),
            ),
        }
    }

    /// `profile.setPhoto` — `{ "path", "share" }`, a file the person picked.
    /// Read here because the QML sandbox cannot read files. The original is
    /// never kept: only a small square copy with no metadata.
    pub fn profile_set_photo(&mut self, raw: &str) -> String {
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let share = request.get("share").and_then(Value::as_bool).unwrap_or(false);
        let path = local_path(request.get("path").and_then(Value::as_str).unwrap_or(""));
        if path.is_empty() {
            return api::envelope_err(api::code::BAD_REQUEST, "`path` is required");
        }
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => return api::envelope_err(api::code::MEDIA, format!("cannot read the photo: {e}")),
        };
        match self.social.set_photo(&bytes, share) {
            Ok(profile) => api::envelope_ok(profile),
            Err(e) => api::envelope_err(api::code::MEDIA, e),
        }
    }

    /// `profile.clearPhoto`
    pub fn profile_clear_photo(&mut self) -> String {
        match self.social.clear_photo() {
            Ok(profile) => api::envelope_ok(profile),
            Err(e) => api::envelope_err(api::code::REJECTED, e),
        }
    }

    /// The experiment named by `experimentId`, if this node holds it.
    fn joined(&self, request: &Value) -> Result<(String, Option<String>), String> {
        let Some(id) = request.get("experimentId").and_then(Value::as_str) else {
            return Err(api::envelope_err(api::code::BAD_REQUEST, "`experimentId` is required"));
        };
        let Some(experiment) = self
            .experiments
            .iter()
            .find(|e| e.get("id").and_then(Value::as_str) == Some(id))
        else {
            return Err(api::envelope_err(
                api::code::BAD_REQUEST,
                format!("experiment `{id}` is not open on this node"),
            ));
        };
        let creator = experiment.pointer("/creator/key").and_then(Value::as_str).map(str::to_string);
        Ok((id.to_string(), creator))
    }

    /// `messages.list` — `{ "experimentId" }`.
    pub fn messages_list(&mut self, raw: &str) -> String {
        let request: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
        let (id, creator) = match self.joined(&request) {
            Ok(j) => j,
            Err(e) => return e,
        };
        let me = self.creator_key.clone();
        let mut data = self.social.list_messages(&id, &me);
        if let Some(list) = data["messages"].as_array_mut() {
            for m in list {
                m["creator"] = json!(creator.as_deref() == m["from"].as_str());
            }
        }
        data["can_send"] = json!(self.can_announce());
        data["rln"] = self.rln_status().unwrap_or(Value::Null);
        api::envelope_ok(data)
    }

    /// `messages.send` — `{ "experimentId", "text", "replyTo"? }`.
    ///
    /// Kept in history only once Delivery accepted it, so the list never
    /// shows a message nobody else will see.
    pub fn messages_send(&mut self, raw: &str, now: DateTime<Utc>) -> String {
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let (id, _) = match self.joined(&request) {
            Ok(j) => j,
            Err(e) => return e,
        };
        let me = self.creator_key.clone();
        let text = request.get("text").and_then(Value::as_str).unwrap_or("");
        let reply_to = request.get("replyTo").and_then(Value::as_str);
        let message = match self.social.compose(&id, &me, text, reply_to, now) {
            Ok(m) => m,
            Err(e) => return api::envelope_err(api::code::INVALID_VALUES, e),
        };
        let signed = match self.signed(&message) {
            Ok(s) => s,
            Err(e) => return api::envelope_err(api::code::NOT_WIRED, e),
        };
        if let Err(e) = self.announce(&id, &message, schema::Kind::MessageAnnouncement) {
            return e;
        }
        match self.social.receive_message(&id, &signed, &me, now) {
            Received::Kept(m) => api::envelope_ok(json!({ "sent": true, "message": m })),
            _ => api::envelope_ok(json!({ "sent": true })),
        }
    }

    /// `members.list` — `{ "experimentId" }`: who is in this project, and
    /// what each has shared with us.
    pub fn members_list(&self, raw: &str) -> String {
        let request: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
        match self.joined(&request) {
            Ok((id, creator)) => api::envelope_ok(self.social.list_members(&id, creator.as_deref())),
            Err(e) => e,
        }
    }

    /// Introduce ourselves, or re-seal our profile, on every experiment that
    /// needs it. Called from `drain`, so it runs on the UI's timer. Returns
    /// the experiments a profile went out on.
    fn share_profiles(&mut self, now: DateTime<Utc>) -> Vec<String> {
        if self.identity.is_none() || !self.can_announce() {
            return Vec::new();
        }
        let me = self.creator_key.clone();
        let ids: Vec<String> = self
            .experiments
            .iter()
            .filter_map(|e| e.get("id").and_then(Value::as_str).map(str::to_string))
            .collect();
        let mut sent = Vec::new();
        for id in ids {
            if !self.social.needs_share(&id, &me, now) {
                continue;
            }
            let card = match self.social.compose_profile(&id, &me, now) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("collect_core: cannot compose profile for {id}: {e}");
                    continue;
                }
            };
            match self.announce(&id, &card, schema::Kind::ProfileAnnouncement) {
                Ok(_) => {
                    self.social.mark_shared(&id, &me, now);
                    sent.push(id);
                }
                // Tried again on the next drain. RLN not being ready is the
                // usual reason and is reported by `delivery.status`.
                Err(e) => eprintln!("collect_core: profile for {id} not sent: {e}"),
            }
        }
        sent
    }

    pub fn announce_submission(&mut self, raw: &str, now: DateTime<Utc>) -> String {
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let text = |k: &str| request.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let experiment_id = text("experimentId");
        let cid = text("cid");
        let protocol = text("protocolId");
        let version = request.get("version").and_then(Value::as_u64).unwrap_or(0);
        for (name, value) in [("experimentId", &experiment_id), ("cid", &cid), ("protocolId", &protocol)] {
            if value.is_empty() {
                return api::envelope_err(api::code::BAD_REQUEST, format!("`{name}` is required"));
            }
        }
        if version == 0 {
            return api::envelope_err(api::code::BAD_REQUEST, "`version` is required");
        }

        let announcement = delivery::submission(
            &experiment_id,
            &self.creator_key.clone(),
            &cid,
            &protocol,
            version,
            &now.to_rfc3339(),
        );
        match self.announce(&experiment_id, &announcement, schema::Kind::SubmissionAnnouncement) {
            Ok(request_id) => api::envelope_ok(json!({
                "announced": true,
                "request_id": request_id,
                "topic": delivery::topic(&experiment_id),
                // Whether the creator has confirmed it. SPEC 3.2 step 3 shows
                // this as "waiting" until a receipt arrives.
                "receipt": self.receipts.get(&cid).cloned().unwrap_or(Value::Null),
                "announcement": announcement,
            })),
            Err(e) => e,
        }
    }

    /// `delivery.drain` — handle everything that has arrived.
    ///
    /// Called on a timer by the UI. The one place inbound messages turn into
    /// indexed submissions, so a creator's node makes progress whether or not
    /// anyone is looking at the right screen.
    pub fn drain(&mut self, now: DateTime<Utc>) -> String {
        let Some(courier) = self.courier.as_deref() else {
            return api::envelope_err(
                api::code::NOT_WIRED,
                "no Logos Delivery is attached to this build",
            );
        };
        let arrived = courier.inbox();
        let mut results: Vec<Value> = Vec::new();
        let mut handled = 0usize;
        for message in arrived {
            // A refused send is not a message to deliver; it is news about
            // one of ours. Reported through the same results so the UI has
            // one place to look.
            if message.get("kind").and_then(Value::as_str) == Some("sent") {
                if self.send_failure.take().is_some() {
                    eprintln!("collect_core: messages are getting through again");
                }
                continue;
            }
            if message.get("kind").and_then(Value::as_str) == Some("sendFailed") {
                let detail = message
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("the network refused the message")
                    .to_string();
                eprintln!("collect_core: announcement refused: {detail}");
                self.send_failure = Some(detail.clone());
                results.push(json!({
                    "handled": false,
                    "reason": "send_failed",
                    "request_id": message.get("requestId").cloned().unwrap_or(Value::Null),
                    "detail": detail,
                }));
                continue;
            }
            let outcome: Value = match serde_json::from_str(&self.deliver(&message.to_string(), now)) {
                Ok(v) => v,
                Err(e) => json!({ "ok": false, "error": { "message": e.to_string() } }),
            };
            if outcome["data"]["handled"] == json!(true) {
                handled += 1;
            }
            results.push(outcome["data"].clone());
        }
        let resent = self.resend_outbox(now);
        let shared = self.share_profiles(now);
        let thumbnails = self.fetch_thumbnails();
        api::envelope_ok(json!({
            "received": results.len(),
            "handled": handled,
            "results": results,
            "profiles_shared": shared,
            "thumbnails_fetched": thumbnails,
            "resent": resent,
        }))
    }

    /// `delivery.deliver` — one message arrived on a topic we watch.
    ///
    /// The whole receive side, for both roles, because which role a message
    /// concerns is a property of the message and not of the node: a creator
    /// gets `submission`, a submitter gets `receipt` and `head`, and a node
    /// can be both at once for different experiments.
    ///
    /// Order matters here and is deliberate:
    ///
    /// 1. **Deduplicate first**, on `messageHash`. Store catch-up replays
    ///    everything missed while the node was down, so without this a
    ///    restart re-indexes the backlog.
    /// 2. **The topic has to agree with the announcement.** Otherwise a
    ///    message on one experiment's topic could claim to be about another,
    ///    and a creator would index a submission for an experiment they do
    ///    not run.
    /// 3. **Contract, then signature.** An invalid announcement is never
    ///    acted on, and the signature is what ties it to the `from` key.
    pub fn deliver(&mut self, raw: &str, now: DateTime<Utc>) -> String {
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let hash = request.get("messageHash").and_then(Value::as_str).unwrap_or("");
        let source = request.get("source").and_then(Value::as_str).unwrap_or("live");
        let Some(topic) = request.get("topic").and_then(Value::as_str) else {
            return api::envelope_err(api::code::BAD_REQUEST, "`topic` is required");
        };
        let Some(payload) = request.get("payload").and_then(Value::as_str) else {
            return api::envelope_err(api::code::BAD_REQUEST, "`payload` is required");
        };

        if !self.seen.accept(hash) {
            return api::envelope_ok(json!({
                "handled": false,
                "reason": "duplicate",
                "detail": format!("message {hash} was already handled"),
            }));
        }

        let Some(from_topic) = delivery::experiment_of(topic) else {
            return api::envelope_ok(json!({
                "handled": false,
                "reason": "not_ours",
                "detail": format!("`{topic}` is not a Collect topic"),
            }));
        };
        let from_topic = from_topic.to_string();

        let announcement: Value = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(e) => {
                return api::envelope_ok(json!({
                    "handled": false,
                    "reason": "unreadable",
                    "detail": format!("the payload is not JSON: {e}"),
                }))
            }
        };
        if announcement.get("experiment").and_then(Value::as_str) != Some(from_topic.as_str()) {
            return api::envelope_ok(json!({
                "handled": false,
                "reason": "wrong_topic",
                "detail": "the announcement names a different experiment than the topic it arrived on",
            }));
        }

        let kind = match announcement.get("kind").and_then(Value::as_str) {
            Some("head") => schema::Kind::HeadAnnouncement,
            Some("submission") => schema::Kind::SubmissionAnnouncement,
            Some("receipt") => schema::Kind::ReceiptAnnouncement,
            Some("message") => schema::Kind::MessageAnnouncement,
            Some("profile") => schema::Kind::ProfileAnnouncement,
            other => {
                return api::envelope_ok(json!({
                    "handled": false,
                    "reason": "unknown_kind",
                    "detail": format!("`{}` is not an announcement kind", other.unwrap_or("")),
                }))
            }
        };
        let errors = schema::errors_as(&announcement, kind);
        if !errors.is_empty() {
            return api::envelope_ok(json!({
                "handled": false,
                "reason": "invalid_document",
                "detail": errors.join("; "),
            }));
        }
        if let Err(e) = crate::sign::verify(&announcement) {
            return api::envelope_ok(json!({
                "handled": false,
                "reason": "bad_signature",
                "detail": e.to_string(),
            }));
        }

        let _ = source;
        match kind {
            schema::Kind::MessageAnnouncement => {
                let me = self.creator_key.clone();
                match self.social.receive_message(&from_topic, &announcement, &me, now) {
                    Received::Kept(m) => api::envelope_ok(json!({
                        "handled": true, "kind": "message", "experiment": from_topic, "message": m,
                    })),
                    _ => api::envelope_ok(json!({ "handled": false, "reason": "duplicate", "detail": "message already kept" })),
                }
            }
            schema::Kind::ProfileAnnouncement => {
                let me = self.creator_key.clone();
                match self.social.receive_profile(&from_topic, &announcement, &me, now) {
                    Received::Kept(p) => api::envelope_ok(json!({
                        "handled": true, "kind": "profile", "experiment": from_topic, "from": p["from"],
                    })),
                    Received::NothingForUs => api::envelope_ok(json!({
                        "handled": true, "kind": "profile", "experiment": from_topic, "shared": false,
                    })),
                    Received::Duplicate => api::envelope_ok(json!({ "handled": false, "reason": "ours", "detail": "our own profile echoed back" })),
                    Received::Refused(why) => api::envelope_ok(json!({ "handled": false, "reason": "unsealable", "detail": why })),
                }
            }
            schema::Kind::SubmissionAnnouncement => self.receive_submission(&from_topic, &announcement, now),
            schema::Kind::ReceiptAnnouncement => {
                let cid = announcement.get("cid").and_then(Value::as_str).unwrap_or("").to_string();
                // Only the creator's receipt counts: anyone can post on the
                // topic, and a stranger's receipt would stop the re-announcing
                // that is the only way the creator ever hears of it.
                let creator = self
                    .experiments
                    .iter()
                    .find(|e| e.get("id").and_then(Value::as_str) == Some(from_topic.as_str()))
                    .and_then(|e| e.pointer("/creator/key").and_then(Value::as_str))
                    .map(str::to_string);
                let from = announcement.get("from").and_then(Value::as_str).unwrap_or("");
                if creator.as_deref().is_some_and(|c| c != from) {
                    return api::envelope_ok(json!({
                        "handled": false,
                        "reason": "not_the_creator",
                        "detail": "a receipt from someone other than the experiment's creator",
                    }));
                }
                self.receipts.insert(cid.clone(), announcement.clone());
                for entry in self.outbox.iter_mut() {
                    if entry["cid"] == json!(cid) && entry["state"] != json!("received") {
                        entry["state"] = json!("received");
                        entry["received_at"] = json!(now.to_rfc3339());
                        entry["index_page"] = announcement.get("index_page").cloned().unwrap_or(Value::Null);
                    }
                }
                api::envelope_ok(json!({
                    "handled": true,
                    "kind": "receipt",
                    "cid": cid,
                    "index_page": announcement.get("index_page").cloned().unwrap_or(Value::Null),
                }))
            }
            schema::Kind::HeadAnnouncement => {
                // Only the creator's own head counts. Anyone can publish on a
                // public topic, and a head from a stranger naming a manifest
                // CID would otherwise send submitters off to fetch it.
                let expected = self
                    .experiments
                    .iter()
                    .find(|e| e.get("id").and_then(Value::as_str) == Some(from_topic.as_str()))
                    .and_then(|e| e.pointer("/creator/key").and_then(Value::as_str))
                    .map(str::to_string);
                let from = announcement.get("from").and_then(Value::as_str).unwrap_or("");
                if let Some(creator) = expected {
                    if creator != from {
                        return api::envelope_ok(json!({
                            "handled": false,
                            "reason": "not_the_creator",
                            "detail": "a head announcement from someone other than the experiment's creator",
                        }));
                    }
                }
                self.heads.insert(from_topic.clone(), announcement.clone());
                api::envelope_ok(json!({
                    "handled": true,
                    "kind": "head",
                    "experiment": from_topic,
                    "manifest": announcement.get("manifest").cloned().unwrap_or(Value::Null),
                    "manifest_version": announcement.get("manifest_version").cloned().unwrap_or(Value::Null),
                    "index": announcement.get("index").cloned().unwrap_or(Value::Null),
                    "index_page": announcement.get("index_page").cloned().unwrap_or(Value::Null),
                }))
            }
            _ => api::envelope_err(api::code::BAD_REQUEST, "not an announcement"),
        }
    }

    /// The creator's side of a submission announcement: fetch, judge, index,
    /// receipt.
    ///
    /// The announcement carries only a CID, so the document has to come off
    /// Storage before anything can be decided about it — the announcement
    /// cannot be trusted about the submission's contents, only about where to
    /// find it.
    fn receive_submission(
        &mut self,
        experiment_id: &str,
        announcement: &Value,
        now: DateTime<Utc>,
    ) -> String {
        let Some(experiment) = self
            .experiments
            .iter()
            .find(|e| e.get("id").and_then(Value::as_str) == Some(experiment_id))
            .cloned()
        else {
            return api::envelope_ok(json!({
                "handled": false,
                "reason": "not_ours",
                "detail": format!("no experiment `{experiment_id}` on this node"),
            }));
        };
        // Only the creator indexes. A submitter watching the same topic sees
        // other people's announcements and has nothing to do with them.
        if experiment.pointer("/creator/key").and_then(Value::as_str) != Some(&self.creator_key) {
            return api::envelope_ok(json!({
                "handled": false,
                "reason": "not_the_creator",
                "detail": "this node does not index this experiment",
            }));
        }

        let cid = announcement.get("cid").and_then(Value::as_str).unwrap_or("").to_string();
        let protocol_id = announcement.get("protocol").and_then(Value::as_str).unwrap_or("").to_string();
        let Some(store) = self.store.as_deref() else {
            return api::envelope_ok(json!({
                "handled": false,
                "reason": "storage_unavailable",
                "detail": "the submission cannot be fetched without Logos Storage",
                "cid": cid,
            }));
        };
        // `local = false`: this is someone else's document and has to come
        // over the network.
        let bytes = match store.get(store::bare_cid(&cid), false) {
            Ok(b) => b,
            Err(e) => {
                return api::envelope_ok(json!({
                    "handled": false,
                    "reason": e.code(),
                    "detail": e.to_string(),
                    "cid": cid,
                    "retryable": e.retryable(),
                }))
            }
        };
        let submission: Value = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(e) => {
                return api::envelope_ok(json!({
                    "handled": false,
                    "reason": "unreadable",
                    "detail": format!("what is at `{cid}` is not JSON: {e}"),
                }))
            }
        };

        let Some(protocol) = self
            .protocols
            .iter()
            .find(|p| p.get("id").and_then(Value::as_str) == Some(protocol_id.as_str()))
            .cloned()
        else {
            return api::envelope_ok(json!({
                "handled": false,
                "reason": "protocol_not_open",
                "detail": format!("no protocol `{protocol_id}` on this node"),
            }));
        };

        // Every accept rule, through the one tested path: contract,
        // signature, window, repeat and rate limits, hidden keys, fields.
        let pages: Vec<Value> = self
            .index_pages
            .iter()
            .filter(|p| p.get("experiment").and_then(Value::as_str) == Some(experiment_id))
            .cloned()
            .collect();
        let verdict: Value = match serde_json::from_str(&api::index_accept(
            &json!({
                "submission": submission,
                "protocol": protocol,
                "experiment": experiment,
                "pages": pages,
                "now": now.to_rfc3339(),
            })
            .to_string(),
        )) {
            Ok(v) => v,
            Err(e) => {
                return api::envelope_err(
                    api::code::INVALID_DOCUMENT,
                    format!("the accept verdict was not readable: {e}"),
                )
            }
        };
        if verdict["data"]["accepted"] != json!(true) {
            return api::envelope_ok(json!({
                "handled": false,
                "reason": verdict["data"]["reason"].as_str().unwrap_or("rejected"),
                "detail": verdict["data"]["detail"].clone(),
                "cid": cid,
            }));
        }

        let entry = crate::indexer::Entry {
            cid: cid.clone(),
            protocol: protocol_id.clone(),
            version: announcement.get("version").and_then(Value::as_u64).unwrap_or(1),
            submitter: submission
                .pointer("/submitter/key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            submitted_at: submission
                .get("submitted_at")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            received_at: now.to_rfc3339(),
        };
        let submitter = entry.submitter.clone();
        let page = match self.index_entry(experiment_id, entry, now) {
            Ok(page) => page,
            Err(e) => return api::envelope_err(api::code::INVALID_DOCUMENT, e),
        };

        // Keep the document, so the results table and exports can show it.
        self.submissions.push(submission);

        // And tell the submitter, which is what turns S3's "waiting" into
        // "received".
        let receipt = delivery::receipt(
            experiment_id,
            &self.creator_key.clone(),
            &cid,
            page,
            &now.to_rfc3339(),
        );
        let receipted = self
            .announce(experiment_id, &receipt, schema::Kind::ReceiptAnnouncement)
            .is_ok();

        api::envelope_ok(json!({
            "handled": true,
            "kind": "submission",
            "cid": cid,
            "protocol": protocol_id,
            "submitter": submitter,
            "index_page": page,
            // A receipt that did not go out is not a failed submission: it is
            // indexed either way, and the submitter re-announces until they
            // get one (SPEC 3.2).
            "receipted": receipted,
        }))
    }

    /// Add one entry to the index, publishing whatever pages that changes.
    ///
    /// Returns the page number the entry landed on, which is what a receipt
    /// has to name.
    fn index_entry(
        &mut self,
        experiment_id: &str,
        entry: crate::indexer::Entry,
        now: DateTime<Utc>,
    ) -> Result<u64, String> {
        let hidden = self.hidden_for(experiment_id);
        let newest = self
            .index_pages
            .iter()
            .filter(|p| p.get("experiment").and_then(Value::as_str) == Some(experiment_id))
            .max_by_key(|p| p.get("page").and_then(Value::as_u64).unwrap_or(0))
            .cloned();
        let newest_cid = newest
            .as_ref()
            .and_then(|p| p.get("page").and_then(Value::as_u64))
            .map(|n| self.page_key(experiment_id, n))
            .and_then(|key| self.cid_of(&key).map(str::to_string));

        let pages = crate::indexer::page_entries(
            experiment_id,
            newest.as_ref(),
            newest_cid.as_deref(),
            std::slice::from_ref(&entry),
            &hidden,
            now,
        );

        let mut landed = 0u64;
        let mut newest_page_cid = None;
        for page in pages {
            let page = self.signed(&page)?;
            let errors = schema::errors_as(&page, schema::Kind::Index);
            if !errors.is_empty() {
                return Err(format!(
                    "the index page does not validate: {}",
                    errors.join("; ")
                ));
            }
            let number = page.get("page").and_then(Value::as_u64).unwrap_or(1);
            // Which page the entry is actually on, rather than assuming the
            // last one: a full page returns unchanged and only its successor
            // carries the new entry.
            if page
                .get("entries")
                .and_then(Value::as_array)
                .map(|e| e.iter().any(|v| v.get("cid").and_then(Value::as_str) == Some(entry.cid.as_str())))
                .unwrap_or(false)
            {
                landed = number;
            }
            let key = self.page_key(experiment_id, number);
            self.index_pages.retain(|p| {
                !(p.get("experiment").and_then(Value::as_str) == Some(experiment_id)
                    && p.get("page").and_then(Value::as_u64) == Some(number))
            });
            self.index_pages.push(page.clone());
            self.publish_doc("index", &key, &page);
            if let Some(cid) = self.cid_of(&key) {
                newest_page_cid = Some(cid.to_string());
            }
        }

        if let Some(cid) = newest_page_cid {
            self.reseal_with(experiment_id, move |doc| {
                doc["index"] = json!(cid);
            })?;
        }
        Ok(landed)
    }

    /// `submissions.submit` — make, sign, publish and keep a submission.
    ///
    /// The whole of SPEC 3.2 step 3 bar the announcement: validate against
    /// the protocol, apply each field's store rule (notably `rounded_1km`,
    /// which has to happen *before* anything is stored), sign, upload, keep.
    ///
    /// A creator submitting to their own experiment is an ordinary submitter
    /// here, not a special case — the same validation, the same signature,
    /// the same document. The only difference is that this node is also the
    /// indexer, so the submission can be indexed immediately instead of
    /// waiting to hear about itself over Delivery.
    pub fn submit(&mut self, raw: &str, now: DateTime<Utc>) -> String {
        self.start_publish_budget();
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let Some(experiment_id) = request.get("experimentId").and_then(Value::as_str) else {
            return api::envelope_err(api::code::BAD_REQUEST, "`experimentId` is required");
        };
        let experiment_id = experiment_id.to_string();
        let Some(protocol_id) = request.get("protocolId").and_then(Value::as_str) else {
            return api::envelope_err(api::code::BAD_REQUEST, "`protocolId` is required");
        };
        let protocol_id = protocol_id.to_string();
        let Some(values) = request.get("values").filter(|v| v.is_object()).cloned() else {
            return api::envelope_err(api::code::BAD_REQUEST, "`values` is required");
        };

        let Some(experiment) = self
            .experiments
            .iter()
            .find(|e| e.get("id").and_then(Value::as_str) == Some(experiment_id.as_str()))
            .cloned()
        else {
            return api::envelope_err(
                api::code::BAD_REQUEST,
                format!("no experiment `{experiment_id}` on this node"),
            );
        };
        let Some(protocol) = self
            .protocols
            .iter()
            .find(|p| p.get("id").and_then(Value::as_str) == Some(protocol_id.as_str()))
            .cloned()
        else {
            return api::envelope_err(
                api::code::BAD_REQUEST,
                format!("no form `{protocol_id}` on this node"),
            );
        };

        // The window, from the experiment rather than from the caller.
        if Self::standing(&experiment, now) != Standing::Open {
            return api::envelope_err(
                api::code::REJECTED,
                "this experiment is not open for submissions right now",
            );
        }

        // Store rules first, then validate — in that order, and the order
        // matters. Rounding is what `prepare_values` does, and the validator
        // refuses an unrounded point for a `rounded_1km` field, so validating
        // first rejects every honest location submission with "the point is
        // not rounded". Preparing first also means the values that are
        // checked are the values that will be stored, which is the pair that
        // ought to agree.
        //
        // An exact coordinate that reaches a document is public forever
        // (SPEC 2), so this has to happen before the document exists.
        let prepared: Value = match serde_json::from_str(&api::submissions_prepare_values(
            &json!({ "protocol": protocol, "values": values }).to_string(),
        )) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::INVALID_VALUES, e.to_string()),
        };
        if prepared["ok"] != json!(true) {
            return prepared.to_string();
        }
        let values = prepared["data"]["values"].clone();

        // Then validate, so a bad form comes back as field problems the
        // person can fix rather than as a rejected document.
        let report: Value = match serde_json::from_str(&api::submissions_validate(
            &json!({ "protocol": protocol, "values": values, "now": now.to_rfc3339() }).to_string(),
        )) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::INVALID_VALUES, e.to_string()),
        };
        if report["data"]["ok"] != json!(true) {
            return api::envelope_ok(json!({
                "submitted": false,
                "missing": report["data"]["missing"].clone(),
                "errors": report["data"]["errors"].clone(),
            }));
        }

        // The wallet is public and permanent once submitted, so it goes in
        // only when the person ticked the box for this submission, and only
        // as the address their own profile holds.
        let wallet = if request.get("includeWallet").and_then(Value::as_bool) == Some(true) {
            match self.social.own_wallet() {
                Some(w) if w.chars().any(char::is_whitespace) => {
                    return api::envelope_err(
                        api::code::BAD_REQUEST,
                        "the wallet address in your profile has a space in it; correct it there first",
                    )
                }
                Some(w) => Some(w),
                None => {
                    return api::envelope_err(
                        api::code::BAD_REQUEST,
                        "there is no wallet address in your profile to include",
                    )
                }
            }
        } else {
            None
        };

        let mut submission = json!({
            "schema": "collect/submission@1",
            "id": uuid::Uuid::now_v7().to_string(),
            "experiment": experiment_id,
            "protocol": {
                "id": protocol_id,
                "version": protocol.get("version").and_then(Value::as_u64).unwrap_or(1),
            },
            "submitter": {
                "key": self.creator_key,
                "name": request.get("submitterName").and_then(Value::as_str).unwrap_or(""),
            },
            "submitted_at": now.to_rfc3339(),
            "values": values,
            "sig": "00".repeat(64),
        });
        if let Some(wallet) = wallet {
            submission["wallet"] = json!(wallet);
        }

        let submission = match self.signed(&submission) {
            Ok(signed) => signed,
            Err(e) => return api::envelope_err(api::code::NOT_WIRED, e),
        };
        let errors = schema::errors_as(&submission, schema::Kind::Submission);
        if !errors.is_empty() {
            return api::envelope_err(
                api::code::INVALID_DOCUMENT,
                format!("the submission does not validate: {}", errors.join("; ")),
            );
        }

        let submission_key = format!("submission:{}", submission["id"].as_str().unwrap_or(""));
        self.publish_doc("submission", &submission_key, &submission);
        let cid = self.cid_of(&submission_key).map(str::to_string);

        self.submissions.push(submission.clone());

        // This node is the creator, so it is also the indexer: index now
        // rather than waiting to hear about our own submission over
        // Delivery. Without a CID there is nothing to index — the entry is
        // an address — so that waits for a successful publish.
        let mut indexed = Value::Null;
        if experiment.pointer("/creator/key").and_then(Value::as_str) == Some(&self.creator_key) {
            if let Some(cid) = cid.clone() {
                let entry = crate::indexer::Entry {
                    cid,
                    protocol: protocol_id.clone(),
                    version: submission.pointer("/protocol/version").and_then(Value::as_u64).unwrap_or(1),
                    submitter: self.creator_key.clone(),
                    submitted_at: submission["submitted_at"].as_str().unwrap_or("").to_string(),
                    received_at: now.to_rfc3339(),
                };
                match self.index_entry(&experiment_id, entry, now) {
                    Ok(page) => indexed = json!(page),
                    Err(e) => eprintln!("collect_core: could not index our own submission: {e}"),
                }
            }
        }

        // Someone else's experiment: the creator hears of it only through an
        // announcement, so it goes in the outbox and is announced now. Not
        // being able to announce yet (no Delivery, no RLN membership) is not
        // a failed submission — it is signed and on Storage — so it waits
        // in the outbox and the drain keeps trying.
        let mut announce = Value::Null;
        if experiment.pointer("/creator/key").and_then(Value::as_str) != Some(&self.creator_key) {
            if let Some(cid) = cid.clone() {
                self.outbox.push(json!({
                    "cid": cid,
                    "experiment": experiment_id,
                    "protocol": protocol_id,
                    "version": submission.pointer("/protocol/version").and_then(Value::as_u64).unwrap_or(1),
                    "submitted_at": submission["submitted_at"],
                    "state": "waiting",
                    "attempts": 0,
                    "last_attempt": Value::Null,
                    "last_error": Value::Null,
                }));
                let at = self.outbox.len() - 1;
                announce = self.announce_outbox(at, now);
            } else {
                announce = json!({ "sent": false, "error": "not on Logos Storage yet, so there is nothing to announce; publish it first" });
            }
        }

        let mut data = json!({
            "submitted": true,
            "missing": [],
            "errors": {},
            "submission": submission,
            "cid": cid,
            "index_page": indexed,
            "announce": announce,
        });
        merge(&mut data, self.publish_report(&submission_key));
        api::envelope_ok(data)
    }

    /// `results.export` — write the results table out as a file.
    ///
    /// A file rather than a string, because the UI cannot make one: the QML
    /// side has no filesystem at all, so if the creator is to end up with a
    /// CSV, this is what writes it. The path comes back for the UI to show,
    /// along with the text itself when it is small enough to preview.
    ///
    /// Built on top of `results`, deliberately, rather than reading the
    /// submissions again. Hidden submitters, the protocol's column order and
    /// the label-for-a-removed-option rule all live there, and an export that
    /// re-derived them could disagree with the table on screen. A hidden
    /// submitter reappearing in a spreadsheet is exactly the failure that
    /// would not be noticed until it mattered.
    pub fn export(&mut self, raw: &str, now: DateTime<Utc>) -> String {
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let format = match request.get("format").and_then(Value::as_str) {
            // CSV is the default because it is what a spreadsheet opens.
            None => export::Format::Csv,
            Some(name) => match export::Format::parse(name) {
                Some(f) => f,
                None => {
                    return api::envelope_err(
                        api::code::BAD_REQUEST,
                        format!("`{name}` is not a format: use `csv` or `json`"),
                    )
                }
            },
        };
        let with_media = request
            .get("includeMedia")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // The same table the creator is looking at.
        let table: Value = match serde_json::from_str(&self.results(raw)) {
            Ok(v) => v,
            Err(e) => {
                return api::envelope_err(
                    api::code::INVALID_DOCUMENT,
                    format!("the results table was not readable: {e}"),
                )
            }
        };
        if table["ok"] != json!(true) {
            // Pass the reason straight through rather than restating it.
            return table.to_string();
        }
        let data = &table["data"];
        let empty: Vec<Value> = Vec::new();
        let columns = data["columns"].as_array().unwrap_or(&empty);
        let rows = data["rows"].as_array().unwrap_or(&empty);

        let experiment_id = request.get("experimentId").and_then(Value::as_str).unwrap_or("");
        let protocol_id = request.get("protocolId").and_then(Value::as_str).unwrap_or("");
        let name = export::filename(experiment_id, protocol_id, format, &now.to_rfc3339());
        let text = match format {
            export::Format::Csv => export::csv(columns, rows),
            export::Format::Json => export::json_export(&data["protocol"], columns, rows),
        };

        let Some(state) = self.state_dir.clone() else {
            return api::envelope_err(
                api::code::NOT_WIRED,
                "there is nowhere to write to yet: the module context is not ready",
            );
        };

        // With media the export becomes a folder, because a CSV naming photo
        // CIDs is no use without the photos beside it.
        let stem = name.trim_end_matches(&format!(".{}", format.extension())).to_string();
        let dir = if with_media {
            state.join("exports").join(&stem)
        } else {
            state.join("exports")
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return api::envelope_err(
                api::code::REJECTED,
                format!("cannot make {}: {e}", dir.display()),
            );
        }
        let path = dir.join(&name);
        if let Err(e) = std::fs::write(&path, text.as_bytes()) {
            return api::envelope_err(
                api::code::REJECTED,
                format!("cannot write {}: {e}", path.display()),
            );
        }

        let media = if with_media {
            self.export_media(rows, &dir.join("media"))
        } else {
            json!({ "requested": false })
        };

        api::envelope_ok(json!({
            "format": format.extension(),
            "mime": format.mime(),
            "filename": name,
            "path": path.to_string_lossy(),
            "bytes": text.len(),
            "rows": rows.len(),
            "columns": columns.len(),
            "media": media,
            // Small enough to show or copy in the UI. Withheld rather than
            // truncated above the cap: half a CSV looks like a whole one.
            "text": if text.len() <= PREVIEW_CAP { json!(text) } else { Value::Null },
        }))
    }

    /// Fetch every photo named by `rows` into `dir`.
    ///
    /// A fetch that fails is reported and does not fail the export: the table
    /// is the data, the photos are attachments to it, and losing the CSV
    /// because one image is not on this node would be the wrong trade. `local`
    /// is true — these are documents this node already received, and a
    /// network round trip per photo would turn an export into a long wait.
    fn export_media(&self, rows: &[Value], dir: &Path) -> Value {
        let mut wanted: Vec<String> = Vec::new();
        for row in rows {
            if let Some(values) = row.get("values").and_then(Value::as_object) {
                for value in values.values() {
                    collect_media_cids(value, &mut wanted);
                }
            }
        }
        wanted.sort();
        wanted.dedup();

        if wanted.is_empty() {
            return json!({ "requested": true, "wanted": 0, "written": 0, "failed": [] });
        }
        let Some(store) = self.store.as_deref() else {
            return json!({
                "requested": true,
                "wanted": wanted.len(),
                "written": 0,
                "failed": [{ "code": "storage_unavailable",
                             "message": "no Logos Storage is attached, so the photos cannot be fetched" }],
            });
        };
        if let Err(e) = std::fs::create_dir_all(dir) {
            return json!({
                "requested": true,
                "wanted": wanted.len(),
                "written": 0,
                "failed": [{ "code": "rejected", "message": format!("cannot make {}: {e}", dir.display()) }],
            });
        }

        let mut written = 0usize;
        let mut failed: Vec<Value> = Vec::new();
        for cid in &wanted {
            match store.get(store::bare_cid(cid), true) {
                Ok(bytes) => {
                    // Named by CID: the CSV cell holds the CID, so the row
                    // and the file match without a lookup table.
                    let file = dir.join(format!("{}.bin", store::bare_cid(cid)));
                    match std::fs::write(&file, &bytes) {
                        Ok(()) => written += 1,
                        Err(e) => failed.push(json!({
                            "cid": cid,
                            "code": "rejected",
                            "message": format!("cannot write {}: {e}", file.display()),
                        })),
                    }
                }
                Err(e) => failed.push(json!({
                    "cid": cid,
                    "code": e.code(),
                    "message": e.to_string(),
                })),
            }
        }
        json!({
            "requested": true,
            "wanted": wanted.len(),
            "written": written,
            "failed": failed,
        })
    }

    /// `submitters.hide` and `submitters.show` — drop a submitter's rows from
    /// the index, or put them back.
    ///
    /// Hiding is **not** deletion and cannot be: the submission documents are
    /// on Storage, `remove()` is local-only, and the DHT record is never
    /// withdrawn (SPEC 2). What this does is rewrite the index so the
    /// creator's results, table and exports all stop counting them — which
    /// works because every one of those reads through `hidden_for`.
    ///
    /// It is a published act, not a private setting: the new index page is
    /// signed and uploaded, so a submitter can see they were hidden. That is
    /// deliberate — a moderation decision nobody can observe is worse than
    /// one they can.
    pub fn set_hidden(&mut self, raw: &str, hide: bool, now: DateTime<Utc>) -> String {
        self.start_publish_budget();
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let Some(experiment_id) = request.get("experimentId").and_then(Value::as_str) else {
            return api::envelope_err(api::code::BAD_REQUEST, "`experimentId` is required");
        };
        let experiment_id = experiment_id.to_string();
        let Some(submitter) = request.get("submitter").and_then(Value::as_str) else {
            return api::envelope_err(api::code::BAD_REQUEST, "`submitter` is required");
        };
        let submitter = submitter.to_string();
        if !ids::is_key(&submitter) {
            return api::envelope_err(
                api::code::BAD_REQUEST,
                "`submitter` must be a 64-character hex key",
            );
        }
        let Some(experiment) = self
            .experiments
            .iter()
            .find(|e| e.get("id").and_then(Value::as_str) == Some(experiment_id.as_str()))
        else {
            return api::envelope_err(
                api::code::BAD_REQUEST,
                format!("no experiment `{experiment_id}` on this node"),
            );
        };
        if experiment.pointer("/creator/key").and_then(Value::as_str) != Some(&self.creator_key) {
            return api::envelope_err(
                api::code::NOT_YOURS,
                "only the creator of an experiment can hide a submitter",
            );
        }
        // The creator hiding themselves would make their own experiment
        // unreadable to them for no reason anyone wants.
        if submitter == self.creator_key && hide {
            return api::envelope_err(
                api::code::BAD_REQUEST,
                "the creator cannot hide themselves",
            );
        }

        let mut hidden = self.hidden_for(&experiment_id);
        let already = hidden.iter().any(|k| k == &submitter);
        if hide && !already {
            hidden.push(submitter.clone());
        } else if !hide && already {
            hidden.retain(|k| k != &submitter);
        } else {
            // Nothing to publish. Saying so beats emitting an identical page
            // at a new address and calling it a change.
            return api::envelope_ok(json!({
                "changed": false,
                "hidden": hidden,
                "published": false,
                "note": "Already in that state.",
            }));
        }

        // Rebuilt through the same pager the indexer uses, with no new
        // entries: one code path for paging, so hiding cannot produce a page
        // shape that indexing never would.
        let newest = self
            .index_pages
            .iter()
            .filter(|p| p.get("experiment").and_then(Value::as_str) == Some(experiment_id.as_str()))
            .max_by_key(|p| p.get("page").and_then(Value::as_u64).unwrap_or(0))
            .cloned();
        let newest_cid = newest
            .as_ref()
            .and_then(|p| p.get("page").and_then(Value::as_u64))
            .map(|n| self.page_key(&experiment_id, n))
            .and_then(|key| self.cid_of(&key).map(str::to_string));

        let pages = crate::indexer::page_entries(
            &experiment_id,
            newest.as_ref(),
            newest_cid.as_deref(),
            &[],
            &hidden,
            now,
        );

        let mut newest_page_cid: Option<String> = None;
        for page in pages {
            let page = match self.signed(&page) {
                Ok(signed) => signed,
                Err(e) => return api::envelope_err(api::code::NOT_WIRED, e),
            };
            let errors = schema::errors_as(&page, schema::Kind::Index);
            if !errors.is_empty() {
                return api::envelope_err(
                    api::code::INVALID_DOCUMENT,
                    format!("the rewritten index page does not validate: {}", errors.join("; ")),
                );
            }
            let number = page.get("page").and_then(Value::as_u64).unwrap_or(1);
            let key = self.page_key(&experiment_id, number);
            self.index_pages.retain(|p| {
                !(p.get("experiment").and_then(Value::as_str) == Some(experiment_id.as_str())
                    && p.get("page").and_then(Value::as_u64) == Some(number))
            });
            self.index_pages.push(page.clone());
            self.publish_doc("index", &key, &page);
            if let Some(cid) = self.cid_of(&key) {
                newest_page_cid = Some(cid.to_string());
            }
        }

        // The manifest names the latest index page, so it has to be re-signed
        // too — otherwise a reader following the manifest would fetch the old
        // page and not see the change.
        if let Some(cid) = newest_page_cid.clone() {
            if let Err(e) = self.reseal_with(&experiment_id, move |doc| {
                doc["index"] = json!(cid);
            }) {
                return api::envelope_err(api::code::INVALID_DOCUMENT, e);
            }
        }

        let mut data = json!({
            "changed": true,
            "hidden": self.hidden_for(&experiment_id),
            "submitter": submitter,
            "hiding": hide,
        });
        merge(&mut data, self.publish_report(&experiment_id));
        api::envelope_ok(data)
    }

    /// How an index page's publication is keyed. Pages have no `id`, so this
    /// is the identity used in its place.
    fn page_key(&self, experiment_id: &str, page: u64) -> String {
        format!("{experiment_id}#index{page}")
    }

    /// `experiments.publish` — try again to put a document on Storage.
    ///
    /// Exists because `storage_timeout` and `storage_rejected` are retryable:
    /// without this, one bad moment would leave a signed, valid experiment
    /// permanently unaddressable with no way forward but recreating it. The
    /// document is republished exactly as it stands, so a retry that succeeds
    /// lands on the same CID a first attempt would have.
    pub fn publish_now(&mut self, raw: &str) -> String {
        self.start_publish_budget();
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let Some(wanted) = request.get("experimentId").and_then(Value::as_str) else {
            return api::envelope_err(api::code::BAD_REQUEST, "`experimentId` is required");
        };
        let Some(doc) = self
            .experiments
            .iter()
            .find(|e| e.get("id").and_then(Value::as_str) == Some(wanted))
            .cloned()
        else {
            return api::envelope_err(
                api::code::BAD_REQUEST,
                format!("no experiment `{wanted}` on this node"),
            );
        };
        if !self.can_publish() {
            return api::envelope_err(
                api::code::NOT_WIRED,
                "no Logos Storage is attached to this build",
            );
        }

        let id = wanted.to_string();
        self.publish_doc("experiment", &id, &doc);
        let mut data = json!({ "experiment": doc });
        merge(&mut data, self.publish_report(&id));
        api::envelope_ok(data)
    }

    /// `protocols.saveDraft` — attach or replace a protocol on an experiment.
    ///
    /// Validated as a protocol document and through `parse_fields`, so a form
    /// the renderer could not draw is rejected here rather than on the
    /// submitter's screen.
    pub fn save_protocol(&mut self, raw: &str, now: DateTime<Utc>) -> String {
        self.start_publish_budget();
        let request: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return api::envelope_err(api::code::BAD_REQUEST, format!("not JSON: {e}")),
        };
        let Some(experiment_id) = request.get("experimentId").and_then(Value::as_str) else {
            return api::envelope_err(api::code::BAD_REQUEST, "`experimentId` is required");
        };
        let Some(draft) = request.get("protocol").filter(|p| p.is_object()) else {
            return api::envelope_err(api::code::BAD_REQUEST, "`protocol` is required");
        };
        if !self
            .experiments
            .iter()
            .any(|e| e.get("id").and_then(Value::as_str) == Some(experiment_id))
        {
            return api::envelope_err(
                api::code::BAD_REQUEST,
                format!("no experiment `{experiment_id}` on this node"),
            );
        }

        let name = draft.get("name").and_then(Value::as_str).unwrap_or("").trim();
        if name.is_empty() {
            return api::envelope_ok(json!({
                "saved": false,
                "problems": [{ "field": "name", "message": "A form name is required" }]
            }));
        }
        let id = match draft.get("id").and_then(Value::as_str) {
            Some(i) if ids::is_protocol_id(i) => i.to_string(),
            _ => slug(name),
        };

        let existing_version = self
            .protocols
            .iter()
            .find(|p| p.get("id").and_then(Value::as_str) == Some(id.as_str()))
            .and_then(|p| p.get("version").and_then(Value::as_u64))
            .unwrap_or(0);

        let protocol = json!({
            "schema": "collect/protocol@1",
            "id": id,
            // SPEC 4: a new version on every publish where anything changed.
            "version": existing_version + 1,
            "experiment": experiment_id,
            "name": name,
            "instructions": draft.get("instructions").and_then(Value::as_str).unwrap_or(""),
            "fields": draft.get("fields").cloned().unwrap_or_else(|| json!([])),
            "allow_multiple": draft.get("allow_multiple").and_then(Value::as_bool).unwrap_or(true),
            "allow_drafts": draft.get("allow_drafts").and_then(Value::as_bool).unwrap_or(true),
            "created_at": now.to_rfc3339(),
            "sig": "00".repeat(64),
        });

        let protocol = match self.signed(&protocol) {
            Ok(signed) => signed,
            Err(e) => return api::envelope_err(api::code::NOT_WIRED, e),
        };

        let mut problems: Vec<Value> = schema::errors_as(&protocol, schema::Kind::Protocol)
            .into_iter()
            .map(|m| json!({ "field": "fields", "message": m }))
            .collect();
        if problems.is_empty() {
            if let Err(e) = fields::parse_fields(&protocol) {
                problems.push(json!({ "field": "fields", "message": e.to_string() }));
            }
        }
        if !problems.is_empty() {
            return api::envelope_ok(json!({ "saved": false, "problems": problems }));
        }

        let protocol_id = protocol["id"].as_str().unwrap().to_string();
        let version = protocol["version"].as_u64().unwrap();
        self.protocols
            .retain(|p| p.get("id").and_then(Value::as_str) != Some(protocol_id.as_str()));
        self.protocols.push(protocol.clone());

        // The form goes up first, so the manifest entry can name a real
        // address. `logos:pending` stands in only while there is no Storage;
        // it is a placeholder a reader can recognise, not a fake CID.
        self.publish_doc("protocol", &protocol_id, &protocol);
        let protocol_cid = self
            .cid_of(&protocol_id)
            .unwrap_or("logos:pending")
            .to_string();

        // Point the manifest at it, replacing any earlier entry for this id.
        let experiment_id = experiment_id.to_string();
        if let Some(experiment) = self
            .experiments
            .iter_mut()
            .find(|e| e.get("id").and_then(Value::as_str) == Some(experiment_id.as_str()))
        {
            let entry = json!({
                "id": protocol_id,
                "version": version,
                "cid": protocol_cid,
                "status": "open"
            });
            match experiment.get_mut("protocols").and_then(Value::as_array_mut) {
                Some(list) => {
                    list.retain(|p| p.get("id").and_then(Value::as_str) != Some(protocol_id.as_str()));
                    list.push(entry);
                }
                None => {
                    experiment["protocols"] = json!([entry]);
                }
            }
        }

        // The manifest now says something different from what it was signed
        // over, so it has to become a new, re-signed version of itself.
        let manifest = match self.reseal(&experiment_id) {
            Ok(doc) => doc,
            Err(e) => return api::envelope_err(api::code::INVALID_DOCUMENT, e),
        };

        let mut data = json!({
            "saved": true,
            "problems": [],
            "protocol": protocol,
            "experiment": manifest,
        });
        merge(&mut data, self.publish_report(&protocol_id));
        api::envelope_ok(data)
    }
}

/// How long one call may spend uploading, across every document it touches.
///
/// Comfortably inside the twenty-second protocol default, so the UI always
/// gets an answer: a partial publish it can retry beats a reply that never
/// comes.
/// Where thumbnail images are cached, under the state directory, by sha256.
const THUMB_DIR: &str = "thumbnails";

/// The largest thumbnail this node will show. Ours are about 60 KB; this is
/// room for someone else's encoder, not an invitation.
const THUMB_MAX_BYTES: usize = 1024 * 1024;

/// How many missing thumbnails one drain may go looking for on the network.
const THUMB_FETCHES_PER_DRAIN: usize = 2;

/// Do these bytes match the manifest's media entry? The signed manifest names
/// a hash and a type; anything else at that CID is not shown.
fn thumbnail_matches(entry: &Value, bytes: &[u8]) -> bool {
    use sha2::{Digest, Sha256};
    let sha = entry.get("sha256").and_then(Value::as_str).unwrap_or("");
    let mime = entry.get("mime").and_then(Value::as_str).unwrap_or("");
    bytes.len() <= THUMB_MAX_BYTES
        && hex::encode(Sha256::digest(bytes)) == sha
        && crate::media::Format::sniff(bytes).map(|f| f.mime()) == Some(mime)
}

/// A file path from what the UI sends: a plain path, or a `file://` URL as a
/// file dialog returns it, percent-encoded — so a photo under "Application
/// Support" arrives as `Application%20Support`.
fn local_path(raw: &str) -> String {
    let Some(rest) = raw.strip_prefix("file://") else {
        return raw.to_string();
    };
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&rest[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| rest.to_string())
}

/// Experiments, forms, submissions and index pages this node holds, in the
/// host's per-instance directory (SPEC Q8).
const STATE_FILE: &str = "state.json";

/// How soon an unconfirmed submission is announced again, and the most the
/// gap grows to.
const OUTBOX_FIRST_GAP: chrono::Duration = chrono::Duration::minutes(3);
const OUTBOX_MAX_GAP: chrono::Duration = chrono::Duration::hours(1);

/// How long a results push may spend fetching photos it does not hold.
/// Anything not fetched by then is listed as missing in the snapshot, and
/// pushing again tries it again.
const RESULTS_MEDIA_BUDGET: std::time::Duration = std::time::Duration::from_secs(60);

const PUBLISH_BUDGET: std::time::Duration = std::time::Duration::from_secs(14);

/// How much export text travels back to the UI for preview. Above this the
/// path is all it gets: half a CSV looks exactly like a whole one.
const PREVIEW_CAP: usize = 256 * 1024;

/// Every media CID inside one submitted value, at any depth.
///
/// Recursive because a photo field holds an array of media objects while a
/// future field type might nest them differently, and a missed CID means a
/// silently incomplete media export.
fn collect_media_cids(value: &Value, into: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_media_cids(item, into);
            }
        }
        Value::Object(map) => {
            // A media entry, as the contract defines it: a CID with the
            // hash and type of the bytes it addresses. The `sha256` is what
            // distinguishes it from any other object that happens to carry a
            // `cid`, such as a protocol reference in a manifest.
            match (map.get("cid").and_then(Value::as_str), map.get("sha256")) {
                (Some(cid), Some(_)) => into.push(cid.to_string()),
                _ => {
                    for nested in map.values() {
                        collect_media_cids(nested, into);
                    }
                }
            }
        }
        _ => {}
    }
}

/// The experiment detail fields, validated once for both `create` and
/// `update`.
///
/// Shared so the two cannot drift: a title length the form accepts on create
/// and refuses on edit would be a bug only an editing user ever sees.
#[derive(Clone)]
struct Details {
    title: String,
    description: String,
    /// RFC 3339, already parsed and re-formatted, so the caller cannot get a
    /// string in that was never a date.
    starts_at: String,
    ends_at: String,
    timezone: String,
    creator_name: String,
}

impl Details {
    /// `Err` carries the field-level complaints the C2 form shows inline.
    fn read(request: &Value) -> Result<Details, Vec<Value>> {
        let text = |key: &str| {
            request
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let title = text("title");
        let description = text("description");
        let timezone = {
            let tz = text("timezone");
            if tz.is_empty() { "UTC".to_string() } else { tz }
        };

        let mut problems: Vec<Value> = Vec::new();
        if title.is_empty() {
            problems.push(json!({ "field": "title", "message": "A title is required" }));
        }
        if title.chars().count() > 140 {
            problems.push(json!({ "field": "title", "message": "At most 140 characters" }));
        }
        if description.chars().count() > 5000 {
            problems.push(json!({ "field": "description", "message": "At most 5000 characters" }));
        }
        let from = DateTime::parse_from_rfc3339(&text("startsAt"));
        let to = DateTime::parse_from_rfc3339(&text("endsAt"));
        if from.is_err() {
            problems.push(json!({ "field": "startsAt", "message": "A start date is required" }));
        }
        if to.is_err() {
            problems.push(json!({ "field": "endsAt", "message": "An end date is required" }));
        }
        // SPEC 3.1: the end must not be before the start.
        if let (Ok(f), Ok(t)) = (&from, &to) {
            if t < f {
                problems.push(json!({
                    "field": "endsAt",
                    "message": "The end date cannot be before the start date"
                }));
            }
        }
        if !problems.is_empty() {
            return Err(problems);
        }

        Ok(Details {
            title,
            description,
            starts_at: from.unwrap().to_rfc3339(),
            ends_at: to.unwrap().to_rfc3339(),
            timezone,
            creator_name: text("creatorName"),
        })
    }
}

/// Copy `extra`'s keys onto `target`, so a response can be assembled from a
/// literal plus a shared report without one of them having to know the other's
/// shape. Non-objects are ignored rather than replacing the target.
fn merge(target: &mut Value, extra: Value) {
    if let (Some(into), Some(from)) = (target.as_object_mut(), extra.as_object()) {
        for (k, v) in from {
            into.insert(k.clone(), v.clone());
        }
    }
}

/// A protocol id from a form name: lower-case alphanumeric groups joined by
/// single hyphens, which is what the contract's pattern allows.
fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut pending_hyphen = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if pending_hyphen && !out.is_empty() {
                out.push('-');
            }
            pending_hyphen = false;
            out.push(c.to_ascii_lowercase());
        } else {
            pending_hyphen = true;
        }
    }
    if out.is_empty() {
        out.push_str("form");
    }
    out.truncate(64);
    out.trim_end_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    // Only the tests build a failing store, so this would be dead weight in
    // the module itself.
    use crate::store::StoreError;

    fn at(ts: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(ts).unwrap().with_timezone(&Utc)
    }

    fn envelope(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    fn data(text: &str) -> Value {
        let e = envelope(text);
        assert_eq!(e["ok"], json!(true), "{e}");
        e["data"].clone()
    }

    /// A scratch directory that cleans itself up.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            // A counter, not just the name: tests run in parallel and several
            // share a name, so a shared directory means one test's cleanup
            // deletes another's signing key mid-run.
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir()
                .join(format!("collect-session-{name}-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A session with no fixtures and a real signing key, so creation starts
    /// from nothing but can still sign.
    fn empty_in(scratch: &Scratch) -> Session {
        let mut session = Session {
            experiments: Vec::new(),
            protocols: Vec::new(),
            submissions: Vec::new(),
            index_pages: Vec::new(),
            creator_key: "3a".repeat(32),
            identity: None,
            store: None,
            publications: BTreeMap::new(),
            publish_until: None,
            state_dir: None,
            saved_state: None,
            thumbs: Default::default(),
            thumbs_tried: Default::default(),
            send_failure: None,
            last_level: Default::default(),
            approvals: BTreeMap::new(),
            payouts: Vec::new(),
            ledger: None,
            outbox: Vec::new(),
            courier: None,
            seen: delivery::Seen::default(),
            receipts: BTreeMap::new(),
            heads: BTreeMap::new(),
            social: Social::default(),
        };
        let public = session
            .attach_identity(scratch.0.to_str().unwrap())
            .expect("identity");
        // Outside fixtures mode the signing key becomes the creator.
        session.creator_key = public;
        session
    }

    fn create_request(starts: &str, ends: &str) -> String {
        json!({
            "title": "Urban birds",
            "description": "Counting birds.",
            "startsAt": starts,
            "endsAt": ends,
            "timezone": "Asia/Kolkata",
            "creatorName": "Dr Rao"
        })
        .to_string()
    }

    /// A session that can sign *and* publish, to an in-process store.
    fn publishing_in(scratch: &Scratch) -> Session {
        let mut session = empty_in(scratch);
        session.attach_store(Box::new(crate::store::MemoryStore::new()));
        session
    }

    /// The protocol draft used by the publishing tests.
    fn draft_for(id: &str) -> String {
        json!({
            "experimentId": id,
            "protocol": {
                "name": "Bird sighting",
                "fields": [
                    { "key": "count", "type": "number", "label": "How many", "required": true }
                ]
            }
        })
        .to_string()
    }

    #[test]
    fn without_a_store_nothing_is_published_and_the_response_says_so() {
        let scratch = Scratch::new("nostore");
        let mut s = empty_in(&scratch);
        let made = data(&s.create(
            &create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        assert_eq!(made["published"], json!(false));
        // Never tried is not the same as failed, so there is no error to show.
        assert_eq!(made["publish_error"], Value::Null);
        assert!(made["note"].as_str().unwrap().contains("no Logos Storage"));
        assert!(!s.can_publish());
    }

    /// A photo on disk, as the file dialog would leave it, under a directory
    /// with a space in it so the URL form is percent-encoded.
    fn photo_on_disk(scratch: &Scratch) -> (std::path::PathBuf, String) {
        let dir = scratch.0.join("My Photos");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cover.jpg");
        std::fs::write(&path, crate::media::tests_support::gps_jpeg_upright()).unwrap();
        let url = format!("file://{}", path.display()).replace(' ', "%20");
        (path, url)
    }

    fn made(s: &mut Session) -> String {
        let made = data(&s.create(
            &create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        made["experiment"]["id"].as_str().or(made["id"].as_str()).unwrap().to_string()
    }

    #[test]
    fn a_form_photo_is_cleaned_stored_and_its_location_read() {
        let scratch = Scratch::new("formphoto");
        let mut s = publishing_in(&scratch);
        let (_, url) = photo_on_disk(&scratch);
        let out = data(&s.add_photo(&json!({ "path": url, "maxMbEach": 5 }).to_string()));

        let entry = &out["entry"];
        let cid = entry["cid"].as_str().expect("a cid");
        assert!(cid.starts_with("logos:"), "{out}");
        let held = s.store.as_deref().unwrap().get(store::bare_cid(cid), true).unwrap();
        assert_eq!(crate::media::remaining_metadata(&held), None, "GPS went to Storage");
        assert_eq!(entry["bytes"], json!(held.len()));
        // What was read before cleaning comes back for the form to prefill.
        assert!(out["exif"]["location"].is_object(), "{out}");
        assert_eq!(out["name"], json!("cover.jpg"));

        // And it is exactly what a submission's photo field accepts.
        let doc = json!({ "photos": [entry] });
        assert!(doc["photos"][0]["sha256"].as_str().unwrap().len() == 64);
    }

    #[test]
    fn a_form_photo_needs_storage_and_respects_the_size_limit() {
        let scratch = Scratch::new("formphoto-limits");
        let (path, _) = photo_on_disk(&scratch);
        let mut offline = empty_in(&scratch);
        let out = envelope(&offline.add_photo(&json!({ "path": path.to_str().unwrap() }).to_string()));
        assert_eq!(out["error"]["code"], json!(api::code::NOT_WIRED));

        let mut s = publishing_in(&scratch);
        let out = envelope(&s.add_photo(
            &json!({ "path": path.to_str().unwrap(), "maxMbEach": 0.000001 }).to_string(),
        ));
        assert_eq!(out["error"]["code"], json!(api::code::MEDIA), "{out}");
    }

    #[test]
    fn a_thumbnail_goes_to_storage_and_into_a_new_signed_manifest() {
        let scratch = Scratch::new("thumb");
        let mut s = publishing_in(&scratch);
        let id = made(&mut s);
        let (_, url) = photo_on_disk(&scratch);

        let out = data(&s.set_thumbnail(&json!({ "experimentId": id, "path": url }).to_string()));
        let manifest = &out["experiment"];
        assert_eq!(manifest["version"], json!(2), "{out}");
        assert!(crate::sign::verify(manifest).is_ok());
        assert!(schema::errors_as(manifest, schema::Kind::Experiment).is_empty());
        let entry = &manifest["thumbnail"];
        assert_eq!(entry["mime"], json!("image/jpeg"));
        assert_eq!(entry["width"], json!(crate::media::THUMB_WIDTH));

        // What Storage holds is the cleaned thumbnail the entry describes.
        let store = s.store.as_deref().unwrap();
        let held = store.get(store::bare_cid(entry["cid"].as_str().unwrap()), true).unwrap();
        assert!(thumbnail_matches(entry, &held));
        assert_eq!(crate::media::remaining_metadata(&held), None, "GPS went to Storage");

        // And both screens get something to show.
        let uri = out["thumbnail"].as_str().expect("the detail page gets an image");
        assert!(uri.starts_with("data:image/jpeg;base64,"));
        let card = &data(&s.list(at("2026-10-28T08:00:00Z")))["experiments"][0];
        assert_eq!(card["thumbnail"], json!(uri));
    }

    #[test]
    fn a_thumbnail_is_refused_without_storage_and_nothing_changes() {
        let scratch = Scratch::new("thumb-nostore");
        let mut s = empty_in(&scratch);
        let id = made(&mut s);
        let (path, _) = photo_on_disk(&scratch);
        let out = envelope(&s.set_thumbnail(
            &json!({ "experimentId": id, "path": path.to_str().unwrap() }).to_string(),
        ));
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"]["code"], json!(api::code::NOT_WIRED));
        let manifest = &data(&s.get(&json!({ "experimentId": id }).to_string()))["experiment"];
        assert_eq!(manifest["version"], json!(1));
        assert!(manifest.get("thumbnail").is_none());
    }

    #[test]
    fn only_the_creator_can_set_a_thumbnail() {
        let scratch = Scratch::new("thumb-other");
        let mut s = publishing_in(&scratch);
        let id = made(&mut s);
        s.creator_key = "3a".repeat(32);
        let (path, _) = photo_on_disk(&scratch);
        let out = envelope(&s.set_thumbnail(
            &json!({ "experimentId": id, "path": path.to_str().unwrap() }).to_string(),
        ));
        assert_eq!(out["error"]["code"], json!(api::code::REJECTED), "{out}");
    }

    #[test]
    fn clearing_a_thumbnail_makes_a_version_without_one() {
        let scratch = Scratch::new("thumb-clear");
        let mut s = publishing_in(&scratch);
        let id = made(&mut s);
        let (path, _) = photo_on_disk(&scratch);
        s.set_thumbnail(&json!({ "experimentId": id, "path": path.to_str().unwrap() }).to_string());
        let out = data(&s.clear_thumbnail(&json!({ "experimentId": id }).to_string()));
        assert_eq!(out["experiment"]["version"], json!(3));
        assert!(out["experiment"].get("thumbnail").is_none());
        assert_eq!(out["thumbnail"], Value::Null);
        // Clearing again changes nothing.
        let again = data(&s.clear_thumbnail(&json!({ "experimentId": id }).to_string()));
        assert_eq!(again["experiment"]["version"], json!(3));
    }

    #[test]
    fn a_thumbnail_that_does_not_match_its_manifest_is_not_shown() {
        let scratch = Scratch::new("thumb-tamper");
        let mut s = publishing_in(&scratch);
        let id = made(&mut s);
        let (path, _) = photo_on_disk(&scratch);
        let out = data(&s.set_thumbnail(
            &json!({ "experimentId": id, "path": path.to_str().unwrap() }).to_string(),
        ));
        let sha = out["experiment"]["thumbnail"]["sha256"].as_str().unwrap().to_string();
        // Swap the cached bytes for something else, and forget the memo.
        std::fs::write(scratch.0.join(THUMB_DIR).join(format!("{sha}.img")), b"\xFF\xD8 not it").unwrap();
        s.thumbs.borrow_mut().clear();
        let card = &data(&s.list(at("2026-10-28T08:00:00Z")))["experiments"][0];
        assert_eq!(card["thumbnail"], Value::Null);
    }

    #[test]
    fn file_urls_are_decoded_to_paths() {
        assert_eq!(local_path("/a/b.jpg"), "/a/b.jpg");
        assert_eq!(
            local_path("file:///Users/m/Library/Application%20Support/x.jpg"),
            "/Users/m/Library/Application Support/x.jpg"
        );
        assert_eq!(local_path("file:///tmp/%E0%A4%95.jpg"), "/tmp/\u{915}.jpg");
        assert_eq!(local_path("file:///tmp/50%.jpg"), "/tmp/50%.jpg");
    }

    #[test]
    fn connection_reads_green_amber_or_red() {
        let scratch = Scratch::new("connection");
        let mut s = empty_in(&scratch);
        assert_eq!(s.connection()["level"], json!("offline"), "nothing attached");

        s.attach_store(Box::new(crate::store::MemoryStore::new()));
        assert_eq!(s.connection()["level"], json!("degraded"), "storage without delivery");

        for (reported, expected) in [
            (Some("Connected"), "online"),
            (Some("PartiallyConnected"), "degraded"),
            (Some("Disconnected"), "offline"),
            (None, "degraded"),
        ] {
            s.attach_courier(Box::new(delivery::MemoryCourier::connected(reported)));
            let c = s.connection();
            assert_eq!(c["level"], json!(expected), "{reported:?}: {c}");
            assert!(!c["reason"].as_str().unwrap().is_empty());
        }
    }

    #[test]
    fn a_refused_send_turns_the_dot_amber_until_one_gets_out() {
        let scratch = Scratch::new("connection-refused");
        let mut s = publishing_in(&scratch);
        let courier = std::sync::Arc::new(crate::delivery::MemoryCourier::new());
        s.attach_courier(Box::new(SharedCourier(courier.clone())));
        assert_eq!(s.connection()["level"], json!("online"));

        courier.push_raw(json!({ "kind": "sendFailed", "requestId": "r1", "messageHash": "0x1",
                             "error": "Unable to send within retry time window" }));
        s.drain(at("2026-10-30T09:16:53Z"));
        let c = s.connection();
        assert_eq!(c["level"], json!("degraded"), "{c}");
        assert!(c["reason"].as_str().unwrap().contains("RLN membership"), "{c}");

        courier.push_raw(json!({ "kind": "sent", "requestId": "r2", "messageHash": "0x2" }));
        s.drain(at("2026-10-30T09:20:00Z"));
        assert_eq!(s.connection()["level"], json!("online"));
    }

    #[test]
    fn connected_without_rln_is_amber_not_green() {
        let scratch = Scratch::new("connection-rln");
        let mut s = publishing_in(&scratch);
        s.attach_courier(Box::new(delivery::MemoryCourier::without_rln()));
        let c = s.connection();
        assert_eq!(c["level"], json!("degraded"), "{c}");
        assert!(c["reason"].as_str().unwrap().contains("RLN"), "{c}");
    }

    #[test]
    fn experiments_survive_a_relaunch() {
        // The bug that emptied the dashboard after every update: experiments
        // lived only in memory.
        let scratch = Scratch::new("relaunch");
        let mut s = publishing_in(&scratch);
        let made = data(&s.create(
            &create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        let id = made["experiment"]["id"].as_str().or(made["id"].as_str()).unwrap().to_string();
        let cid = made["cid"].as_str().unwrap().to_string();
        s.save_state();
        drop(s);

        let again = empty_in(&scratch);
        let cards = data(&again.list(at("2026-10-28T08:00:00Z")))["experiments"].clone();
        assert_eq!(cards.as_array().unwrap().len(), 1, "{cards}");
        assert_eq!(cards[0]["id"], json!(id));
        assert_eq!(cards[0]["cid"], json!(cid), "the published address was forgotten");
    }

    #[test]
    fn an_unchanged_session_does_not_rewrite_the_state_file() {
        let scratch = Scratch::new("unchanged");
        let mut s = empty_in(&scratch);
        s.create(
            &create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        );
        s.save_state();
        let path = scratch.0.join(STATE_FILE);
        std::fs::write(&path, b"{\"marker\": true}").unwrap();
        s.save_state();
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"marker\": true}");
    }

    #[test]
    fn a_saved_document_that_fails_the_schema_is_dropped_not_shown() {
        let scratch = Scratch::new("badstate");
        let mut s = empty_in(&scratch);
        s.create(
            &create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        );
        s.save_state();
        let path = scratch.0.join(STATE_FILE);
        let mut state: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let good = state["experiments"][0].clone();
        state["experiments"] = json!([good, { "kind": "experiment", "title": 7 }]);
        std::fs::write(&path, state.to_string()).unwrap();

        let again = empty_in(&scratch);
        let cards = data(&again.list(at("2026-10-28T08:00:00Z")))["experiments"].clone();
        assert_eq!(cards.as_array().unwrap().len(), 1, "{cards}");
    }

    #[test]
    fn an_unreadable_state_file_is_moved_aside_not_overwritten() {
        let scratch = Scratch::new("garbled");
        std::fs::write(scratch.0.join(STATE_FILE), b"not json").unwrap();
        let mut s = empty_in(&scratch);
        assert!(s.experiments.is_empty());
        let aside = scratch.0.join(format!("{STATE_FILE}.unreadable"));
        assert_eq!(std::fs::read(&aside).unwrap(), b"not json");
        s.create(
            &create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        );
        s.save_state();
        assert_eq!(std::fs::read(&aside).unwrap(), b"not json", "the old file was clobbered");
    }

    #[test]
    fn creating_with_a_store_publishes_the_manifest_and_reports_its_cid() {
        let scratch = Scratch::new("pubcreate");
        let mut s = publishing_in(&scratch);
        let made = data(&s.create(
            &create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        assert_eq!(made["published"], json!(true), "{made}");
        let cid = made["cid"].as_str().expect("a cid");
        assert!(cid.starts_with("logos:"), "{cid}");
        assert!(s.can_publish());

        // And the dashboard card carries the same address, so the share link
        // stops saying `pending`.
        let card = &data(&s.list(at("2026-10-28T08:00:00Z")))["experiments"][0];
        assert_eq!(card["cid"], json!(cid));
        assert_eq!(card["published"], json!(true));
        assert!(card["link"].as_str().unwrap().contains(cid.trim_start_matches("logos:")));
    }

    #[test]
    fn the_published_bytes_are_the_canonical_document() {
        // The CID has to address exactly what a reader would canonicalize
        // from the document it fetched — otherwise the address and the
        // content disagree and verification is impossible.
        let scratch = Scratch::new("pubbytes");
        let store = std::sync::Arc::new(crate::store::MemoryStore::new());
        let mut s = empty_in(&scratch);
        s.attach_store(Box::new(SharedStore(store.clone())));
        let made = data(&s.create(
            &create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        let cid = made["cid"].as_str().unwrap();
        let fetched = store.get(crate::store::bare_cid(cid), true).unwrap();
        assert_eq!(
            fetched,
            crate::canonical::to_bytes(&made["experiment"]).unwrap()
        );
    }

    #[test]
    fn a_failing_store_does_not_lose_the_experiment() {
        // Storage being down is not a reason to throw away a signed, valid
        // document the creator just made. It is kept, and the failure is
        // reported with a code the UI can offer a retry on.
        let scratch = Scratch::new("pubfail");
        let mut s = empty_in(&scratch);
        s.attach_store(Box::new(crate::store::MemoryStore::failing(
            StoreError::Timeout("no completion in 30s".into()),
        )));
        let made = data(&s.create(
            &create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        assert_eq!(made["created"], json!(true));
        assert_eq!(made["published"], json!(false));
        assert_eq!(made["publish_error"]["code"], json!("storage_timeout"));
        assert_eq!(data(&s.list(at("2026-10-28T08:00:00Z")))["experiments"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn attaching_a_form_publishes_it_and_names_its_cid_in_the_manifest() {
        let scratch = Scratch::new("pubform");
        let mut s = publishing_in(&scratch);
        let now = at("2026-10-28T08:00:00Z");
        let made = data(&s.create(&create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"), now));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();

        let saved = data(&s.save_protocol(&draft_for(&id), now));
        assert_eq!(saved["published"], json!(true), "{saved}");
        let form_cid = saved["cid"].as_str().expect("a cid").to_string();

        // The manifest entry points at the form's real address rather than
        // the placeholder.
        let entry = &saved["experiment"]["protocols"][0];
        assert_eq!(entry["cid"], json!(form_cid));
        assert_ne!(entry["cid"], json!("logos:pending"));
    }

    #[test]
    fn amending_a_manifest_resigns_it_as_a_new_version() {
        // Editing a signed document invalidates its signature. Attaching a
        // form edits the manifest, so the manifest has to become a new signed
        // version of itself — otherwise we would publish a document that no
        // reader could verify.
        let scratch = Scratch::new("reseal");
        let mut s = publishing_in(&scratch);
        let now = at("2026-10-28T08:00:00Z");
        let made = data(&s.create(&create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"), now));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();
        let first_cid = made["cid"].as_str().unwrap().to_string();
        assert_eq!(made["experiment"]["version"], json!(1));
        assert_eq!(made["experiment"]["prev"], Value::Null);

        let saved = data(&s.save_protocol(&draft_for(&id), now));
        let manifest = &saved["experiment"];
        assert_eq!(manifest["version"], json!(2));
        // `prev` is the address the previous version was published to, which
        // is what makes the version chain walkable.
        assert_eq!(manifest["prev"], json!(first_cid));

        // The signature covers the amended document.
        let mut key = [0u8; 32];
        hex::decode_to_slice(s.creator_key(), &mut key).unwrap();
        assert!(crate::sign::verify_with(manifest, &key).is_ok(), "{manifest}");

        // And the manifest moved to a new address, because its bytes changed.
        let second_cid = s.publication(&id).unwrap().cid().unwrap().to_string();
        assert_ne!(second_cid, first_cid);
    }

    #[test]
    fn a_resealed_manifest_is_what_gets_published() {
        // Guards the ordering: if the manifest were published before being
        // re-signed, the address would hold the stale signature.
        let scratch = Scratch::new("resealbytes");
        let store = std::sync::Arc::new(crate::store::MemoryStore::new());
        let mut s = empty_in(&scratch);
        s.attach_store(Box::new(SharedStore(store.clone())));
        let now = at("2026-10-28T08:00:00Z");
        let made = data(&s.create(&create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"), now));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();
        let saved = data(&s.save_protocol(&draft_for(&id), now));

        let cid = s.publication(&id).unwrap().cid().unwrap().to_string();
        let bytes = store.get(crate::store::bare_cid(&cid), true).unwrap();
        let published: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(published, saved["experiment"]);
        let mut key = [0u8; 32];
        hex::decode_to_slice(s.creator_key(), &mut key).unwrap();
        assert!(crate::sign::verify_with(&published, &key).is_ok());
    }

    #[test]
    fn republishing_an_unchanged_document_does_not_move_it() {
        // The address is the content, so the same document is the same CID.
        // This is what makes a retry safe.
        let scratch = Scratch::new("idempotent");
        let mut s = publishing_in(&scratch);
        let now = at("2026-10-28T08:00:00Z");
        let made = data(&s.create(&create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"), now));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();
        let first = s.publication(&id).unwrap().cid().unwrap().to_string();
        let doc = made["experiment"].clone();
        s.publish_doc("experiment", &id, &doc);
        assert_eq!(s.publication(&id).unwrap().cid().unwrap(), first);
    }

    #[test]
    fn a_failed_publish_can_be_retried_and_lands_on_the_same_address() {
        let scratch = Scratch::new("retry");
        let mut s = empty_in(&scratch);
        // Fails first...
        s.attach_store(Box::new(crate::store::MemoryStore::failing(
            StoreError::Timeout("no completion in 120s".into()),
        )));
        let made = data(&s.create(
            &create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();
        assert_eq!(made["publish_error"]["code"], json!("storage_timeout"));

        // ...then Storage comes back, and the retry publishes the document
        // exactly as it stands.
        s.attach_store(Box::new(crate::store::MemoryStore::new()));
        let out = data(&s.publish_now(&json!({ "experimentId": id }).to_string()));
        assert_eq!(out["published"], json!(true), "{out}");
        assert_eq!(out["experiment"], made["experiment"]);
        let cid = out["cid"].as_str().unwrap();
        assert_eq!(
            cid,
            crate::store::as_document_cid(&format!(
                "mem1{}",
                {
                    use sha2::{Digest, Sha256};
                    let mut h = Sha256::new();
                    h.update(crate::canonical::to_bytes(&made["experiment"]).unwrap());
                    hex::encode(h.finalize())
                }
            ))
            .unwrap(),
            "the retry must address the same bytes"
        );
    }

    #[test]
    fn retrying_without_a_store_is_refused_rather_than_reported_as_published() {
        let scratch = Scratch::new("retrynostore");
        let mut s = empty_in(&scratch);
        let made = data(&s.create(
            &create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        let id = made["experiment"]["id"].as_str().unwrap();
        let out = envelope(&s.publish_now(&json!({ "experimentId": id }).to_string()));
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"]["code"], json!("not_wired"));
    }

    #[test]
    fn publishing_an_unknown_experiment_is_a_bad_request() {
        let scratch = Scratch::new("retryunknown");
        let mut s = publishing_in(&scratch);
        let out = envelope(&s.publish_now(&json!({ "experimentId": "nope" }).to_string()));
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"]["code"], json!("bad_request"));
    }

    fn update_request(id: &str, title: &str, starts: &str, ends: &str) -> String {
        json!({
            "experimentId": id,
            "title": title,
            "description": "Counting birds, revised.",
            "startsAt": starts,
            "endsAt": ends,
            "timezone": "Asia/Kolkata",
            "creatorName": "Dr Rao"
        })
        .to_string()
    }

    #[test]
    fn editing_an_experiment_makes_a_new_signed_version() {
        let scratch = Scratch::new("edit");
        let mut s = publishing_in(&scratch);
        let now = at("2026-10-28T08:00:00Z");
        let made = data(&s.create(&create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"), now));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();
        let first_cid = made["cid"].as_str().unwrap().to_string();

        let out = data(&s.update(
            &update_request(&id, "Urban birds, 2027", "2026-10-20T00:00:00Z", "2026-12-20T00:00:00Z"),
            now,
        ));
        assert_eq!(out["updated"], json!(true), "{out}");
        let doc = &out["experiment"];
        assert_eq!(doc["title"], json!("Urban birds, 2027"));
        assert_eq!(doc["description"], json!("Counting birds, revised."));
        assert_eq!(doc["ends_at"], json!("2026-12-20T00:00:00+00:00"));
        assert_eq!(doc["version"], json!(2));
        assert_eq!(doc["prev"], json!(first_cid));

        let mut key = [0u8; 32];
        hex::decode_to_slice(s.creator_key(), &mut key).unwrap();
        assert!(crate::sign::verify_with(doc, &key).is_ok(), "{doc}");

        // The edit is what the dashboard and the detail screen now show.
        assert_eq!(data(&s.list(now))["experiments"][0]["title"], json!("Urban birds, 2027"));
        assert_eq!(
            data(&s.get(&json!({ "experimentId": id }).to_string()))["experiment"]["title"],
            json!("Urban birds, 2027")
        );
    }

    #[test]
    fn an_edit_is_republished_at_a_new_address() {
        let scratch = Scratch::new("editpub");
        let mut s = publishing_in(&scratch);
        let now = at("2026-10-28T08:00:00Z");
        let made = data(&s.create(&create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"), now));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();
        let first = made["cid"].as_str().unwrap().to_string();
        let out = data(&s.update(&update_request(&id, "Renamed", "2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"), now));
        assert_eq!(out["published"], json!(true));
        assert_ne!(out["cid"].as_str().unwrap(), first);
    }

    #[test]
    fn an_edit_cannot_change_the_creator_key() {
        // The key is what makes the attribution mean anything, so it is left
        // exactly as signed even though the display name is editable.
        let scratch = Scratch::new("editkey");
        let mut s = publishing_in(&scratch);
        let now = at("2026-10-28T08:00:00Z");
        let made = data(&s.create(&create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"), now));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();
        let mut request: Value =
            serde_json::from_str(&update_request(&id, "Renamed", "2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z")).unwrap();
        request["creator"] = json!({ "key": "ff".repeat(32), "name": "Someone else" });
        let out = data(&s.update(&request.to_string(), now));
        assert_eq!(out["experiment"]["creator"]["key"], json!(s.creator_key()));
        assert_eq!(out["experiment"]["creator"]["name"], json!("Dr Rao"));
    }

    #[test]
    fn an_experiment_someone_else_created_cannot_be_edited() {
        // No scratch directory: the ownership check comes before signing, so
        // this needs no key at all.
        let mut s = seeded();
        // The fixture is authored by the fixture key; make this node someone
        // else, so the example is no longer ours.
        s.creator_key = "bb".repeat(32);
        let out = envelope(&s.update(
            &update_request("q7k2m3x4w3b5n6tz", "Hijacked", "2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"]["code"], json!("not_yours"));
    }

    #[test]
    fn an_edit_is_validated_with_the_same_rules_as_creation() {
        let scratch = Scratch::new("editvalid");
        let mut s = publishing_in(&scratch);
        let now = at("2026-10-28T08:00:00Z");
        let made = data(&s.create(&create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"), now));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();

        let blank = data(&s.update(&update_request(&id, "", "2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"), now));
        assert_eq!(blank["updated"], json!(false));
        assert_eq!(blank["problems"][0]["field"], json!("title"));

        let backwards = data(&s.update(
            &update_request(&id, "Fine", "2026-11-20T00:00:00Z", "2026-10-20T00:00:00Z"),
            now,
        ));
        assert_eq!(backwards["updated"], json!(false));
        assert_eq!(backwards["problems"][0]["field"], json!("endsAt"));

        // And a refused edit leaves the document alone — no version bump.
        assert_eq!(
            data(&s.get(&json!({ "experimentId": id }).to_string()))["experiment"]["version"],
            json!(1)
        );
    }

    /// An experiment of our own with one submission inside its window.
    /// Only the two fields the window check reads are needed, and inventing a
    /// whole signed submission would test nothing extra here.
    fn with_one_submission(scratch: &Scratch) -> (Session, String) {
        let mut s = publishing_in(scratch);
        let made = data(&s.create(
            &create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();
        s.submissions.push(json!({
            "experiment": id,
            "submitted_at": "2026-10-25T09:00:00Z",
        }));
        (s, id)
    }

    #[test]
    fn narrowing_the_window_past_an_accepted_submission_warns_rather_than_dropping_it() {
        // The submission was valid when it was made and the index that
        // recorded it is published. Dropping it would rewrite history the
        // submitter can see.
        let scratch = Scratch::new("narrow");
        let (mut s, id) = with_one_submission(&scratch);
        let out = data(&s.update(
            &update_request(&id, "Narrowed", "2026-11-01T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        assert_eq!(out["updated"], json!(true), "{out}");
        assert_eq!(out["warnings"][0]["code"], json!("submissions_outside_window"));
        assert!(out["warnings"][0]["message"].as_str().unwrap().contains('1'));
        // Still there.
        assert_eq!(s.submissions.len(), 1);
    }

    #[test]
    fn an_edit_that_keeps_every_submission_in_range_warns_about_nothing() {
        let scratch = Scratch::new("widen");
        let (mut s, id) = with_one_submission(&scratch);
        let out = data(&s.update(
            &update_request(&id, "Widened", "2026-10-01T00:00:00Z", "2026-12-31T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        assert_eq!(out["warnings"], json!([]));
    }

    #[test]
    fn an_experiment_this_node_cannot_sign_for_is_not_resealed() {
        // The embedded fixture displays as "created by you" in fixtures mode,
        // but it is signed by the fixture key and we hold a different one.
        // Resealing it would emit a document whose signature does not match
        // the creator it names, and nothing downstream would catch that.
        let scratch = Scratch::new("foreignsign");
        let mut s = seeded();
        let public = s.attach_identity(scratch.0.to_str().unwrap()).expect("identity");
        // What fixtures mode does: the displayed creator stays the fixture's,
        // while the signing key is this node's real one. (`attach_identity`
        // only skips that overwrite when COLLECT_FIXTURES is set, and the
        // tests do not run with it.)
        s.creator_key = "3a9d0f213a9d0f213a9d0f213a9d0f213a9d0f213a9d0f213a9d0f213a9d0f21".into();
        assert_ne!(public, s.creator_key(), "the fixture creator is not this key");
        let out = envelope(&s.update(
            &update_request("q7k2m3x4w3b5n6tz", "Hijacked", "2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        assert_eq!(out["ok"], json!(false));
        assert!(
            out["error"]["message"].as_str().unwrap().contains("cannot sign for"),
            "{out}"
        );
    }

    #[test]
    fn editing_an_unknown_experiment_is_a_bad_request() {
        let scratch = Scratch::new("editunknown");
        let mut s = publishing_in(&scratch);
        let out = envelope(&s.update(
            &update_request("nope", "x", "2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"]["code"], json!("bad_request"));
    }

    /// A signing, publishing session holding the fixture experiment, protocol,
    /// submission and index page, re-authored so this node really is the
    /// creator — which hiding requires, since it re-signs the index.
    fn creator_of_the_example(scratch: &Scratch) -> Session {
        let mut s = seeded();
        let public = s.attach_identity(scratch.0.to_str().unwrap()).expect("identity");
        s.attach_store(Box::new(crate::store::MemoryStore::new()));
        s.creator_key = public.clone();
        // Re-sign the fixture documents under this node's key, so the chain
        // this node is about to extend actually verifies.
        s.experiments[0]["creator"]["key"] = json!(public);
        let resigned = s.signed(&s.experiments[0].clone()).expect("sign");
        s.experiments[0] = resigned;
        s
    }

    /// The submitter in the fixture submission.
    fn example_submitter(s: &Session) -> String {
        s.submissions[0]["submitter"]["key"].as_str().unwrap().to_string()
    }

    /// The fixture index page already hides one key, so a test about hiding a
    /// second one has to say which it means.
    const FIXTURE_HIDDEN: &str = "9b104c2d9b104c2d9b104c2d9b104c2d9b104c2d9b104c2d9b104c2d9b104c2d";

    /// The fixture index page is page 2.
    const FIXTURE_PAGE: &str = "q7k2m3x4w3b5n6tz#index2";

    #[test]
    fn hiding_a_submitter_drops_their_rows_from_the_results() {
        let scratch = Scratch::new("hide");
        let mut s = creator_of_the_example(&scratch);
        let submitter = example_submitter(&s);
        let before = data(&s.results(&results_request()));
        assert!(before["rows"].as_array().unwrap().len() > 0, "{before}");

        let out = data(&s.set_hidden(
            &json!({ "experimentId": "q7k2m3x4w3b5n6tz", "submitter": submitter })
                .to_string(),
            true,
            at("2026-10-28T08:00:00Z"),
        ));
        assert_eq!(out["changed"], json!(true), "{out}");
        // Added to whoever the fixture page already hid, not replacing them.
        let hidden = out["hidden"].as_array().unwrap();
        assert!(hidden.contains(&json!(submitter)), "{out}");
        assert!(hidden.contains(&json!(FIXTURE_HIDDEN)), "{out}");

        let after = data(&s.results(&results_request()));
        assert_eq!(after["rows"], json!([]));
        assert_eq!(after["summary"]["hidden"], json!(2));
        // The keys travel too, so the UI can offer to undo it.
        assert!(after["hidden_keys"].as_array().unwrap().contains(&json!(submitter)));
        assert_eq!(after["summary"]["shown"], json!(0));
    }

    #[test]
    fn showing_a_hidden_submitter_puts_their_rows_back() {
        // The reason `indexer::hidden_keys` takes the newest page rather than
        // the union of every page: with a union, this is impossible.
        let scratch = Scratch::new("show");
        let mut s = creator_of_the_example(&scratch);
        let submitter = example_submitter(&s);
        let request = json!({ "experimentId": "q7k2m3x4w3b5n6tz", "submitter": submitter }).to_string();
        let now = at("2026-10-28T08:00:00Z");

        data(&s.set_hidden(&request, true, now));
        assert_eq!(data(&s.results(&results_request()))["rows"], json!([]));

        let out = data(&s.set_hidden(&request, false, now));
        assert_eq!(out["changed"], json!(true), "{out}");
        // Back to only the key the fixture page hid.
        assert_eq!(out["hidden"], json!([FIXTURE_HIDDEN]));
        assert!(data(&s.results(&results_request()))["rows"].as_array().unwrap().len() > 0);
    }

    #[test]
    fn hiding_publishes_a_new_signed_index_page_and_points_the_manifest_at_it() {
        // Hiding is a published act: the index page is signed and uploaded so
        // the submitter can see it, and the manifest names the new page or a
        // reader would keep fetching the old one.
        let scratch = Scratch::new("hidepub");
        let mut s = creator_of_the_example(&scratch);
        let submitter = example_submitter(&s);
        let out = data(&s.set_hidden(
            &json!({ "experimentId": "q7k2m3x4w3b5n6tz", "submitter": submitter }).to_string(),
            true,
            at("2026-10-28T08:00:00Z"),
        ));
        assert_eq!(out["published"], json!(true), "{out}");

        let page_cid = s
            .publication(FIXTURE_PAGE)
            .and_then(|p| p.cid())
            .expect("the index page was published")
            .to_string();
        let manifest = &data(&s.get(&json!({ "experimentId": "q7k2m3x4w3b5n6tz" }).to_string()))["experiment"];
        assert_eq!(manifest["index"], json!(page_cid));

        // Both documents verify under this node's key.
        let mut key = [0u8; 32];
        hex::decode_to_slice(s.creator_key(), &mut key).unwrap();
        assert!(crate::sign::verify_with(manifest, &key).is_ok(), "{manifest}");
        let page = s
            .index_pages
            .iter()
            .find(|p| p.get("page").and_then(Value::as_u64) == Some(2))
            .unwrap();
        assert!(crate::sign::verify_with(page, &key).is_ok(), "{page}");
    }

    #[test]
    fn hiding_someone_already_hidden_changes_nothing() {
        let scratch = Scratch::new("hidetwice");
        let mut s = creator_of_the_example(&scratch);
        let submitter = example_submitter(&s);
        let request = json!({ "experimentId": "q7k2m3x4w3b5n6tz", "submitter": submitter }).to_string();
        let now = at("2026-10-28T08:00:00Z");
        data(&s.set_hidden(&request, true, now));
        let again = data(&s.set_hidden(&request, true, now));
        assert_eq!(again["changed"], json!(false));
        // No identical page republished at a new address.
        assert_eq!(again["published"], json!(false));
    }

    #[test]
    fn only_the_creator_can_hide_a_submitter() {
        let mut s = seeded();
        s.creator_key = "bb".repeat(32);
        let out = envelope(&s.set_hidden(
            &json!({ "experimentId": "q7k2m3x4w3b5n6tz", "submitter": "cc".repeat(32) }).to_string(),
            true,
            at("2026-10-28T08:00:00Z"),
        ));
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"]["code"], json!("not_yours"));
    }

    #[test]
    fn a_creator_cannot_hide_themselves() {
        let scratch = Scratch::new("hideself");
        let mut s = creator_of_the_example(&scratch);
        let me = s.creator_key().to_string();
        let out = envelope(&s.set_hidden(
            &json!({ "experimentId": "q7k2m3x4w3b5n6tz", "submitter": me }).to_string(),
            true,
            at("2026-10-28T08:00:00Z"),
        ));
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"]["code"], json!("bad_request"));
    }

    #[test]
    fn a_submitter_that_is_not_a_key_is_refused() {
        // Upper-case hex is a different string to every comparison we make,
        // so accepting it would let a hidden submitter return by changing
        // case.
        let scratch = Scratch::new("hidebadkey");
        let mut s = creator_of_the_example(&scratch);
        for bad in ["", "nope", &"AB".repeat(32), &"ab".repeat(31)] {
            let out = envelope(&s.set_hidden(
                &json!({ "experimentId": "q7k2m3x4w3b5n6tz", "submitter": bad }).to_string(),
                true,
                at("2026-10-28T08:00:00Z"),
            ));
            assert_eq!(out["ok"], json!(false), "{bad} should be refused");
        }
    }

    #[test]
    fn exporting_writes_a_csv_and_reports_where_it_went() {
        let scratch = Scratch::new("exportcsv");
        let mut s = creator_of_the_example(&scratch);
        let out = data(&s.export(&results_request(), at("2026-10-29T08:00:00Z")));
        assert_eq!(out["format"], json!("csv"));
        assert_eq!(out["mime"], json!("text/csv"));
        assert_eq!(
            out["filename"],
            json!("collect-q7k2m3x4w3b5n6tz-wildlife-sighting-20261029.csv")
        );

        let path = out["path"].as_str().unwrap();
        let written = std::fs::read_to_string(path).expect("the file is there");
        assert_eq!(written.len(), out["bytes"].as_u64().unwrap() as usize);
        // The preview is the same bytes, not a second rendering of them.
        assert_eq!(out["text"], json!(written));
        assert!(written.starts_with("Document CID,Submitter key"), "{written}");
        assert_eq!(out["rows"].as_u64().unwrap() as usize, written.lines().count() - 1);
    }

    #[test]
    fn an_export_never_contains_a_hidden_submitter() {
        // The reason export is built on `results` rather than reading the
        // submissions again: this is the failure nobody would notice.
        let scratch = Scratch::new("exporthidden");
        let mut s = creator_of_the_example(&scratch);
        let submitter = example_submitter(&s);
        let now = at("2026-10-29T08:00:00Z");

        let before = data(&s.export(&results_request(), now));
        assert!(
            std::fs::read_to_string(before["path"].as_str().unwrap())
                .unwrap()
                .contains(&submitter),
            "the submitter should be there to start with"
        );

        data(&s.set_hidden(
            &json!({ "experimentId": "q7k2m3x4w3b5n6tz", "submitter": submitter }).to_string(),
            true,
            now,
        ));
        let after = data(&s.export(&results_request(), now));
        let text = std::fs::read_to_string(after["path"].as_str().unwrap()).unwrap();
        assert!(!text.contains(&submitter), "{text}");
        assert_eq!(after["rows"], json!(0));
    }

    #[test]
    fn exporting_json_round_trips_through_serde() {
        let scratch = Scratch::new("exportjson");
        let mut s = creator_of_the_example(&scratch);
        let mut request: Value = serde_json::from_str(&results_request()).unwrap();
        request["format"] = json!("json");
        let out = data(&s.export(&request.to_string(), at("2026-10-29T08:00:00Z")));
        assert_eq!(out["format"], json!("json"));
        let text = std::fs::read_to_string(out["path"].as_str().unwrap()).unwrap();
        let back: Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(back["format"], json!("collect/export@1"));
        assert_eq!(back["protocol"]["id"], json!("wildlife-sighting"));
    }

    #[test]
    fn an_unknown_format_is_refused_before_anything_is_written() {
        let scratch = Scratch::new("exportbadfmt");
        let mut s = creator_of_the_example(&scratch);
        let mut request: Value = serde_json::from_str(&results_request()).unwrap();
        request["format"] = json!("xlsx");
        let out = envelope(&s.export(&request.to_string(), at("2026-10-29T08:00:00Z")));
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"]["code"], json!("bad_request"));
    }

    #[test]
    fn an_export_of_an_unknown_protocol_passes_the_tables_own_reason_through() {
        let scratch = Scratch::new("exportnoproto");
        let mut s = creator_of_the_example(&scratch);
        let out = envelope(&s.export(
            &json!({ "experimentId": "q7k2m3x4w3b5n6tz", "protocolId": "nope" }).to_string(),
            at("2026-10-29T08:00:00Z"),
        ));
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"]["code"], json!("bad_request"));
        assert!(out["error"]["message"].as_str().unwrap().contains("nope"), "{out}");
    }

    #[test]
    fn exporting_with_media_fetches_the_photos_beside_the_csv() {
        let scratch = Scratch::new("exportmedia");
        let store = std::sync::Arc::new(crate::store::MemoryStore::new());
        let mut s = seeded();
        let public = s.attach_identity(scratch.0.to_str().unwrap()).expect("identity");
        s.attach_store(Box::new(SharedStore(store.clone())));
        s.creator_key = public;

        // Put a photo on the submission and the bytes in the store, so the
        // fetch has something real to find.
        let cid = store.put("photo.jpg", b"not really a jpeg").unwrap();
        let document_cid = crate::store::as_document_cid(&cid).unwrap();
        s.submissions[0]["values"]["photos"] = json!([{
            "cid": document_cid,
            "sha256": "ab".repeat(32),
            "mime": "image/jpeg",
            "bytes": 17
        }]);

        let mut request: Value = serde_json::from_str(&results_request()).unwrap();
        request["includeMedia"] = json!(true);
        let out = data(&s.export(&request.to_string(), at("2026-10-29T08:00:00Z")));
        let media = &out["media"];
        assert_eq!(media["requested"], json!(true), "{out}");
        assert_eq!(media["wanted"], json!(1), "{out}");
        assert_eq!(media["written"], json!(1), "{out}");
        assert_eq!(media["failed"], json!([]));

        // The CSV sits in its own folder with the photos next to it.
        let path = std::path::Path::new(out["path"].as_str().unwrap());
        let photo = path
            .parent()
            .unwrap()
            .join("media")
            .join(format!("{}.bin", crate::store::bare_cid(&document_cid)));
        assert_eq!(std::fs::read(&photo).unwrap(), b"not really a jpeg");
        // And the row names the same CID, so file and row match.
        assert!(std::fs::read_to_string(path).unwrap().contains(&document_cid));
    }

    #[test]
    fn a_photo_that_cannot_be_fetched_is_reported_without_losing_the_export() {
        // The table is the data; the photos are attachments to it.
        let scratch = Scratch::new("exportmediafail");
        let mut s = creator_of_the_example(&scratch);
        s.submissions[0]["values"]["photos"] = json!([{
            "cid": "logos:zMissing",
            "sha256": "ab".repeat(32),
            "mime": "image/jpeg",
            "bytes": 1
        }]);
        let mut request: Value = serde_json::from_str(&results_request()).unwrap();
        request["includeMedia"] = json!(true);
        let out = data(&s.export(&request.to_string(), at("2026-10-29T08:00:00Z")));
        assert_eq!(out["media"]["written"], json!(0));
        assert_eq!(out["media"]["failed"][0]["cid"], json!("logos:zMissing"));
        // The CSV is still there.
        assert!(std::fs::metadata(out["path"].as_str().unwrap()).is_ok());
    }

    #[test]
    fn a_protocol_reference_is_not_mistaken_for_a_photo() {
        // Manifest protocol entries carry a `cid` too; only a media entry has
        // the hash of the bytes beside it.
        let mut cids = Vec::new();
        collect_media_cids(&json!({ "id": "wildlife", "version": 2, "cid": "logos:zForm" }), &mut cids);
        assert_eq!(cids, Vec::<String>::new());

        collect_media_cids(
            &json!([{ "cid": "logos:zPic", "sha256": "ab", "mime": "image/jpeg", "bytes": 1 }]),
            &mut cids,
        );
        assert_eq!(cids, vec!["logos:zPic".to_string()]);
    }

    #[test]
    fn exporting_before_the_context_is_ready_says_so_rather_than_writing_nowhere() {
        let mut s = seeded();
        let out = envelope(&s.export(&results_request(), at("2026-10-29T08:00:00Z")));
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"]["code"], json!("not_wired"));
    }

    // ------------------------------------------------------- Delivery tests

    /// A creator with Storage and Delivery, one experiment and one open form.
    /// Returns the session, the experiment id and the handles a test needs to
    /// look at what was sent.
    fn creator_with_delivery(
        scratch: &Scratch,
    ) -> (
        Session,
        String,
        std::sync::Arc<crate::store::MemoryStore>,
        std::sync::Arc<crate::delivery::MemoryCourier>,
    ) {
        let store = std::sync::Arc::new(crate::store::MemoryStore::new());
        let courier = std::sync::Arc::new(crate::delivery::MemoryCourier::new());
        let mut s = empty_in(scratch);
        s.attach_store(Box::new(SharedStore(store.clone())));
        s.attach_courier(Box::new(SharedCourier(courier.clone())));

        let now = at("2026-10-28T08:00:00Z");
        let made = data(&s.create(&create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"), now));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();
        data(&s.save_protocol(&draft_for(&id), now));
        (s, id, store, courier)
    }

    /// A signed submission from somebody else, on `store`, with its CID.
    fn a_submission_from_a_stranger(
        scratch: &Scratch,
        store: &crate::store::MemoryStore,
        experiment_id: &str,
    ) -> (String, String) {
        let identity = Identity::load_or_create(scratch.0.as_path()).expect("identity");
        let submission = json!({
            "schema": "collect/submission@1",
            "id": "0192f7a4-6b2e-7c1d-9a3f-5e8b2c4d6f10",
            "experiment": experiment_id,
            "protocol": { "id": "bird-sighting", "version": 1 },
            "submitter": { "key": identity.public_hex(), "name": "Nisha" },
            "submitted_at": "2026-10-25T09:00:00Z",
            "values": { "count": 3 },
            "sig": "00".repeat(64),
        });
        let signed = identity.sign(&submission).expect("sign");
        let cid = crate::store::publish(store, "submission", &signed).expect("published");
        (cid, identity.public_hex())
    }

    /// The `delivery.deliver` request for an announcement, as the shim builds
    /// it from a `messageReceived` event.
    fn arriving(experiment_id: &str, announcement: &Value, hash: &str) -> String {
        json!({
            "topic": crate::delivery::topic(experiment_id),
            "messageHash": hash,
            "source": "live",
            "payload": announcement.to_string(),
        })
        .to_string()
    }

    /// Sign an announcement as `identity` would.
    fn signed_by(dir: &Scratch, announcement: Value) -> Value {
        Identity::load_or_create(dir.0.as_path())
            .expect("identity")
            .sign(&announcement)
            .expect("sign")
    }

    #[test]
    fn a_creator_indexes_an_announced_submission_and_sends_a_receipt() {
        // The whole of SPEC 3.2 step 3 from the creator's side: the
        // announcement names a CID, the document comes off Storage, every
        // accept rule runs, the index page is published, and the submitter is
        // told.
        let scratch = Scratch::new("e2ecreator");
        let submitter_dir = Scratch::new("e2esubmitter");
        let (mut s, id, store, courier) = creator_with_delivery(&scratch);
        let (cid, submitter) = a_submission_from_a_stranger(&submitter_dir, &store, &id);

        let announcement = signed_by(
            &submitter_dir,
            crate::delivery::submission(&id, &submitter, &cid, "bird-sighting", 1, "2026-10-25T09:00:10Z"),
        );
        let out = data(&s.deliver(&arriving(&id, &announcement, "0xaaa"), at("2026-10-25T09:00:20Z")));

        assert_eq!(out["handled"], json!(true), "{out}");
        assert_eq!(out["cid"], json!(cid));
        assert_eq!(out["submitter"], json!(submitter));
        assert_eq!(out["index_page"], json!(1));
        assert_eq!(out["receipted"], json!(true), "{out}");

        // It is in the results table.
        let table = data(&s.results(
            &json!({ "experimentId": id, "protocolId": "bird-sighting" }).to_string(),
        ));
        assert_eq!(table["rows"].as_array().unwrap().len(), 1, "{table}");
        assert_eq!(table["rows"][0]["cid"], json!(cid));
        assert_eq!(table["rows"][0]["values"]["count"], json!(3));

        // And a receipt naming the same CID went out on the experiment's
        // topic.
        let receipt = courier
            .messages()
            .into_iter()
            .find(|m| m["kind"] == json!("receipt"))
            .expect("a receipt was sent");
        assert_eq!(receipt["cid"], json!(cid));
        assert_eq!(receipt["index_page"], json!(1));
        assert_eq!(courier.topics().last().unwrap(), &crate::delivery::topic(&id));
    }

    #[test]
    fn the_same_message_arriving_twice_is_indexed_once() {
        // Store catch-up replays everything missed while the node was down,
        // so a restart would otherwise re-index the backlog.
        let scratch = Scratch::new("dedupe");
        let submitter_dir = Scratch::new("dedupesub");
        let (mut s, id, store, _courier) = creator_with_delivery(&scratch);
        let (cid, submitter) = a_submission_from_a_stranger(&submitter_dir, &store, &id);
        let announcement = signed_by(
            &submitter_dir,
            crate::delivery::submission(&id, &submitter, &cid, "bird-sighting", 1, "2026-10-25T09:00:10Z"),
        );
        let arrival = arriving(&id, &announcement, "0xaaa");
        let now = at("2026-10-25T09:00:20Z");

        assert_eq!(data(&s.deliver(&arrival, now))["handled"], json!(true));
        let again = data(&s.deliver(&arrival, now));
        assert_eq!(again["handled"], json!(false));
        assert_eq!(again["reason"], json!("duplicate"));

        let table = data(&s.results(
            &json!({ "experimentId": id, "protocolId": "bird-sighting" }).to_string(),
        ));
        assert_eq!(table["rows"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn an_announcement_on_the_wrong_topic_is_not_acted_on() {
        // Otherwise a message on one experiment's topic could claim to be
        // about another, and a creator would index a submission for an
        // experiment they do not run.
        let scratch = Scratch::new("wrongtopic");
        let submitter_dir = Scratch::new("wrongtopicsub");
        let (mut s, id, store, _c) = creator_with_delivery(&scratch);
        let (cid, submitter) = a_submission_from_a_stranger(&submitter_dir, &store, &id);
        let announcement = signed_by(
            &submitter_dir,
            crate::delivery::submission(&id, &submitter, &cid, "bird-sighting", 1, "2026-10-25T09:00:10Z"),
        );
        let arrival = json!({
            "topic": crate::delivery::topic("zzzzzzzzzzzzzzzz"),
            "messageHash": "0xbbb",
            "payload": announcement.to_string(),
        })
        .to_string();
        let out = data(&s.deliver(&arrival, at("2026-10-25T09:00:20Z")));
        assert_eq!(out["handled"], json!(false));
        assert_eq!(out["reason"], json!("wrong_topic"));
    }

    #[test]
    fn an_unsigned_or_tampered_announcement_is_refused() {
        let scratch = Scratch::new("badsig");
        let submitter_dir = Scratch::new("badsigsub");
        let (mut s, id, store, _c) = creator_with_delivery(&scratch);
        let (cid, submitter) = a_submission_from_a_stranger(&submitter_dir, &store, &id);
        let mut announcement = signed_by(
            &submitter_dir,
            crate::delivery::submission(&id, &submitter, &cid, "bird-sighting", 1, "2026-10-25T09:00:10Z"),
        );
        // Change the CID after signing: the signature no longer covers it.
        announcement["cid"] = json!("logos:zSomethingElse");
        let out = data(&s.deliver(&arriving(&id, &announcement, "0xccc"), at("2026-10-25T09:00:20Z")));
        assert_eq!(out["handled"], json!(false));
        assert_eq!(out["reason"], json!("bad_signature"));
    }

    #[test]
    fn a_submission_whose_document_is_not_on_storage_is_reported_as_retryable() {
        // The announcement is only a pointer; it cannot be trusted about the
        // contents, and a CID that cannot be fetched is a transport problem
        // rather than a bad submitter.
        let scratch = Scratch::new("nofetch");
        let submitter_dir = Scratch::new("nofetchsub");
        let (mut s, id, _store, _c) = creator_with_delivery(&scratch);
        let submitter = Identity::load_or_create(submitter_dir.0.as_path())
            .unwrap()
            .public_hex();
        let announcement = signed_by(
            &submitter_dir,
            crate::delivery::submission(&id, &submitter, "logos:zMissing", "bird-sighting", 1, "2026-10-25T09:00:10Z"),
        );
        let out = data(&s.deliver(&arriving(&id, &announcement, "0xddd"), at("2026-10-25T09:00:20Z")));
        assert_eq!(out["handled"], json!(false));
        assert_eq!(out["cid"], json!("logos:zMissing"));
        assert!(out["reason"].as_str().unwrap().starts_with("storage_"), "{out}");
    }

    #[test]
    fn a_head_from_someone_other_than_the_creator_is_ignored() {
        // Anyone can publish on a public topic. A head from a stranger naming
        // a manifest CID would otherwise send submitters off to fetch it.
        let scratch = Scratch::new("fakehead");
        let stranger_dir = Scratch::new("fakeheadstranger");
        let (mut s, id, _store, _c) = creator_with_delivery(&scratch);
        let stranger = Identity::load_or_create(stranger_dir.0.as_path())
            .unwrap()
            .public_hex();
        let announcement = signed_by(
            &stranger_dir,
            crate::delivery::head(&id, &stranger, "logos:zEvil", 9, None, 0, "2026-10-25T09:00:10Z"),
        );
        let out = data(&s.deliver(&arriving(&id, &announcement, "0xeee"), at("2026-10-25T09:00:20Z")));
        assert_eq!(out["handled"], json!(false));
        assert_eq!(out["reason"], json!("not_the_creator"));
    }

    #[test]
    fn a_creator_announces_a_head_naming_the_manifest_and_index() {
        let scratch = Scratch::new("head");
        let (mut s, id, _store, courier) = creator_with_delivery(&scratch);
        let out = data(&s.publish_head(
            &json!({ "experimentId": id }).to_string(),
            at("2026-10-28T09:00:00Z"),
        ));
        assert_eq!(out["announced"], json!(true), "{out}");
        assert_eq!(out["topic"], json!(crate::delivery::topic(&id)));

        let head = courier
            .messages()
            .into_iter()
            .find(|m| m["kind"] == json!("head"))
            .expect("a head was sent");
        // The manifest CID it names is the one the manifest is actually at.
        assert_eq!(head["manifest"], json!(s.publication(&id).unwrap().cid().unwrap()));
        // Nothing received yet, so there is no index page to name — which is
        // a state the creator announces rather than withholds.
        assert_eq!(head["index"], Value::Null);
        assert_eq!(head["index_page"], json!(0));
    }

    #[test]
    fn a_head_is_not_announced_before_the_manifest_has_an_address() {
        let scratch = Scratch::new("headnocid");
        let mut s = empty_in(&scratch);
        s.attach_courier(Box::new(crate::delivery::MemoryCourier::new()));
        let made = data(&s.create(
            &create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();
        let out = envelope(&s.publish_head(
            &json!({ "experimentId": id }).to_string(),
            at("2026-10-28T09:00:00Z"),
        ));
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"]["code"], json!("not_wired"));
    }

    #[test]
    fn a_submitter_without_an_rln_membership_is_told_what_is_missing() {
        // SPEC Q9: on logos.test a node needs a funded membership before it
        // may publish. Watching a spinner is the wrong way to find out.
        let scratch = Scratch::new("rln");
        let mut s = publishing_in(&scratch);
        s.attach_courier(Box::new(crate::delivery::MemoryCourier::without_rln()));
        let out = envelope(&s.announce_submission(
            &json!({
                "experimentId": "q7k2m3x4w3b5n6tz",
                "cid": "logos:zSub",
                "protocolId": "wildlife-sighting",
                "version": 2
            })
            .to_string(),
            at("2026-10-28T09:00:00Z"),
        ));
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"]["code"], json!("rln_not_ready"));
        assert_eq!(out["error"]["retryable"], json!(false));
        assert_eq!(out["error"]["rln"]["required"], json!(true));
        assert_eq!(out["error"]["rln"]["ready"], json!(false));
        assert!(!s.can_announce());
    }

    #[test]
    fn a_receipt_arriving_marks_the_submission_as_received() {
        let scratch = Scratch::new("receipt");
        let creator_dir = Scratch::new("receiptcreator");
        let mut s = publishing_in(&scratch);
        s.attach_courier(Box::new(crate::delivery::MemoryCourier::new()));
        // The fixture experiment, whose creator is the fixture key — so the
        // head check does not apply and a receipt is simply recorded.
        let creator = Identity::load_or_create(creator_dir.0.as_path()).unwrap().public_hex();
        let announcement = signed_by(
            &creator_dir,
            crate::delivery::receipt("q7k2m3x4w3b5n6tz", &creator, "logos:zSub", 2, "2026-10-28T09:00:00Z"),
        );
        let out = data(&s.deliver(
            &arriving("q7k2m3x4w3b5n6tz", &announcement, "0xfff"),
            at("2026-10-28T09:00:10Z"),
        ));
        assert_eq!(out["handled"], json!(true), "{out}");
        assert_eq!(out["kind"], json!("receipt"));
        assert_eq!(out["cid"], json!("logos:zSub"));
        assert_eq!(out["index_page"], json!(2));

        // And announcing that submission now reports the receipt, which is
        // what turns S3's "waiting" into "received".
        let announced = data(&s.announce_submission(
            &json!({
                "experimentId": "q7k2m3x4w3b5n6tz",
                "cid": "logos:zSub",
                "protocolId": "wildlife-sighting",
                "version": 2
            })
            .to_string(),
            at("2026-10-28T09:00:20Z"),
        ));
        assert_eq!(announced["receipt"]["cid"], json!("logos:zSub"));
    }

    #[test]
    fn watching_subscribes_to_every_experiment_this_node_knows() {
        // No scratch directory: subscribing needs no signing key.
        let courier = std::sync::Arc::new(crate::delivery::MemoryCourier::new());
        let mut s = seeded();
        s.attach_courier(Box::new(SharedCourier(courier.clone())));
        let watched = s.watch_topics();
        assert_eq!(watched, vec![crate::delivery::topic("q7k2m3x4w3b5n6tz")]);
        assert_eq!(courier.subscriptions(), watched);
    }

    #[test]
    fn a_message_on_a_topic_that_is_not_ours_is_left_alone() {
        let scratch = Scratch::new("nottours");
        let mut s = publishing_in(&scratch);
        let out = data(&s.deliver(
            &json!({ "topic": "/waku/2/default-waku/proto", "messageHash": "0x1", "payload": "{}" })
                .to_string(),
            at("2026-10-28T09:00:00Z"),
        ));
        assert_eq!(out["handled"], json!(false));
        assert_eq!(out["reason"], json!("not_ours"));
    }

    #[test]
    fn an_unreadable_payload_is_reported_rather_than_panicking() {
        let scratch = Scratch::new("garbage");
        let mut s = publishing_in(&scratch);
        let out = data(&s.deliver(
            &json!({
                "topic": crate::delivery::topic("q7k2m3x4w3b5n6tz"),
                "messageHash": "0x2",
                "payload": "not json"
            })
            .to_string(),
            at("2026-10-28T09:00:00Z"),
        ));
        assert_eq!(out["handled"], json!(false));
        assert_eq!(out["reason"], json!("unreadable"));
    }

    #[test]
    fn draining_handles_everything_that_arrived() {
        let scratch = Scratch::new("drain");
        let submitter_dir = Scratch::new("drainsub");
        let (mut s, id, store, courier) = creator_with_delivery(&scratch);
        let (cid, submitter) = a_submission_from_a_stranger(&submitter_dir, &store, &id);
        let announcement = signed_by(
            &submitter_dir,
            crate::delivery::submission(&id, &submitter, &cid, "bird-sighting", 1, "2026-10-25T09:00:10Z"),
        );
        courier.arrive(&crate::delivery::topic(&id), "0xaaa", &announcement);
        // And one that is not ours, to prove a bad message does not stop the
        // batch.
        courier.arrive("/waku/2/default-waku/proto", "0xbbb", &json!({}));

        let out = data(&s.drain(at("2026-10-25T09:00:20Z")));
        assert_eq!(out["received"], json!(2), "{out}");
        assert_eq!(out["handled"], json!(1), "{out}");
        assert_eq!(out["results"][0]["cid"], json!(cid));
        assert_eq!(out["results"][1]["reason"], json!("not_ours"));

        // Drained: a second pass has nothing to do, rather than re-handling
        // the batch.
        assert_eq!(data(&s.drain(at("2026-10-25T09:00:30Z")))["received"], json!(0));
    }

    #[test]
    fn a_refused_send_is_reported_by_drain_not_swallowed() {
        let scratch = Scratch::new("sendfailed");
        let courier = std::sync::Arc::new(crate::delivery::MemoryCourier::new());
        let mut s = publishing_in(&scratch);
        s.attach_courier(Box::new(SharedCourier(courier.clone())));
        courier.arrive_raw(json!({
            "kind": "sendFailed", "requestId": "req-7", "error": "rln: no usable membership"
        }));
        let out = data(&s.drain(at("2026-10-28T09:00:00Z")));
        assert_eq!(out["received"], json!(1));
        assert_eq!(out["handled"], json!(0));
        assert_eq!(out["results"][0]["reason"], json!("send_failed"));
        assert_eq!(out["results"][0]["request_id"], json!("req-7"));
        assert!(out["results"][0]["detail"].as_str().unwrap().contains("membership"));
    }

    #[test]
    fn draining_without_delivery_says_so() {
        let scratch = Scratch::new("drainnone");
        let mut s = publishing_in(&scratch);
        let out = envelope(&s.drain(at("2026-10-25T09:00:20Z")));
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"]["code"], json!("not_wired"));
    }

    /// A courier two owners can look at, like `SharedStore`.
    struct SharedCourier(std::sync::Arc<crate::delivery::MemoryCourier>);

    impl Courier for SharedCourier {
        fn send(&self, topic: &str, payload: &[u8]) -> Result<String, crate::delivery::DeliveryError> {
            self.0.send(topic, payload)
        }
        fn subscribe(&self, topic: &str) -> Result<(), crate::delivery::DeliveryError> {
            self.0.subscribe(topic)
        }
        fn rln(&self) -> crate::delivery::RlnStatus {
            self.0.rln()
        }
        fn connection(&self) -> Option<String> {
            self.0.connection()
        }
        fn inbox(&self) -> Vec<Value> {
            self.0.inbox()
        }
    }

    #[test]
    fn the_seeded_example_becomes_this_nodes_own() {
        // It used to read as "created by you" while being signed by a key
        // nobody holds, so every creator action on it failed at the
        // signature with nothing the person could do about it.
        let scratch = Scratch::new("reauthor");
        let mut s = seeded();
        let public = s.attach_identity(scratch.0.to_str().unwrap()).expect("identity");
        s.reauthor_fixtures();

        assert_eq!(s.creator_key(), public);
        let doc = &s.experiments[0];
        assert_eq!(doc["creator"]["key"], json!(public));
        let mut key = [0u8; 32];
        hex::decode_to_slice(&public, &mut key).unwrap();
        assert!(crate::sign::verify_with(doc, &key).is_ok(), "{doc}");
        // And it verifies on its own terms, which is what reseal checks.
        assert!(crate::sign::verify(doc).is_ok());
    }

    #[test]
    fn a_form_can_be_added_to_the_seeded_example() {
        // The failure this fixes, end to end.
        let scratch = Scratch::new("reauthorform");
        let mut s = seeded();
        s.attach_identity(scratch.0.to_str().unwrap()).expect("identity");
        s.reauthor_fixtures();
        s.attach_store(Box::new(crate::store::MemoryStore::new()));

        // The example does not start at version 1, so the check is that the
        // amendment bumped it rather than what it bumped it to.
        let before = s.experiments[0]["version"].as_u64().unwrap();
        let saved = data(&s.save_protocol(&draft_for("q7k2m3x4w3b5n6tz"), at("2026-10-28T08:00:00Z")));
        assert_eq!(saved["saved"], json!(true), "{saved}");
        assert_eq!(saved["experiment"]["version"], json!(before + 1));
    }

    #[test]
    fn the_examples_contributors_are_not_turned_into_this_node() {
        // Re-signing the submissions would make every contributor us.
        let scratch = Scratch::new("reauthorsubs");
        let mut s = seeded();
        let before = s.submissions[0]["submitter"]["key"].clone();
        let public = s.attach_identity(scratch.0.to_str().unwrap()).expect("identity");
        s.reauthor_fixtures();
        assert_eq!(s.submissions[0]["submitter"]["key"], before);
        assert_ne!(s.submissions[0]["submitter"]["key"], json!(public));
    }

    #[test]
    fn a_call_that_runs_out_of_time_keeps_the_document_and_offers_a_retry() {
        // Saving a form publishes two documents, each separately bounded.
        // Together they could outlast the protocol timeout on the call the UI
        // is waiting on, and then the UI gets no reply at all — no result, no
        // error. Running out of budget has to be an answer, not a hang.
        let scratch = Scratch::new("budget");
        let mut s = publishing_in(&scratch);
        let made = data(&s.create(
            &create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();
        assert_eq!(made["published"], json!(true), "the budget was fresh");

        // Now spend it.
        s.publish_until = Some(std::time::Instant::now());
        let doc = made["experiment"].clone();
        let outcome = s.publish_doc("experiment", &id, &doc).expect("a store is attached");
        assert!(matches!(outcome, Publication::Failed { .. }), "{outcome:?}");
        assert_eq!(s.publish_report(&id)["published"], json!(false));
        assert_eq!(s.publish_report(&id)["publish_error"]["code"], json!("storage_timeout"));

        // The experiment is still here, and publishing again works once the
        // budget is fresh.
        assert_eq!(data(&s.list(at("2026-10-28T08:00:00Z")))["experiments"].as_array().unwrap().len(), 1);
        let retried = data(&s.publish_now(&json!({ "experimentId": id }).to_string()));
        assert_eq!(retried["published"], json!(true), "{retried}");
    }

    #[test]
    fn saving_a_form_answers_even_when_storage_is_unusable() {
        // The shape of the failure the budget exists for: the call returns a
        // result rather than nothing.
        let scratch = Scratch::new("budgetform");
        let mut s = empty_in(&scratch);
        s.attach_store(Box::new(crate::store::MemoryStore::failing(
            StoreError::Timeout("no completion".into()),
        )));
        let now = at("2026-10-28T08:00:00Z");
        let made = data(&s.create(&create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"), now));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();

        let saved = data(&s.save_protocol(&draft_for(&id), now));
        // Saved locally, honestly unpublished.
        assert_eq!(saved["saved"], json!(true), "{saved}");
        assert_eq!(saved["published"], json!(false));
        assert_eq!(
            data(&s.get(&json!({ "experimentId": id }).to_string()))["protocols"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    /// A creator with an open experiment and one form, ready to submit to it.
    fn creator_ready_to_submit(scratch: &Scratch) -> (Session, String) {
        let mut s = publishing_in(scratch);
        let now = at("2026-10-28T08:00:00Z");
        let made = data(&s.create(&create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"), now));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();
        data(&s.save_protocol(&draft_for(&id), now));
        (s, id)
    }

    fn submit_request(id: &str, count: i64) -> String {
        json!({
            "experimentId": id,
            "protocolId": "bird-sighting",
            "values": { "count": count },
            "submitterName": "Dr Rao"
        })
        .to_string()
    }

    fn with_wallet(s: &mut Session, wallet: &str) {
        let saved = s.profile_save(
            &json!({ "fields": { "wallet": { "value": wallet, "share": false } } }).to_string(),
        );
        assert_eq!(envelope(&saved)["ok"], json!(true), "{saved}");
    }

    fn submit_with_wallet(id: &str, include: bool) -> String {
        let mut request: Value = serde_json::from_str(&submit_request(id, 2)).unwrap();
        request["includeWallet"] = json!(include);
        request.to_string()
    }

    #[test]
    fn the_wallet_goes_in_only_when_ticked_and_only_from_the_profile() {
        let scratch = Scratch::new("wallet");
        let (mut s, id) = creator_ready_to_submit(&scratch);
        with_wallet(&mut s, "lez1qexamplewalletaddress");

        let without = data(&s.submit(&submit_with_wallet(&id, false), at("2026-10-28T09:00:00Z")));
        assert!(without["submission"].get("wallet").is_none(), "{without}");

        // A wallet in the request itself is ignored: only the profile's counts.
        let mut forged: Value = serde_json::from_str(&submit_with_wallet(&id, true)).unwrap();
        forged["wallet"] = json!("someone-elses-wallet");
        let with = data(&s.submit(&forged.to_string(), at("2026-10-28T09:05:00Z")));
        let doc = &with["submission"];
        assert_eq!(doc["wallet"], json!("lez1qexamplewalletaddress"), "{with}");
        assert!(crate::sign::verify(doc).is_ok(), "the wallet is not covered by the signature");
        assert!(schema::errors_as(doc, schema::Kind::Submission).is_empty());

        let table = data(&s.results(
            &json!({ "experimentId": id, "protocolId": "bird-sighting" }).to_string(),
        ));
        let wallets: Vec<&str> = table["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["submitter_wallet"].as_str().unwrap())
            .collect();
        assert!(wallets.contains(&"lez1qexamplewalletaddress"), "{table}");
        assert!(wallets.contains(&""), "an unticked submission showed a wallet");
    }

    #[test]
    fn ticking_the_wallet_box_with_no_wallet_saved_is_refused() {
        let scratch = Scratch::new("wallet-none");
        let (mut s, id) = creator_ready_to_submit(&scratch);
        let out = envelope(&s.submit(&submit_with_wallet(&id, true), at("2026-10-28T09:00:00Z")));
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"]["code"], json!(api::code::BAD_REQUEST));
        assert!(s.submissions.is_empty(), "something was submitted anyway");

        with_wallet(&mut s, "two words");
        let out = envelope(&s.submit(&submit_with_wallet(&id, true), at("2026-10-28T09:00:00Z")));
        assert_eq!(out["ok"], json!(false), "a wallet with a space was put in a document");
    }

    // ---- rewards ----

    /// A 32-byte account in the wallet's base58: thirty-one zero bytes then
    /// `last`, which is enough to tell test accounts apart.
    fn account(last: u8) -> String {
        const A: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
        let mut n = last as u32;
        let mut tail = String::new();
        while n > 0 {
            tail.insert(0, A[(n % 58) as usize] as char);
            n /= 58;
        }
        "1".repeat(31) + &tail
    }

    fn account_hex_of(last: u8) -> String {
        format!("{}{:02x}", "0".repeat(62), last)
    }

    /// A creator whose experiment offers 0.0005 per approved submission, up
    /// to two per person, with received submissions from three people:
    /// alice (wallet, three submissions), bob (no wallet), carol (wallet).
    fn rewarding(scratch: &Scratch) -> (Session, String, std::sync::Arc<crate::rewards::MemoryLedger>) {
        let mut s = publishing_in(scratch);
        let made = data(&s.create(
            &create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();
        let set = data(&s.set_reward(
            &json!({ "experimentId": id, "amount": "0.0005", "maxPerSubmitter": 2 }).to_string(),
        ));
        assert_eq!(set["experiment"]["reward"]["amount"], json!("500000"), "{set}");

        let people = [("alice", Some(account(1)), 3), ("bob", None, 1), ("carol", Some(account(3)), 1)];
        let mut entries = Vec::new();
        for (who, wallet, n) in people {
            let key = format!("{:0>64}", hex::encode(who));
            for i in 0..n {
                let at = format!("2026-10-2{}T0{}:00:00Z", 5, i);
                let mut doc = json!({
                    "experiment": id, "protocol": { "id": "bird-sighting", "version": 1 },
                    "submitter": { "key": key, "name": who }, "submitted_at": at, "values": {},
                });
                if let Some(w) = &wallet {
                    doc["wallet"] = json!(w);
                }
                s.submissions.push(doc);
                entries.push(json!({ "cid": format!("logos:z{who}{i}"), "submitter": key,
                                     "submitted_at": at, "protocol": "bird-sighting" }));
            }
        }
        s.index_pages.push(json!({ "experiment": id, "page": 1, "entries": entries, "hidden_keys": [] }));
        let ledger = std::sync::Arc::new(crate::rewards::MemoryLedger::default());
        ledger.balances.lock().unwrap().insert(account_hex_of(9), 10_000_000);
        s.attach_ledger(Box::new(ledger.clone()));
        (s, id, ledger)
    }

    fn approve(s: &mut Session, id: &str, cid: &str) {
        let out = s.set_approval(&json!({ "experimentId": id, "cid": cid, "status": "approved" }).to_string());
        assert_eq!(envelope(&out)["ok"], json!(true), "{out}");
    }

    const AFTER_END: &str = "2026-11-21T00:00:00Z";

    fn plan(s: &Session, id: &str) -> Value {
        data(&s.rewards_plan(&json!({ "experimentId": id, "from": account(9) }).to_string(), at(AFTER_END)))
    }

    fn pay(s: &mut Session, id: &str, payee: &Value) -> Value {
        envelope(&s.rewards_pay(
            &json!({ "experimentId": id, "from": account(9), "submitter": payee["submitter"],
                     "cids": payee["submissions"], "amountLepta": payee["amount_lepta"] }).to_string(),
            at(AFTER_END),
        ))
    }

    #[test]
    fn only_approved_submissions_with_a_wallet_are_paid_and_the_cap_holds() {
        let scratch = Scratch::new("rewards-plan");
        let (mut s, id, _) = rewarding(&scratch);
        for cid in ["logos:zalice0", "logos:zalice1", "logos:zalice2", "logos:zbob0"] {
            approve(&mut s, &id, cid);
        }
        let p = plan(&s, &id);
        let payees = p["payees"].as_array().unwrap();
        assert_eq!(payees.len(), 1, "{p}");
        assert_eq!(payees[0]["name"], json!("alice"));
        assert_eq!(payees[0]["count"], json!(2), "the cap of two was not applied");
        assert_eq!(payees[0]["amount"], json!("0.001"));
        assert_eq!(payees[0]["wallet_hex"], json!(account_hex_of(1)));
        let reasons: Vec<&str> = p["skipped"].as_array().unwrap().iter()
            .map(|r| r["reason"].as_str().unwrap()).collect();
        assert!(reasons.iter().any(|r| r.contains("over the limit")), "{p}");
        assert!(reasons.iter().any(|r| r.contains("wallet")), "{p}");
        assert_eq!(p["pending"], json!(1), "carol's unreviewed submission");
        assert_eq!(p["from"]["covers"], json!(true));
    }

    #[test]
    fn paying_sends_once_records_it_and_never_pays_the_same_submission_twice() {
        let scratch = Scratch::new("rewards-pay");
        let (mut s, id, ledger) = rewarding(&scratch);
        approve(&mut s, &id, "logos:zcarol0");
        let payee = plan(&s, &id)["payees"][0].clone();

        // Not before the end date.
        let early = envelope(&s.rewards_pay(
            &json!({ "experimentId": id, "from": account(9), "submitter": payee["submitter"],
                     "cids": payee["submissions"], "amountLepta": payee["amount_lepta"] }).to_string(),
            at("2026-11-01T00:00:00Z"),
        ));
        assert_eq!(early["ok"], json!(false));
        assert!(ledger.sent.lock().unwrap().is_empty());

        let paid = pay(&mut s, &id, &payee);
        assert_eq!(paid["ok"], json!(true), "{paid}");
        assert_eq!(paid["data"]["payout"]["state"], json!("sent"));
        assert_eq!(*ledger.sent.lock().unwrap(), vec![(account_hex_of(9), account_hex_of(3), 500_000)]);

        // Paying the same row again is refused, and carol drops off the plan.
        let again = pay(&mut s, &id, &payee);
        assert_eq!(again["ok"], json!(false));
        assert_eq!(ledger.sent.lock().unwrap().len(), 1, "paid twice");
        assert!(plan(&s, &id)["payees"].as_array().unwrap().is_empty());
        // A paid submission can no longer be un-approved, nor the reward changed.
        let undo = envelope(&s.set_approval(
            &json!({ "experimentId": id, "cid": "logos:zcarol0", "status": "rejected" }).to_string()));
        assert_eq!(undo["ok"], json!(false));
        let change = envelope(&s.set_reward(&json!({ "experimentId": id, "amount": "1" }).to_string()));
        assert_eq!(change["ok"], json!(false));

        // And the record survives a restart.
        s.save_state();
        let again = empty_in(&scratch);
        assert_eq!(again.payouts_of(&id).len(), 1);
        assert!(again.paid_cids(&id).contains("logos:zcarol0"));
    }

    #[test]
    fn a_plan_that_changed_since_review_is_not_paid() {
        let scratch = Scratch::new("rewards-stale");
        let (mut s, id, ledger) = rewarding(&scratch);
        approve(&mut s, &id, "logos:zalice0");
        let reviewed = plan(&s, &id)["payees"][0].clone();
        approve(&mut s, &id, "logos:zalice1");
        let out = pay(&mut s, &id, &reviewed);
        assert_eq!(out["ok"], json!(false), "{out}");
        assert!(out["error"]["message"].as_str().unwrap().contains("changed"));
        assert!(ledger.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn a_refused_transfer_is_recorded_and_can_be_retried() {
        let scratch = Scratch::new("rewards-fail");
        let (mut s, id, ledger) = rewarding(&scratch);
        approve(&mut s, &id, "logos:zcarol0");
        let payee = plan(&s, &id)["payees"][0].clone();
        *ledger.fail_next.lock().unwrap() = Some("sequencer unreachable".into());
        let out = pay(&mut s, &id, &payee);
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"]["payout"]["state"], json!("failed"));
        // Failed means not paid: carol is owed again, and a retry works.
        let payee = plan(&s, &id)["payees"][0].clone();
        assert_eq!(pay(&mut s, &id, &payee)["ok"], json!(true));
        assert_eq!(ledger.sent.lock().unwrap().len(), 1);
    }

    #[test]
    fn an_unknown_outcome_blocks_further_payments_until_resolved() {
        let scratch = Scratch::new("rewards-unknown");
        let (mut s, id, ledger) = rewarding(&scratch);
        approve(&mut s, &id, "logos:zcarol0");
        approve(&mut s, &id, "logos:zalice0");
        // As if the module died between recording and sending.
        s.payouts.push(json!({ "id": "p1", "experiment": id, "state": "sending",
                               "submissions": ["logos:zcarol0"] }));
        let alice = plan(&s, &id)["payees"][0].clone();
        assert_eq!(alice["name"], json!("alice"), "carol's unknown payment still counts as paid");
        assert_eq!(pay(&mut s, &id, &alice)["ok"], json!(false));
        assert!(ledger.sent.lock().unwrap().is_empty());

        let resolved = envelope(&s.rewards_resolve(
            &json!({ "experimentId": id, "payoutId": "p1", "outcome": "failed" }).to_string()));
        assert_eq!(resolved["ok"], json!(true));
        assert_eq!(plan(&s, &id)["payees"].as_array().unwrap().len(), 2, "carol is owed again");
    }

    #[test]
    fn a_reward_is_public_validated_and_removable() {
        let scratch = Scratch::new("rewards-set");
        let (mut s, id, _) = rewarding(&scratch);
        let manifest = &data(&s.get(&json!({ "experimentId": id }).to_string()))["experiment"];
        assert!(schema::errors_as(manifest, schema::Kind::Experiment).is_empty());
        assert!(crate::sign::verify(manifest).is_ok());
        let bad = envelope(&s.set_reward(&json!({ "experimentId": id, "amount": "0.0000000001" }).to_string()));
        assert!(bad["error"]["problems"]["amount"].is_string(), "{bad}");
        let gone = data(&s.set_reward(&json!({ "experimentId": id, "amount": "" }).to_string()));
        assert!(gone["experiment"].get("reward").is_none());
    }

    // ---- published results ----

    /// A creator whose form has a count and photos, with `n` of their own
    /// submissions received and indexed. Returns the CIDs in order.
    fn with_photo_submissions(scratch: &Scratch, n: usize) -> (Session, String, Vec<String>) {
        let mut s = publishing_in(scratch);
        let now = at("2026-10-28T08:00:00Z");
        let made = data(&s.create(&create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"), now));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();
        let draft = json!({ "experimentId": id, "protocol": { "name": "Bird sighting", "fields": [
            { "key": "count", "type": "number", "label": "How many", "required": true },
            { "key": "photos", "type": "photo", "label": "Photos", "required": false, "config": { "min": 0, "max": 3 } }
        ] } });
        let saved = data(&s.save_protocol(&draft.to_string(), now));
        let protocol_id = saved["protocol"]["id"].as_str().unwrap_or("bird-sighting").to_string();
        let (_, url) = photo_on_disk(scratch);
        let mut cids = Vec::new();
        for i in 0..n {
            let photo = data(&s.add_photo(&json!({ "path": url }).to_string()))["entry"].clone();
            let out = data(&s.submit(
                &json!({ "experimentId": id, "protocolId": protocol_id,
                         "values": { "count": i + 1, "photos": [photo] } }).to_string(),
                at(&format!("2026-10-28T09:0{i}:00Z")),
            ));
            cids.push(out["cid"].as_str().expect("indexed").to_string());
        }
        (s, id, cids)
    }

    fn push(s: &mut Session, id: &str) -> Value {
        let protocol = s.protocols[0]["id"].as_str().unwrap().to_string();
        envelope(&s.publish_results(
            &json!({ "experimentId": id, "protocolId": protocol }).to_string(),
            at("2026-10-29T10:00:00Z"),
        ))
    }

    #[test]
    fn pushing_publishes_only_approved_rows_with_the_experiment_and_form_as_signed() {
        let scratch = Scratch::new("results-push");
        let (mut s, id, cids) = with_photo_submissions(&scratch, 3);
        approve(&mut s, &id, &cids[0]);
        approve(&mut s, &id, &cids[2]);
        s.set_approval(&json!({ "experimentId": id, "cid": cids[1], "status": "rejected" }).to_string());

        let out = push(&mut s, &id);
        assert_eq!(out["ok"], json!(true), "{out}");
        let cid = out["data"]["cid"].as_str().unwrap();
        assert_eq!(out["data"]["rows"], json!(2));
        // Both rows carry the same photo file, which cleans to the same bytes
        // and so the same CID: counted once, held once.
        assert_eq!(out["data"]["media"], json!({ "named": 1, "held": 1 }), "{out}");

        // What Storage holds is a valid, signed snapshot whose parts verify alone.
        let bytes = s.store.as_deref().unwrap().get(store::bare_cid(cid), true).unwrap();
        let doc: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(schema::errors_as(&doc, schema::Kind::Results).is_empty(), "{:?}", schema::errors_as(&doc, schema::Kind::Results));
        // Like a protocol or an index page, the snapshot names no signer of
        // its own: it is signed by the experiment's creator, whose key the
        // embedded manifest carries.
        let creator = doc.pointer("/experiment/creator/key").and_then(Value::as_str).unwrap();
        let mut key = [0u8; 32];
        hex::decode_to_slice(creator, &mut key).unwrap();
        assert!(crate::sign::verify_with(&doc, &key).is_ok(), "the snapshot is not the creator's");
        assert!(crate::sign::verify(&doc["experiment"]).is_ok());
        assert!(crate::sign::verify_with(&doc["protocol"], &key).is_ok());
        let published: Vec<&str> = doc["rows"].as_array().unwrap().iter()
            .map(|r| { assert!(crate::sign::verify(&r["submission"]).is_ok()); r["cid"].as_str().unwrap() })
            .collect();
        assert_eq!(published, vec![cids[0].as_str(), cids[2].as_str()], "a rejected row was published");

        // And the experiment now points at it.
        let manifest = &data(&s.get(&json!({ "experimentId": id }).to_string()))["experiment"];
        assert_eq!(manifest["results"][0]["cid"], json!(cid));
        assert!(schema::errors_as(manifest, schema::Kind::Experiment).is_empty());
    }

    #[test]
    fn pushing_again_links_to_the_previous_snapshot() {
        let scratch = Scratch::new("results-again");
        let (mut s, id, cids) = with_photo_submissions(&scratch, 2);
        approve(&mut s, &id, &cids[0]);
        let first = push(&mut s, &id)["data"]["cid"].clone();
        approve(&mut s, &id, &cids[1]);
        let second = push(&mut s, &id);
        assert_eq!(second["data"]["prev"], first, "{second}");
        assert_eq!(second["data"]["rows"], json!(2));
        let manifest = &data(&s.get(&json!({ "experimentId": id }).to_string()))["experiment"];
        assert_eq!(manifest["results"].as_array().unwrap().len(), 1, "one entry per form, the newest");
        assert_eq!(manifest["results"][0]["cid"], second["data"]["cid"]);
    }

    #[test]
    fn nothing_approved_means_nothing_is_pushed() {
        let scratch = Scratch::new("results-none");
        let (mut s, id, _) = with_photo_submissions(&scratch, 1);
        let out = push(&mut s, &id);
        assert_eq!(out["ok"], json!(false));
        assert!(out["error"]["message"].as_str().unwrap().contains("approved"), "{out}");
    }

    #[test]
    fn a_photo_nobody_holds_is_listed_as_missing_not_hidden() {
        let scratch = Scratch::new("results-missing");
        let (mut s, id, cids) = with_photo_submissions(&scratch, 1);
        // The submission names a photo that is nowhere: swap in a store
        // that has the documents but not the photo.
        let photo_cid = s.submissions[0]["values"]["photos"][0]["cid"].as_str().unwrap().to_string();
        let old = s.store.take().unwrap();
        let fresh = crate::store::MemoryStore::new();
        for doc in s.submissions.iter().chain(s.protocols.iter()) {
            let _ = crate::store::publish(&fresh, "doc", doc);
        }
        drop(old);
        s.attach_store(Box::new(fresh));
        approve(&mut s, &id, &cids[0]);
        let out = push(&mut s, &id);
        assert_eq!(out["ok"], json!(true), "{out}");
        assert_eq!(out["data"]["media"]["held"], json!(0));
        assert_eq!(out["data"]["media"]["missing"], json!([photo_cid]));
    }

    #[test]
    fn only_the_creator_can_push_and_storage_is_required() {
        let scratch = Scratch::new("results-who");
        let (mut s, id, cids) = with_photo_submissions(&scratch, 1);
        approve(&mut s, &id, &cids[0]);
        let mine = s.creator_key.clone();
        s.creator_key = "3a".repeat(32);
        assert_eq!(push(&mut s, &id)["ok"], json!(false));
        s.creator_key = mine;
        s.store = None;
        assert_eq!(push(&mut s, &id)["error"]["code"], json!(api::code::NOT_WIRED));
    }

    // ---- joining by link, outbox, receipts ----

    /// A creator with a published experiment and form, and a submitter on a
    /// separate node that shares the creator's Storage and has its own
    /// Delivery client. Messages between them are carried by hand.
    fn two_nodes(
        creator_dir: &Scratch,
        submitter_dir: &Scratch,
    ) -> (Session, Session, String, String, std::sync::Arc<crate::delivery::MemoryCourier>, std::sync::Arc<crate::delivery::MemoryCourier>) {
        let (creator, id, store, creator_courier) = creator_with_delivery(creator_dir);
        let link = data(&creator.list(at("2026-10-28T08:00:00Z")))["experiments"][0]["link"]
            .as_str().unwrap().to_string();
        let mut submitter = empty_in(submitter_dir);
        submitter.attach_store(Box::new(SharedStore(store)));
        let submitter_courier = std::sync::Arc::new(crate::delivery::MemoryCourier::new());
        submitter.attach_courier(Box::new(SharedCourier(submitter_courier.clone())));
        (creator, submitter, id, link, creator_courier, submitter_courier)
    }

    fn open(s: &mut Session, link: &str) -> Value {
        envelope(&s.open_link(&json!({ "link": link }).to_string()))
    }

    #[test]
    fn a_link_brings_the_experiment_and_its_forms_checked_and_watched() {
        let (a, b) = (Scratch::new("join-c"), Scratch::new("join-s"));
        let (_creator, mut submitter, id, link, _, submitter_courier) = two_nodes(&a, &b);
        let out = open(&mut submitter, &link);
        assert_eq!(out["ok"], json!(true), "{out}");
        assert_eq!(out["data"]["experiment"]["id"], json!(id));
        assert_eq!(out["data"]["protocols"].as_array().unwrap().len(), 1, "the form did not come with it");
        assert_eq!(out["data"]["missing_forms"], json!([]));
        assert!(submitter_courier.subscriptions().contains(&crate::delivery::topic(&id)),
                "joined without watching the topic, so no receipt could ever arrive");
        // It is on the dashboard as one to contribute to, not one of ours.
        let card = &data(&submitter.list(at("2026-10-28T08:00:00Z")))["experiments"][0];
        assert_eq!(card["mine"], json!(false));
        // Opening it again is harmless.
        assert_eq!(open(&mut submitter, &link)["ok"], json!(true));
        assert_eq!(submitter.experiments.len(), 1);
    }

    #[test]
    fn a_tampered_or_foreign_link_is_refused() {
        let (a, b) = (Scratch::new("join-bad-c"), Scratch::new("join-bad-s"));
        let (_creator, mut submitter, id, link, _, _) = two_nodes(&a, &b);
        assert_eq!(open(&mut submitter, "https://example.org/x")["ok"], json!(false));
        // A link whose id does not match the document it points at.
        let other_id = if id.starts_with('a') { format!("b{}", &id[1..]) } else { format!("a{}", &id[1..]) };
        let wrong = link.replace(&id, &other_id);
        let out = open(&mut submitter, &wrong);
        assert_eq!(out["ok"], json!(false), "{out}");
        assert!(submitter.experiments.is_empty());
        // A manifest CID nobody holds.
        let missing = format!("logos-collect://exp/{id}?m=logos:zNothingHere");
        assert_eq!(open(&mut submitter, &missing)["ok"], json!(false));
    }

    #[test]
    fn a_submission_is_announced_kept_in_the_outbox_and_settled_by_the_creators_receipt() {
        let (a, b) = (Scratch::new("outbox-c"), Scratch::new("outbox-s"));
        let (mut creator, mut submitter, id, link, creator_courier, submitter_courier) = two_nodes(&a, &b);
        assert_eq!(open(&mut submitter, &link)["ok"], json!(true));

        let out = data(&submitter.submit(&submit_request(&id, 4), at("2026-10-28T09:00:00Z")));
        assert_eq!(out["announce"]["sent"], json!(true), "{out}");
        let cid = out["cid"].as_str().unwrap().to_string();
        let mine = data(&submitter.submissions_mine(&json!({ "experimentId": id }).to_string()));
        assert_eq!(mine["submissions"][0]["state"], json!("waiting"));

        // The announcement reaches the creator, who indexes it and receipts.
        let announcement = submitter_courier.messages().last().cloned().unwrap();
        assert_eq!(announcement["kind"], json!("submission"));
        let handled = data(&creator.deliver(&arriving(&id, &announcement, "0xsub1"), at("2026-10-28T09:01:00Z")));
        assert_eq!(handled["handled"], json!(true), "{handled}");
        let receipt = creator_courier.messages().into_iter().rev()
            .find(|m| m["kind"] == json!("receipt")).expect("a receipt was sent");

        // The receipt reaches the submitter and settles the outbox.
        let got = data(&submitter.deliver(&arriving(&id, &receipt, "0xrcpt1"), at("2026-10-28T09:02:00Z")));
        assert_eq!(got["handled"], json!(true), "{got}");
        let mine = data(&submitter.submissions_mine(&json!({ "experimentId": id }).to_string()));
        assert_eq!(mine["submissions"][0]["state"], json!("received"));
        assert_eq!(mine["submissions"][0]["cid"], json!(cid));
    }

    #[test]
    fn a_receipt_from_anyone_but_the_creator_settles_nothing() {
        let (a, b) = (Scratch::new("outbox-forged-c"), Scratch::new("outbox-forged-s"));
        let (_creator, mut submitter, id, link, _, _) = two_nodes(&a, &b);
        open(&mut submitter, &link);
        let out = data(&submitter.submit(&submit_request(&id, 1), at("2026-10-28T09:00:00Z")));
        let cid = out["cid"].as_str().unwrap();
        let stranger = Scratch::new("outbox-stranger");
        let stranger_key = Identity::load_or_create(stranger.0.as_path()).unwrap().public_hex();
        let forged = signed_by(&stranger, crate::delivery::receipt(&id, &stranger_key, cid, 1, "2026-10-28T09:05:00Z"));
        let got = data(&submitter.deliver(&arriving(&id, &forged, "0xforged"), at("2026-10-28T09:05:00Z")));
        assert_eq!(got["reason"], json!("not_the_creator"), "{got}");
        let mine = data(&submitter.submissions_mine("{}"));
        assert_eq!(mine["submissions"][0]["state"], json!("waiting"));
    }

    #[test]
    fn an_unconfirmed_submission_is_announced_again_with_growing_gaps_and_survives_a_restart() {
        let (a, b) = (Scratch::new("outbox-resend-c"), Scratch::new("outbox-resend-s"));
        let (_creator, mut submitter, id, link, _, submitter_courier) = two_nodes(&a, &b);
        open(&mut submitter, &link);
        submitter.submit(&submit_request(&id, 1), at("2026-10-28T09:00:00Z"));
        let sent = || submitter_courier.messages().iter().filter(|m| m["kind"] == json!("submission")).count();
        assert_eq!(sent(), 1);

        submitter.drain(at("2026-10-28T09:01:00Z"));
        assert_eq!(sent(), 1, "re-sent before the first gap");
        submitter.drain(at("2026-10-28T09:03:00Z"));
        assert_eq!(sent(), 2, "not re-sent after three minutes");
        submitter.drain(at("2026-10-28T09:07:00Z"));
        assert_eq!(sent(), 2, "the second gap did not grow");
        submitter.drain(at("2026-10-28T09:09:00Z"));
        assert_eq!(sent(), 3);

        submitter.save_state();
        let again = empty_in(&b);
        let mine = data(&again.submissions_mine("{}"));
        assert_eq!(mine["submissions"][0]["state"], json!("waiting"), "the outbox was lost on restart");
        assert_eq!(mine["submissions"][0]["attempts"], json!(3));
    }

    #[test]
    fn a_creators_own_submission_needs_no_outbox() {
        let scratch = Scratch::new("outbox-own");
        let (mut s, id) = creator_ready_to_submit(&scratch);
        let out = data(&s.submit(&submit_request(&id, 2), at("2026-10-28T09:00:00Z")));
        assert_eq!(out["announce"], Value::Null);
        assert!(s.outbox.is_empty());
    }

    #[test]
    fn a_creator_can_submit_to_their_own_experiment() {
        // An ordinary submitter, not a special case: same validation, same
        // signature, same document. Only the indexing differs, because this
        // node is also the indexer.
        let scratch = Scratch::new("selfsubmit");
        let (mut s, id) = creator_ready_to_submit(&scratch);
        let out = data(&s.submit(&submit_request(&id, 3), at("2026-10-28T09:00:00Z")));

        assert_eq!(out["submitted"], json!(true), "{out}");
        assert_eq!(out["published"], json!(true), "{out}");
        let cid = out["cid"].as_str().expect("a cid");
        assert!(cid.starts_with("logos:"), "{cid}");
        // Indexed immediately rather than waiting to hear about itself.
        assert_eq!(out["index_page"], json!(1));

        // And it is in the creator's own results table.
        let table = data(&s.results(
            &json!({ "experimentId": id, "protocolId": "bird-sighting" }).to_string(),
        ));
        assert_eq!(table["rows"].as_array().unwrap().len(), 1, "{table}");
        assert_eq!(table["rows"][0]["values"]["count"], json!(3));
        assert_eq!(table["rows"][0]["cid"], json!(cid));
        assert_eq!(table["rows"][0]["submitter_name"], json!("Dr Rao"));
    }

    #[test]
    fn a_submission_is_signed_by_the_submitter() {
        let scratch = Scratch::new("submitsig");
        let (mut s, id) = creator_ready_to_submit(&scratch);
        let out = data(&s.submit(&submit_request(&id, 2), at("2026-10-28T09:00:00Z")));
        let doc = &out["submission"];
        assert_eq!(doc["submitter"]["key"], json!(s.creator_key()));
        assert!(crate::sign::verify(doc).is_ok(), "{doc}");
    }

    #[test]
    fn a_submission_that_fails_the_form_comes_back_as_field_problems() {
        // Not a rejected document: the person has to be able to fix it.
        let scratch = Scratch::new("submitbad");
        let (mut s, id) = creator_ready_to_submit(&scratch);
        let out = data(&s.submit(
            &json!({ "experimentId": id, "protocolId": "bird-sighting", "values": {} }).to_string(),
            at("2026-10-28T09:00:00Z"),
        ));
        assert_eq!(out["submitted"], json!(false), "{out}");
        assert!(out["missing"].as_array().unwrap().contains(&json!("count")), "{out}");
        // Nothing was kept.
        assert_eq!(s.submissions.len(), 0);
    }

    #[test]
    fn a_closed_experiment_refuses_a_submission() {
        let scratch = Scratch::new("submitclosed");
        let (mut s, id) = creator_ready_to_submit(&scratch);
        // Well after the end date.
        let out = envelope(&s.submit(&submit_request(&id, 1), at("2027-01-01T00:00:00Z")));
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"]["code"], json!("rejected"));
    }

    #[test]
    fn store_rules_are_applied_before_the_document_exists() {
        // An exact coordinate that reaches a document is public forever.
        let scratch = Scratch::new("submitround");
        let mut s = publishing_in(&scratch);
        let now = at("2026-10-28T08:00:00Z");
        let made = data(&s.create(&create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"), now));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();
        data(&s.save_protocol(
            &json!({
                "experimentId": id,
                "protocol": {
                    "name": "Where",
                    "fields": [ {
                        "key": "where", "type": "location", "label": "Where", "required": true,
                        "config": { "methods": ["manual"], "store": "rounded_1km" }
                    } ]
                }
            })
            .to_string(),
            now,
        ));

        let out = data(&s.submit(
            &json!({
                "experimentId": id,
                "protocolId": "where",
                "values": { "where": { "lat": 12.97712, "lon": 77.59581, "method": "manual" } }
            })
            .to_string(),
            at("2026-10-28T09:00:00Z"),
        ));
        assert_eq!(out["submitted"], json!(true), "{out}");
        let stored = &out["submission"]["values"]["where"];
        // Snapped to its cell centre, not the coordinate that was handed in.
        assert_ne!(stored["lat"], json!(12.97712));
        assert_eq!(stored["lat"], json!(12.975));
        assert_eq!(stored["lon"], json!(77.595));
    }

    #[test]
    fn the_seeded_example_does_not_keep_someone_elses_name_on_our_key() {
        // It read "Nisha Kumar" over a key that is ours, which is worse than
        // saying nothing.
        let scratch = Scratch::new("reauthorname");
        let mut s = seeded();
        s.attach_identity(scratch.0.to_str().unwrap()).expect("identity");
        s.reauthor_fixtures();
        assert_eq!(s.experiments[0]["creator"]["name"], json!(""));
        assert_eq!(s.experiments[0]["creator"]["key"], json!(s.creator_key()));
    }

    /// A store two owners can look at: the session takes a `Box`, and a test
    /// that wants to inspect what landed needs its own handle.
    struct SharedStore(std::sync::Arc<crate::store::MemoryStore>);

    impl crate::store::DocumentStore for SharedStore {
        fn put(&self, filename: &str, bytes: &[u8]) -> Result<String, StoreError> {
            self.0.put(filename, bytes)
        }
        fn get(&self, cid: &str, local: bool) -> Result<Vec<u8>, StoreError> {
            self.0.get(cid, local)
        }
    }

    #[test]
    fn a_new_session_lists_nothing_without_fixtures() {
        let scratch = Scratch::new("listnothing");
        let out = data(&empty_in(&scratch).list(at("2026-10-28T08:00:00Z")));
        assert_eq!(out["experiments"], json!([]));
        assert_eq!(out["mine"], json!(0));
    }

    #[test]
    fn create_then_list_shows_the_card() {
        let scratch = Scratch::new("s");
        let mut s = empty_in(&scratch);
        let made = data(&s.create(
            &create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        assert_eq!(made["created"], json!(true));
        // Not published — Storage is not wired, and saying so beats implying
        // it reached the network.
        assert_eq!(made["published"], json!(false));

        let out = data(&s.list(at("2026-10-28T08:00:00Z")));
        let cards = out["experiments"].as_array().unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0]["title"], json!("Urban birds"));
        assert_eq!(cards[0]["standing"], json!("open"));
        assert_eq!(cards[0]["mine"], json!(true));
        assert_eq!(cards[0]["protocol_count"], json!(0));
        assert_eq!(out["mine"], json!(1));
    }

    #[test]
    fn a_created_experiment_validates_against_the_contract() {
        let scratch = Scratch::new("s");
        let mut s = empty_in(&scratch);
        let made = data(&s.create(
            &create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        let experiment = &made["experiment"];
        assert!(schema::is_valid(experiment), "{:?}", schema::errors(experiment));
    }

    #[test]
    fn create_rejects_an_end_before_the_start() {
        // SPEC 3.1, the inline error the C2 form shows.
        let scratch = Scratch::new("s");
        let mut s = empty_in(&scratch);
        let out = data(&s.create(
            &create_request("2026-11-20T00:00:00Z", "2026-10-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        assert_eq!(out["created"], json!(false));
        let problems = out["problems"].as_array().unwrap();
        assert!(problems.iter().any(|p| p["field"] == json!("endsAt")), "{out}");
        // And nothing entered the list.
        assert_eq!(data(&s.list(at("2026-10-28T08:00:00Z")))["experiments"], json!([]));
    }

    #[test]
    fn create_requires_a_title_and_dates() {
        let scratch = Scratch::new("s");
        let mut s = empty_in(&scratch);
        let out = data(&s.create(&json!({}).to_string(), at("2026-10-28T08:00:00Z")));
        assert_eq!(out["created"], json!(false));
        let fields: Vec<&str> = out["problems"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["field"].as_str().unwrap())
            .collect();
        assert!(fields.contains(&"title"), "{out}");
        assert!(fields.contains(&"startsAt"), "{out}");
        assert!(fields.contains(&"endsAt"), "{out}");
    }

    #[test]
    fn standing_follows_the_dates() {
        let scratch = Scratch::new("s");
        let mut s = empty_in(&scratch);
        s.create(
            &create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        );
        let standing = |now: &str| {
            data(&s.list(at(now)))["experiments"][0]["standing"].as_str().unwrap().to_string()
        };
        assert_eq!(standing("2026-10-01T00:00:00Z"), "draft");
        assert_eq!(standing("2026-10-28T08:00:00Z"), "open");
        assert_eq!(standing("2026-12-01T00:00:00Z"), "ended");
    }

    #[test]
    fn get_returns_the_manifest_and_its_held_protocols() {
        let scratch = Scratch::new("s");
        let mut s = empty_in(&scratch);
        let made = data(&s.create(
            &create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();

        let out = data(&s.get(&json!({ "experimentId": id }).to_string()));
        assert_eq!(out["experiment"]["id"], json!(id));
        assert_eq!(out["protocols"], json!([]));
        assert_eq!(out["missing_protocols"], json!(0));
    }

    #[test]
    fn get_rejects_an_unknown_experiment() {
        let scratch = Scratch::new("get");
        let out = envelope(&empty_in(&scratch).get(&json!({ "experimentId": "q7k2m3x4w3b5n6tz" }).to_string()));
        assert_eq!(out["ok"], json!(false));
    }

    #[test]
    fn saving_a_protocol_attaches_it_to_the_manifest() {
        let scratch = Scratch::new("s");
        let mut s = empty_in(&scratch);
        let now = at("2026-10-28T08:00:00Z");
        let made = data(&s.create(&create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"), now));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();

        let saved = data(&s.save_protocol(
            &json!({
                "experimentId": id,
                "protocol": {
                    "name": "Bird sighting",
                    "instructions": "Count what you see.",
                    "fields": [
                        { "key": "count", "type": "number", "label": "How many", "required": true,
                          "config": { "min": 1, "integer": true } }
                    ]
                }
            })
            .to_string(),
            now,
        ));
        assert_eq!(saved["saved"], json!(true), "{saved}");
        // Name turned into a contract-legal id.
        assert_eq!(saved["protocol"]["id"], json!("bird-sighting"));
        assert_eq!(saved["protocol"]["version"], json!(1));

        // And the manifest now names it, so S1 can list it.
        let out = data(&s.get(&json!({ "experimentId": id }).to_string()));
        assert_eq!(out["experiment"]["protocols"][0]["id"], json!("bird-sighting"));
        assert_eq!(out["experiment"]["protocols"][0]["status"], json!("open"));
        assert_eq!(out["protocols"].as_array().unwrap().len(), 1);
        assert_eq!(data(&s.list(now))["experiments"][0]["protocol_count"], json!(1));
    }

    #[test]
    fn saving_the_same_protocol_again_bumps_the_version() {
        // SPEC 4: editing publishes a new version; earlier submissions name
        // the version they used.
        let scratch = Scratch::new("s");
        let mut s = empty_in(&scratch);
        let now = at("2026-10-28T08:00:00Z");
        let made = data(&s.create(&create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"), now));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();
        let draft = json!({
            "experimentId": id,
            "protocol": {
                "name": "Bird sighting",
                "fields": [ { "key": "count", "type": "number", "label": "How many", "required": true } ]
            }
        })
        .to_string();

        assert_eq!(data(&s.save_protocol(&draft, now))["protocol"]["version"], json!(1));
        assert_eq!(data(&s.save_protocol(&draft, now))["protocol"]["version"], json!(2));
        // Still one entry in the manifest, not two.
        let out = data(&s.get(&json!({ "experimentId": id }).to_string()));
        assert_eq!(out["experiment"]["protocols"].as_array().unwrap().len(), 1);
        assert_eq!(out["experiment"]["protocols"][0]["version"], json!(2));
    }

    #[test]
    fn a_protocol_the_renderer_could_not_draw_is_refused() {
        // A choice field with no options passes nothing: the contract rejects
        // it, and so must we — before it reaches a submitter's screen.
        let scratch = Scratch::new("s");
        let mut s = empty_in(&scratch);
        let now = at("2026-10-28T08:00:00Z");
        let made = data(&s.create(&create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"), now));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();

        let out = data(&s.save_protocol(
            &json!({
                "experimentId": id,
                "protocol": {
                    "name": "Broken",
                    "fields": [ { "key": "pick", "type": "choice", "label": "Pick", "required": true } ]
                }
            })
            .to_string(),
            now,
        ));
        assert_eq!(out["saved"], json!(false), "{out}");
        assert!(!out["problems"].as_array().unwrap().is_empty());
    }

    #[test]
    fn a_protocol_needs_a_name() {
        let scratch = Scratch::new("s");
        let mut s = empty_in(&scratch);
        let now = at("2026-10-28T08:00:00Z");
        let made = data(&s.create(&create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"), now));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();
        let out = data(&s.save_protocol(
            &json!({ "experimentId": id, "protocol": { "name": "  " } }).to_string(),
            now,
        ));
        assert_eq!(out["saved"], json!(false));
        assert_eq!(out["problems"][0]["field"], json!("name"));
    }

    #[test]
    fn saving_to_an_unknown_experiment_is_refused() {
        let scratch = Scratch::new("save");
        let out = envelope(&empty_in(&scratch).save_protocol(
            &json!({ "experimentId": "q7k2m3x4w3b5n6tz", "protocol": { "name": "X" } }).to_string(),
            at("2026-10-28T08:00:00Z"),
        ));
        assert_eq!(out["ok"], json!(false));
    }

    #[test]
    fn a_created_experiment_is_really_signed() {
        // The placeholder is gone: this must verify against the creator key
        // the document itself names.
        let scratch = Scratch::new("signed");
        let mut s = empty_in(&scratch);
        let made = data(&s.create(
            &create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        let experiment = &made["experiment"];
        assert_ne!(experiment["sig"], json!("00".repeat(64)));
        assert!(crate::sign::verify(experiment).is_ok(), "{experiment}");
        assert_eq!(experiment["creator"]["key"], json!(s.creator_key()));
    }

    #[test]
    fn a_saved_protocol_is_really_signed() {
        let scratch = Scratch::new("signedproto");
        let mut s = empty_in(&scratch);
        let now = at("2026-10-28T08:00:00Z");
        let made = data(&s.create(&create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"), now));
        let id = made["experiment"]["id"].as_str().unwrap().to_string();
        let saved = data(&s.save_protocol(
            &json!({
                "experimentId": id,
                "protocol": { "name": "Birds",
                    "fields": [ { "key": "count", "type": "number", "label": "How many", "required": true } ] }
            }).to_string(),
            now,
        ));
        let protocol = &saved["protocol"];
        assert_ne!(protocol["sig"], json!("00".repeat(64)));
        // A protocol carries no author key, so it verifies against the
        // creator key from the manifest.
        let mut key = [0u8; 32];
        hex::decode_to_slice(s.creator_key(), &mut key).unwrap();
        assert!(crate::sign::verify_with(protocol, &key).is_ok(), "{protocol}");
    }

    #[test]
    fn creating_without_a_signing_key_is_refused() {
        // Emitting an unsigned document would make a failure look like a
        // success — every reader would reject it later.
        let mut s = Session {
            experiments: Vec::new(),
            protocols: Vec::new(),
            submissions: Vec::new(),
            index_pages: Vec::new(),
            creator_key: "3a".repeat(32),
            identity: None,
            store: None,
            publications: BTreeMap::new(),
            publish_until: None,
            state_dir: None,
            saved_state: None,
            thumbs: Default::default(),
            thumbs_tried: Default::default(),
            send_failure: None,
            last_level: Default::default(),
            approvals: BTreeMap::new(),
            payouts: Vec::new(),
            ledger: None,
            outbox: Vec::new(),
            courier: None,
            seen: delivery::Seen::default(),
            receipts: BTreeMap::new(),
            heads: BTreeMap::new(),
            social: Social::default(),
        };
        let out = envelope(&s.create(
            &create_request("2026-10-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            at("2026-10-28T08:00:00Z"),
        ));
        assert_eq!(out["ok"], json!(false));
        assert!(!s.can_sign());
    }

    /// A session holding the example experiment, protocol, submission and
    /// index page — what fixtures mode seeds.
    fn seeded() -> Session {
        Session {
            experiments: vec![serde_json::from_str(api::fixture_experiment()).unwrap()],
            protocols: vec![serde_json::from_str(api::fixture_protocol()).unwrap()],
            submissions: vec![serde_json::from_str(api::fixture_submission()).unwrap()],
            index_pages: vec![serde_json::from_str(api::fixture_index()).unwrap()],
            creator_key: "3a9d0f213a9d0f213a9d0f213a9d0f213a9d0f213a9d0f213a9d0f213a9d0f21".into(),
            identity: None,
            store: None,
            publications: BTreeMap::new(),
            publish_until: None,
            state_dir: None,
            saved_state: None,
            thumbs: Default::default(),
            thumbs_tried: Default::default(),
            send_failure: None,
            last_level: Default::default(),
            approvals: BTreeMap::new(),
            payouts: Vec::new(),
            ledger: None,
            outbox: Vec::new(),
            courier: None,
            seen: delivery::Seen::default(),
            receipts: BTreeMap::new(),
            heads: BTreeMap::new(),
            social: Social::default(),
        }
    }

    fn results_request() -> String {
        json!({ "experimentId": "q7k2m3x4w3b5n6tz", "protocolId": "wildlife-sighting" }).to_string()
    }

    #[test]
    fn results_columns_follow_the_protocol_fields() {
        // SPEC 3.3: the table's columns are the form's fields, in order.
        let out = data(&seeded().results(&results_request()));
        let keys: Vec<&str> = out["columns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["key"].as_str().unwrap())
            .collect();
        assert_eq!(
            keys,
            ["photos", "category", "species", "count", "location", "observed", "behaviour", "notes"]
        );
        // The type travels too, so a cell can be rendered for what it is.
        assert_eq!(out["columns"][0]["type"], json!("photo"));
        assert_eq!(out["columns"][4]["type"], json!("location"));
        // A choice column carries its options so the cell can show a label.
        assert_eq!(out["columns"][1]["options"][0]["value"], json!("birds"));
        assert_eq!(out["columns"][1]["options"][0]["label"], json!("Birds"));
        // A column with no options says so rather than inventing an empty
        // list, so a cell can tell the difference.
        assert_eq!(out["columns"][2]["options"], json!(null));
    }

    #[test]
    fn results_rows_carry_the_values_and_both_timestamps() {
        let out = data(&seeded().results(&results_request()));
        let rows = out["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["values"]["category"], json!("birds"));
        assert_eq!(rows[0]["version"], json!(2));
        // submitted_at comes from the submission; received_at only exists in
        // the index, which is the creator's own record.
        assert_eq!(rows[0]["submitted_at"], json!("2026-10-28T07:50:12+05:30"));
        assert_eq!(rows[0]["received_at"], json!("2026-10-28T07:50:31+05:30"));
    }

    #[test]
    fn results_summary_separates_held_from_indexed() {
        // The index names two entries but only one belongs to this protocol,
        // and this node holds that one.
        let out = data(&seeded().results(&results_request()));
        assert_eq!(out["summary"]["shown"], json!(1));
        assert_eq!(out["summary"]["indexed"], json!(1));
        assert_eq!(out["summary"]["not_fetched"], json!(0));
        assert_eq!(out["summary"]["submitters"], json!(1));
    }

    #[test]
    fn results_drop_a_hidden_submitter() {
        // SPEC 3.3: hiding removes the submitter from results and export.
        let mut s = seeded();
        let submitter = s.submissions[0]["submitter"]["key"].as_str().unwrap().to_string();
        s.index_pages[0]["hidden_keys"] = json!([submitter]);
        let out = data(&s.results(&results_request()));
        assert_eq!(out["rows"], json!([]));
        assert_eq!(out["summary"]["shown"], json!(0));
    }

    #[test]
    fn results_reject_an_unknown_protocol() {
        let out = envelope(&seeded().results(
            &json!({ "experimentId": "q7k2m3x4w3b5n6tz", "protocolId": "nope" }).to_string(),
        ));
        assert_eq!(out["ok"], json!(false));
    }

    #[test]
    fn results_require_both_ids() {
        for request in [json!({}), json!({ "experimentId": "q7k2m3x4w3b5n6tz" })] {
            let out = envelope(&seeded().results(&request.to_string()));
            assert_eq!(out["ok"], json!(false), "{out}");
        }
    }

    #[test]
    fn slug_produces_contract_legal_ids() {
        assert_eq!(slug("Wildlife sighting"), "wildlife-sighting");
        assert_eq!(slug("Nest  monitoring!"), "nest-monitoring");
        assert_eq!(slug("  Trailing  "), "trailing");
        assert_eq!(slug("!!!"), "form");
        for name in ["Wildlife sighting", "Nest  monitoring!", "!!!", "A"] {
            assert!(ids::is_protocol_id(&slug(name)), "{name} -> {}", slug(name));
        }
    }

    #[test]
    fn fixtures_seed_the_example_experiment_as_mine() {
        // The embedded example is authored by the fixture creator key, so it
        // shows under "created by you" and the creator screens have something
        // to open.
        let s = Session {
            experiments: vec![serde_json::from_str(api::fixture_experiment()).unwrap()],
            protocols: vec![serde_json::from_str(api::fixture_protocol()).unwrap()],
            submissions: Vec::new(),
            index_pages: Vec::new(),
            creator_key: "3a9d0f213a9d0f213a9d0f213a9d0f213a9d0f213a9d0f213a9d0f213a9d0f21".into(),
            identity: None,
            store: None,
            publications: BTreeMap::new(),
            publish_until: None,
            state_dir: None,
            saved_state: None,
            thumbs: Default::default(),
            thumbs_tried: Default::default(),
            send_failure: None,
            last_level: Default::default(),
            approvals: BTreeMap::new(),
            payouts: Vec::new(),
            ledger: None,
            outbox: Vec::new(),
            courier: None,
            seen: delivery::Seen::default(),
            receipts: BTreeMap::new(),
            heads: BTreeMap::new(),
            social: Social::default(),
        };
        let out = data(&s.list(at("2026-10-28T08:00:00Z")));
        let cards = out["experiments"].as_array().unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0]["mine"], json!(true));
        assert_eq!(cards[0]["standing"], json!("open"));
        assert_eq!(cards[0]["protocol_count"], json!(3));
        assert_eq!(cards[0]["open_protocol_count"], json!(2));
    }

    // ------------------------------------------- profile and discussion tests

    /// Another project member: an identity and a `Social` of their own.
    struct Member {
        dir: Scratch,
        social: crate::social::Social,
    }

    impl Member {
        fn new(name: &str) -> Self {
            let dir = Scratch::new(name);
            let mut social = crate::social::Social::default();
            Identity::load_or_create(dir.0.as_path()).expect("identity");
            social.attach(dir.0.as_path()).expect("social");
            Member { dir, social }
        }
        fn key(&self) -> String {
            Identity::load_or_create(self.dir.0.as_path()).unwrap().public_hex()
        }
        fn say(&self, experiment: &str, text: &str, now: DateTime<Utc>) -> Value {
            let m = self.social.compose(experiment, &self.key(), text, None, now).unwrap();
            signed_by(&self.dir, m)
        }
    }

    #[test]
    fn a_sent_message_goes_out_signed_on_the_topic_and_never_to_storage() {
        let scratch = Scratch::new("msgsend");
        let (mut s, id, store, courier) = creator_with_delivery(&scratch);
        let stored_before = store.len();
        let out = data(&s.messages_send(
            &json!({ "experimentId": id, "text": "  Welcome, everyone  " }).to_string(),
            at("2026-10-28T09:00:00Z"),
        ));
        assert_eq!(out["sent"], json!(true));
        let sent: Vec<Value> = courier.messages().into_iter().filter(|m| m["kind"] == json!("message")).collect();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["text"], json!("Welcome, everyone"));
        crate::sign::verify(&sent[0]).expect("signed");
        assert_eq!(schema::errors_as(&sent[0], schema::Kind::MessageAnnouncement), Vec::<String>::new());
        assert_eq!(store.len(), stored_before, "a message was written to Logos Storage");
        let list = data(&s.messages_list(&json!({ "experimentId": id }).to_string()));
        assert_eq!(list["messages"][0]["mine"], json!(true));
        assert_eq!(list["messages"][0]["creator"], json!(true));
    }

    #[test]
    fn a_message_that_cannot_go_out_is_not_kept() {
        let scratch = Scratch::new("msgrln");
        let (mut s, id, _store, _courier) = creator_with_delivery(&scratch);
        s.attach_courier(Box::new(crate::delivery::MemoryCourier::without_rln()));
        let e = envelope(&s.messages_send(
            &json!({ "experimentId": id, "text": "hello" }).to_string(),
            at("2026-10-28T09:00:00Z"),
        ));
        assert_eq!(e["error"]["code"], json!("rln_not_ready"));
        let list = data(&s.messages_list(&json!({ "experimentId": id }).to_string()));
        assert_eq!(list["messages"], json!([]), "an unsent message was listed");
    }

    #[test]
    fn a_message_for_an_experiment_this_node_does_not_hold_is_refused() {
        let scratch = Scratch::new("msgunknown");
        let (mut s, _id, _store, _courier) = creator_with_delivery(&scratch);
        let e = envelope(&s.messages_send(
            &json!({ "experimentId": "zzzzzzzzzzzzzzzz", "text": "hello" }).to_string(),
            at("2026-10-28T09:00:00Z"),
        ));
        assert_eq!(e["ok"], json!(false));
    }

    #[test]
    fn a_members_message_arrives_once_through_the_receive_path() {
        let scratch = Scratch::new("msgrecv");
        let (mut s, id, _store, _courier) = creator_with_delivery(&scratch);
        let bob = Member::new("msgrecvbob");
        let m = bob.say(&id, "Two kites at the north gate", at("2026-10-28T09:00:00Z"));
        let first = data(&s.deliver(&arriving(&id, &m, "0xm1"), at("2026-10-28T09:00:01Z")));
        assert_eq!(first["handled"], json!(true), "{first}");
        assert_eq!(first["kind"], json!("message"));
        // Same message replayed under a different hash (Store catch-up after
        // a restart re-wraps it): still kept once.
        let again = data(&s.deliver(&arriving(&id, &m, "0xm2"), at("2026-10-28T09:00:02Z")));
        assert_eq!(again["handled"], json!(false));
        let list = data(&s.messages_list(&json!({ "experimentId": id }).to_string()));
        assert_eq!(list["messages"].as_array().unwrap().len(), 1);
        assert_eq!(list["messages"][0]["mine"], json!(false));
        assert_eq!(list["messages"][0]["creator"], json!(false));
    }

    #[test]
    fn a_forged_message_is_refused() {
        let scratch = Scratch::new("msgforged");
        let (mut s, id, _store, _courier) = creator_with_delivery(&scratch);
        let bob = Member::new("msgforgedbob");
        let mut m = bob.say(&id, "original", at("2026-10-28T09:00:00Z"));
        m["text"] = json!("altered after signing");
        let out = data(&s.deliver(&arriving(&id, &m, "0xf1"), at("2026-10-28T09:00:01Z")));
        assert_eq!(out["reason"], json!("bad_signature"));
    }

    #[test]
    fn draining_introduces_us_and_seals_shared_fields_for_members() {
        let scratch = Scratch::new("share");
        let (mut s, id, store, courier) = creator_with_delivery(&scratch);
        data(&s.profile_save(&json!({ "fields": {
            "alias": { "value": "Mahboob", "share": true },
            "phone": { "value": "+91 98450 12345", "share": true },
            "email": { "value": "m@example.org", "share": false },
        }}).to_string()));
        let mut bob = Member::new("sharebob");
        let hello = bob.say(&id, "hi", at("2026-10-28T09:00:00Z"));
        courier.arrive(&crate::delivery::topic(&id), "0xh1", &hello);
        let stored_before = store.len();
        let out = data(&s.drain(at("2026-10-28T09:00:05Z")));
        assert_eq!(out["profiles_shared"], json!([id.clone()]), "{out}");
        assert_eq!(store.len(), stored_before, "a profile was written to Logos Storage");

        let card = courier.messages().into_iter().find(|m| m["kind"] == json!("profile")).expect("a profile went out");
        let wire = card.to_string();
        assert!(!wire.contains("98450") && !wire.contains("m@example.org") && !wire.contains("Mahboob"),
                "a profile field went out in the clear: {wire}");
        crate::sign::verify(&card).expect("signed");
        let bob_key = bob.key();
        match bob.social.receive_profile(&id, &card, &bob_key, at("2026-10-28T09:00:06Z")) {
            crate::social::Received::Kept(v) => {
                assert_eq!(v["profile"], json!({ "alias": "Mahboob", "phone": "+91 98450 12345" }));
            }
            other => panic!("bob could not open the profile: {other:?}"),
        }
        // Nothing changed: the next drain sends nothing.
        let quiet = data(&s.drain(at("2026-10-28T09:00:10Z")));
        assert_eq!(quiet["profiles_shared"], json!([]));
    }

    #[test]
    fn a_profile_with_bad_fields_reports_each_one() {
        let scratch = Scratch::new("profbad");
        let mut s = publishing_in(&scratch);
        let e = envelope(&s.profile_save(&json!({ "fields": {
            "age": { "value": "old", "share": false },
            "email": { "value": "x", "share": false },
        }}).to_string()));
        assert_eq!(e["error"]["code"], json!("invalid_values"));
        assert!(e["error"]["problems"]["age"].is_string());
        assert!(e["error"]["problems"]["email"].is_string());
    }

    #[test]
    fn the_profile_survives_a_restart() {
        let scratch = Scratch::new("profkeep");
        let mut s = publishing_in(&scratch);
        data(&s.profile_save(&json!({ "fields": { "location": { "value": "Bengaluru", "share": true } } }).to_string()));
        let mut again = Session::new();
        again.attach_identity(scratch.0.to_str().unwrap()).unwrap();
        let p = data(&again.profile_get());
        assert_eq!(p["fields"]["location"], json!({ "value": "Bengaluru", "share": true }));
    }

    #[test]
    fn a_missing_photo_file_is_reported() {
        let scratch = Scratch::new("profphoto");
        let mut s = publishing_in(&scratch);
        let e = envelope(&s.profile_set_photo(&json!({ "path": "file:///no/such/photo.jpg" }).to_string()));
        assert_eq!(e["error"]["code"], json!("media"));
    }
}

