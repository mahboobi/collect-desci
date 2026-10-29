//! The JSON boundary (SPEC section 6).
//!
//! Only `String`, `i64`, `u64`, `f64`, `bool`, `Vec<u8>` and `Option<T>` cross
//! the Logos ABI (`docs/api-verification.md` Q1), so a protocol or submission
//! has to travel as a JSON string. That is not a style choice.
//!
//! Every function here is `&str -> String` and pure, which is the point: the
//! module shim in `module.rs` is a name-to-function table with no logic of its
//! own, and all of this is testable without a node.
//!
//! The envelope is `{"ok": true, "data": ...}` or
//! `{"ok": false, "error": {"code", "message"}}`. It is our convention, not
//! the platform's — nothing below inherits it.

use crate::{canonical, fields, geo, ids, indexer, media, schema, sign};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

/// The example documents, embedded.
///
/// SPEC section 6: with `COLLECT_FIXTURES=1` the read methods return
/// `examples/*.json`. They are compiled in rather than read from disk because
/// the Nix build's source root is the module — the same reason the contract is
/// copied in — and because a module that reads the repository at runtime would
/// not work once installed into a profile.
///
/// `examples/` stays the source of truth; `fixtures_match_the_repository`
/// fails if these drift.
mod fixture {
    pub const EXPERIMENT: &str = include_str!("../fixtures/experiment.json");
    pub const PROTOCOL: &str = include_str!("../fixtures/protocol.json");
    pub const SUBMISSION: &str = include_str!("../fixtures/submission.json");
    pub const INDEX: &str = include_str!("../fixtures/index.json");
}

/// Is this build serving fixtures rather than live data?
///
/// Reads `COLLECT_FIXTURES` from the environment, so one binary does both and
/// the UI does not need to know which it is talking to.
pub fn fixtures_enabled() -> bool {
    std::env::var("COLLECT_FIXTURES").map(|v| v == "1").unwrap_or(false)
}

/// Error codes a caller may branch on. Kept short and stable: the UI shows
/// `message`, but anything it *acts* on keys off `code`.
pub mod code {
    pub const BAD_REQUEST: &str = "bad_request";
    pub const INVALID_DOCUMENT: &str = "invalid_document";
    pub const INVALID_PROTOCOL: &str = "invalid_protocol";
    pub const INVALID_VALUES: &str = "invalid_values";
    pub const BAD_SIGNATURE: &str = "bad_signature";
    pub const MEDIA: &str = "media";
    pub const REJECTED: &str = "rejected";
    pub const UNKNOWN_METHOD: &str = "unknown_method";
    /// The call is understood but its transport is not built yet.
    pub const NOT_WIRED: &str = "not_wired";
    /// The caller is not the creator of the thing it is trying to change.
    /// Local only: it stops this node editing a document it cannot sign for,
    /// and is not a security boundary — the signature is.
    pub const NOT_YOURS: &str = "not_yours";
}

/// The success envelope, for callers outside this module (notably `session`).
pub fn envelope_ok(data: Value) -> String {
    ok(data)
}

/// The failure envelope, for callers outside this module.
pub fn envelope_err(code: &str, message: impl AsRef<str>) -> String {
    err(code, message)
}

/// The embedded example experiment, for `session`'s fixtures seeding.
pub fn fixture_experiment() -> &'static str {
    fixture::EXPERIMENT
}

/// The embedded example protocol.
pub fn fixture_protocol() -> &'static str {
    fixture::PROTOCOL
}

/// The embedded example submission.
pub fn fixture_submission() -> &'static str {
    fixture::SUBMISSION
}

/// The embedded example index page.
pub fn fixture_index() -> &'static str {
    fixture::INDEX
}

fn ok(data: Value) -> String {
    json!({ "ok": true, "data": data }).to_string()
}

fn err(code: &str, message: impl AsRef<str>) -> String {
    json!({
        "ok": false,
        "error": { "code": code, "message": message.as_ref() }
    })
    .to_string()
}

/// Parse a request body, or produce the error envelope for it.
fn body(raw: &str) -> Result<Value, String> {
    serde_json::from_str(raw).map_err(|e| err(code::BAD_REQUEST, format!("not JSON: {e}")))
}

fn field_str<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| err(code::BAD_REQUEST, format!("`{key}` is required and must be a string")))
}

fn field_obj<'a>(v: &'a Value, key: &str) -> Result<&'a Value, String> {
    v.get(key)
        .filter(|f| f.is_object())
        .ok_or_else(|| err(code::BAD_REQUEST, format!("`{key}` is required and must be an object")))
}

/// Timestamp from the request, or the real clock.
///
/// The caller passes `now` so the creator can re-judge a submission at the
/// time it actually arrived rather than at parse time, and so tests are not
/// bound to the wall clock.
fn now_from(v: &Value) -> Result<DateTime<Utc>, String> {
    match v.get("now").and_then(Value::as_str) {
        None => Ok(Utc::now()),
        Some(raw) => DateTime::parse_from_rfc3339(raw)
            .map(|t| t.with_timezone(&Utc))
            .map_err(|_| err(code::BAD_REQUEST, "`now` must be an RFC 3339 timestamp")),
    }
}

// ---------------------------------------------------------------- documents

/// `documents.canonicalize` — the exact bytes a signature will cover.
///
/// Exposed because it is the one thing a caller cannot reproduce for itself:
/// if two builds disagree here, every signature disagrees.
pub fn documents_canonicalize(raw: &str) -> String {
    let request = match body(raw) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let doc = match field_obj(&request, "document") {
        Ok(d) => d,
        Err(e) => return e,
    };
    match sign::signing_payload(doc) {
        Ok(bytes) => ok(json!({
            "canonical": String::from_utf8_lossy(&bytes),
            "bytes": bytes.len(),
        })),
        Err(e) => err(code::INVALID_DOCUMENT, e.to_string()),
    }
}

/// `documents.validate` — check a document against the contract.
pub fn documents_validate(raw: &str) -> String {
    let request = match body(raw) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let doc = match field_obj(&request, "document") {
        Ok(d) => d,
        Err(e) => return e,
    };
    let errors = schema::errors(doc);
    if errors.is_empty() {
        let kind = schema::kind(doc).map(|k| k.label()).unwrap_or("unknown");
        ok(json!({ "valid": true, "kind": kind }))
    } else {
        // Every error, not just the first: an invalid document is dropped and
        // never retried, so a partial reason is a dead end.
        ok(json!({ "valid": false, "errors": errors }))
    }
}

