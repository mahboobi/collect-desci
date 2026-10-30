//! The person behind this installation, as they choose to describe themselves.
//!
//! Kept in the per-instance directory and **never published**. Each field
//! carries its own `share` switch; a shared field leaves the machine only
//! sealed for a project's members (`seal`), never in the clear and never on
//! Logos Storage. Anything not shared stays here.
//!
//! Every field is optional, and validation only checks shape — an age is a
//! whole number in a human range, an email has an `@` and a domain — because
//! this is self-description, not verification.

use serde_json::{json, Map, Value};
use std::io::Cursor;
use std::path::{Path, PathBuf};

/// Filename inside the persistence directory.
const PROFILE_FILE: &str = "profile.json";

/// The fields, in display order. `photo` is handled apart: it arrives as a
/// file, not as text.
pub const FIELDS: &[&str] = &[
    "alias", "affiliation", "location", "sex", "age", "email", "phone", "wallet", "photo",
];

/// What `sex` may be. "prefer_not" is a real answer, distinct from blank.
pub const SEX_OPTIONS: &[&str] = &["female", "male", "intersex", "other", "prefer_not"];

/// The kept photo is a square this many pixels on a side. Small on purpose:
/// it is an avatar, and the shared copy has to fit a Delivery message once
/// per recipient.
const PHOTO_SIDE: u32 = 128;
const SHARED_PHOTO_SIDE: u32 = 64;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not read or write {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{0} is not a profile")]
    Malformed(String),
    #[error("the photo could not be used: {0}")]
    Photo(String),
}

/// Problems with a submitted profile, by field. Empty means it saved.
pub type Problems = Map<String, Value>;

pub struct Profile {
    /// `{ field: { "value": ..., "share": bool } }`. Absent means blank.
    fields: Map<String, Value>,
    path: Option<PathBuf>,
}

impl Default for Profile {
    fn default() -> Self {
        Profile { fields: Map::new(), path: None }
    }
}

impl Profile {
    /// Load from `dir`, or start blank. A corrupt file is an error rather
    /// than a silent reset: overwriting it would lose what someone typed.
    pub fn load(dir: &Path) -> Result<Self, Error> {
        let path = dir.join(PROFILE_FILE);
        match std::fs::read(&path) {
            Ok(bytes) => {
                let v: Value = serde_json::from_slice(&bytes)
                    .map_err(|_| Error::Malformed(path.display().to_string()))?;
                let fields = v
                    .get("fields")
                    .and_then(Value::as_object)
                    .cloned()
                    .ok_or_else(|| Error::Malformed(path.display().to_string()))?;
                Ok(Profile { fields, path: Some(path) })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(Profile { fields: Map::new(), path: Some(path) })
            }
            Err(source) => Err(Error::Io { path: path.display().to_string(), source }),
        }
    }

    /// The whole profile for this person's own screen.
    pub fn as_json(&self) -> Value {
        let mut out = Map::new();
        for name in FIELDS {
            out.insert(
                (*name).to_string(),
                self.fields
                    .get(*name)
                    .cloned()
                    .unwrap_or_else(|| json!({ "value": null, "share": false })),
            );
        }
        Value::Object(out)
    }

