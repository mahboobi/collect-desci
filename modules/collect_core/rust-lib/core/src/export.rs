//! Turning a results table into CSV or JSON.
//!
//! Pure: in goes the `columns` and `rows` shape `session::Session::results`
//! already produces, out comes text. That reuse is the point — an export that
//! built its own view of the data could disagree with the table the creator is
//! looking at, and the first thing anyone would notice is a hidden submitter
//! reappearing in a spreadsheet.
//!
//! ## Why cells are escaped twice over
//!
//! Submitters control the text in these cells. RFC 4180 quoting stops a comma
//! or a newline from breaking the row shape. It does **not** stop a cell
//! beginning `=`, `+`, `-` or `@` from being run as a formula when the file is
//! opened in a spreadsheet — which is a submitter-supplied string executing on
//! the creator's machine. So such a cell is additionally prefixed with an
//! apostrophe. That does change the value, which is why it is done only to the
//! cells that need it, and why the JSON export — which no spreadsheet
//! evaluates — leaves every value exactly as submitted.

use serde_json::{json, Value};

/// Which file to produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Csv,
    Json,
}

impl Format {
    pub fn parse(name: &str) -> Option<Format> {
        match name.trim().to_ascii_lowercase().as_str() {
            "csv" => Some(Format::Csv),
            "json" => Some(Format::Json),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Json => "json",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Format::Csv => "text/csv",
            Format::Json => "application/json",
        }
    }
}

/// The columns every export carries before the protocol's own fields.
///
/// Provenance first, because a row without it cannot be checked against the
/// index later: the CID is what the creator signed for, and `submitted_at`
/// versus `received_at` is the difference between when it was filled in and
/// when it arrived.
const PROVENANCE: [(&str, &str); 6] = [
    ("cid", "Document CID"),
    ("submitter", "Submitter key"),
    ("submitter_name", "Submitter name"),
    ("version", "Form version"),
    ("submitted_at", "Submitted at"),
    ("received_at", "Received at"),
];

/// Render a results table as CSV.
pub fn csv(columns: &[Value], rows: &[Value]) -> String {
    let mut out = String::new();

    let header: Vec<String> = PROVENANCE
        .iter()
        .map(|(_, label)| (*label).to_string())
        .chain(columns.iter().map(|c| label_of(c)))
        .collect();
    out.push_str(&header.iter().map(|h| cell(h)).collect::<Vec<_>>().join(","));
    out.push_str("\r\n");

    for row in rows {
        let mut line: Vec<String> = PROVENANCE
            .iter()
            .map(|(key, _)| cell(&scalar(row.get(*key).unwrap_or(&Value::Null))))
            .collect();
        for column in columns {
            let key = column.get("key").and_then(Value::as_str).unwrap_or("");
            let value = row.pointer(&format!("/values/{}", pointer_escape(key)));
            line.push(cell(&flatten(value.unwrap_or(&Value::Null), column)));
        }
        out.push_str(&line.join(","));
        // CRLF: RFC 4180's line ending, and the one Excel reads without
        // guessing.
        out.push_str("\r\n");
    }
    out
}

/// Render a results table as JSON.
///
/// Values are untouched — no flattening, no apostrophes — because this is the
/// format for re-reading the data, not for a spreadsheet. The columns travel
/// with it so a reader knows the shape of each value without the protocol.
pub fn json_export(protocol: &Value, columns: &[Value], rows: &[Value]) -> String {
    let doc = json!({
        "format": "collect/export@1",
        "protocol": protocol,
        "columns": columns,
        "rows": rows,
    });
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".to_string())
}

/// A filename for an export, safe on every platform we might land on.
pub fn filename(experiment: &str, protocol: &str, format: Format, at: &str) -> String {
    let day: String = at.chars().take(10).filter(|c| *c != '-').collect();
    let stem = format!("collect-{experiment}-{protocol}-{day}");
    let safe: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    format!("{}.{}", safe.trim_matches('-'), format.extension())
}

fn label_of(column: &Value) -> String {
    column
        .get("label")
        .and_then(Value::as_str)
        .filter(|l| !l.is_empty())
        .or_else(|| column.get("key").and_then(Value::as_str))
        .unwrap_or("")
        .to_string()
}

/// JSON Pointer escaping, for a field key used as a pointer segment. Field
/// keys cannot contain `/` or `~` by the contract, but building a pointer from
/// untrusted text without escaping it is the kind of thing that stops being
/// true after one schema change.
fn pointer_escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// One value as a single cell.
///
/// Multi-choice joins with `; ` rather than `,` so it stays one CSV field even
/// unquoted; a location becomes `lat, lon`; photos become their CIDs, which is
/// what ties a row to the files beside it in a media export.
fn flatten(value: &Value, column: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Array(items) => items
            .iter()
            .map(|i| flatten_one(i, column))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("; "),
        other => flatten_one(other, column),
    }
}

