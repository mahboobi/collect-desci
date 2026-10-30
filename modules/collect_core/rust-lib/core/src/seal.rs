//! Sealing a profile for one project member.
//!
//! Profiles are shared per field with the other members of a project, over
//! the experiment's Delivery topic — which anyone holding the link can read.
//! So a shared field is encrypted once per recipient, and only that recipient
//! can open it.
//!
//! ## The scheme
//!
//! Each installation keeps a second key next to its Ed25519 signing key: an
//! X25519 "box" key, kept in the same per-instance directory. The box public
//! key travels inside signed announcements, so the signature is what binds a
//! box key to a member's Ed25519 identity — nobody can claim someone else's.
//!
//! For each recipient:
//!
//! 1. X25519 between our box secret and their box public key. A
//!    non-contributory result (a low-order point) is refused.
//! 2. HKDF-SHA256 over that secret, with an `info` naming the purpose, both
//!    box keys and both identities, so a key derived for one pair and one
//!    purpose cannot be reused for another.
//! 3. ChaCha20-Poly1305 with a fresh random 96-bit nonce, and the experiment
//!    id plus both identities as associated data, so a sealed entry lifted
//!    into another experiment's announcement, or re-addressed, fails to open.
//!
//! What this does not hide: who shares with whom, when, and roughly how much.
//! The announcement is public and lists recipients by key.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use data_encoding::BASE64;
use hkdf::Hkdf;
use serde_json::{json, Value};
use sha2::Sha256;
use std::path::{Path, PathBuf};
use x25519_dalek::{PublicKey, StaticSecret};

/// Filename inside the persistence directory, beside `identity.ed25519`.
const KEY_FILE: &str = "identity.x25519";

/// Domain separation for the key derivation. Bump the suffix if the sealed
/// format ever changes, so old and new can never be confused.
const INFO: &[u8] = b"collect/profile-seal@1";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("could not read or write {0}")]
    Io(String),
    #[error("{0} is not a 32-byte X25519 key")]
    Malformed(String),
    #[error("`{0}` is not a 64-character hex X25519 public key")]
    BadPublicKey(String),
    #[error("the key exchange produced no shared secret (low-order key)")]
    NotContributory,
    #[error("the sealed entry is malformed: {0}")]
    BadEntry(String),
    #[error("the sealed entry does not open with this key")]
    DoesNotOpen,
}

/// This installation's box key.
pub struct BoxKey {
    secret: StaticSecret,
    path: PathBuf,
}