    /// The display name, if one is set. Local: used to label this person's
    /// own messages on their own screen.
    pub fn alias(&self) -> Option<String> {
        self.fields
            .get("alias")
            .and_then(|f| f.get("value"))
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    /// Replace the text fields from a `profile.save` request.
    ///
    /// All-or-nothing: if any field is wrong nothing changes, and the problems
    /// come back keyed by field so the screen can put each one in its place.
    /// `photo` is not taken from here — only its `share` switch is, since the
    /// image itself arrives through `set_photo`.
    pub fn update(&mut self, request: &Value) -> Result<(), Problems> {
        let incoming = request.get("fields").and_then(Value::as_object);
        let mut next = self.fields.clone();
        let mut problems = Problems::new();
        for name in FIELDS {
            let Some(entry) = incoming.and_then(|m| m.get(*name)) else {
                continue;
            };
            let share = entry.get("share").and_then(Value::as_bool).unwrap_or(false);
            if *name == "photo" {
                if let Some(existing) = next.get_mut("photo") {
                    existing["share"] = json!(share);
                }
                continue;
            }
            match clean(name, entry.get("value").unwrap_or(&Value::Null)) {
                Ok(Value::Null) => {
                    next.remove(*name);
                }
                Ok(value) => {
                    next.insert((*name).to_string(), json!({ "value": value, "share": share }));
                }
                Err(problem) => {
                    problems.insert((*name).to_string(), json!(problem));
                }
            }
        }
        if !problems.is_empty() {
            return Err(problems);
        }
        self.fields = next;
        Ok(())
    }

    /// Take a picked image file: strip its metadata, crop it square, shrink
    /// it, and keep the result. The original is never kept.
    pub fn set_photo(&mut self, original: &[u8], share: bool) -> Result<(), Error> {
        let uri = avatar(original, PHOTO_SIDE, 80)?;
        self.fields.insert("photo".into(), json!({ "value": uri, "share": share }));
        Ok(())
    }

    pub fn clear_photo(&mut self) {
        self.fields.remove("photo");
    }

    /// What this person has chosen to share, ready to seal. `None` when
    /// nothing is shared, so a node with nothing to say sends no sealed
    /// entries at all.
    pub fn shared(&self) -> Option<Value> {
        let mut out = Map::new();
        for (name, entry) in &self.fields {
            if entry.get("share").and_then(Value::as_bool) != Some(true) {
                continue;
            }
            let Some(value) = entry.get("value").filter(|v| !v.is_null()) else {
                continue;
            };
            if name == "photo" {
                // A smaller copy: this one is sent once per recipient.
                if let Some(small) = value
                    .as_str()
                    .and_then(data_uri_bytes)
                    .and_then(|b| avatar(&b, SHARED_PHOTO_SIDE, 70).ok())
                {
                    out.insert(name.clone(), json!(small));
                }
                continue;
            }
            out.insert(name.clone(), value.clone());
        }
        if out.is_empty() {
            None
        } else {
            Some(Value::Object(out))
        }
    }

    /// Write it back, owner-readable only.
    pub fn save(&self) -> Result<(), Error> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|source| Error::Io {
                path: dir.display().to_string(),
                source,
            })?;
        }
        let bytes = serde_json::to_vec_pretty(&json!({ "fields": self.fields }))
            .expect("a JSON map serializes");
        write_private(path, &bytes).map_err(|source| Error::Io {
            path: path.display().to_string(),
            source,
        })
    }
}

/// Check and normalize one field. `Ok(Null)` means "cleared".
fn clean(name: &str, raw: &Value) -> Result<Value, String> {
    if raw.is_null() || raw.as_str().map(|s| s.trim().is_empty()) == Some(true) {
        return Ok(Value::Null);
    }
    let text = |max: usize| -> Result<Value, String> {
        let s = raw
            .as_str()
            .ok_or_else(|| "Must be text".to_string())?
            .trim()
            .to_string();
        if s.chars().count() > max {
            return Err(format!("At most {max} characters"));
        }
        if s.chars().any(char::is_control) {
            return Err("Contains a control character".into());
        }
        Ok(json!(s))
    };
    match name {
        "alias" => text(80),
        "affiliation" => text(120),
        "location" => text(120),
        "wallet" => text(200),
        "sex" => {
            let s = raw.as_str().unwrap_or("");
            if SEX_OPTIONS.contains(&s) {
                Ok(json!(s))
            } else {
                Err("Pick one of the options".into())
            }
        }
        "age" => {
            let n = match raw {
                Value::Number(n) => n.as_f64(),
                Value::String(s) => s.trim().parse::<f64>().ok(),
                _ => None,
            }
            .ok_or_else(|| "Must be a number".to_string())?;
            if n.fract() != 0.0 || !(1.0..=130.0).contains(&n) {
                return Err("A whole number from 1 to 130".into());
            }
            Ok(json!(n as u64))
        }
        "email" => {
            let v = text(254)?;
            let s = v.as_str().unwrap_or("");
            let ok = match s.split_once('@') {
                Some((local, domain)) => {
                    !local.is_empty()
                        && domain.contains('.')
                        && !domain.starts_with('.')
                        && !domain.ends_with('.')
                        && !s.contains(char::is_whitespace)
                }
                None => false,
            };
            if ok {
                Ok(v)
            } else {
                Err("Not an email address".into())
            }
        }
        "phone" => {
            let v = text(32)?;
            let s = v.as_str().unwrap_or("");
            let digits = s.chars().filter(char::is_ascii_digit).count();
            let allowed = s
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '+' | ' ' | '-' | '(' | ')' | '.'));
            if allowed && (5..=17).contains(&digits) {
                Ok(v)
            } else {
                Err("Digits, spaces and + - ( ) only".into())
            }
        }
        _ => Err("Unknown field".into()),
    }
}