/// `documents.verify` — signature plus contract, in that order.
pub fn documents_verify(raw: &str) -> String {
    let request = match body(raw) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let doc = match field_obj(&request, "document") {
        Ok(d) => d,
        Err(e) => return e,
    };

    let errors = schema::errors(doc);
    if !errors.is_empty() {
        return ok(json!({ "verified": false, "reason": "invalid_document", "errors": errors }));
    }

    // A protocol or index page carries no author key, so the caller supplies
    // the creator key from the manifest. Refusing to guess is the point.
    let outcome = match request.get("key").and_then(Value::as_str) {
        Some(hex_key) => match decode_key(hex_key) {
            Some(key) => sign::verify_with(doc, &key).map(|()| hex_key.to_string()),
            None => return err(code::BAD_REQUEST, "`key` must be 64 lower-case hex characters"),
        },
        None => sign::verify(doc).map(hex::encode),
    };

    match outcome {
        Ok(signer) => ok(json!({ "verified": true, "signer": signer })),
        Err(e) => ok(json!({ "verified": false, "reason": "bad_signature", "detail": e.to_string() })),
    }
}

fn decode_key(s: &str) -> Option<[u8; 32]> {
    let mut out = [0u8; 32];
    if s.len() != 64 {
        return None;
    }
    hex::decode_to_slice(s, &mut out).ok()?;
    Some(out)
}

// ---------------------------------------------------------------- protocols

/// `protocols.validateDraft` — SPEC section 6, returning `{errors, changes}`.
pub fn protocols_validate_draft(raw: &str) -> String {
    let request = match body(raw) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let draft = match field_obj(&request, "protocol") {
        Ok(d) => d,
        Err(e) => return e,
    };

    let mut errors: Vec<String> = schema::errors_as(draft, schema::Kind::Protocol);

    // Duplicate field keys and duplicate option values are things the
    // contract cannot express, so they are checked here or nowhere.
    match fields::parse_fields(draft) {
        Ok(parsed) => {
            let changes = match request.get("previous").filter(|p| p.is_object()) {
                Some(previous) => protocol_changes(previous, &parsed),
                None => Vec::new(),
            };
            if errors.is_empty() {
                return ok(json!({ "errors": [], "changes": changes, "fields": parsed.len() }));
            }
        }
        Err(e) => errors.push(e.to_string()),
    }

    ok(json!({ "errors": errors, "changes": [] }))
}

/// What changed against the published version, and whether it is allowed.
///
/// SPEC section 4: keys and option values may not change once published,
/// labels may. A new version is created when anything changed.
fn protocol_changes(previous: &Value, next: &[fields::Field]) -> Vec<Value> {
    let Ok(old) = fields::parse_fields(previous) else {
        return vec![json!({
            "change": "unreadable_previous",
            "breaking": false,
            "detail": "the published version could not be parsed; treating every field as new"
        })];
    };

    let mut out = Vec::new();

    for field in next {
        match old.iter().find(|o| o.key == field.key) {
            None => out.push(json!({ "change": "field_added", "key": field.key, "breaking": false })),
            Some(before) => {
                if before.ty != field.ty {
                    out.push(json!({
                        "change": "type_changed", "key": field.key, "breaking": true,
                        "from": before.ty.as_str(), "to": field.ty.as_str()
                    }));
                }
                if before.label != field.label {
                    out.push(json!({ "change": "label_changed", "key": field.key, "breaking": false }));
                }
                if before.required != field.required {
                    // Not breaking for documents already submitted, but it
                    // changes what the next ones must carry.
                    out.push(json!({
                        "change": "required_changed", "key": field.key, "breaking": false,
                        "to": field.required
                    }));
                }
                for removed in removed_options(before, field) {
                    out.push(json!({
                        "change": "option_removed", "key": field.key,
                        "value": removed, "breaking": true
                    }));
                }
            }
        }
    }

    for before in &old {
        if !next.iter().any(|f| f.key == before.key) {
            // Earlier submissions carry this key, so dropping it orphans
            // their values.
            out.push(json!({ "change": "field_removed", "key": before.key, "breaking": true }));
        }
    }

    out
}

fn removed_options(before: &fields::Field, after: &fields::Field) -> Vec<String> {
    fn options(f: &fields::Field) -> Vec<String> {
        match &f.config {
            fields::Config::Choice { options } | fields::Config::Multi { options, .. } => {
                options.iter().map(|o| o.value.clone()).collect()
            }
            _ => Vec::new(),
        }
    }
    let now = options(after);
    options(before)
        .into_iter()
        .filter(|v| !now.contains(v))
        .collect()
}

// -------------------------------------------------------------- submissions

/// `submissions.validate` — SPEC section 6, returning `{missing, errors}`.
pub fn submissions_validate(raw: &str) -> String {
    let request = match body(raw) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let protocol = match field_obj(&request, "protocol") {
        Ok(p) => p,
        Err(e) => return e,
    };
    let values = match request.get("values") {
        Some(v) if v.is_object() => v,
        _ => return err(code::BAD_REQUEST, "`values` is required and must be an object"),
    };
    let now = match now_from(&request) {
        Ok(n) => n,
        Err(e) => return e,
    };

    let parsed = match fields::parse_fields(protocol) {
        Ok(p) => p,
        Err(e) => return err(code::INVALID_PROTOCOL, e.to_string()),
    };

    let report = fields::validate_values(&parsed, values, now);
    ok(json!({
        "ok": report.is_ok(),
        "missing": report.missing,
        "errors": report.errors,
    }))
}

/// `submissions.prepareValues` — apply the store rules before anything is
/// signed.
///
/// `CLAUDE.md`: rounding happens before anything is stored. Running it here,
/// on the way into a document, is what makes that true — a caller that skips
/// this gets caught by `submissions.validate`, but by then the exact point has
/// already been in a document.
pub fn submissions_prepare_values(raw: &str) -> String {
    let request = match body(raw) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let protocol = match field_obj(&request, "protocol") {
        Ok(p) => p,
        Err(e) => return e,
    };
    let values = match request.get("values") {
        Some(v) if v.is_object() => v.clone(),
        _ => return err(code::BAD_REQUEST, "`values` is required and must be an object"),
    };

    let parsed = match fields::parse_fields(protocol) {
        Ok(p) => p,
        Err(e) => return err(code::INVALID_PROTOCOL, e.to_string()),
    };

    let mut out = values;
    for field in &parsed {
        if let Some(value) = out.get_mut(&field.key) {
            fields::apply_store_rule(field, value);
        }
    }
    ok(json!({ "values": out }))
}

// -------------------------------------------------------------------- media

