//! Validation against `schemas/collect.schema.json`.
//!
//! `CLAUDE.md`: every document is validated before upload and after download,
//! and an invalid document is never indexed. The contract is compiled into the
//! binary from the repository file, so there is one source of truth and no
//! runtime path to go missing inside the sandbox.
//!
//! The contract's top level is a `oneOf` across all five shapes, so validating
//! against it reports only "not valid under any of the schemas listed in the
//! 'oneOf' keyword", anchored at the document root. Invalid submissions are
//! ignored and logged locally, and that log is the only place a creator can
//! see *why* — so we read the declared kind first and validate against that
//! branch alone, which reports the offending member by path.

use serde_json::{Value, json};
use std::sync::OnceLock;

/// The data contract, embedded at build time from `schemas/collect.schema.json`.
pub const SCHEMA_SOURCE: &str = include_str!("../../../schemas/collect.schema.json");

/// Which document or announcement shape a value claims to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Experiment,
    Protocol,
    Submission,
    Index,
    HeadAnnouncement,
    SubmissionAnnouncement,
    ReceiptAnnouncement,
}

impl Kind {
    const ALL: [Kind; 7] = [
        Kind::Experiment,
        Kind::Protocol,
        Kind::Submission,
        Kind::Index,
        Kind::HeadAnnouncement,
        Kind::SubmissionAnnouncement,
        Kind::ReceiptAnnouncement,
    ];

    fn slot(self) -> usize {
        Kind::ALL.iter().position(|k| *k == self).expect("in ALL")
    }

    /// How this shape identifies itself on the wire.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Experiment => "collect/experiment@1",
            Kind::Protocol => "collect/protocol@1",
            Kind::Submission => "collect/submission@1",
            Kind::Index => "collect/index@1",
            Kind::HeadAnnouncement => "announcement:head",
            Kind::SubmissionAnnouncement => "announcement:submission",
            Kind::ReceiptAnnouncement => "announcement:receipt",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{kind} does not validate: {detail}")]
    Invalid { kind: &'static str, detail: String },
    #[error("document declares no recognised `schema` or `kind`")]
    Unidentified,
}

fn contract() -> &'static Value {
    static C: OnceLock<Value> = OnceLock::new();
    C.get_or_init(|| serde_json::from_str(SCHEMA_SOURCE).expect("embedded schema is valid JSON"))
}

fn compile(schema: &Value) -> jsonschema::Validator {
    jsonschema::options()
        // `format` is an annotation by default in draft 2020-12. The contract
        // leans on date-time and uuid, and tools/validate.py checks them, so
        // assert them here too.
        .should_validate_formats(true)
        .build(schema)
        .expect("schema compiles")
}

/// The whole contract — the `oneOf` over every shape.
fn whole() -> &'static jsonschema::Validator {
    static V: OnceLock<jsonschema::Validator> = OnceLock::new();
    V.get_or_init(|| compile(contract()))
}

/// A schema matching exactly one shape, carrying the contract's `$defs` so
/// internal `$ref`s still resolve.
fn branch_schema(kind: Kind) -> Value {
    let defs = contract()["$defs"].clone();
    let mut schema = match kind {
        Kind::Experiment => json!({ "$ref": "#/$defs/experiment" }),
        Kind::Protocol => json!({ "$ref": "#/$defs/protocol" }),
        Kind::Submission => json!({ "$ref": "#/$defs/submission" }),
        Kind::Index => json!({ "$ref": "#/$defs/index" }),
        // `announcement` is itself a oneOf over the three kinds, which would
        // put us back to a root-anchored error. Split it: the shared members
        // (base, minus its oneOf) and the one matching branch, combined with
        // allOf. The branch carries its own required list and
        // additionalProperties, so nothing is lost.
        Kind::HeadAnnouncement | Kind::SubmissionAnnouncement | Kind::ReceiptAnnouncement => {
            let ann = &defs["announcement"];
            let mut base = ann.clone();
            base.as_object_mut()
                .expect("announcement is an object schema")
                .remove("oneOf");
            let which = match kind {
                Kind::HeadAnnouncement => "head",
                Kind::SubmissionAnnouncement => "submission",
                _ => "receipt",
            };
            let branch = ann["oneOf"]
                .as_array()
                .expect("announcement has a oneOf")
                .iter()
                .find(|b| {
                    b.pointer("/properties/kind/const").and_then(Value::as_str) == Some(which)
                })
                .cloned()
                .expect("announcement branch exists for every kind");
            json!({ "allOf": [base, branch] })
        }
    };
    schema
        .as_object_mut()
        .expect("branch schema is an object")
        .insert("$defs".into(), defs);
    schema
}