/// Strip, orient, crop square and shrink to a JPEG data URI.
///
/// `media::process` does the metadata removal and orientation, as it does for
/// submission photos. The re-encode below then guarantees nothing survives,
/// and the self-check confirms it.
fn avatar(original: &[u8], side: u32, quality: u8) -> Result<String, Error> {
    let cleaned = crate::media::process(original, 25.0).map_err(|e| Error::Photo(e.to_string()))?;
    let img = image::load_from_memory(&cleaned.bytes).map_err(|e| Error::Photo(e.to_string()))?;
    let (w, h) = (img.width(), img.height());
    let edge = w.min(h);
    let square = img.crop_imm((w - edge) / 2, (h - edge) / 2, edge, edge);
    let small = square.resize_exact(side, side, image::imageops::FilterType::Triangle).to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut Cursor::new(&mut out), quality)
        .encode_image(&small)
        .map_err(|e| Error::Photo(e.to_string()))?;
    if let Some(found) = crate::media::remaining_metadata(&out) {
        return Err(Error::Photo(format!("metadata survived: {found}")));
    }
    Ok(format!(
        "data:image/jpeg;base64,{}",
        data_encoding::BASE64.encode(&out)
    ))
}

fn data_uri_bytes(uri: &str) -> Option<Vec<u8>> {
    let b64 = uri.strip_prefix("data:image/jpeg;base64,")?;
    data_encoding::BASE64.decode(b64.as_bytes()).ok()
}

/// Is this a photo another member shared, in the only shape we accept?
/// The UI shows it as an image source, so nothing else may get through.
pub fn is_shared_photo(v: &Value) -> bool {
    v.as_str()
        .and_then(data_uri_bytes)
        .map(|b| b.len() <= 16 * 1024 && b.starts_with(&[0xFF, 0xD8]))
        .unwrap_or(false)
}

/// Check a profile another member shared before keeping it.
///
/// Their node validated it, but it arrived from the network: the same rules
/// apply here, unknown fields are dropped, and a photo must be a small JPEG.
pub fn accept_shared(raw: &Value) -> Option<Value> {
    let map = raw.as_object()?;
    let mut out = Map::new();
    for (name, value) in map {
        if name == "photo" {
            if is_shared_photo(value) {
                out.insert(name.clone(), value.clone());
            }
            continue;
        }
        if !FIELDS.contains(&name.as_str()) {
            continue;
        }
        if let Ok(v) = clean(name, value) {
            if !v.is_null() {
                out.insert(name.clone(), v);
            }
        }
    }
    Some(Value::Object(out))
}