/// `submissions.processMedia` — read the metadata, then produce the cleaned
/// bytes to upload.
///
/// Takes and returns base64 rather than a path: the caller may be holding
/// bytes it got from a file dialog, and this keeps the function pure.
pub fn submissions_process_media(raw: &str) -> String {
    use data_encoding::BASE64;

    let request = match body(raw) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let encoded = match field_str(&request, "bytes") {
        Ok(b) => b,
        Err(e) => return e,
    };
    let max_mb = request
        .get("max_mb_each")
        .and_then(Value::as_f64)
        .unwrap_or(fields::PHOTO_DEFAULT_MAX_MB);

    let original = match BASE64.decode(encoded.as_bytes()) {
        Ok(b) => b,
        Err(e) => return err(code::BAD_REQUEST, format!("`bytes` is not base64: {e}")),
    };

    match media::process(&original, max_mb) {
        Ok(cleaned) => {
            let (location, datetime) = media::prefills(&cleaned.exif);
            ok(json!({
                // The cleaned file, which is the only thing that may be
                // uploaded.
                "bytes": BASE64.encode(&cleaned.bytes),
                "sha256": cleaned.sha256,
                "mime": cleaned.mime,
                "width": cleaned.width,
                "height": cleaned.height,
                "reencoded": cleaned.reencoded,
                // What section 5 step 3 prefills from.
                "prefills": { "location": location, "datetime": datetime },
                "exif_datetime_naive": cleaned.exif.datetime_naive,
            }))
        }
        Err(e) => err(code::MEDIA, e.to_string()),
    }
}

// ------------------------------------------------------------------ indexer

/// `index.accept` — the creator's verdict on a received submission.
pub fn index_accept(raw: &str) -> String {
    let request = match body(raw) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let submission = match field_obj(&request, "submission") {
        Ok(s) => s,
        Err(e) => return e,
    };
    let protocol_doc = match field_obj(&request, "protocol") {
        Ok(p) => p,
        Err(e) => return e,
    };
    let experiment = match field_obj(&request, "experiment") {
        Ok(e) => e,
        Err(e) => return e,
    };
    let received_at = match now_from(&request) {
        Ok(n) => n,
        Err(e) => return e,
    };

    // The contract first: an invalid document is never indexed.
    let errors = schema::errors_as(submission, schema::Kind::Submission);
    if !errors.is_empty() {
        return ok(json!({
            "accepted": false,
            "reason": "invalid_document",
            "detail": errors.join("; ")
        }));
    }

    // Then the signature, which is what ties the values to a submitter.
    if let Err(e) = sign::verify(submission) {
        return ok(json!({ "accepted": false, "reason": "bad_signature", "detail": e.to_string() }));
    }

    let window = match build_window(experiment) {
        Ok(w) => w,
        Err(e) => return e,
    };
    let protocol = match build_protocol(experiment, protocol_doc) {
        Ok(p) => p,
        Err(e) => return e,
    };

    let pages: Vec<Value> = request
        .get("pages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut state = indexer::IndexState::from_pages(&pages);
    if let Some(limit) = request.get("daily_limit").and_then(Value::as_u64) {
        state.daily_limit = usize::try_from(limit).ok();
    }

    match indexer::accept(submission, &window, &protocol, &state, received_at) {
        Ok(()) => ok(json!({ "accepted": true })),
        Err(reject) => ok(json!({
            "accepted": false,
            "reason": reject_code(&reject),
            "detail": reject.to_string()
        })),
    }
}

/// A stable short code per rejection, so a caller can count them without
/// parsing prose.
fn reject_code(reject: &indexer::Reject) -> &'static str {
    match reject {
        indexer::Reject::HiddenSubmitter => "hidden_submitter",
        indexer::Reject::BeforeStart { .. } => "before_start",
        indexer::Reject::AfterEnd { .. } => "after_end",
        indexer::Reject::SubmittedOutsideWindow { .. } => "submitted_outside_window",
        indexer::Reject::ProtocolNotOpen { .. } => "protocol_not_open",
        indexer::Reject::ProtocolVersionMismatch { .. } => "protocol_version_mismatch",
        indexer::Reject::RateLimited { .. } => "rate_limited",
        indexer::Reject::AlreadySubmitted => "already_submitted",
        indexer::Reject::Fields(_) => "invalid_values",
        indexer::Reject::WrongExperiment => "wrong_experiment",
    }
}

fn build_window(experiment: &Value) -> Result<indexer::Window, String> {
    let id = field_str(experiment, "id")?;
    let starts = field_str(experiment, "starts_at")?;
    let ends = field_str(experiment, "ends_at")?;
    let parse = |raw: &str, which: &str| {
        DateTime::parse_from_rfc3339(raw)
            .map(|t| t.with_timezone(&Utc))
            .map_err(|_| err(code::BAD_REQUEST, format!("`{which}` must be RFC 3339")))
    };
    Ok(indexer::Window {
        experiment_id: id.to_string(),
        starts_at: parse(starts, "starts_at")?,
        ends_at: parse(ends, "ends_at")?,
    })
}

fn build_protocol(experiment: &Value, protocol: &Value) -> Result<indexer::OpenProtocol, String> {
    let id = field_str(protocol, "id")?;
    let version = protocol
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| err(code::BAD_REQUEST, "protocol `version` is required"))?;

    // `status` lives on the manifest's entry, not on the protocol document —
    // that is how a creator closes a form without republishing it.
    let open = experiment
        .get("protocols")
        .and_then(Value::as_array)
        .and_then(|list| {
            list.iter().find(|p| {
                p.get("id").and_then(Value::as_str) == Some(id)
                    && p.get("version").and_then(Value::as_u64) == Some(version)
            })
        })
        .and_then(|p| p.get("status").and_then(Value::as_str))
        .map(|s| s == "open")
        .unwrap_or(false);

    let parsed = fields::parse_fields(protocol)
        .map_err(|e| err(code::INVALID_PROTOCOL, e.to_string()))?;

    Ok(indexer::OpenProtocol {
        id: id.to_string(),
        version,
        open,
        allow_multiple: protocol
            .get("allow_multiple")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        allow_drafts: protocol
            .get("allow_drafts")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        fields: parsed,
    })
}

/// `index.page` — lay accepted entries out across index pages.
pub fn index_page(raw: &str) -> String {
    let request = match body(raw) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let experiment_id = match field_str(&request, "experiment") {
        Ok(i) => i,
        Err(e) => return e,
    };
    let created_at = match now_from(&request) {
        Ok(n) => n,
        Err(e) => return e,
    };

    let mut entries = Vec::new();
    for raw_entry in request
        .get("entries")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
    {
        let get = |k: &str| raw_entry.get(k).and_then(Value::as_str).unwrap_or_default();
        entries.push(indexer::Entry {
            cid: get("cid").to_string(),
            protocol: get("protocol").to_string(),
            version: raw_entry.get("version").and_then(Value::as_u64).unwrap_or(1),
            submitter: get("submitter").to_string(),
            submitted_at: get("submitted_at").to_string(),
            received_at: get("received_at").to_string(),
        });
    }

    let hidden: Vec<String> = request
        .get("hidden_keys")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let pages = indexer::page_entries(
        experiment_id,
        request.get("current").filter(|c| c.is_object()),
        request.get("current_cid").and_then(Value::as_str),
        &entries,
        &hidden,
        created_at,
    );
    ok(json!({ "pages": pages }))
}

