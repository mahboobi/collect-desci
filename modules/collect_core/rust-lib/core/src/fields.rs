//! The protocol field model and value validation (SPEC section 4).
//!
//! Two callers, one set of rules:
//!
//! - the submitter's form, which must disable Submit until required fields are
//!   valid, and
//! - the creator's indexer, which re-runs field validation on every received
//!   submission and never indexes one that fails.
//!
//! They have to agree, so neither gets its own copy. `CLAUDE.md`: the form is
//! rendered from the protocol JSON, never hard-coded.

use crate::{geo, ids};
use chrono::{DateTime, FixedOffset, Utc};
use serde_json::Value;
use std::collections::BTreeMap;

/// Default `max_length` for a `text` field.
pub const TEXT_DEFAULT_MAX: usize = 200;
/// Default `max_length` for a `longtext` field.
pub const LONGTEXT_DEFAULT_MAX: usize = 5000;
/// Default `max_mb_each` for a `photo` field.
pub const PHOTO_DEFAULT_MAX_MB: f64 = 25.0;
/// Hard ceiling on photos per field, from the contract.
pub const PHOTO_MAX_COUNT: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    Text,
    LongText,
    Number,
    Choice,
    Multi,
    DateTime,
    Photo,
    Location,
    YesNo,
}

impl FieldType {
    pub fn as_str(self) -> &'static str {
        match self {
            FieldType::Text => "text",
            FieldType::LongText => "longtext",
            FieldType::Number => "number",
            FieldType::Choice => "choice",
            FieldType::Multi => "multi",
            FieldType::DateTime => "datetime",
            FieldType::Photo => "photo",
            FieldType::Location => "location",
            FieldType::YesNo => "yesno",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "text" => FieldType::Text,
            "longtext" => FieldType::LongText,
            "number" => FieldType::Number,
            "choice" => FieldType::Choice,
            "multi" => FieldType::Multi,
            "datetime" => FieldType::DateTime,
            "photo" => FieldType::Photo,
            "location" => FieldType::Location,
            "yesno" => FieldType::YesNo,
            _ => return None,
        })
    }
}

/// How a `location` value may be captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Photo,
    Map,
    Manual,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Photo => "photo",
            Method::Map => "map",
            Method::Manual => "manual",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "photo" => Method::Photo,
            "map" => Method::Map,
            "manual" => Method::Manual,
            _ => return None,
        })
    }
}

/// Whether a `location` field stores the exact point or only its cell centre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Store {
    Exact,
    Rounded1km,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Opt {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Config {
    Text {
        max_length: usize,
    },
    Number {
        min: Option<f64>,
        max: Option<f64>,
        integer: bool,
        unit: Option<String>,
    },
    Choice {
        options: Vec<Opt>,
    },
    Multi {
        options: Vec<Opt>,
        min: Option<usize>,
        max: Option<usize>,
    },
    DateTime {
        fill_from: Vec<String>,
        no_future: bool,
    },
    Photo {
        min: Option<usize>,
        max: Option<usize>,
        max_mb_each: f64,
        fill_from_exif: Vec<String>,
    },
    Location {
        methods: Vec<Method>,
        max_accuracy_m: Option<f64>,
        store: Store,
    },
    YesNo,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub key: String,
    pub ty: FieldType,
    pub label: String,
    pub help: Option<String>,
    pub required: bool,
    pub config: Config,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("`fields` is missing or not an array")]
    NotAnArray,
    #[error("field {at}: {detail}")]
    Field { at: usize, detail: String },
    #[error("duplicate field key `{0}`")]
    DuplicateKey(String),
}

/// Read a protocol's `fields` into the model the form and the indexer share.
///
/// Assumes the protocol already validated against the contract; this reports
/// what the contract cannot express, such as duplicate keys across fields.
pub fn parse_fields(protocol: &Value) -> Result<Vec<Field>, ParseError> {
    let list = protocol
        .get("fields")
        .and_then(Value::as_array)
        .ok_or(ParseError::NotAnArray)?;

    let mut out = Vec::with_capacity(list.len());
    let mut seen: Vec<String> = Vec::with_capacity(list.len());
    for (at, raw) in list.iter().enumerate() {
        let field = parse_field(raw).map_err(|detail| ParseError::Field { at, detail })?;
        if seen.contains(&field.key) {
            return Err(ParseError::DuplicateKey(field.key));
        }
        seen.push(field.key.clone());
        out.push(field);
    }
    Ok(out)
}