fn branch(kind: Kind) -> &'static jsonschema::Validator {
    static V: OnceLock<Vec<jsonschema::Validator>> = OnceLock::new();
    &V.get_or_init(|| {
        Kind::ALL
            .iter()
            .map(|k| compile(&branch_schema(*k)))
            .collect()
    })[kind.slot()]
}

/// Is `doc` a valid Collect document or announcement?
///
/// Checked against the whole contract, so a document that declares nothing
/// recognisable fails here too.
pub fn is_valid(doc: &Value) -> bool {
    whole().is_valid(doc)
}

/// What shape does `doc` declare? Reads `schema` for documents and `kind` for
/// announcements. This is the document's own claim, not a validation — pair it
/// with `validate`.
pub fn kind(doc: &Value) -> Result<Kind, Error> {
    if let Some(s) = doc.get("schema").and_then(Value::as_str) {
        return match s {
            "collect/experiment@1" => Ok(Kind::Experiment),
            "collect/protocol@1" => Ok(Kind::Protocol),
            "collect/submission@1" => Ok(Kind::Submission),
            "collect/index@1" => Ok(Kind::Index),
            _ => Err(Error::Unidentified),
        };
    }
    match doc.get("kind").and_then(Value::as_str) {
        Some("head") => Ok(Kind::HeadAnnouncement),
        Some("submission") => Ok(Kind::SubmissionAnnouncement),
        Some("receipt") => Ok(Kind::ReceiptAnnouncement),
        _ => Err(Error::Unidentified),
    }
}

/// Every validation error against the shape `doc` declares, as
/// `path: message`, with the root written `/`.
///
/// An empty list means valid. A document declaring no recognised shape yields
/// a single error saying so.
pub fn errors(doc: &Value) -> Vec<String> {
    let Ok(k) = kind(doc) else {
        return vec!["/: document declares no recognised `schema` or `kind`".to_string()];
    };
    errors_as(doc, k)
}

/// Every validation error against a shape you name, regardless of what `doc`
/// declares. Use when the expected shape is known from context — a CID fetched
/// from a manifest's `protocols` list must be a protocol, whatever it says.
pub fn errors_as(doc: &Value, kind: Kind) -> Vec<String> {
    branch(kind)
        .iter_errors(doc)
        .map(|e| {
            let path = e.instance_path().to_string();
            let path = if path.is_empty() { "/" } else { &path };
            format!("{path}: {e}")
        })
        .collect()
}

/// Validate `doc` against the shape it declares.
pub fn validate(doc: &Value) -> Result<Kind, Error> {
    let k = kind(doc)?;
    validate_as(doc, k)?;
    Ok(k)
}

/// Validate `doc` against a shape you name.
pub fn validate_as(doc: &Value, kind: Kind) -> Result<(), Error> {
    let errs = errors_as(doc, kind);
    if errs.is_empty() {
        return Ok(());
    }
    Err(Error::Invalid {
        kind: kind.label(),
        // Report every error, not just the first: a rejected submission is
        // logged once and never retried, so a partial reason is a dead end.
        detail: errs.join("; "),
    })
}

/// Strip members the contract does not allow for `kind`.
///
/// Documents are validated after download, and anything unknown is a reason to
/// reject rather than repair — this is for the writing path, where a draft
/// assembled in the UI may carry scratch members that must not be signed.
pub fn retain_known_members(doc: &mut Value, kind: Kind) {
    let allowed = allowed_members(kind);
    if let Some(obj) = doc.as_object_mut() {
        obj.retain(|k, _| allowed.contains(&k.as_str()));
    }
}

fn allowed_members(kind: Kind) -> Vec<&'static str> {
    fn names(schema: &Value, into: &mut Vec<&'static str>) {
        if let Some(props) = schema.get("properties").and_then(Value::as_object) {
            for key in props.keys() {
                // Keys come from the embedded contract, which lives for the
                // whole process.
                into.push(Box::leak(key.clone().into_boxed_str()));
            }
        }
        for combinator in ["allOf", "oneOf", "anyOf"] {
            if let Some(list) = schema.get(combinator).and_then(Value::as_array) {
                for sub in list {
                    names(sub, into);
                }
            }
        }
    }

    static CACHE: OnceLock<Vec<Vec<&'static str>>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            Kind::ALL
                .iter()
                .map(|k| {
                    let defs = &contract()["$defs"];
                    let schema: &Value = match k {
                        Kind::Experiment => &defs["experiment"],
                        Kind::Protocol => &defs["protocol"],
                        Kind::Submission => &defs["submission"],
                        Kind::Index => &defs["index"],
                        _ => &defs["announcement"],
                    };
                    let mut out = Vec::new();
                    names(schema, &mut out);
                    out.sort_unstable();
                    out.dedup();
                    out
                })
                .collect()
        })
        .get(kind.slot())
        .cloned()
        .unwrap_or_default()
}