fn flatten_one(value: &Value, column: &Value) -> String {
    match value {
        Value::Object(map) => {
            // A location.
            if let (Some(lat), Some(lon)) = (
                map.get("lat").and_then(Value::as_f64),
                map.get("lon").and_then(Value::as_f64),
            ) {
                return format!("{lat}, {lon}");
            }
            // A photo.
            if let Some(cid) = map.get("cid").and_then(Value::as_str) {
                return cid.to_string();
            }
            value.to_string()
        }
        Value::String(s) => label_for_option(s, column),
        other => scalar(other),
    }
}

/// A stored choice value, shown as its label.
///
/// Falls back to the raw value when no option matches, because a value whose
/// option was removed in a later form version is still what the submitter
/// chose — dropping it would lose data that the submission still holds.
fn label_for_option(value: &str, column: &Value) -> String {
    column
        .get("options")
        .and_then(Value::as_array)
        .and_then(|options| {
            options
                .iter()
                .find(|o| o.get("value").and_then(Value::as_str) == Some(value))
                .and_then(|o| o.get("label").and_then(Value::as_str))
        })
        .unwrap_or(value)
        .to_string()
}

/// A JSON scalar as plain text, without JSON's quoting.
fn scalar(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => if *b { "yes" } else { "no" }.to_string(),
        other => other.to_string(),
    }
}