fn parse_field(raw: &Value) -> Result<Field, String> {
    let key = raw
        .get("key")
        .and_then(Value::as_str)
        .ok_or("`key` is missing")?;
    if !ids::is_field_key(key) {
        return Err(format!("`{key}` is not a valid field key"));
    }
    let ty = raw
        .get("type")
        .and_then(Value::as_str)
        .and_then(FieldType::parse)
        .ok_or("`type` is missing or unknown")?;
    let label = raw
        .get("label")
        .and_then(Value::as_str)
        .ok_or("`label` is missing")?;
    let required = raw
        .get("required")
        .and_then(Value::as_bool)
        .ok_or("`required` is missing")?;
    let cfg = raw.get("config");

    let config = match ty {
        FieldType::Text | FieldType::LongText => {
            let default = if ty == FieldType::Text {
                TEXT_DEFAULT_MAX
            } else {
                LONGTEXT_DEFAULT_MAX
            };
            Config::Text {
                max_length: usize_at(cfg, "max_length").unwrap_or(default),
            }
        }
        FieldType::Number => Config::Number {
            min: f64_at(cfg, "min"),
            max: f64_at(cfg, "max"),
            integer: bool_at(cfg, "integer").unwrap_or(false),
            unit: str_at(cfg, "unit").map(str::to_string),
        },
        FieldType::Choice => Config::Choice {
            options: parse_options(cfg)?,
        },
        FieldType::Multi => Config::Multi {
            options: parse_options(cfg)?,
            min: usize_at(cfg, "min"),
            max: usize_at(cfg, "max"),
        },
        FieldType::DateTime => Config::DateTime {
            fill_from: string_list(cfg, "fill_from"),
            no_future: bool_at(cfg, "no_future").unwrap_or(false),
        },
        FieldType::Photo => Config::Photo {
            min: usize_at(cfg, "min"),
            max: usize_at(cfg, "max"),
            max_mb_each: f64_at(cfg, "max_mb_each").unwrap_or(PHOTO_DEFAULT_MAX_MB),
            fill_from_exif: string_list(cfg, "fill_from_exif"),
        },
        FieldType::Location => {
            let methods: Vec<Method> = cfg
                .and_then(|c| c.get("methods"))
                .and_then(Value::as_array)
                .ok_or("`config.methods` is missing")?
                .iter()
                .filter_map(Value::as_str)
                .filter_map(Method::parse)
                .collect();
            if methods.is_empty() {
                return Err("`config.methods` lists no usable method".into());
            }
            let store = match str_at(cfg, "store") {
                Some("exact") => Store::Exact,
                Some("rounded_1km") => Store::Rounded1km,
                _ => return Err("`config.store` must be `exact` or `rounded_1km`".into()),
            };
            Config::Location {
                methods,
                max_accuracy_m: f64_at(cfg, "max_accuracy_m"),
                store,
            }
        }
        FieldType::YesNo => Config::YesNo,
    };

    Ok(Field {
        key: key.to_string(),
        ty,
        label: label.to_string(),
        help: raw.get("help").and_then(Value::as_str).map(str::to_string),
        required,
        config,
    })
}

fn parse_options(cfg: Option<&Value>) -> Result<Vec<Opt>, String> {
    let raw = cfg
        .and_then(|c| c.get("options"))
        .and_then(Value::as_array)
        .ok_or("`config.options` is missing")?;
    let mut out = Vec::with_capacity(raw.len());
    for o in raw {
        let value = o
            .get("value")
            .and_then(Value::as_str)
            .ok_or("an option has no `value`")?;
        let label = o
            .get("label")
            .and_then(Value::as_str)
            .ok_or("an option has no `label`")?;
        if out.iter().any(|p: &Opt| p.value == value) {
            return Err(format!("duplicate option value `{value}`"));
        }
        out.push(Opt {
            value: value.to_string(),
            label: label.to_string(),
        });
    }
    Ok(out)
}

fn str_at<'a>(cfg: Option<&'a Value>, key: &str) -> Option<&'a str> {
    cfg?.get(key)?.as_str()
}
fn f64_at(cfg: Option<&Value>, key: &str) -> Option<f64> {
    cfg?.get(key)?.as_f64()
}
fn bool_at(cfg: Option<&Value>, key: &str) -> Option<bool> {
    cfg?.get(key)?.as_bool()
}
fn usize_at(cfg: Option<&Value>, key: &str) -> Option<usize> {
    usize::try_from(cfg?.get(key)?.as_u64()?).ok()
}
fn string_list(cfg: Option<&Value>, key: &str) -> Vec<String> {
    cfg.and_then(|c| c.get(key))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The result of checking a set of values against a protocol's fields.
///
/// Shaped to match `submissions.validate` in SPEC section 6: `missing` lists
/// required fields with no answer, `errors` maps a field key to why its answer
/// is unusable. A submission is acceptable when both are empty.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    pub missing: Vec<String>,
    pub errors: BTreeMap<String, String>,
}