/// Write a file owner-readable only. Shared by every key and personal file
/// this module keeps.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        std::env::temp_dir().join(format!("collect-profile-{}", uuid::Uuid::now_v7()))
    }

    fn save(p: &mut Profile, fields: Value) -> Result<(), Problems> {
        p.update(&json!({ "fields": fields }))
    }

    #[test]
    fn a_blank_profile_lists_every_field_unshared() {
        let p = Profile::default();
        let v = p.as_json();
        for name in FIELDS {
            assert_eq!(v[*name], json!({ "value": null, "share": false }), "{name}");
        }
        assert_eq!(p.shared(), None);
    }

    #[test]
    fn only_shared_fields_are_offered_for_sealing() {
        let mut p = Profile::default();
        save(&mut p, json!({
            "alias": { "value": "Mahboob", "share": true },
            "email": { "value": "m@example.org", "share": false },
            "age": { "value": "34", "share": true },
        }))
        .unwrap();
        assert_eq!(p.shared(), Some(json!({ "alias": "Mahboob", "age": 34 })));
    }

    #[test]
    fn a_bad_field_changes_nothing_and_says_where() {
        let mut p = Profile::default();
        save(&mut p, json!({ "alias": { "value": "Before", "share": false } })).unwrap();
        let problems = save(&mut p, json!({
            "alias": { "value": "After", "share": false },
            "age": { "value": 200, "share": false },
            "email": { "value": "nope", "share": false },
            "phone": { "value": "call me", "share": false },
            "sex": { "value": "unknown", "share": false },
        }))
        .unwrap_err();
        assert!(problems.contains_key("age"));
        assert!(problems.contains_key("email"));
        assert!(problems.contains_key("phone"));
        assert!(problems.contains_key("sex"));
        assert!(!problems.contains_key("alias"));
        assert_eq!(p.alias().as_deref(), Some("Before"), "a failed save half-applied");
    }

    #[test]
    fn clearing_a_field_removes_it() {
        let mut p = Profile::default();
        save(&mut p, json!({ "phone": { "value": "+91 98450 12345", "share": true } })).unwrap();
        save(&mut p, json!({ "phone": { "value": "  ", "share": true } })).unwrap();
        assert_eq!(p.as_json()["phone"]["value"], Value::Null);
        assert_eq!(p.shared(), None);
    }

    #[test]
    fn it_survives_a_reload_and_is_private() {
        let dir = temp();
        let mut p = Profile::load(&dir).unwrap();
        save(&mut p, json!({ "wallet": { "value": "lez1qxyz", "share": true } })).unwrap();
        p.save().unwrap();
        let again = Profile::load(&dir).unwrap();
        assert_eq!(again.as_json()["wallet"], json!({ "value": "lez1qxyz", "share": true }));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(PROFILE_FILE)).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "profile.json is readable by others");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_file_is_an_error_not_a_reset() {
        let dir = temp();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(PROFILE_FILE), b"{ not json").unwrap();
        assert!(Profile::load(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([200, 120, 40]));
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut Cursor::new(&mut out), 90)
            .encode_image(&img)
            .unwrap();
        out
    }

    #[test]
    fn a_photo_is_kept_small_square_and_clean() {
        let mut p = Profile::default();
        p.set_photo(&jpeg(400, 300), true).unwrap();
        let uri = p.as_json()["photo"]["value"].as_str().unwrap().to_string();
        let bytes = data_uri_bytes(&uri).unwrap();
        let img = image::load_from_memory(&bytes).unwrap();
        assert_eq!((img.width(), img.height()), (PHOTO_SIDE, PHOTO_SIDE));
        assert_eq!(crate::media::remaining_metadata(&bytes), None);
        let shared = p.shared().unwrap();
        assert!(is_shared_photo(&shared["photo"]), "the shared copy is not acceptable to a receiver");
        let small = image::load_from_memory(&data_uri_bytes(shared["photo"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(small.width(), SHARED_PHOTO_SIDE);
    }

    #[test]
    fn a_non_image_is_refused() {
        let mut p = Profile::default();
        assert!(p.set_photo(b"not an image at all", false).is_err());
        assert_eq!(p.as_json()["photo"]["value"], Value::Null);
    }

    #[test]
    fn received_profiles_are_rechecked() {
        let got = accept_shared(&json!({
            "alias": "Bob",
            "age": 999,
            "photo": "data:image/png;base64,AAAA",
            "nickname": "extra",
        }))
        .unwrap();
        assert_eq!(got, json!({ "alias": "Bob" }));
    }
}