/// The declared members of a document, in contract order — handy for building
/// a document without typos in member names.
pub fn members(kind: Kind) -> Vec<&'static str> {
    allowed_members(kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPERIMENT: &str = include_str!("../../../examples/experiment.json");
    const PROTOCOL: &str = include_str!("../../../examples/protocol.json");
    const SUBMISSION: &str = include_str!("../../../examples/submission.json");
    const INDEX: &str = include_str!("../../../examples/index.json");
    const ANNOUNCEMENTS: &str = include_str!("../../../examples/announcements.json");

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    fn announcements() -> Vec<Value> {
        parse(ANNOUNCEMENTS).as_array().unwrap().clone()
    }

    #[test]
    fn embedded_schema_compiles() {
        assert!(!is_valid(&json!({})));
    }

    #[test]
    fn every_branch_schema_compiles() {
        for k in Kind::ALL {
            // Forces the lazy build; a bad split would panic here.
            let _ = branch(k);
        }
    }

    #[test]
    fn every_example_document_validates() {
        for src in [EXPERIMENT, PROTOCOL, SUBMISSION, INDEX] {
            let doc = parse(src);
            assert!(is_valid(&doc), "{:?}", errors(&doc));
            assert!(validate(&doc).is_ok(), "{:?}", errors(&doc));
        }
    }

    #[test]
    fn every_example_announcement_validates() {
        let anns = announcements();
        assert_eq!(anns.len(), 3);
        for a in &anns {
            assert!(is_valid(a), "{:?}", errors(a));
            assert!(validate(a).is_ok(), "{:?}", errors(a));
        }
    }

    #[test]
    fn identifies_each_shape() {
        assert_eq!(validate(&parse(EXPERIMENT)).unwrap(), Kind::Experiment);
        assert_eq!(validate(&parse(PROTOCOL)).unwrap(), Kind::Protocol);
        assert_eq!(validate(&parse(SUBMISSION)).unwrap(), Kind::Submission);
        assert_eq!(validate(&parse(INDEX)).unwrap(), Kind::Index);

        let anns = announcements();
        assert_eq!(kind(&anns[0]).unwrap(), Kind::HeadAnnouncement);
        assert_eq!(kind(&anns[1]).unwrap(), Kind::SubmissionAnnouncement);
        assert_eq!(kind(&anns[2]).unwrap(), Kind::ReceiptAnnouncement);
    }

    #[test]
    fn refuses_to_identify_an_unknown_shape() {
        assert!(matches!(kind(&json!({})), Err(Error::Unidentified)));
        assert!(matches!(
            kind(&json!({"schema": "collect/experiment@2"})),
            Err(Error::Unidentified)
        ));
        assert!(matches!(
            kind(&json!({"kind": "gossip"})),
            Err(Error::Unidentified)
        ));
        assert!(matches!(validate(&json!({})), Err(Error::Unidentified)));
    }

    #[test]
    fn branch_validation_names_the_offending_member() {
        // The point of the per-kind split: a path into the document, not a
        // root-level "no branch matched".
        let mut s = parse(SUBMISSION);
        s["values"]["photos"][0]["mime"] = json!("image/gif");
        let errs = errors(&s);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].starts_with("/values/photos"), "{errs:?}");

        // `/values/photos` is as deep as the contract alone can go: `value` is
        // an untagged anyOf (string | number | bool | option list | media list
        // | location), so nothing here knows `photos` should be a media list.
        // Narrowing that to "mime must be one of ..." needs the protocol's
        // declared field types, which is the field-validation layer's job.
        assert!(!errs[0].starts_with("/values/photos/0/mime"));

        // A member the contract types directly still reports exactly.
        let mut s = parse(SUBMISSION);
        s["experiment"] = json!("NOT-BASE32");
        assert!(
            errors(&s).iter().any(|e| e.starts_with("/experiment")),
            "{:?}",
            errors(&s)
        );
    }

    #[test]
    fn branch_validation_names_the_offending_announcement_member() {
        let mut r = announcements()[2].clone();
        r.as_object_mut().unwrap().remove("cid");
        let errs = errors(&r);
        assert!(errs.iter().any(|e| e.contains("cid")), "{errs:?}");
    }

    #[test]
    fn validate_as_checks_an_expected_shape() {
        // A CID named in a manifest's `protocols` list must be a protocol,
        // whatever the fetched document claims about itself.
        let submission = parse(SUBMISSION);
        assert!(validate_as(&submission, Kind::Protocol).is_err());
        assert!(validate_as(&parse(PROTOCOL), Kind::Protocol).is_ok());
    }

    #[test]
    fn error_lists_every_problem_not_just_the_first() {
        let mut s = parse(SUBMISSION);
        s["submitted_at"] = json!("28-10-2026");
        s["submitter"]["key"] = json!("abcd");
        let detail = match validate(&s) {
            Err(Error::Invalid { detail, .. }) => detail,
            other => panic!("expected Invalid, got {other:?}"),
        };
        assert!(detail.contains("/submitted_at"), "{detail}");
        assert!(detail.contains("/submitter/key"), "{detail}");
    }

    // The negative cases below mirror tools/validate.py, so the Rust core and
    // the CI validator agree on what the contract rejects.

    #[test]
    fn rejects_choice_field_without_options() {
        let mut p = parse(PROTOCOL);
        p["fields"][1]["config"]
            .as_object_mut()
            .unwrap()
            .remove("options");
        assert!(!is_valid(&p));
        assert!(validate(&p).is_err());
    }

    #[test]
    fn rejects_location_field_without_store_rule() {
        let mut p = parse(PROTOCOL);
        p["fields"][4]["config"]
            .as_object_mut()
            .unwrap()
            .remove("store");
        assert!(!is_valid(&p));
    }

    #[test]
    fn rejects_unknown_field_type() {
        let mut p = parse(PROTOCOL);
        p["fields"][0]["type"] = json!("audio");
        assert!(!is_valid(&p));
    }

    #[test]
    fn rejects_submission_with_bad_field_key() {
        let mut s = parse(SUBMISSION);
        s["values"]["Bad Key"] = json!("x");
        assert!(!is_valid(&s));
    }

    #[test]
    fn rejects_submission_without_signature() {
        let mut s = parse(SUBMISSION);
        s.as_object_mut().unwrap().remove("sig");
        assert!(!is_valid(&s));
    }

    #[test]
    fn rejects_photo_with_unsupported_type() {
        let mut s = parse(SUBMISSION);
        s["values"]["photos"][0]["mime"] = json!("image/gif");
        assert!(!is_valid(&s));
    }

    #[test]
    fn rejects_index_page_over_1000_entries() {
        let mut i = parse(INDEX);
        let entries = i["entries"].as_array().unwrap().clone();
        let mut blown = Vec::new();
        for _ in 0..501 {
            blown.extend(entries.iter().cloned());
        }
        i["entries"] = Value::Array(blown);
        assert!(!is_valid(&i));
    }

    #[test]
    fn rejects_receipt_missing_cid() {
        let mut r = announcements()[2].clone();
        r.as_object_mut().unwrap().remove("cid");
        assert!(!is_valid(&r));
    }

    #[test]
    fn rejects_head_with_extra_field() {
        let mut h = announcements()[0].clone();
        h["cid"] = json!("logos:x");
        assert!(!is_valid(&h));
        assert!(validate(&h).is_err());
    }

    #[test]
    fn rejects_malformed_timestamps() {
        let mut s = parse(SUBMISSION);
        s["submitted_at"] = json!("28-10-2026");
        assert!(
            !is_valid(&s),
            "format assertion for date-time is not active"
        );
    }

    #[test]
    fn rejects_malformed_submission_id() {
        let mut s = parse(SUBMISSION);
        s["id"] = json!("not-a-uuid");
        assert!(!is_valid(&s), "format assertion for uuid is not active");
    }

    #[test]
    fn rejects_short_key_and_sig() {
        let mut s = parse(SUBMISSION);
        s["submitter"]["key"] = json!("abcd");
        assert!(!is_valid(&s));

        let mut s = parse(SUBMISSION);
        s["sig"] = json!("abcd");
        assert!(!is_valid(&s));
    }

    #[test]
    fn rejects_a_cid_without_the_logos_prefix() {
        let mut i = parse(INDEX);
        i["entries"][0]["cid"] = json!("QmSomethingElse");
        assert!(!is_valid(&i));
    }

    #[test]
    fn lists_allowed_members_per_shape() {
        let m = members(Kind::Submission);
        for expected in [
            "schema",
            "id",
            "experiment",
            "protocol",
            "submitter",
            "values",
            "sig",
        ] {
            assert!(m.contains(&expected), "{expected} missing from {m:?}");
        }
        assert!(!m.contains(&"manifest"));

        // The announcement union contributes every branch's members.
        let h = members(Kind::HeadAnnouncement);
        assert!(h.contains(&"manifest"));
        assert!(h.contains(&"index_page"));
    }

    #[test]
    fn retains_only_known_members() {
        let mut s = parse(SUBMISSION);
        s["scratch_draft_id"] = json!("local-only");
        assert!(!is_valid(&s));
        retain_known_members(&mut s, Kind::Submission);
        assert!(is_valid(&s), "{:?}", errors(&s));
        assert!(s.get("scratch_draft_id").is_none());
    }
}
