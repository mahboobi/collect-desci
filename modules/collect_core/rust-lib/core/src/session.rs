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

use crate::store::{self, DocumentStore};
use crate::{api, fields, identity::Identity, ids, schema};
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

        // Who the creator has hidden, across every page.
        let hidden: Vec<String> = self
            .index_pages
            .iter()
            .filter_map(|p| p.get("hidden_keys").and_then(Value::as_array))
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();

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
    fn publish_doc(&mut self, kind: &str, doc: &Value) -> Option<Publication> {
        let id = doc
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
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

        self.experiments[at] = doc.clone();
        self.publish_doc("experiment", &doc);
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

        let text = |key: &str| request.get(key).and_then(Value::as_str).unwrap_or("").trim().to_string();
        let title = text("title");
        let description = text("description");
        let starts_at = text("startsAt");
        let ends_at = text("endsAt");
        let timezone = {
            let tz = text("timezone");
            if tz.is_empty() { "UTC".to_string() } else { tz }
        };

        // The field-level complaints the C2 form shows inline.
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
        let from = DateTime::parse_from_rfc3339(&starts_at);
        let to = DateTime::parse_from_rfc3339(&ends_at);
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
            return api::envelope_ok(json!({ "created": false, "problems": problems }));
        }

        let id = ids::experiment_id(&[0u8; 32], &ids::new_nonce());
        let experiment = json!({
            "schema": "collect/experiment@1",
            "id": id,
            "version": 1,
            "creator": { "key": self.creator_key, "name": text("creatorName") },
            "title": title,
            "description": description,
            "starts_at": from.unwrap().to_rfc3339(),
            "ends_at": to.unwrap().to_rfc3339(),
            "timezone": timezone,
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
        self.publish_doc("experiment", &experiment);
        let mut data = json!({
            "created": true,
            "problems": [],
            "experiment": experiment,
        });
        merge(&mut data, self.publish_report(&id));
        api::envelope_ok(data)
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
        self.publish_doc("experiment", &doc);
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
        self.publish_doc("protocol", &protocol);
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
        s.publish_doc("experiment", &doc);
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