impl Report {
    pub fn is_ok(&self) -> bool {
        self.missing.is_empty() && self.errors.is_empty()
    }
}

/// Check `values` against `fields`.
///
/// `now` is passed in rather than read from the clock: the submitter checks
/// against their own time, and the creator re-checks later against theirs, so
/// the caller decides which instant applies.
pub fn validate_values(fields: &[Field], values: &Value, now: DateTime<Utc>) -> Report {
    let mut report = Report::default();

    let Some(map) = values.as_object() else {
        report
            .errors
            .insert("/".into(), "values is not an object".into());
        return report;
    };

    // A value for a field the protocol does not declare is a rejection, not
    // something to ignore: it was signed, so it is part of what the submitter
    // claimed, and the creator's table has no column for it.
    for key in map.keys() {
        if !fields.iter().any(|f| &f.key == key) {
            report.errors.insert(
                key.clone(),
                format!("`{key}` is not a field of this protocol"),
            );
        }
    }

    for field in fields {
        match map.get(&field.key) {
            None | Some(Value::Null) => {
                if field.required {
                    report.missing.push(field.key.clone());
                }
            }
            Some(value) => {
                if is_blank(value) {
                    if field.required {
                        report.missing.push(field.key.clone());
                    }
                    continue;
                }
                if let Err(detail) = check_value(field, value, now) {
                    report.errors.insert(field.key.clone(), detail);
                }
            }
        }
    }

    report
}

/// Is this answer empty rather than wrong? An empty string or empty list means
/// the field was left alone, so a required field reports as missing rather
/// than as an error.
fn is_blank(value: &Value) -> bool {
    match value {
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        _ => false,
    }
}

