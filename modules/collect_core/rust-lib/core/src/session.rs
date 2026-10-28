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

use crate::{api, fields, ids, schema};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

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

/// What this node knows.
pub struct Session {
    /// Experiment manifests, newest last.
    experiments: Vec<Value>,
    /// Protocol documents, by id.
    protocols: Vec<Value>,
    /// This node's identity. Fixed in fixtures mode; from the module's
    /// identity key once that is wired.
    creator_key: String,
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
            // The example experiment's creator, so the embedded fixture reads
            // as "created by you" and the creator screens have something to
            // open.
            creator_key: "3a9d0f213a9d0f213a9d0f213a9d0f213a9d0f213a9d0f213a9d0f213a9d0f21".into(),
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
    }

    pub fn creator_key(&self) -> &str {
        &self.creator_key
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
            "link": ids::link(id, "logos:pending"),
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

        api::envelope_ok(json!({
            "experiment": experiment,
            "protocols": held,
            "missing_protocols": named.len() - held.len(),
        }))
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
            // Unsigned until publish: signing is the module identity key's
            // job and publishing needs Storage. A placeholder keeps the shape
            // checkable now.
            "sig": "00".repeat(64),
        });

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
        api::envelope_ok(json!({
            "created": true,
            "problems": [],
            "experiment": experiment,
            "published": false,
            "note": "Saved on this node only. Publishing needs Logos Storage, which is not wired yet.",
        }))
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

        // Point the manifest at it, replacing any earlier entry for this id.
        if let Some(experiment) = self
            .experiments
            .iter_mut()
            .find(|e| e.get("id").and_then(Value::as_str) == Some(experiment_id))
        {
            let entry = json!({
                "id": protocol_id,
                "version": version,
                "cid": "logos:pending",
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

        api::envelope_ok(json!({
            "saved": true,
            "problems": [],
            "protocol": protocol,
            "published": false,
            "note": "Saved on this node only. Publishing needs Logos Storage, which is not wired yet.",
        }))
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

    /// A session with no fixtures, so creation starts from nothing.
    fn empty() -> Session {
        Session {
            experiments: Vec::new(),
            protocols: Vec::new(),
            creator_key: "3a".repeat(32),
        }
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

    #[test]
    fn a_new_session_lists_nothing_without_fixtures() {
        let out = data(&empty().list(at("2026-10-28T08:00:00Z")));
        assert_eq!(out["experiments"], json!([]));
        assert_eq!(out["mine"], json!(0));
    }

    #[test]
    fn create_then_list_shows_the_card() {
        let mut s = empty();
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
        let mut s = empty();
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
        let mut s = empty();
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
        let mut s = empty();
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
        let mut s = empty();
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
        let mut s = empty();
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
        let out = envelope(&empty().get(&json!({ "experimentId": "q7k2m3x4w3b5n6tz" }).to_string()));
        assert_eq!(out["ok"], json!(false));
    }

    #[test]
    fn saving_a_protocol_attaches_it_to_the_manifest() {
        let mut s = empty();
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
        let mut s = empty();
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
        let mut s = empty();
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
        let mut s = empty();
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
        let out = envelope(&empty().save_protocol(
            &json!({ "experimentId": "q7k2m3x4w3b5n6tz", "protocol": { "name": "X" } }).to_string(),
            at("2026-10-28T08:00:00Z"),
        ));
        assert_eq!(out["ok"], json!(false));
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
            creator_key: "3a9d0f213a9d0f213a9d0f213a9d0f213a9d0f213a9d0f213a9d0f213a9d0f21".into(),
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
