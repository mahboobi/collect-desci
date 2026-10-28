//! Ed25519 signing over RFC 8785 canonical JSON.
//!
//! Every Collect document is signed by its author over the canonical form of
//! itself with the `sig` member removed, per `CLAUDE.md`. Keys and signatures
//! travel as lower-case hex, matching `key` and `sig` in the schema.

use crate::canonical;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde_json::Value;

pub const PUBLIC_KEY_LEN: usize = 32;
pub const SIGNATURE_LEN: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("document is not a JSON object")]
    NotAnObject,
    #[error("document has no `sig`")]
    MissingSignature,
    #[error("`sig` is not a hex string of {SIGNATURE_LEN} bytes")]
    MalformedSignature,
    #[error("`{field}` is not a hex string of {PUBLIC_KEY_LEN} bytes")]
    MalformedKey { field: &'static str },
    #[error("document does not carry its author's key; pass one explicitly")]
    NoAuthorKey,
    #[error("signature does not verify against key {key}")]
    BadSignature { key: String },
    #[error(transparent)]
    Canonical(#[from] canonical::Error),
}

/// The exact bytes a signature covers: canonical JSON of the document with
/// `sig` removed. Both signing and verification go through this, so they
/// cannot drift apart.
pub fn signing_payload(doc: &Value) -> Result<Vec<u8>, Error> {
    let obj = doc.as_object().ok_or(Error::NotAnObject)?;
    let mut stripped = obj.clone();
    stripped.remove("sig");
    canonical::to_bytes(&Value::Object(stripped)).map_err(Error::from)
}

/// Sign `doc` and return it with `sig` set to the lower-case hex signature.
///
/// Any existing `sig` is ignored for the payload and replaced in the result,
/// so re-signing an edited document is safe.
pub fn sign(doc: &Value, key: &SigningKey) -> Result<Value, Error> {
    let payload = signing_payload(doc)?;
    let sig = key.sign(&payload);
    let mut out = doc.as_object().ok_or(Error::NotAnObject)?.clone();
    out.insert("sig".into(), Value::String(hex::encode(sig.to_bytes())));
    Ok(Value::Object(out))
}

/// Verify `doc` against an explicit public key.
///
/// Required for `collect/protocol@1` and `collect/index@1`, which carry no
/// author key of their own — the creator key comes from the experiment
/// manifest, and checking against that is what ties them to the experiment.
pub fn verify_with(doc: &Value, public_key: &[u8; PUBLIC_KEY_LEN]) -> Result<(), Error> {
    let sig_hex = doc
        .get("sig")
        .and_then(Value::as_str)
        .ok_or(Error::MissingSignature)?;
    let sig_bytes: [u8; SIGNATURE_LEN] = hex_exact(sig_hex).ok_or(Error::MalformedSignature)?;

    let verifying = VerifyingKey::from_bytes(public_key).map_err(|_| Error::MalformedKey {
        field: "public key",
    })?;
    let payload = signing_payload(doc)?;

    // verify_strict rejects small-order public keys, which plain verify
    // accepts. Those keys would let one signature verify under several keys.
    verifying
        .verify_strict(&payload, &Signature::from_bytes(&sig_bytes))
        .map_err(|_| Error::BadSignature {
            key: hex::encode(public_key),
        })
}

/// Verify a document that carries its own author key, and return that key.
///
/// Works for `collect/experiment@1` (`creator.key`), `collect/submission@1`
/// (`submitter.key`) and announcements (`from`).
pub fn verify(doc: &Value) -> Result<[u8; PUBLIC_KEY_LEN], Error> {
    let (field, key) = author_key(doc)?;
    verify_with(doc, &key).map_err(|e| match e {
        // Surface which member held the unusable key.
        Error::MalformedKey { .. } => Error::MalformedKey { field },
        other => other,
    })?;
    Ok(key)
}

/// The author key a self-contained document carries, with the member it came
/// from. `Err(NoAuthorKey)` for protocol and index documents.
pub fn author_key(doc: &Value) -> Result<(&'static str, [u8; PUBLIC_KEY_LEN]), Error> {
    for (field, pointer) in [
        ("creator.key", "/creator/key"),
        ("submitter.key", "/submitter/key"),
        ("from", "/from"),
    ] {
        if let Some(raw) = doc.pointer(pointer).and_then(Value::as_str) {
            let key = hex_exact(raw).ok_or(Error::MalformedKey { field })?;
            return Ok((field, key));
        }
    }
    Err(Error::NoAuthorKey)
}

/// Decode lower-case hex into exactly `N` bytes, rejecting anything else.
///
/// The schema pins `key` and `sig` to lower-case hex of a fixed width, so an
/// upper-case or short string is a malformed document, not a lenient case.
fn hex_exact<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != N * 2 || !s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return None;
    }
    let mut out = [0u8; N];
    hex::decode_to_slice(s, &mut out).ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key_from_seed(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn doc() -> Value {
        json!({
            "schema": "collect/submission@1",
            "experiment": "q7k2m3x4w3b5n6tz",
            "submitter": { "key": "" },
            "values": { "count": 3 }
        })
    }

    /// A document whose `submitter.key` matches `key`.
    fn doc_for(key: &SigningKey) -> Value {
        let mut d = doc();
        d["submitter"]["key"] = json!(hex::encode(key.verifying_key().to_bytes()));
        d
    }

    #[test]
    fn signs_and_verifies_round_trip() {
        let key = key_from_seed(1);
        let signed = sign(&doc_for(&key), &key).unwrap();
        assert_eq!(verify(&signed).unwrap(), key.verifying_key().to_bytes());
    }

    #[test]
    fn signature_is_128_hex_characters() {
        let key = key_from_seed(1);
        let signed = sign(&doc_for(&key), &key).unwrap();
        let sig = signed["sig"].as_str().unwrap();
        assert_eq!(sig.len(), 128);
        assert!(sig.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    }

    #[test]
    fn payload_excludes_sig() {
        let key = key_from_seed(1);
        let unsigned = doc_for(&key);
        let signed = sign(&unsigned, &key).unwrap();
        // Adding sig must not change what the signature covers, otherwise no
        // signed document could ever verify.
        assert_eq!(
            signing_payload(&unsigned).unwrap(),
            signing_payload(&signed).unwrap()
        );
    }

    #[test]
    fn payload_is_canonical_regardless_of_member_order() {
        let a: Value = serde_json::from_str(r#"{"a":1,"b":2,"sig":"ab"}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"sig":"cd","b":2,"a":1}"#).unwrap();
        assert_eq!(signing_payload(&a).unwrap(), signing_payload(&b).unwrap());
    }

    #[test]
    fn resigning_replaces_an_existing_sig() {
        let key = key_from_seed(1);
        let once = sign(&doc_for(&key), &key).unwrap();
        let twice = sign(&once, &key).unwrap();
        assert_eq!(once["sig"], twice["sig"]);
        assert!(verify(&twice).is_ok());
    }

    #[test]
    fn rejects_a_tampered_value() {
        let key = key_from_seed(1);
        let mut signed = sign(&doc_for(&key), &key).unwrap();
        signed["values"]["count"] = json!(4);
        assert!(matches!(
            verify(&signed),
            Err(Error::BadSignature { .. })
        ));
    }

    #[test]
    fn rejects_an_added_member() {
        let key = key_from_seed(1);
        let mut signed = sign(&doc_for(&key), &key).unwrap();
        signed["app"] = json!("collect/9.9.9");
        assert!(matches!(verify(&signed), Err(Error::BadSignature { .. })));
    }

    #[test]
    fn rejects_a_removed_member() {
        let key = key_from_seed(1);
        let mut signed = sign(&doc_for(&key), &key).unwrap();
        signed.as_object_mut().unwrap().remove("experiment");
        assert!(matches!(verify(&signed), Err(Error::BadSignature { .. })));
    }

    #[test]
    fn rejects_the_wrong_key() {
        let signer = key_from_seed(1);
        let other = key_from_seed(2);
        let signed = sign(&doc_for(&signer), &signer).unwrap();
        let wrong = other.verifying_key().to_bytes();
        assert!(matches!(
            verify_with(&signed, &wrong),
            Err(Error::BadSignature { .. })
        ));
    }

    #[test]
    fn rejects_a_signature_from_another_document() {
        let key = key_from_seed(1);
        let mut a = doc_for(&key);
        a["values"]["count"] = json!(1);
        let mut b = doc_for(&key);
        b["values"]["count"] = json!(2);
        let signed_a = sign(&a, &key).unwrap();
        // Lift a's signature onto b: same author, different content.
        b["sig"] = signed_a["sig"].clone();
        assert!(matches!(verify(&b), Err(Error::BadSignature { .. })));
    }

    #[test]
    fn rejects_missing_or_malformed_sig() {
        let key = key_from_seed(1);
        let unsigned = doc_for(&key);
        assert!(matches!(
            verify(&unsigned),
            Err(Error::MissingSignature)
        ));

        let mut short = sign(&unsigned, &key).unwrap();
        short["sig"] = json!("abcd");
        assert!(matches!(verify(&short), Err(Error::MalformedSignature)));

        let mut upper = sign(&unsigned, &key).unwrap();
        upper["sig"] = json!(upper["sig"].as_str().unwrap().to_uppercase());
        assert!(matches!(verify(&upper), Err(Error::MalformedSignature)));

        let mut not_hex = sign(&unsigned, &key).unwrap();
        not_hex["sig"] = json!("z".repeat(128));
        assert!(matches!(verify(&not_hex), Err(Error::MalformedSignature)));
    }

    #[test]
    fn finds_the_author_key_for_each_self_contained_shape() {
        let k = hex::encode([0xabu8; 32]);
        assert_eq!(
            author_key(&json!({"creator": {"key": k.clone()}})).unwrap().0,
            "creator.key"
        );
        assert_eq!(
            author_key(&json!({"submitter": {"key": k.clone()}})).unwrap().0,
            "submitter.key"
        );
        assert_eq!(author_key(&json!({"from": k})).unwrap().0, "from");
    }

    #[test]
    fn protocol_and_index_need_an_explicit_key() {
        // Neither shape carries an author key; the creator key comes from the
        // manifest, and verify() must refuse to guess.
        let protocol = json!({"schema": "collect/protocol@1", "experiment": "q7k2m3x4w3b5n6tz"});
        assert!(matches!(verify(&protocol), Err(Error::NoAuthorKey)));

        let key = key_from_seed(7);
        let signed = sign(&protocol, &key).unwrap();
        let creator = key.verifying_key().to_bytes();
        assert!(verify_with(&signed, &creator).is_ok());
    }

    #[test]
    fn rejects_a_non_object_document() {
        assert!(matches!(
            signing_payload(&json!([1, 2, 3])),
            Err(Error::NotAnObject)
        ));
    }

    #[test]
    fn hex_exact_is_strict() {
        assert!(hex_exact::<32>(&"a".repeat(64)).is_some());
        assert!(hex_exact::<32>(&"a".repeat(63)).is_none());
        assert!(hex_exact::<32>(&"a".repeat(65)).is_none());
        assert!(hex_exact::<32>(&"A".repeat(64)).is_none());
        assert!(hex_exact::<32>("").is_none());
    }
}
