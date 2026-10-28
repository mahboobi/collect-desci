//! Identifier derivation.

use data_encoding::BASE32_NOPAD;
use sha2::{Digest, Sha256};

/// Bytes of the SHA-256 digest that make up an experiment id. Ten bytes is
/// exactly 16 base32 characters, so the id needs no padding.
const EXPERIMENT_ID_BYTES: usize = 10;

/// Length of the random nonce mixed into an experiment id.
pub const NONCE_LEN: usize = 16;

/// Derive an experiment id: base32 of the first 10 bytes of
/// SHA-256(creator key || nonce), lower-case.
///
/// The nonce is what stops a creator's experiment ids from being guessable
/// from their public key alone.
pub fn experiment_id(creator_key: &[u8; 32], nonce: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(creator_key);
    hasher.update(nonce);
    let digest = hasher.finalize();
    BASE32_NOPAD
        .encode(&digest[..EXPERIMENT_ID_BYTES])
        .to_lowercase()
}

/// A fresh random nonce for `experiment_id`.
pub fn new_nonce() -> [u8; NONCE_LEN] {
    let mut nonce = [0u8; NONCE_LEN];
    rand::fill(&mut nonce);
    nonce
}

/// Does `s` look like an experiment id? Mirrors the schema's pattern.
pub fn is_experiment_id(s: &str) -> bool {
    s.len() == 16 && s.bytes().all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
}

/// Does `s` look like a protocol id? Mirrors the schema's pattern:
/// lower-case alphanumeric groups joined by single hyphens, up to 64 chars.
pub fn is_protocol_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.split('-').all(|part| {
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// Does `s` look like a field key or option value? Mirrors the schema's
/// pattern: a lower-case letter, then up to 39 more of letter/digit/underscore.
pub fn is_field_key(s: &str) -> bool {
    let mut bytes = s.bytes();
    match bytes.next() {
        Some(b) if b.is_ascii_lowercase() => {}
        _ => return false,
    }
    s.len() <= 40 && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Build the Delivery content topic for an experiment.
pub fn topic(experiment_id: &str) -> String {
    format!("/collect/1/exp-{experiment_id}/json")
}

/// Build the link a creator shares.
pub fn link(experiment_id: &str, manifest_cid: &str) -> String {
    format!("logos-collect://exp/{experiment_id}?m={manifest_cid}")
}

/// Pull the experiment id and manifest CID back out of a shared link.
pub fn parse_link(link: &str) -> Option<(String, String)> {
    let rest = link.strip_prefix("logos-collect://exp/")?;
    let (id, query) = rest.split_once('?')?;
    if !is_experiment_id(id) {
        return None;
    }
    // `m` is the only parameter v1 defines, but tolerate it appearing
    // alongside others rather than only as the first.
    let manifest = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("m="))?;
    if manifest.is_empty() {
        return None;
    }
    Some((id.to_string(), manifest.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn experiment_id_matches_the_schema_pattern() {
        let id = experiment_id(&[0x11; 32], &[0x22; NONCE_LEN]);
        assert_eq!(id.len(), 16);
        assert!(is_experiment_id(&id), "{id}");
    }

    #[test]
    fn experiment_id_is_deterministic() {
        let a = experiment_id(&[7; 32], b"same-nonce");
        let b = experiment_id(&[7; 32], b"same-nonce");
        assert_eq!(a, b);
    }

    #[test]
    fn experiment_id_varies_with_key_and_nonce() {
        let base = experiment_id(&[7; 32], b"nonce");
        assert_ne!(base, experiment_id(&[8; 32], b"nonce"));
        assert_ne!(base, experiment_id(&[7; 32], b"other"));
    }

    #[test]
    fn new_nonce_is_not_constant() {
        assert_ne!(new_nonce(), new_nonce());
    }

    #[test]
    fn rejects_ids_outside_the_base32_alphabet() {
        // Base32 has no 0, 1, 8 or 9, and the id is lower-case only.
        assert!(!is_experiment_id("q7k2m3x4w3b5n6t0"));
        assert!(!is_experiment_id("q7k2m3x4w3b5n6t1"));
        assert!(!is_experiment_id("q7k2m3x4w3b5n6t8"));
        assert!(!is_experiment_id("Q7K2M3X4W3B5N6TZ"));
        assert!(!is_experiment_id("q7k2m3x4w3b5n6t"));
        assert!(!is_experiment_id("q7k2m3x4w3b5n6tzz"));
        assert!(is_experiment_id("q7k2m3x4w3b5n6tz"));
    }

    #[test]
    fn validates_protocol_ids() {
        assert!(is_protocol_id("wildlife-sighting"));
        assert!(is_protocol_id("nest2"));
        assert!(!is_protocol_id("-leading"));
        assert!(!is_protocol_id("trailing-"));
        assert!(!is_protocol_id("double--hyphen"));
        assert!(!is_protocol_id("Upper"));
        assert!(!is_protocol_id("under_score"));
        assert!(!is_protocol_id(""));
        assert!(!is_protocol_id(&"a".repeat(65)));
    }

    #[test]
    fn validates_field_keys() {
        assert!(is_field_key("photos"));
        assert!(is_field_key("max_mb_each"));
        assert!(is_field_key("a1"));
        assert!(!is_field_key("1leading"));
        assert!(!is_field_key("_leading"));
        assert!(!is_field_key("Upper"));
        assert!(!is_field_key("has-hyphen"));
        assert!(!is_field_key(""));
        assert!(is_field_key(&"a".repeat(40)));
        assert!(!is_field_key(&"a".repeat(41)));
    }

    #[test]
    fn builds_the_delivery_topic() {
        assert_eq!(
            topic("q7k2m3x4w3b5n6tz"),
            "/collect/1/exp-q7k2m3x4w3b5n6tz/json"
        );
    }

    #[test]
    fn link_round_trips() {
        let l = link("q7k2m3x4w3b5n6tz", "logos:zb2rhManifestV4");
        assert_eq!(l, "logos-collect://exp/q7k2m3x4w3b5n6tz?m=logos:zb2rhManifestV4");
        assert_eq!(
            parse_link(&l),
            Some((
                "q7k2m3x4w3b5n6tz".to_string(),
                "logos:zb2rhManifestV4".to_string()
            ))
        );
    }

    #[test]
    fn parse_link_finds_m_among_other_parameters() {
        assert_eq!(
            parse_link("logos-collect://exp/q7k2m3x4w3b5n6tz?x=1&m=logos:abc")
                .map(|(_, cid)| cid),
            Some("logos:abc".to_string())
        );
    }

    #[test]
    fn parse_link_rejects_malformed_input() {
        assert!(parse_link("https://example.com/exp/q7k2m3x4w3b5n6tz?m=logos:abc").is_none());
        assert!(parse_link("logos-collect://exp/q7k2m3x4w3b5n6tz").is_none());
        assert!(parse_link("logos-collect://exp/TOOSHORT?m=logos:abc").is_none());
        assert!(parse_link("logos-collect://exp/q7k2m3x4w3b5n6tz?m=").is_none());
        assert!(parse_link("logos-collect://exp/q7k2m3x4w3b5n6tz?n=logos:abc").is_none());
    }
}