// ------------------------------------------------------------------ helpers

/// `geo.round` — the cell centre for a coordinate.
pub fn geo_round(raw: &str) -> String {
    let request = match body(raw) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let (Some(lat), Some(lon)) = (
        request.get("lat").and_then(Value::as_f64),
        request.get("lon").and_then(Value::as_f64),
    ) else {
        return err(code::BAD_REQUEST, "`lat` and `lon` are required numbers");
    };
    let (rlat, rlon) = geo::round_1km(lat, lon);
    ok(json!({ "lat": rlat, "lon": rlon, "rounded": true }))
}

/// `experiments.parseLink` — pull the id and manifest CID out of a link.
pub fn experiments_parse_link(raw: &str) -> String {
    let request = match body(raw) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let link = match field_str(&request, "link") {
        Ok(l) => l,
        Err(e) => return e,
    };
    match ids::parse_link(link) {
        Some((id, manifest)) => ok(json!({
            "experiment": id,
            "manifest": manifest,
            "topic": ids::topic(&id),
        })),
        None => err(code::BAD_REQUEST, "not a Collect link"),
    }
}

/// `experiments.open` — the view model S1 needs for a link.
///
/// In fixtures mode this answers from the embedded documents and ignores the
/// link. Live, it would fetch the manifest from the CID in the link and then
/// each protocol it names; the shape returned here is what that will produce,
/// so the UI does not change when the fetch becomes real.
pub fn experiments_open(_raw: &str) -> String {
    if !fixtures_enabled() {
        return err(
            code::NOT_WIRED,
            "opening a link needs Logos Storage, which is not wired yet (SPEC Q9/Q12). \
             Set COLLECT_FIXTURES=1 to work against the example documents.",
        );
    }

    let experiment: Value = match serde_json::from_str(fixture::EXPERIMENT) {
        Ok(v) => v,
        Err(e) => return err(code::INVALID_DOCUMENT, format!("embedded experiment: {e}")),
    };
    let protocol: Value = match serde_json::from_str(fixture::PROTOCOL) {
        Ok(v) => v,
        Err(e) => return err(code::INVALID_PROTOCOL, format!("embedded protocol: {e}")),
    };

    // Validate on the way out, not just on the way in. A fixture that drifted
    // from the contract would otherwise reach the form renderer.
    for (label, doc, kind) in [
        ("experiment", &experiment, schema::Kind::Experiment),
        ("protocol", &protocol, schema::Kind::Protocol),
    ] {
        let errors = schema::errors_as(doc, kind);
        if !errors.is_empty() {
            return err(
                code::INVALID_DOCUMENT,
                format!("embedded {label} does not validate: {}", errors.join("; ")),
            );
        }
    }

    ok(json!({
        "experiment": experiment,
        // Only the protocols we actually hold a document for. The example
        // manifest names three; one fixture exists.
        "protocols": [protocol],
        "fixtures": true,
    }))
}

/// `protocols.get` — one protocol document by id.
pub fn protocols_get(raw: &str) -> String {
    let request = match body(raw) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let wanted = match field_str(&request, "protocolId") {
        Ok(i) => i,
        Err(e) => return e,
    };
    if !fixtures_enabled() {
        return err(code::NOT_WIRED, "fetching a protocol needs Logos Storage, which is not wired yet");
    }
    let protocol: Value = match serde_json::from_str(fixture::PROTOCOL) {
        Ok(v) => v,
        Err(e) => return err(code::INVALID_PROTOCOL, format!("embedded protocol: {e}")),
    };
    if protocol.get("id").and_then(Value::as_str) != Some(wanted) {
        return err(code::BAD_REQUEST, format!("no protocol `{wanted}` in this build"));
    }
    ok(json!({ "protocol": protocol }))
}

/// `session.capabilities` — what this build can actually do.
///
/// Honest rather than aspirational: the Storage and Delivery methods are not
/// wired yet (SPEC Q9, Q12), and a caller is better off being told than
/// discovering it at the first upload.
pub fn session_capabilities(_raw: &str) -> String {
    ok(json!({
        "documents": true,
        "protocols": true,
        "submissions": true,
        "media": true,
        "indexer": true,
        "storage": false,
        "delivery": false,
        "fixtures": fixtures_enabled(),
        "heic": false,
        "schema": "collect.schema.json",
        "app": concat!("collect/", env!("CARGO_PKG_VERSION")),
    }))
}

/// Dispatch by method name. The module shim is this table and nothing else.
pub fn dispatch(method: &str, raw: &str) -> String {
    match method {
        "documents.canonicalize" => documents_canonicalize(raw),
        "documents.validate" => documents_validate(raw),
        "documents.verify" => documents_verify(raw),
        "protocols.validateDraft" => protocols_validate_draft(raw),
        "submissions.validate" => submissions_validate(raw),
        "submissions.prepareValues" => submissions_prepare_values(raw),
        "submissions.processMedia" => submissions_process_media(raw),
        "index.accept" => index_accept(raw),
        "index.page" => index_page(raw),
        "geo.round" => geo_round(raw),
        "experiments.parseLink" => experiments_parse_link(raw),
        "experiments.open" => experiments_open(raw),
        "protocols.get" => protocols_get(raw),
        "session.capabilities" => session_capabilities(raw),
        other => err(code::UNKNOWN_METHOD, format!("no method `{other}`")),
    }
}

