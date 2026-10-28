//! RFC 8785 (JCS) canonical JSON.
//!
//! Documents are signed over their canonical form with `sig` removed, so this
//! must be byte-stable across machines and releases. The three rules that do
//! the work: object members sorted by UTF-16 code unit, numbers rendered as
//! ECMAScript `Number::toString`, strings escaped minimally.

use serde_json::Value;
use std::fmt::Write as _;

/// Serialize `value` to its RFC 8785 canonical form.
pub fn to_string(value: &Value) -> Result<String, Error> {
    let mut out = String::new();
    write_value(value, &mut out)?;
    Ok(out)
}

/// Canonical form as bytes — what we actually sign and hash.
pub fn to_bytes(value: &Value) -> Result<Vec<u8>, Error> {
    to_string(value).map(String::into_bytes)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// RFC 8785 rules out NaN and the infinities; they have no canonical form.
    #[error("number {0} is not representable in canonical JSON")]
    NonFiniteNumber(String),
}

fn write_value(value: &Value, out: &mut String) -> Result<(), Error> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => write_number(n, out)?,
        Value::String(s) => write_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out)?;
            }
            out.push(']');
        }
        Value::Object(map) => {
            // Sort by UTF-16 code unit, per RFC 8785. Not UTF-8 byte order:
            // the two disagree for code points at or above U+10000, whose
            // surrogates sort below BMP characters from U+E000 up.
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| utf16_cmp(a, b));
            out.push('{');
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(key, out);
                out.push(':');
                write_value(&map[key.as_str()], out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

fn write_number(n: &serde_json::Number, out: &mut String) -> Result<(), Error> {
    // Integers that arrived as integers stay integers — no float round-trip,
    // so u64/i64 values beyond 2^53 keep their exact digits.
    if let Some(u) = n.as_u64() {
        let _ = write!(out, "{u}");
        return Ok(());
    }
    if let Some(i) = n.as_i64() {
        let _ = write!(out, "{i}");
        return Ok(());
    }
    let f = n
        .as_f64()
        .ok_or_else(|| Error::NonFiniteNumber(n.to_string()))?;
    if !f.is_finite() {
        return Err(Error::NonFiniteNumber(n.to_string()));
    }
    // ryu-js renders exactly what ECMAScript `Number::toString` would, which
    // is what RFC 8785 defers to — including `1e+30` and `5e-324`.
    let mut buf = ryu_js::Buffer::new();
    out.push_str(buf.format_finite(f));
    Ok(())
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Everything else below U+0020 takes the \u00xx form; nothing
            // above it is escaped, so the output stays UTF-8.
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn utf16_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    // Fast path: while both strings are ASCII, UTF-8 byte order already
    // matches UTF-16. Only pay for re-encoding when they are not.
    if a.is_ascii() && b.is_ascii() {
        return a.as_bytes().cmp(b.as_bytes());
    }
    a.encode_utf16().cmp(b.encode_utf16())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn canon(v: Value) -> String {
        to_string(&v).unwrap()
    }

    /// The two-character JSON escape for `code`, e.g. 7 -> backslash + "u0007".
    /// Built rather than written literally so the source carries no raw
    /// control bytes and no ambiguous escape sequences.
    fn esc(code: u32) -> String {
        format!("{}u{:04x}", '\\', code)
    }

    fn ch(code: u32) -> char {
        char::from_u32(code).unwrap()
    }

    #[test]
    fn sorts_object_keys() {
        assert_eq!(canon(json!({"b": 1, "a": 2})), r#"{"a":2,"b":1}"#);
    }

    #[test]
    fn sorts_nested_object_keys() {
        assert_eq!(
            canon(json!({"z": {"y": 1, "x": 2}, "a": 3})),
            r#"{"a":3,"z":{"x":2,"y":1}}"#
        );
    }

    #[test]
    fn preserves_array_order() {
        assert_eq!(canon(json!([3, 1, 2])), "[3,1,2]");
    }

    #[test]
    fn integers_have_no_decimal_point() {
        assert_eq!(canon(json!(3)), "3");
        assert_eq!(canon(json!(3.0)), "3");
        assert_eq!(canon(json!(-0.0)), "0");
    }

    #[test]
    fn large_integers_keep_exact_digits() {
        // Beyond 2^53 an f64 round-trip would corrupt these.
        assert_eq!(canon(json!(9007199254740993u64)), "9007199254740993");
        assert_eq!(
            canon(json!(18446744073709551615u64)),
            "18446744073709551615"
        );
    }

    #[test]
    fn decimals_use_shortest_round_trip() {
        assert_eq!(canon(json!(12.97712)), "12.97712");
        assert_eq!(canon(json!(0.1)), "0.1");
    }

    #[test]
    fn exponents_follow_ecmascript() {
        // RFC 8785 defers to Number::toString, which switches to exponent
        // form at 1e21 and for very small magnitudes.
        assert_eq!(canon(json!(1e30)), "1e+30");
        assert_eq!(canon(json!(1e-7)), "1e-7");
        assert_eq!(canon(json!(5e-324)), "5e-324");
    }

    #[test]
    fn escapes_only_what_json_requires() {
        assert_eq!(canon(json!("a\"b\\c")), r#""a\"b\\c""#);
        assert_eq!(canon(json!("tab\there")), r#""tab\there""#);
        assert_eq!(canon(json!("nl\nhere")), r#""nl\nhere""#);
        assert_eq!(canon(json!("bell\u{07}")), format!("\"bell{}\"", esc(7)));
        // Non-ASCII is emitted literally, never as an escape.
        assert_eq!(canon(json!("h\u{e9}llo \u{2603}")), "\"h\u{e9}llo \u{2603}\"");
    }

    #[test]
    fn control_character_escape_table() {
        assert_eq!(canon(json!(String::from(ch(0x0)))), format!("\"{}\"", esc(0x0)));
        assert_eq!(canon(json!(String::from(ch(0x1)))), format!("\"{}\"", esc(0x1)));
        assert_eq!(canon(json!(String::from(ch(0x1f)))), format!("\"{}\"", esc(0x1f)));
        assert_eq!(canon(json!("\u{20}")), r#"" ""#);
        // U+007F is not a JSON escape case, so it passes through.
        assert_eq!(canon(json!("\u{7f}")), "\"\u{7f}\"");
    }

    #[test]
    fn sorts_astral_keys_by_utf16_not_utf8() {
        // U+10000 encodes as the surrogate pair D800 DC00, which sorts BELOW
        // U+E000 in UTF-16 but ABOVE it in UTF-8 byte order. Getting this
        // backwards would produce a different signing payload.
        let v = json!({"\u{10000}": 1, "\u{e000}": 2});
        assert_eq!(
            canon(v),
            format!("{{{}{}{}:1,{}{}{}:2}}", '"', ch(0x10000), '"', '"', ch(0xe000), '"')
        );
    }

    #[test]
    fn rfc8785_worked_example() {
        // The worked example from the RFC, covering number formatting and the
        // full string escape table in one document.
        // Every escaped character goes through esc(), so the source holds no
        // raw control bytes and no literal backslash runs to miscount. The
        // string field decodes to: euro, $, U+000F, LF, A, ', B, ", \, \, ", /
        let raw = format!(
            concat!(
                "{{",
                "  \"numbers\": [333333333.33333329, 1E30, 4.50, 2e-3, 1e-27],",
                "  \"string\": \"{euro}${f}{nl}A'{b}{q}{bs}{bs}{q}/\",",
                "  \"literals\": [null, true, false]",
                "}}"
            ),
            euro = esc(0x20ac),
            f = esc(0x0f),
            nl = esc(0x0a),
            b = esc(0x42),
            q = esc(0x22),
            bs = esc(0x5c),
        );
        let input: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            to_string(&input).unwrap(),
            concat!(
                r#"{"literals":[null,true,false],"#,
                r#""numbers":[333333333.3333333,1e+30,4.5,0.002,1e-27],"#,
                "\"string\":\"\u{20ac}$\\u000f\\nA'B\\\"\\\\\\\\\\\"/\"}"
            )
        );
    }

    #[test]
    fn is_stable_across_input_key_order() {
        let a: Value = serde_json::from_str(r#"{"a":1,"b":{"c":2,"d":3}}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"b":{"d":3,"c":2},"a":1}"#).unwrap();
        assert_eq!(to_string(&a).unwrap(), to_string(&b).unwrap());
    }
}