fn check_value(field: &Field, value: &Value, now: DateTime<Utc>) -> Result<(), String> {
    match (&field.config, field.ty) {
        (Config::Text { max_length }, _) => {
            let s = value.as_str().ok_or("must be text")?;
            // Count characters, not bytes: the contract's maxLength counts
            // code points, and a byte count would reject a shorter answer
            // written in a non-Latin script.
            let len = s.chars().count();
            if len > *max_length {
                return Err(format!(
                    "must be at most {max_length} characters, got {len}"
                ));
            }
            Ok(())
        }
        (
            Config::Number {
                min,
                max,
                integer,
                unit: _,
            },
            _,
        ) => {
            let n = value.as_f64().ok_or("must be a number")?;
            if !n.is_finite() {
                return Err("must be a finite number".into());
            }
            if *integer && n.fract() != 0.0 {
                return Err("must be a whole number".into());
            }
            if let Some(lo) = min
                && n < *lo
            {
                return Err(format!("must be at least {lo}"));
            }
            if let Some(hi) = max
                && n > *hi
            {
                return Err(format!("must be at most {hi}"));
            }
            Ok(())
        }
        (Config::Choice { options }, _) => {
            let s = value.as_str().ok_or("must be one of the options")?;
            if !options.iter().any(|o| o.value == s) {
                return Err(format!("`{s}` is not one of the options"));
            }
            Ok(())
        }
        (Config::Multi { options, min, max }, _) => {
            let list = value.as_array().ok_or("must be a list of options")?;
            let mut seen: Vec<&str> = Vec::with_capacity(list.len());
            for item in list {
                let s = item.as_str().ok_or("every selection must be an option")?;
                if !options.iter().any(|o| o.value == s) {
                    return Err(format!("`{s}` is not one of the options"));
                }
                if seen.contains(&s) {
                    return Err(format!("`{s}` is selected more than once"));
                }
                seen.push(s);
            }
            if let Some(lo) = min
                && list.len() < *lo
            {
                return Err(format!("select at least {lo}"));
            }
            if let Some(hi) = max
                && list.len() > *hi
            {
                return Err(format!("select at most {hi}"));
            }
            Ok(())
        }
        (Config::DateTime { no_future, .. }, _) => {
            let s = value.as_str().ok_or("must be a date and time")?;
            let parsed = parse_offset_datetime(s)?;
            if *no_future && parsed.with_timezone(&Utc) > now {
                return Err("must not be in the future".into());
            }
            Ok(())
        }
        (
            Config::Photo {
                min,
                max,
                max_mb_each,
                ..
            },
            _,
        ) => {
            let list = value.as_array().ok_or("must be a list of photos")?;
            let hi = max.unwrap_or(PHOTO_MAX_COUNT).min(PHOTO_MAX_COUNT);
            if list.len() > hi {
                return Err(format!("at most {hi} photos, got {}", list.len()));
            }
            if let Some(lo) = min
                && list.len() < *lo
            {
                return Err(format!("at least {lo} photos"));
            }
            let limit_bytes = (max_mb_each * 1024.0 * 1024.0) as u64;
            for (i, item) in list.iter().enumerate() {
                let bytes = item
                    .get("bytes")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| format!("photo {i} has no `bytes`"))?;
                if bytes > limit_bytes {
                    return Err(format!(
                        "photo {i} is {bytes} bytes, over the {max_mb_each} MB limit"
                    ));
                }
                for member in ["cid", "sha256", "mime"] {
                    if item.get(member).and_then(Value::as_str).is_none() {
                        return Err(format!("photo {i} has no `{member}`"));
                    }
                }
            }
            Ok(())
        }
        (
            Config::Location {
                methods,
                max_accuracy_m,
                store,
            },
            _,
        ) => {
            let obj = value.as_object().ok_or("must be a location")?;
            let lat = obj
                .get("lat")
                .and_then(Value::as_f64)
                .ok_or("needs `lat`")?;
            let lon = obj
                .get("lon")
                .and_then(Value::as_f64)
                .ok_or("needs `lon`")?;
            if !(-90.0..=90.0).contains(&lat) {
                return Err(format!("latitude {lat} is out of range"));
            }
            if !(-180.0..=180.0).contains(&lon) {
                return Err(format!("longitude {lon} is out of range"));
            }

            let method = obj
                .get("method")
                .and_then(Value::as_str)
                .and_then(Method::parse)
                .ok_or("needs a `method`")?;
            if !methods.contains(&method) {
                return Err(format!(
                    "`{}` is not an allowed method for this field",
                    method.as_str()
                ));
            }

            if let Some(limit) = max_accuracy_m {
                // A missing accuracy is not a pass: the field asked for a
                // bound and the submission cannot show it meets one.
                let accuracy = obj
                    .get("accuracy_m")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| format!("needs `accuracy_m` within {limit} m"))?;
                if accuracy > *limit {
                    return Err(format!("accuracy {accuracy} m is worse than {limit} m"));
                }
            }

            if *store == Store::Rounded1km {
                // Check the coordinate itself, not the `rounded` flag: the
                // flag is the submitter's claim, and an exact point stored
                // under a rounded_1km protocol is public forever.
                if !geo::is_rounded(lat, lon) {
                    return Err(
                        "this field stores rounded coordinates, but the point is not rounded"
                            .into(),
                    );
                }
                if obj.get("rounded").and_then(Value::as_bool) != Some(true) {
                    return Err("rounded coordinates must be marked `rounded: true`".into());
                }
            }
            Ok(())
        }
        (Config::YesNo, _) => {
            value.as_bool().ok_or("must be yes or no")?;
            Ok(())
        }
    }
}

/// Parse an RFC 3339 timestamp, which SPEC section 4 requires to carry an
/// offset.
fn parse_offset_datetime(s: &str) -> Result<DateTime<FixedOffset>, String> {
    DateTime::parse_from_rfc3339(s).map_err(|_| "must be a date and time with a UTC offset".into())
}

/// Apply a `location` field's store rule to a captured point.
///
/// Call this before the value reaches a document: for a `rounded_1km` field
/// the exact point must never be stored, so rounding happens here and the
/// caller keeps only what comes back.
pub fn apply_store_rule(field: &Field, location: &mut Value) {
    let Config::Location { store, .. } = &field.config else {
        return;
    };
    if *store != Store::Rounded1km {
        return;
    }
    let Some(obj) = location.as_object_mut() else {
        return;
    };
    let (Some(lat), Some(lon)) = (
        obj.get("lat").and_then(Value::as_f64),
        obj.get("lon").and_then(Value::as_f64),
    ) else {
        return;
    };
    let (rlat, rlon) = geo::round_1km(lat, lon);
    obj.insert("lat".into(), json_number(rlat));
    obj.insert("lon".into(), json_number(rlon));
    obj.insert("rounded".into(), Value::Bool(true));
}