/// Canonical JSON of any value, for callers that need the signing bytes of
/// something that is not a whole document.
pub fn canonicalize_value(value: &Value) -> Result<String, canonical::Error> {
    canonical::to_string(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPERIMENT: &str = include_str!("../../../../../examples/experiment.json");
    const PROTOCOL: &str = include_str!("../../../../../examples/protocol.json");
    const SUBMISSION: &str = include_str!("../../../../../examples/submission.json");
    const INDEX: &str = include_str!("../../../../../examples/index.json");

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    /// Call a method and return the parsed envelope.
    fn call(method: &str, request: Value) -> Value {
        parse(&dispatch(method, &request.to_string()))
    }

    /// Call and unwrap `data`, failing loudly on an error envelope.
    fn data(method: &str, request: Value) -> Value {
        let out = call(method, request);
        assert_eq!(out["ok"], json!(true), "{method} failed: {out}");
        out["data"].clone()
    }

    fn error_code(method: &str, request: Value) -> String {
        let out = call(method, request);
        assert_eq!(out["ok"], json!(false), "{method} unexpectedly succeeded: {out}");
        out["error"]["code"].as_str().unwrap().to_string()
    }

    // ---- the envelope ----

    #[test]
    fn every_response_is_a_well_formed_envelope() {
        // Including the failures: a caller that cannot parse the error has no
        // way to report it.
        for (method, request) in [
            ("session.capabilities", json!({})),
            ("documents.validate", json!({ "document": parse(SUBMISSION) })),
            ("documents.validate", json!({})),
            ("no.such.method", json!({})),
        ] {
            let out = parse(&dispatch(method, &request.to_string()));
            assert!(out["ok"].is_boolean(), "{method}: {out}");
            if out["ok"] == json!(false) {
                assert!(out["error"]["code"].is_string(), "{method}: {out}");
                assert!(out["error"]["message"].is_string(), "{method}: {out}");
            } else {
                assert!(out.get("data").is_some(), "{method}: {out}");
            }
        }
    }

    #[test]
    fn an_unknown_method_is_reported_not_ignored() {
        assert_eq!(error_code("experiments.teleport", json!({})), code::UNKNOWN_METHOD);
    }

    #[test]
    fn malformed_json_is_reported_as_bad_request() {
        let out = parse(&dispatch("documents.validate", "{not json"));
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"]["code"], json!(code::BAD_REQUEST));
    }

    #[test]
    fn a_missing_argument_names_itself() {
        let out = call("submissions.validate", json!({ "protocol": parse(PROTOCOL) }));
        assert_eq!(out["ok"], json!(false));
        assert!(
            out["error"]["message"].as_str().unwrap().contains("values"),
            "{out}"
        );
    }

    // ---- documents ----

    #[test]
    fn canonicalize_excludes_sig_and_is_stable() {
        let a = data("documents.canonicalize", json!({ "document": parse(SUBMISSION) }));
        let mut without = parse(SUBMISSION);
        without.as_object_mut().unwrap().remove("sig");
        let b = data("documents.canonicalize", json!({ "document": without }));
        assert_eq!(a["canonical"], b["canonical"]);
        assert_eq!(a["bytes"], b["bytes"]);
    }

    #[test]
    fn validate_accepts_every_example() {
        for src in [EXPERIMENT, PROTOCOL, SUBMISSION, INDEX] {
            let out = data("documents.validate", json!({ "document": parse(src) }));
            assert_eq!(out["valid"], json!(true), "{out}");
        }
    }

    #[test]
    fn validate_reports_the_kind_it_recognised() {
        let out = data("documents.validate", json!({ "document": parse(SUBMISSION) }));
        assert_eq!(out["kind"], json!("collect/submission@1"));
    }

    #[test]
    fn validate_lists_errors_rather_than_just_failing() {
        let mut bad = parse(SUBMISSION);
        bad["submitted_at"] = json!("nope");
        let out = data("documents.validate", json!({ "document": bad }));
        assert_eq!(out["valid"], json!(false));
        assert!(!out["errors"].as_array().unwrap().is_empty(), "{out}");
    }

    #[test]
    fn verify_round_trips_a_signed_document() {
        use ed25519_dalek::SigningKey;
        let key = SigningKey::from_bytes(&[3u8; 32]);
        let mut doc = parse(SUBMISSION);
        doc["submitter"]["key"] = json!(hex::encode(key.verifying_key().to_bytes()));
        let signed = sign::sign(&doc, &key).unwrap();

        let out = data("documents.verify", json!({ "document": signed }));
        assert_eq!(out["verified"], json!(true), "{out}");
        assert_eq!(
            out["signer"],
            json!(hex::encode(key.verifying_key().to_bytes()))
        );
    }

    #[test]
    fn verify_rejects_a_tampered_document() {
        use ed25519_dalek::SigningKey;
        let key = SigningKey::from_bytes(&[3u8; 32]);
        let mut doc = parse(SUBMISSION);
        doc["submitter"]["key"] = json!(hex::encode(key.verifying_key().to_bytes()));
        let mut signed = sign::sign(&doc, &key).unwrap();
        signed["values"]["count"] = json!(99);

        let out = data("documents.verify", json!({ "document": signed }));
        assert_eq!(out["verified"], json!(false));
        assert_eq!(out["reason"], json!("bad_signature"));
    }

    #[test]
    fn verify_checks_the_contract_before_the_signature() {
        // A document that does not validate is reported as invalid, not as a
        // signature problem — the two need different fixes.
        let mut bad = parse(SUBMISSION);
        bad["experiment"] = json!("NOT-BASE32");
        let out = data("documents.verify", json!({ "document": bad }));
        assert_eq!(out["verified"], json!(false));
        assert_eq!(out["reason"], json!("invalid_document"));
    }

    #[test]
    fn verify_takes_an_explicit_key_for_a_protocol() {
        use ed25519_dalek::SigningKey;
        let key = SigningKey::from_bytes(&[9u8; 32]);
        let signed = sign::sign(&parse(PROTOCOL), &key).unwrap();
        let hex_key = hex::encode(key.verifying_key().to_bytes());

        let out = data(
            "documents.verify",
            json!({ "document": signed, "key": hex_key }),
        );
        assert_eq!(out["verified"], json!(true), "{out}");
    }

    #[test]
    fn verify_refuses_to_guess_a_protocols_signer() {
        // No key supplied, and a protocol carries none of its own.
        let out = data("documents.verify", json!({ "document": parse(PROTOCOL) }));
        assert_eq!(out["verified"], json!(false));
        assert_eq!(out["reason"], json!("bad_signature"));
    }

    #[test]
    fn verify_rejects_a_malformed_key_argument() {
        assert_eq!(
            error_code(
                "documents.verify",
                json!({ "document": parse(PROTOCOL), "key": "abcd" })
            ),
            code::BAD_REQUEST
        );
    }

    // ---- protocols ----

    #[test]
    fn validate_draft_accepts_the_example() {
        let out = data("protocols.validateDraft", json!({ "protocol": parse(PROTOCOL) }));
        assert_eq!(out["errors"], json!([]), "{out}");
        assert_eq!(out["fields"], json!(8));
    }

    #[test]
    fn validate_draft_catches_a_duplicate_key_the_contract_cannot() {
        // Two fields with one key makes `values` ambiguous, and JSON Schema
        // has no way to express it.
        let mut p = parse(PROTOCOL);
        p["fields"][2]["key"] = json!("photos");
        assert!(schema::is_valid(&p), "the contract should still accept this");

        let out = data("protocols.validateDraft", json!({ "protocol": p }));
        let errors = out["errors"].as_array().unwrap();
        assert!(
            errors.iter().any(|e| e.as_str().unwrap().contains("duplicate")),
            "{out}"
        );
    }

    #[test]
    fn validate_draft_reports_a_removed_field_as_breaking() {
        let previous = parse(PROTOCOL);
        let mut next = parse(PROTOCOL);
        next["fields"].as_array_mut().unwrap().remove(2); // `species`

        let out = data(
            "protocols.validateDraft",
            json!({ "protocol": next, "previous": previous }),
        );
        let changes = out["changes"].as_array().unwrap();
        let removed = changes
            .iter()
            .find(|c| c["change"] == json!("field_removed"))
            .expect(&format!("no field_removed in {out}"));
        assert_eq!(removed["key"], json!("species"));
        assert_eq!(removed["breaking"], json!(true));
    }

    #[test]
    fn validate_draft_reports_a_changed_type_as_breaking() {
        let previous = parse(PROTOCOL);
        let mut next = parse(PROTOCOL);
        next["fields"][2]["type"] = json!("longtext");

        let out = data(
            "protocols.validateDraft",
            json!({ "protocol": next, "previous": previous }),
        );
        let change = out["changes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["change"] == json!("type_changed"))
            .expect(&format!("no type_changed in {out}"))
            .clone();
        assert_eq!(change["breaking"], json!(true));
        assert_eq!(change["from"], json!("text"));
        assert_eq!(change["to"], json!("longtext"));
    }

    #[test]
    fn validate_draft_reports_a_label_change_as_safe() {
        // SPEC section 4: labels may change once published, keys may not.
        let previous = parse(PROTOCOL);
        let mut next = parse(PROTOCOL);
        next["fields"][2]["label"] = json!("Species, if known");

        let out = data(
            "protocols.validateDraft",
            json!({ "protocol": next, "previous": previous }),
        );
        let change = out["changes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["change"] == json!("label_changed"))
            .expect(&format!("no label_changed in {out}"))
            .clone();
        assert_eq!(change["breaking"], json!(false));
    }

    #[test]
    fn validate_draft_reports_a_removed_option_as_breaking() {
        // Earlier submissions may carry the dropped value.
        let previous = parse(PROTOCOL);
        let mut next = parse(PROTOCOL);
        next["fields"][1]["config"]["options"]
            .as_array_mut()
            .unwrap()
            .remove(0); // `birds`

        let out = data(
            "protocols.validateDraft",
            json!({ "protocol": next, "previous": previous }),
        );
        let change = out["changes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["change"] == json!("option_removed"))
            .expect(&format!("no option_removed in {out}"))
            .clone();
        assert_eq!(change["value"], json!("birds"));
        assert_eq!(change["breaking"], json!(true));
    }

    #[test]
    fn validate_draft_reports_no_changes_against_an_identical_previous() {
        let out = data(
            "protocols.validateDraft",
            json!({ "protocol": parse(PROTOCOL), "previous": parse(PROTOCOL) }),
        );
        assert_eq!(out["changes"], json!([]), "{out}");
    }

    // ---- submissions ----

    #[test]
    fn submissions_validate_accepts_the_example() {
        let out = data(
            "submissions.validate",
            json!({
                "protocol": parse(PROTOCOL),
                "values": parse(SUBMISSION)["values"],
                "now": "2026-10-28T12:00:00Z"
            }),
        );
        assert_eq!(out["ok"], json!(true), "{out}");
        assert_eq!(out["missing"], json!([]));
    }

    #[test]
    fn submissions_validate_separates_missing_from_wrong() {
        let mut values = parse(SUBMISSION)["values"].clone();
        values.as_object_mut().unwrap().remove("count");
        values["category"] = json!("not-an-option");

        let out = data(
            "submissions.validate",
            json!({
                "protocol": parse(PROTOCOL),
                "values": values,
                "now": "2026-10-28T12:00:00Z"
            }),
        );
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["missing"], json!(["count"]));
        assert!(out["errors"]["category"].is_string(), "{out}");
    }

    #[test]
    fn prepare_values_rounds_before_anything_is_signed() {
        let mut protocol = parse(PROTOCOL);
        protocol["fields"][4]["config"]["store"] = json!("rounded_1km");

        let out = data(
            "submissions.prepareValues",
            json!({ "protocol": protocol, "values": parse(SUBMISSION)["values"] }),
        );
        let location = &out["values"]["location"];
        assert_eq!(location["lat"], json!(12.975));
        assert_eq!(location["lon"], json!(77.595));
        assert_eq!(location["rounded"], json!(true));
    }

    #[test]
    fn prepare_values_then_validate_is_accepted_when_validate_alone_was_not() {
        // The order the spec requires: round, then sign. An exact point under
        // a rounded_1km protocol is rejected, and preparing fixes it.
        let mut protocol = parse(PROTOCOL);
        protocol["fields"][4]["config"]["store"] = json!("rounded_1km");
        let values = parse(SUBMISSION)["values"].clone();

        let before = data(
            "submissions.validate",
            json!({ "protocol": protocol, "values": values, "now": "2026-10-28T12:00:00Z" }),
        );
        assert_eq!(before["ok"], json!(false), "an exact point was accepted");

        let prepared = data(
            "submissions.prepareValues",
            json!({ "protocol": protocol, "values": values }),
        );
        let after = data(
            "submissions.validate",
            json!({
                "protocol": protocol,
                "values": prepared["values"],
                "now": "2026-10-28T12:00:00Z"
            }),
        );
        assert_eq!(after["ok"], json!(true), "{after}");
    }

    #[test]
    fn prepare_values_leaves_an_exact_field_alone() {
        let out = data(
            "submissions.prepareValues",
            json!({ "protocol": parse(PROTOCOL), "values": parse(SUBMISSION)["values"] }),
        );
        assert_eq!(out["values"]["location"]["lat"], json!(12.97712));
    }

    // ---- indexer ----

    fn accept_request(received_at: &str) -> Value {
        json!({
            "submission": parse(SUBMISSION),
            "protocol": parse(PROTOCOL),
            "experiment": parse(EXPERIMENT),
            "now": received_at
        })
    }

    #[test]
    fn index_accept_rejects_the_example_for_its_placeholder_signature() {
        // examples/submission.json carries a dummy `sig`, so this is the
        // honest outcome and a useful check that the signature gate runs.
        let out = data("index.accept", accept_request("2026-10-28T08:00:00Z"));
        assert_eq!(out["accepted"], json!(false));
        assert_eq!(out["reason"], json!("bad_signature"));
    }

    /// The example submission, re-signed so it can get past the signature gate.
    fn signed_submission() -> (Value, Value) {
        use ed25519_dalek::SigningKey;
        let key = SigningKey::from_bytes(&[5u8; 32]);
        let mut doc = parse(SUBMISSION);
        doc["submitter"]["key"] = json!(hex::encode(key.verifying_key().to_bytes()));
        let signed = sign::sign(&doc, &key).unwrap();
        (signed, json!(hex::encode(key.verifying_key().to_bytes())))
    }

    #[test]
    fn index_accept_takes_a_good_submission() {
        let (submission, _) = signed_submission();
        let out = data(
            "index.accept",
            json!({
                "submission": submission,
                "protocol": parse(PROTOCOL),
                "experiment": parse(EXPERIMENT),
                "now": "2026-10-28T08:00:00Z"
            }),
        );
        assert_eq!(out["accepted"], json!(true), "{out}");
    }

    #[test]
    fn index_accept_reports_a_stable_reason_code() {
        // The example experiment opens 2026-10-12, so this arrival is a clear
        // 11 days early. Picking a date without reading starts_at is how the
        // first version of this test ended up asserting the wrong reason.
        let (submission, _) = signed_submission();
        let out = data(
            "index.accept",
            json!({
                "submission": submission,
                "protocol": parse(PROTOCOL),
                "experiment": parse(EXPERIMENT),
                "now": "2026-10-01T00:00:00Z"
            }),
        );
        assert_eq!(out["accepted"], json!(false), "{out}");
        assert_eq!(out["reason"], json!("before_start"));
        assert!(out["detail"].as_str().unwrap().contains("before"), "{out}");
    }

    #[test]
    fn index_accept_rejects_a_submission_claiming_a_future_observation() {
        // The case the previous test tripped over, now asserted on purpose: an
        // arrival inside the window whose no_future datetime is ahead of the
        // receive time is an invalid value, not a window problem.
        let (submission, _) = signed_submission();
        let out = data(
            "index.accept",
            json!({
                "submission": submission,
                "protocol": parse(PROTOCOL),
                "experiment": parse(EXPERIMENT),
                "now": "2026-10-19T00:00:00Z"
            }),
        );
        assert_eq!(out["accepted"], json!(false), "{out}");
        assert_eq!(out["reason"], json!("invalid_values"));
        assert!(out["detail"].as_str().unwrap().contains("observed"), "{out}");
    }

    #[test]
    fn index_accept_honours_the_manifests_protocol_status() {
        // A closed protocol must not be indexed even though the protocol
        // document itself says nothing about status.
        let (submission, _) = signed_submission();
        let mut experiment = parse(EXPERIMENT);
        experiment["protocols"][0]["status"] = json!("closed");

        let out = data(
            "index.accept",
            json!({
                "submission": submission,
                "protocol": parse(PROTOCOL),
                "experiment": experiment,
                "now": "2026-10-28T08:00:00Z"
            }),
        );
        assert_eq!(out["accepted"], json!(false));
        assert_eq!(out["reason"], json!("protocol_not_open"));
    }

    #[test]
    fn index_accept_applies_hidden_keys_from_the_pages() {
        let (submission, key) = signed_submission();
        let page = json!({ "entries": [], "hidden_keys": [key] });

        let out = data(
            "index.accept",
            json!({
                "submission": submission,
                "protocol": parse(PROTOCOL),
                "experiment": parse(EXPERIMENT),
                "pages": [page],
                "now": "2026-10-28T08:00:00Z"
            }),
        );
        assert_eq!(out["accepted"], json!(false));
        assert_eq!(out["reason"], json!("hidden_submitter"));
    }

    #[test]
    fn index_accept_honours_a_configured_daily_limit() {
        let (submission, key) = signed_submission();
        let page = json!({
            "hidden_keys": [],
            "entries": [{
                "cid": "logos:a", "protocol": "wildlife-sighting", "version": 2,
                "submitter": key,
                "submitted_at": "2026-10-28T07:00:00Z",
                "received_at": "2026-10-28T07:00:00Z"
            }]
        });

        let out = data(
            "index.accept",
            json!({
                "submission": submission,
                "protocol": parse(PROTOCOL),
                "experiment": parse(EXPERIMENT),
                "pages": [page],
                "daily_limit": 1,
                "now": "2026-10-28T08:00:00Z"
            }),
        );
        assert_eq!(out["accepted"], json!(false));
        assert_eq!(out["reason"], json!("rate_limited"));
    }

    #[test]
    fn index_accept_rejects_an_invalid_document_before_anything_else() {
        let mut submission = parse(SUBMISSION);
        submission["values"]["Bad Key"] = json!("x");
        let out = data(
            "index.accept",
            json!({
                "submission": submission,
                "protocol": parse(PROTOCOL),
                "experiment": parse(EXPERIMENT),
                "now": "2026-10-28T08:00:00Z"
            }),
        );
        assert_eq!(out["accepted"], json!(false));
        assert_eq!(out["reason"], json!("invalid_document"));
    }

    #[test]
    fn index_page_emits_contract_valid_pages() {
        let entries: Vec<Value> = (0..3)
            .map(|n| {
                json!({
                    "cid": format!("logos:zb2rhSub{n}"),
                    "protocol": "wildlife-sighting",
                    "version": 2,
                    "submitter": "a1".repeat(32),
                    "submitted_at": "2026-10-28T07:50:12+05:30",
                    "received_at": "2026-10-28T07:50:31+05:30"
                })
            })
            .collect();

        let out = data(
            "index.page",
            json!({
                "experiment": "q7k2m3x4w3b5n6tz",
                "entries": entries,
                "hidden_keys": [],
                "now": "2026-10-28T08:00:00Z"
            }),
        );
        let pages = out["pages"].as_array().unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0]["entries"].as_array().unwrap().len(), 3);

        // Stub the signature the uploader will add, then check the shape.
        let mut page = pages[0].clone();
        page["sig"] = json!("ab".repeat(64));
        assert!(schema::is_valid(&page), "{:?}", schema::errors(&page));
    }

    #[test]
    fn index_page_appends_to_a_page_with_room() {
        let out = data(
            "index.page",
            json!({
                "experiment": "q7k2m3x4w3b5n6tz",
                "current": parse(INDEX),
                "current_cid": "logos:zb2rhIndexP2",
                "entries": [{
                    "cid": "logos:zb2rhNew",
                    "protocol": "wildlife-sighting",
                    "version": 2,
                    "submitter": "a1".repeat(32),
                    "submitted_at": "2026-10-28T07:50:12+05:30",
                    "received_at": "2026-10-28T07:50:31+05:30"
                }],
                "hidden_keys": [],
                "now": "2026-10-28T08:00:00Z"
            }),
        );
        let pages = out["pages"].as_array().unwrap();
        assert_eq!(pages.len(), 1);
        // The example page is number 2 with two entries, so this makes three.
        assert_eq!(pages[0]["page"], json!(2));
        assert_eq!(pages[0]["entries"].as_array().unwrap().len(), 3);
    }

    // ---- helpers ----

    #[test]
    fn geo_round_returns_the_cell_centre() {
        let out = data("geo.round", json!({ "lat": 12.97712, "lon": 77.59581 }));
        assert_eq!(out["lat"], json!(12.975));
        assert_eq!(out["lon"], json!(77.595));
        assert_eq!(out["rounded"], json!(true));
    }

    #[test]
    fn parse_link_returns_the_topic_too() {
        let out = data(
            "experiments.parseLink",
            json!({ "link": "logos-collect://exp/q7k2m3x4w3b5n6tz?m=logos:zb2rhManifestV4" }),
        );
        assert_eq!(out["experiment"], json!("q7k2m3x4w3b5n6tz"));
        assert_eq!(out["manifest"], json!("logos:zb2rhManifestV4"));
        assert_eq!(out["topic"], json!("/collect/1/exp-q7k2m3x4w3b5n6tz/json"));
    }

    #[test]
    fn parse_link_rejects_a_foreign_link() {
        assert_eq!(
            error_code("experiments.parseLink", json!({ "link": "https://example.com" })),
            code::BAD_REQUEST
        );
    }

    #[test]
    fn capabilities_does_not_claim_storage_or_delivery() {
        // Being told up front beats discovering it at the first upload.
        let out = data("session.capabilities", json!({}));
        assert_eq!(out["storage"], json!(false));
        assert_eq!(out["delivery"], json!(false));
        assert_eq!(out["documents"], json!(true));
        assert_eq!(out["media"], json!(true));
    }

    // ---- fixtures mode ----

    /// Serialise the env change: these tests mutate process-global state.
    static FIXTURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_fixtures<T>(on: bool, body: impl FnOnce() -> T) -> T {
        let _guard = FIXTURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var("COLLECT_FIXTURES").ok();
        // SAFETY: serialised by FIXTURE_LOCK, and restored below.
        unsafe {
            if on {
                std::env::set_var("COLLECT_FIXTURES", "1");
            } else {
                std::env::remove_var("COLLECT_FIXTURES");
            }
        }
        let out = body();
        unsafe {
            match previous {
                Some(v) => std::env::set_var("COLLECT_FIXTURES", v),
                None => std::env::remove_var("COLLECT_FIXTURES"),
            }
        }
        out
    }

    #[test]
    fn fixtures_match_the_repository() {
        // examples/ is the source of truth; the module carries copies only
        // because the Nix build cannot see above its own root.
        assert_eq!(fixture::EXPERIMENT, EXPERIMENT, "fixtures/experiment.json has drifted");
        assert_eq!(fixture::PROTOCOL, PROTOCOL, "fixtures/protocol.json has drifted");
        assert_eq!(fixture::SUBMISSION, SUBMISSION, "fixtures/submission.json has drifted");
        assert_eq!(fixture::INDEX, INDEX, "fixtures/index.json has drifted");
    }

    #[test]
    fn experiments_open_serves_the_embedded_documents() {
        let out = with_fixtures(true, || data("experiments.open", json!({ "link": "ignored" })));
        assert_eq!(out["fixtures"], json!(true));
        assert_eq!(out["experiment"]["id"], json!("q7k2m3x4w3b5n6tz"));
        let protocols = out["protocols"].as_array().unwrap();
        assert_eq!(protocols.len(), 1);
        assert_eq!(protocols[0]["id"], json!("wildlife-sighting"));
        // The form renderer reads these, so the field list has to arrive.
        assert_eq!(protocols[0]["fields"].as_array().unwrap().len(), 8);
    }

    #[test]
    fn experiments_open_refuses_when_not_in_fixtures_mode() {
        // Better to say the transport is missing than to answer with fixtures
        // a caller did not ask for.
        let out = with_fixtures(false, || call("experiments.open", json!({ "link": "x" })));
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"]["code"], json!(code::NOT_WIRED));
    }

    #[test]
    fn protocols_get_returns_a_named_protocol() {
        let out = with_fixtures(true, || {
            data("protocols.get", json!({ "protocolId": "wildlife-sighting" }))
        });
        assert_eq!(out["protocol"]["id"], json!("wildlife-sighting"));
    }

    #[test]
    fn protocols_get_rejects_an_unknown_id() {
        let out = with_fixtures(true, || call("protocols.get", json!({ "protocolId": "nest-monitoring" })));
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"]["code"], json!(code::BAD_REQUEST));
    }

    #[test]
    fn capabilities_reports_whether_fixtures_are_on() {
        assert_eq!(with_fixtures(true, || data("session.capabilities", json!({})))["fixtures"], json!(true));
        assert_eq!(with_fixtures(false, || data("session.capabilities", json!({})))["fixtures"], json!(false));
    }

    // ---- media ----

    #[test]
    fn process_media_cleans_and_prefills() {
        use data_encoding::BASE64;

        // A tiny JPEG with GPS EXIF, built the same way media.rs tests do.
        let original = crate::media::tests_support::gps_jpeg_upright();
        let out = data(
            "submissions.processMedia",
            json!({ "bytes": BASE64.encode(&original), "max_mb_each": 25 }),
        );

        assert_eq!(out["mime"], json!("image/jpeg"));
        let cleaned = BASE64.decode(out["bytes"].as_str().unwrap().as_bytes()).unwrap();
        assert_eq!(media::remaining_metadata(&cleaned), None);
        // The uploaded bytes are never the original.
        assert_ne!(cleaned, original);

        // Section 5 step 3: the values read before cleaning come back.
        assert_eq!(out["prefills"]["location"]["method"], json!("photo"));
        assert_eq!(
            out["prefills"]["datetime"],
            json!("2026-10-28T07:42:00+05:30")
        );
    }

    #[test]
    fn process_media_rejects_a_non_image() {
        use data_encoding::BASE64;
        assert_eq!(
            error_code(
                "submissions.processMedia",
                json!({ "bytes": BASE64.encode(b"not an image") })
            ),
            code::MEDIA
        );
    }

    #[test]
    fn process_media_rejects_bad_base64() {
        assert_eq!(
            error_code("submissions.processMedia", json!({ "bytes": "!!!not base64!!!" })),
            code::BAD_REQUEST
        );
    }
}