/// One CSV field: RFC 4180 quoting, plus the formula guard.
fn cell(text: &str) -> String {
    let guarded = if text.starts_with(['=', '+', '@'])
        || (text.starts_with('-') && !looks_numeric(text))
        // Tab and CR are the other two leads a spreadsheet treats as a
        // formula start once it strips them.
        || text.starts_with(['\t', '\r'])
    {
        format!("'{text}")
    } else {
        text.to_string()
    };

    if guarded.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

/// Whether a leading `-` is a minus sign rather than a formula.
///
/// Without this every negative number in a scientific dataset would come out
/// as text with an apostrophe in front of it, which is a worse outcome than
/// the risk it guards against.
fn looks_numeric(text: &str) -> bool {
    text.parse::<f64>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn columns() -> Vec<Value> {
        vec![
            json!({ "key": "count", "label": "How many", "type": "number", "options": null }),
            json!({ "key": "species", "label": "Species", "type": "choice", "options": [
                { "value": "crow", "label": "House crow" },
                { "value": "myna", "label": "Common myna" }
            ]}),
            json!({ "key": "notes", "label": "Notes", "type": "longtext", "options": null }),
        ]
    }

    fn row() -> Value {
        json!({
            "cid": "logos:zAbc123",
            "submitter": "a1".repeat(32),
            "submitter_name": "Priya",
            "version": 2,
            "submitted_at": "2026-10-25T09:00:00Z",
            "received_at": "2026-10-25T09:00:05Z",
            "values": { "count": 3, "species": "crow", "notes": "On the wire" }
        })
    }

    fn lines(text: &str) -> Vec<&str> {
        text.trim_end_matches("\r\n").split("\r\n").collect()
    }

    #[test]
    fn the_header_is_provenance_then_the_forms_own_fields() {
        let out = csv(&columns(), &[]);
        assert_eq!(
            lines(&out)[0],
            "Document CID,Submitter key,Submitter name,Form version,Submitted at,Received at,How many,Species,Notes"
        );
    }

    #[test]
    fn a_row_follows_the_header_in_order() {
        let out = csv(&columns(), &[row()]);
        let body = lines(&out)[1];
        assert!(body.starts_with("logos:zAbc123,"), "{body}");
        assert!(body.ends_with(",3,House crow,On the wire"), "{body}");
    }

    #[test]
    fn a_choice_is_exported_as_its_label() {
        let out = csv(&columns(), &[row()]);
        assert!(lines(&out)[1].contains("House crow"));
    }

    #[test]
    fn a_choice_whose_option_was_removed_keeps_its_stored_value() {
        // The submitter did choose it; a later form version dropping the
        // option does not unmake that.
        let mut r = row();
        r["values"]["species"] = json!("sparrow");
        let out = csv(&columns(), &[r]);
        assert!(lines(&out)[1].contains("sparrow"));
    }

    #[test]
    fn a_comma_or_a_quote_or_a_newline_cannot_break_the_row_shape() {
        let mut r = row();
        r["values"]["notes"] = json!("Two, maybe \"three\"\nby the gate");
        let out = csv(&columns(), &[r]);
        // One header row and one data row, however many newlines the note
        // contains.
        assert_eq!(out.matches("\r\n").count(), 2, "{out}");
        assert!(out.contains("\"Two, maybe \"\"three\"\"\nby the gate\""), "{out}");
    }

    #[test]
    fn a_cell_that_would_run_as_a_formula_is_defused() {
        // A submitter-controlled string executing on the creator's machine
        // when they open the file.
        let mut r = row();
        r["values"]["notes"] = json!("=HYPERLINK(\"http://evil\",\"click\")");
        let out = csv(&columns(), &[r]);
        assert!(out.contains("\"'=HYPERLINK("), "{out}");

        for lead in ["+1+1", "@SUM(A1)", "\tcmd"] {
            let mut r = row();
            r["values"]["notes"] = json!(lead);
            assert!(csv(&columns(), &[r]).contains(&format!("'{lead}")), "{lead}");
        }
    }

    #[test]
    fn a_negative_number_is_not_mistaken_for_a_formula() {
        // Otherwise every negative measurement in a scientific dataset comes
        // out as apostrophe-prefixed text.
        let mut r = row();
        r["values"]["count"] = json!(-4);
        let out = csv(&columns(), &[r]);
        assert!(out.contains(",-4,"), "{out}");
        assert!(!out.contains("'-4"), "{out}");

        // But a formula disguised with a leading minus is still defused.
        let mut r = row();
        r["values"]["notes"] = json!("-1+cmd|'/bin/sh'!A1");
        assert!(csv(&columns(), &[r]).contains("'-1+cmd"), "{out}");
    }

    #[test]
    fn multiple_values_stay_in_one_field() {
        let mut r = row();
        r["values"]["species"] = json!(["crow", "myna"]);
        let out = csv(&columns(), &[r]);
        // Joined with a semicolon, so the field needs no quoting at all.
        assert!(lines(&out)[1].contains("House crow; Common myna"), "{out}");
    }

    #[test]
    fn a_location_becomes_a_coordinate_pair_and_a_photo_becomes_its_cid() {
        let cols = vec![
            json!({ "key": "where", "label": "Where", "type": "location", "options": null }),
            json!({ "key": "photos", "label": "Photos", "type": "photo", "options": null }),
        ];
        let r = json!({
            "values": {
                "where": { "lat": 12.985, "lon": 77.595, "precision": "rounded_1km" },
                "photos": [ { "cid": "logos:zPic1" }, { "cid": "logos:zPic2" } ]
            }
        });
        let out = csv(&cols, &[r]);
        assert!(out.contains("\"12.985, 77.595\""), "{out}");
        assert!(out.contains("logos:zPic1; logos:zPic2"), "{out}");
    }

    #[test]
    fn a_missing_value_is_an_empty_cell_not_the_word_null() {
        let mut r = row();
        r["values"] = json!({ "count": 1 });
        let out = csv(&columns(), &[r]);
        assert!(lines(&out)[1].ends_with(",1,,"), "{out}");
    }

    #[test]
    fn a_boolean_reads_as_yes_or_no() {
        let cols = vec![json!({ "key": "seen", "label": "Seen", "type": "yesno", "options": null })];
        let out = csv(&cols, &[json!({ "values": { "seen": true } })]);
        assert!(out.contains(",yes"), "{out}");
        let out = csv(&cols, &[json!({ "values": { "seen": false } })]);
        assert!(out.contains(",no"), "{out}");
    }

    #[test]
    fn json_keeps_every_value_exactly_as_submitted() {
        // No flattening and no apostrophes: nothing evaluates this file.
        let mut r = row();
        r["values"]["notes"] = json!("=HYPERLINK(\"x\")");
        let text = json_export(&json!({ "id": "wildlife", "name": "Wildlife" }), &columns(), &[r]);
        let back: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(back["format"], json!("collect/export@1"));
        assert_eq!(back["rows"][0]["values"]["notes"], json!("=HYPERLINK(\"x\")"));
        assert_eq!(back["rows"][0]["values"]["count"], json!(3));
        assert_eq!(back["protocol"]["id"], json!("wildlife"));
        assert_eq!(back["columns"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn a_filename_says_what_and_when_and_is_safe_everywhere() {
        assert_eq!(
            filename("q7k2m3x4w3b5n6tz", "wildlife-sighting", Format::Csv, "2026-10-29T08:00:00Z"),
            "collect-q7k2m3x4w3b5n6tz-wildlife-sighting-20261029.csv"
        );
        assert_eq!(
            filename("a b/c", "x", Format::Json, "2026-10-29T08:00:00Z"),
            "collect-a-b-c-x-20261029.json"
        );
    }

    #[test]
    fn a_format_is_parsed_case_insensitively_and_nothing_else_is_accepted() {
        assert_eq!(Format::parse("CSV"), Some(Format::Csv));
        assert_eq!(Format::parse(" json "), Some(Format::Json));
        assert_eq!(Format::parse("xlsx"), None);
        assert_eq!(Format::parse(""), None);
    }
}