impl BoxKey {
    /// Load from `dir`, creating on first use. Same rules as the signing key:
    /// 32 random bytes, written owner-readable only.
    pub fn load_or_create(dir: &Path) -> Result<Self, Error> {
        let path = dir.join(KEY_FILE);
        if let Ok(bytes) = std::fs::read(&path) {
            let raw: [u8; 32] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| Error::Malformed(path.display().to_string()))?;
            return Ok(BoxKey { secret: StaticSecret::from(raw), path });
        }
        std::fs::create_dir_all(dir).map_err(|_| Error::Io(dir.display().to_string()))?;
        let mut raw = [0u8; 32];
        rand::fill(&mut raw);
        crate::profile::write_private(&path, &raw)
            .map_err(|_| Error::Io(path.display().to_string()))?;
        Ok(BoxKey { secret: StaticSecret::from(raw), path })
    }

    /// A key that lives only in memory. For tests and for a node that has no
    /// persistence path yet.
    pub fn ephemeral() -> Self {
        let mut raw = [0u8; 32];
        rand::fill(&mut raw);
        BoxKey { secret: StaticSecret::from(raw), path: PathBuf::new() }
    }

    pub fn public_hex(&self) -> String {
        hex::encode(PublicKey::from(&self.secret).as_bytes())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Seal `plain` for one recipient.
    ///
    /// `from` and `to` are Ed25519 identities (hex); `to_box` is the
    /// recipient's box public key as it arrived in their signed announcement.
    pub fn seal(
        &self,
        experiment: &str,
        from: &str,
        to: &str,
        to_box: &str,
        plain: &[u8],
    ) -> Result<Value, Error> {
        let theirs = parse_public(to_box)?;
        let cipher = self.cipher(&theirs, &self.public_hex(), to_box, from, to)?;
        let mut nonce = [0u8; 12];
        rand::fill(&mut nonce);
        let aad = associated(experiment, from, to);
        let ct = cipher
            .encrypt(&Nonce::from(nonce), Payload { msg: plain, aad: &aad })
            .map_err(|_| Error::BadEntry("encryption failed".into()))?;
        Ok(json!({
            "to": to,
            "nonce": hex::encode(nonce),
            "ct": BASE64.encode(&ct),
        }))
    }

    /// Open an entry sealed for us by `from`, whose box key is `from_box`.
    pub fn open(
        &self,
        experiment: &str,
        from: &str,
        from_box: &str,
        me: &str,
        entry: &Value,
    ) -> Result<Vec<u8>, Error> {
        let theirs = parse_public(from_box)?;
        let nonce_hex = entry.get("nonce").and_then(Value::as_str).unwrap_or("");
        let nonce: [u8; 12] = hex::decode(nonce_hex)
            .ok()
            .and_then(|b| b.as_slice().try_into().ok())
            .ok_or_else(|| Error::BadEntry("`nonce` is not 12 bytes of hex".into()))?;
        let ct = BASE64
            .decode(entry.get("ct").and_then(Value::as_str).unwrap_or("").as_bytes())
            .map_err(|_| Error::BadEntry("`ct` is not base64".into()))?;
        // Derived with the sender's box key first, exactly as they sealed it.
        let cipher = self.cipher(&theirs, from_box, &self.public_hex(), from, me)?;
        let aad = associated(experiment, from, me);
        cipher
            .decrypt(&Nonce::from(nonce), Payload { msg: &ct, aad: &aad })
            .map_err(|_| Error::DoesNotOpen)
    }

    fn cipher(
        &self,
        theirs: &PublicKey,
        sender_box: &str,
        recipient_box: &str,
        from: &str,
        to: &str,
    ) -> Result<ChaCha20Poly1305, Error> {
        let shared = self.secret.diffie_hellman(theirs);
        if !shared.was_contributory() {
            return Err(Error::NotContributory);
        }
        let mut info = INFO.to_vec();
        for part in [sender_box, recipient_box, from, to] {
            info.push(0);
            info.extend_from_slice(part.as_bytes());
        }
        let mut okm = [0u8; 32];
        Hkdf::<Sha256>::new(None, shared.as_bytes())
            .expand(&info, &mut okm)
            .map_err(|_| Error::BadEntry("key derivation failed".into()))?;
        ChaCha20Poly1305::new_from_slice(&okm).map_err(|_| Error::BadEntry("bad key length".into()))
    }
}

fn associated(experiment: &str, from: &str, to: &str) -> Vec<u8> {
    format!("{experiment}\0{from}\0{to}").into_bytes()
}