fn json_number(v: f64) -> Value {
    serde_json::Number::from_f64(v)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PROTOCOL: &str = include_str!("../../../../../examples/protocol.json");
    const SUBMISSION: &str = include_str!("../../../../../examples/submission.json");

    fn protocol() -> Value {
        serde_json::from_str(PROTOCOL).unwrap()
    }

    fn example_fields() -> Vec<Field> {
        parse_fields(&protocol()).unwrap()
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-28T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn example_values() -> Value {
        serde_json::from_str::<Value>(SUBMISSION).unwrap()["values"].clone()
    }

    fn field(ty: FieldType, required: bool, config: Config) -> Field {
        Field {
            key: "f".into(),
            ty,
            label: "F".into(),
            help: None,
            required,
            config,
        }
    }

    fn check(f: &Field, v: Value) -> Result<(), String> {
        check_value(f, &v, now())
    }

    // ---- parsing ----

    #[test]
    fn parses_the_example_protocol() {
        let fields = example_fields();
        assert_eq!(fields.len(), 8);
        let keys: Vec<&str> = fields.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "photos",
                "category",
                "species",
                "count",
                "location",
                "observed",
                "behaviour",
                "notes"
            ]
        );
    }

    #[test]
    fn parses_each_config_shape() {
        let fields = example_fields();
        assert!(matches!(fields[0].config, Config::Photo { .. }));
        assert!(matches!(fields[1].config, Config::Choice { .. }));
        assert!(matches!(fields[2].config, Config::Text { .. }));
        assert!(matches!(fields[3].config, Config::Number { .. }));
        assert!(matches!(fields[4].config, Config::Location { .. }));
        assert!(matches!(fields[5].config, Config::DateTime { .. }));
        assert!(matches!(fields[6].config, Config::Multi { .. }));
        assert!(matches!(fields[7].config, Config::Text { .. }));
    }

    #[test]
    fn applies_the_documented_defaults() {
        // No max_length on a text field means 200, and on longtext 5000.
        let text = parse_field(&json!({
            "key": "t", "type": "text", "label": "T", "required": false
        }))
        .unwrap();
        assert_eq!(
            text.config,
            Config::Text {
                max_length: TEXT_DEFAULT_MAX
            }
        );

        let long = parse_field(&json!({
            "key": "l", "type": "longtext", "label": "L", "required": false
        }))
        .unwrap();
        assert_eq!(
            long.config,
            Config::Text {
                max_length: LONGTEXT_DEFAULT_MAX
            }
        );

        let photo = parse_field(&json!({
            "key": "p", "type": "photo", "label": "P", "required": false
        }))
        .unwrap();
        match photo.config {
            Config::Photo { max_mb_each, .. } => assert_eq!(max_mb_each, PHOTO_DEFAULT_MAX_MB),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rejects_a_duplicate_field_key() {
        // Two fields with one key would make `values` ambiguous, and the
        // contract cannot express the constraint.
        let p = json!({"fields": [
            {"key": "a", "type": "yesno", "label": "A", "required": false},
            {"key": "a", "type": "text", "label": "A again", "required": false}
        ]});
        assert_eq!(parse_fields(&p), Err(ParseError::DuplicateKey("a".into())));
    }

    #[test]
    fn rejects_a_duplicate_option_value() {
        let p = json!({"fields": [{
            "key": "c", "type": "choice", "label": "C", "required": false,
            "config": {"options": [
                {"value": "x", "label": "One"},
                {"value": "x", "label": "Two"}
            ]}
        }]});
        assert!(matches!(
            parse_fields(&p),
            Err(ParseError::Field { at: 0, .. })
        ));
    }

    #[test]
    fn rejects_a_malformed_field_key() {
        let p = json!({"fields": [
            {"key": "Bad Key", "type": "yesno", "label": "A", "required": false}
        ]});
        assert!(matches!(
            parse_fields(&p),
            Err(ParseError::Field { at: 0, .. })
        ));
    }

    #[test]
    fn rejects_missing_fields_array() {
        assert_eq!(parse_fields(&json!({})), Err(ParseError::NotAnArray));
    }

    // ---- the example submission ----

    #[test]
    fn the_example_submission_validates_against_the_example_protocol() {
        let report = validate_values(&example_fields(), &example_values(), now());
        assert!(report.is_ok(), "{report:?}");
    }

    // ---- required and blank handling ----

    #[test]
    fn reports_a_missing_required_field_as_missing() {
        let mut values = example_values();
        values.as_object_mut().unwrap().remove("count");
        let report = validate_values(&example_fields(), &values, now());
        assert_eq!(report.missing, ["count"]);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
    }

    #[test]
    fn treats_blank_and_null_as_missing_not_invalid() {
        // An untouched field should read as "not answered yet", so the form
        // says "required" rather than showing a validation error.
        for blank in [json!(""), json!([]), json!(null)] {
            let mut values = example_values();
            values["species"] = blank.clone();
            values["photos"] = blank.clone();
            let report = validate_values(&example_fields(), &values, now());
            // `photos` is required, `species` is not.
            assert_eq!(report.missing, ["photos"], "{blank}");
            assert!(report.errors.is_empty(), "{blank}: {:?}", report.errors);
        }
    }

    #[test]
    fn an_absent_optional_field_is_fine() {
        let mut values = example_values();
        for optional in ["species", "behaviour", "notes"] {
            values.as_object_mut().unwrap().remove(optional);
        }
        assert!(validate_values(&example_fields(), &values, now()).is_ok());
    }

    #[test]
    fn rejects_a_value_for_an_undeclared_field() {
        let mut values = example_values();
        values["smuggled"] = json!("whatever");
        let report = validate_values(&example_fields(), &values, now());
        assert!(report.errors.contains_key("smuggled"), "{report:?}");
    }

    #[test]
    fn rejects_values_that_are_not_an_object() {
        let report = validate_values(&example_fields(), &json!([]), now());
        assert!(!report.is_ok());
    }

    // ---- text ----

    #[test]
    fn text_enforces_max_length_in_characters() {
        let f = field(FieldType::Text, false, Config::Text { max_length: 5 });
        assert!(check(&f, json!("12345")).is_ok());
        assert!(check(&f, json!("123456")).is_err());
        // Five characters that take more than five bytes must still pass.
        assert!(check(&f, json!("\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}")).is_ok());
        assert!(check(&f, json!(5)).is_err());
    }

    // ---- number ----

    #[test]
    fn number_enforces_range_and_integrality() {
        let f = field(
            FieldType::Number,
            false,
            Config::Number {
                min: Some(1.0),
                max: Some(10.0),
                integer: true,
                unit: None,
            },
        );
        assert!(check(&f, json!(1)).is_ok());
        assert!(check(&f, json!(10)).is_ok());
        assert!(check(&f, json!(0)).is_err());
        assert!(check(&f, json!(11)).is_err());
        assert!(check(&f, json!(2.5)).is_err());
        assert!(check(&f, json!("3")).is_err());
    }

    #[test]
    fn number_allows_fractions_when_not_integer() {
        let f = field(
            FieldType::Number,
            false,
            Config::Number {
                min: None,
                max: None,
                integer: false,
                unit: None,
            },
        );
        assert!(check(&f, json!(2.5)).is_ok());
        assert!(check(&f, json!(-100.25)).is_ok());
    }

    // ---- choice and multi ----

    #[test]
    fn choice_requires_a_declared_option() {
        let f = field(
            FieldType::Choice,
            false,
            Config::Choice {
                options: vec![
                    Opt {
                        value: "a".into(),
                        label: "A".into(),
                    },
                    Opt {
                        value: "b".into(),
                        label: "B".into(),
                    },
                ],
            },
        );
        assert!(check(&f, json!("a")).is_ok());
        assert!(check(&f, json!("c")).is_err());
        // The label is not an accepted answer; the stored value is the option
        // value.
        assert!(check(&f, json!("A")).is_err());
    }

    #[test]
    fn multi_checks_membership_count_and_repeats() {
        let f = field(
            FieldType::Multi,
            false,
            Config::Multi {
                options: vec![
                    Opt {
                        value: "a".into(),
                        label: "A".into(),
                    },
                    Opt {
                        value: "b".into(),
                        label: "B".into(),
                    },
                    Opt {
                        value: "c".into(),
                        label: "C".into(),
                    },
                ],
                min: Some(1),
                max: Some(2),
            },
        );
        assert!(check(&f, json!(["a"])).is_ok());
        assert!(check(&f, json!(["a", "b"])).is_ok());
        assert!(check(&f, json!(["a", "b", "c"])).is_err());
        assert!(check(&f, json!(["a", "z"])).is_err());
        assert!(check(&f, json!(["a", "a"])).is_err());
        assert!(check(&f, json!("a")).is_err());
    }

    // ---- datetime ----

    #[test]
    fn datetime_requires_an_offset() {
        let f = field(
            FieldType::DateTime,
            false,
            Config::DateTime {
                fill_from: vec![],
                no_future: false,
            },
        );
        assert!(check(&f, json!("2026-10-28T07:42:00+05:30")).is_ok());
        assert!(check(&f, json!("2026-10-28T07:42:00Z")).is_ok());
        // No offset, so the instant is ambiguous.
        assert!(check(&f, json!("2026-10-28T07:42:00")).is_err());
        assert!(check(&f, json!("28-10-2026")).is_err());
    }

    #[test]
    fn datetime_honours_no_future() {
        let f = field(
            FieldType::DateTime,
            false,
            Config::DateTime {
                fill_from: vec![],
                no_future: true,
            },
        );
        assert!(check(&f, json!("2026-10-28T07:42:00+05:30")).is_ok());
        assert!(check(&f, json!("2027-01-01T00:00:00Z")).is_err());
    }

    #[test]
    fn no_future_compares_instants_not_wall_clock() {
        // 13:00+05:30 is 07:30 UTC, which is before `now` (12:00 UTC) even
        // though its local reading is later.
        let f = field(
            FieldType::DateTime,
            false,
            Config::DateTime {
                fill_from: vec![],
                no_future: true,
            },
        );
        assert!(check(&f, json!("2026-10-28T13:00:00+05:30")).is_ok());
    }

    // ---- photo ----

    fn photo(bytes: u64) -> Value {
        json!({
            "cid": "logos:zb2rhPhotoA",
            "sha256": "cd".repeat(32),
            "mime": "image/jpeg",
            "bytes": bytes
        })
    }

    #[test]
    fn photo_enforces_count_and_size() {
        let f = field(
            FieldType::Photo,
            true,
            Config::Photo {
                min: Some(1),
                max: Some(2),
                max_mb_each: 1.0,
                fill_from_exif: vec![],
            },
        );
        assert!(check(&f, json!([photo(1024)])).is_ok());
        assert!(check(&f, json!([photo(1024), photo(1024)])).is_ok());
        assert!(check(&f, json!([photo(1024), photo(1024), photo(1024)])).is_err());
        // One byte over the 1 MB limit.
        assert!(check(&f, json!([photo(1024 * 1024 + 1)])).is_err());
        assert!(check(&f, json!([photo(1024 * 1024)])).is_ok());
    }

    #[test]
    fn photo_count_never_exceeds_the_contract_ceiling() {
        // A protocol asking for more than 10 cannot raise the ceiling.
        let f = field(
            FieldType::Photo,
            false,
            Config::Photo {
                min: None,
                max: Some(50),
                max_mb_each: 25.0,
                fill_from_exif: vec![],
            },
        );
        let eleven: Vec<Value> = (0..11).map(|_| photo(10)).collect();
        assert!(check(&f, Value::Array(eleven)).is_err());
    }

    #[test]
    fn photo_requires_its_media_members() {
        let f = field(
            FieldType::Photo,
            false,
            Config::Photo {
                min: None,
                max: None,
                max_mb_each: 25.0,
                fill_from_exif: vec![],
            },
        );
        for missing in ["cid", "sha256", "mime", "bytes"] {
            let mut p = photo(10);
            p.as_object_mut().unwrap().remove(missing);
            assert!(check(&f, json!([p])).is_err(), "{missing} was not required");
        }
    }

    // ---- location ----

    fn location_field(store: Store, max_accuracy_m: Option<f64>) -> Field {
        field(
            FieldType::Location,
            true,
            Config::Location {
                methods: vec![Method::Photo, Method::Manual],
                max_accuracy_m,
                store,
            },
        )
    }

    #[test]
    fn location_checks_range_and_method() {
        let f = location_field(Store::Exact, None);
        assert!(
            check(
                &f,
                json!({"lat": 12.97712, "lon": 77.59581, "method": "photo"})
            )
            .is_ok()
        );
        assert!(check(&f, json!({"lat": 91.0, "lon": 0.0, "method": "photo"})).is_err());
        assert!(check(&f, json!({"lat": 0.0, "lon": 181.0, "method": "photo"})).is_err());
        // `map` is not in this field's methods.
        assert!(check(&f, json!({"lat": 0.0, "lon": 0.0, "method": "map"})).is_err());
        assert!(check(&f, json!({"lat": 0.0, "lon": 0.0})).is_err());
    }

    #[test]
    fn location_enforces_max_accuracy() {
        let f = location_field(Store::Exact, Some(50.0));
        let at = |acc: Value| {
            let mut v = json!({"lat": 12.97712, "lon": 77.59581, "method": "photo"});
            v["accuracy_m"] = acc;
            check(&f, v)
        };
        assert!(at(json!(12)).is_ok());
        assert!(at(json!(50)).is_ok());
        assert!(at(json!(51)).is_err());
        // A field that asked for a bound cannot accept an answer that shows
        // none.
        assert!(check(&f, json!({"lat": 1.0, "lon": 1.0, "method": "photo"})).is_err());
        assert!(at(json!(null)).is_err());
    }

    #[test]
    fn rounded_field_rejects_an_exact_point() {
        // The privacy rule: all data is public and nothing can be deleted, so
        // an exact point under a rounded_1km protocol must never be indexed.
        let f = location_field(Store::Rounded1km, None);
        let exact = json!({
            "lat": 12.97712, "lon": 77.59581, "method": "photo", "rounded": true
        });
        assert!(check(&f, exact).is_err());
    }

    #[test]
    fn rounded_field_accepts_a_cell_centre() {
        let f = location_field(Store::Rounded1km, None);
        let (lat, lon) = geo::round_1km(12.97712, 77.59581);
        let v = json!({"lat": lat, "lon": lon, "method": "photo", "rounded": true});
        assert!(check(&f, v).is_ok());
    }

    #[test]
    fn rounded_field_requires_the_rounded_flag() {
        let f = location_field(Store::Rounded1km, None);
        let (lat, lon) = geo::round_1km(12.97712, 77.59581);
        let v = json!({"lat": lat, "lon": lon, "method": "photo"});
        assert!(check(&f, v).is_err());
        let v = json!({"lat": lat, "lon": lon, "method": "photo", "rounded": false});
        assert!(check(&f, v).is_err());
    }

    #[test]
    fn exact_field_accepts_an_exact_point() {
        let f = location_field(Store::Exact, None);
        let v = json!({"lat": 12.97712, "lon": 77.59581, "method": "photo", "rounded": false});
        assert!(check(&f, v).is_ok());
    }

    // ---- apply_store_rule ----

    #[test]
    fn store_rule_rounds_and_marks() {
        let f = location_field(Store::Rounded1km, None);
        let mut v = json!({
            "lat": 12.97712, "lon": 77.59581, "accuracy_m": 12, "method": "photo"
        });
        apply_store_rule(&f, &mut v);
        assert_eq!(v["lat"], json!(12.975));
        assert_eq!(v["lon"], json!(77.595));
        assert_eq!(v["rounded"], json!(true));
        // Accuracy describes the original measurement and is not location
        // data, so it is left alone.
        assert_eq!(v["accuracy_m"], json!(12));
        // What comes out must pass the check that rejected the input.
        assert!(check(&f, v).is_ok());
    }

    #[test]
    fn store_rule_leaves_an_exact_field_alone() {
        let f = location_field(Store::Exact, None);
        let before = json!({"lat": 12.97712, "lon": 77.59581, "method": "photo"});
        let mut after = before.clone();
        apply_store_rule(&f, &mut after);
        assert_eq!(before, after);
    }

    #[test]
    fn store_rule_is_idempotent() {
        let f = location_field(Store::Rounded1km, None);
        let mut once = json!({"lat": 12.97712, "lon": 77.59581, "method": "photo"});
        apply_store_rule(&f, &mut once);
        let mut twice = once.clone();
        apply_store_rule(&f, &mut twice);
        assert_eq!(once, twice);
    }

    // ---- yesno ----

    #[test]
    fn yesno_takes_only_booleans() {
        let f = field(FieldType::YesNo, false, Config::YesNo);
        assert!(check(&f, json!(true)).is_ok());
        assert!(check(&f, json!(false)).is_ok());
        assert!(check(&f, json!("yes")).is_err());
        assert!(check(&f, json!(1)).is_err());
    }

    #[test]
    fn false_is_an_answer_not_a_blank() {
        // A required yesno answered "no" must not report as missing.
        let fields = vec![field(FieldType::YesNo, true, Config::YesNo)];
        let report = validate_values(&fields, &json!({"f": false}), now());
        assert!(report.is_ok(), "{report:?}");
    }

    #[test]
    fn zero_is_an_answer_not_a_blank() {
        let fields = vec![field(
            FieldType::Number,
            true,
            Config::Number {
                min: None,
                max: None,
                integer: false,
                unit: None,
            },
        )];
        let report = validate_values(&fields, &json!({"f": 0}), now());
        assert!(report.is_ok(), "{report:?}");
    }

    #[test]
    fn report_collects_every_field_not_just_the_first() {
        let mut values = example_values();
        values["count"] = json!(-1);
        values["category"] = json!("not-an-option");
        values.as_object_mut().unwrap().remove("photos");
        let report = validate_values(&example_fields(), &values, now());
        assert_eq!(report.missing, ["photos"]);
        assert!(report.errors.contains_key("count"), "{report:?}");
        assert!(report.errors.contains_key("category"), "{report:?}");
    }
}
