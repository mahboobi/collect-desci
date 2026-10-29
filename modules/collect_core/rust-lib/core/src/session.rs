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
            state_dir: None,
            courier: None,
            seen: delivery::Seen::default(),
            receipts: BTreeMap::new(),
            heads: BTreeMap::new(),
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
                "version": submission.pointer("/protocol/version").and_then(Value::as_u64).unwrap_or(0),
                "submitted_at": submitted_at,
                // Empty when the index has no entry yet: received but not
                // indexed is a real state, and blanking it says so.
                "received_at": received,
                "values": submission.get("values").cloned().unwrap_or_else(|| json!({})),
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
        if !api::fixtures_enabled() {
            self.creator_key = public.clone();
        }
        self.identity = Some(identity);
        Ok(public)
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
        });
        merge(&mut data, self.publish_report(wanted));
        api::envelope_ok(data)
    }

    /// `experiments.create` — a new manifest from the C2 form.
    ///
    /// Validated against the contract before it is kept, so a draft that
    /// could never be published cannot enter the list. It is **not** uploaded:
    /// `version` stays 1 and `prev`/`index` null until Storage exists.
    pub fn create(&mut self, raw: &str, now: DateTime<Utc>) -> String {
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
            let outcome: Value = match serde_json::from_str(&self.deliver(&message.to_string(), now)) {
                Ok(v) => v,
                Err(e) => json!({ "ok": false, "error": { "message": e.to_string() } }),
            };
            if outcome["data"]["handled"] == json!(true) {
                handled += 1;
            }
            results.push(outcome["data"].clone());
        }
        api::envelope_ok(json!({
            "received": results.len(),
            "handled": handled,
            "results": results,
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
            schema::Kind::SubmissionAnnouncement => self.receive_submission(&from_topic, &announcement, now),
            schema::Kind::ReceiptAnnouncement => {
                let cid = announcement.get("cid").and_then(Value::as_str).unwrap_or("").to_string();
                self.receipts.insert(cid.clone(), announcement.clone());
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
            state_dir: None,
            courier: None,
            seen: delivery::Seen::default(),
            receipts: BTreeMap::new(),
            heads: BTreeMap::new(),
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
        fn inbox(&self) -> Vec<Value> {
            self.0.inbox()
        }
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
            state_dir: None,
            courier: None,
            seen: delivery::Seen::default(),
            receipts: BTreeMap::new(),
            heads: BTreeMap::new(),
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
            state_dir: None,
            courier: None,
            seen: delivery::Seen::default(),
            receipts: BTreeMap::new(),
            heads: BTreeMap::new(),
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
            state_dir: None,
            courier: None,
            seen: delivery::Seen::default(),
            receipts: BTreeMap::new(),
            heads: BTreeMap::new(),
        };
        let out = data(&s.list(at("2026-10-28T08:00:00Z")));
        let cards = out["experiments"].as_array().unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0]["mine"], json!(true));
        assert_eq!(cards[0]["standing"], json!("open"));
        assert_eq!(cards[0]["protocol_count"], json!(3));
        assert_eq!(cards[0]["open_protocol_count"], json!(2));
    }
}