/// A box public key from the contract's lower-case hex.
pub fn parse_public(hex_key: &str) -> Result<PublicKey, Error> {
    let raw: [u8; 32] = hex::decode(hex_key)
        .ok()
        .filter(|_| hex_key.len() == 64 && hex_key.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')))
        .and_then(|b| b.as_slice().try_into().ok())
        .ok_or_else(|| Error::BadPublicKey(hex_key.to_string()))?;
    Ok(PublicKey::from(raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXP: &str = "q7k2m3x4w3b5n6tz";
    const ALICE: &str = "aa00000000000000000000000000000000000000000000000000000000000001";
    const BOB: &str = "bb00000000000000000000000000000000000000000000000000000000000002";
    const CAROL: &str = "cc00000000000000000000000000000000000000000000000000000000000003";

    #[test]
    fn a_sealed_entry_opens_for_its_recipient() {
        let alice = BoxKey::ephemeral();
        let bob = BoxKey::ephemeral();
        let entry = alice.seal(EXP, ALICE, BOB, &bob.public_hex(), b"phone: 123").unwrap();
        assert_eq!(entry["to"], json!(BOB));
        let plain = bob.open(EXP, ALICE, &alice.public_hex(), BOB, &entry).unwrap();
        assert_eq!(plain, b"phone: 123");
    }

    #[test]
    fn another_member_cannot_open_it() {
        let alice = BoxKey::ephemeral();
        let bob = BoxKey::ephemeral();
        let carol = BoxKey::ephemeral();
        let entry = alice.seal(EXP, ALICE, BOB, &bob.public_hex(), b"secret").unwrap();
        // Carol holds the entry (it is public) but not Bob's key.
        assert_eq!(
            carol.open(EXP, ALICE, &alice.public_hex(), BOB, &entry),
            Err(Error::DoesNotOpen)
        );
        assert_eq!(
            carol.open(EXP, ALICE, &alice.public_hex(), CAROL, &entry),
            Err(Error::DoesNotOpen)
        );
    }

    #[test]
    fn an_entry_moved_to_another_experiment_does_not_open() {
        let alice = BoxKey::ephemeral();
        let bob = BoxKey::ephemeral();
        let entry = alice.seal(EXP, ALICE, BOB, &bob.public_hex(), b"x").unwrap();
        assert_eq!(
            bob.open("zzzzzzzzzzzzzzzz", ALICE, &alice.public_hex(), BOB, &entry),
            Err(Error::DoesNotOpen)
        );
    }

    #[test]
    fn a_forged_sender_does_not_open() {
        // Same box key, but the announcement claims a different identity.
        let alice = BoxKey::ephemeral();
        let bob = BoxKey::ephemeral();
        let entry = alice.seal(EXP, ALICE, BOB, &bob.public_hex(), b"x").unwrap();
        assert_eq!(
            bob.open(EXP, CAROL, &alice.public_hex(), BOB, &entry),
            Err(Error::DoesNotOpen)
        );
    }

    #[test]
    fn a_tampered_ciphertext_does_not_open() {
        let alice = BoxKey::ephemeral();
        let bob = BoxKey::ephemeral();
        let mut entry = alice.seal(EXP, ALICE, BOB, &bob.public_hex(), b"hello").unwrap();
        let mut ct = BASE64.decode(entry["ct"].as_str().unwrap().as_bytes()).unwrap();
        ct[0] ^= 1;
        entry["ct"] = json!(BASE64.encode(&ct));
        assert_eq!(
            bob.open(EXP, ALICE, &alice.public_hex(), BOB, &entry),
            Err(Error::DoesNotOpen)
        );
    }

    #[test]
    fn nonces_are_not_reused() {
        let alice = BoxKey::ephemeral();
        let bob = BoxKey::ephemeral();
        let a = alice.seal(EXP, ALICE, BOB, &bob.public_hex(), b"same").unwrap();
        let b = alice.seal(EXP, ALICE, BOB, &bob.public_hex(), b"same").unwrap();
        assert_ne!(a["nonce"], b["nonce"]);
        assert_ne!(a["ct"], b["ct"]);
    }

    #[test]
    fn a_low_order_key_is_refused() {
        let alice = BoxKey::ephemeral();
        let zero = "00".repeat(32);
        assert_eq!(
            alice.seal(EXP, ALICE, BOB, &zero, b"x"),
            Err(Error::NotContributory)
        );
    }

    #[test]
    fn malformed_public_keys_are_refused() {
        assert!(parse_public("abc").is_err());
        assert!(parse_public(&"AA".repeat(32)).is_err(), "upper-case hex is not the contract's form");
        assert!(parse_public(&"0g".repeat(32)).is_err());
    }

    #[test]
    fn the_key_survives_a_reload() {
        let dir = std::env::temp_dir().join(format!("collect-seal-{}", uuid::Uuid::now_v7()));
        let first = BoxKey::load_or_create(&dir).unwrap();
        let again = BoxKey::load_or_create(&dir).unwrap();
        assert_eq!(first.public_hex(), again.public_hex());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
