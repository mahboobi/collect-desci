//! The module's signing identity.
//!
//! `CLAUDE.md`: sign canonical JSON (RFC 8785) without the `sig` field,
//! Ed25519, using the module's identity key. Until Basecamp exposes an
//! identity API there is no key to borrow, so this one is generated on first
//! use and kept in the per-instance persistence directory the host stamps —
//! the only place that survives a relaunch (SPEC Q8).
//!
//! The consequence is worth stating plainly: documents signed by this key are
//! attributed to *this installation*, not to a Basecamp account. When a real
//! identity arrives, anything already published keeps the old key, so the
//! creator of an existing experiment cannot be re-pointed at the new one.

use crate::sign;
use ed25519_dalek::{SigningKey, SECRET_KEY_LENGTH};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Filename inside the persistence directory.
const KEY_FILE: &str = "identity.ed25519";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no persistence path: the host has not stamped the module context yet")]
    NoPersistencePath,
    #[error("could not read or write {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not a {SECRET_KEY_LENGTH}-byte Ed25519 key")]
    Malformed { path: String },
    #[error(transparent)]
    Sign(#[from] sign::Error),
}

pub struct Identity {
    key: SigningKey,
    path: PathBuf,
}

impl Identity {
    /// Load the key from `dir`, creating one on first use.
    pub fn load_or_create(dir: &Path) -> Result<Self, Error> {
        let path = dir.join(KEY_FILE);

        if let Ok(bytes) = std::fs::read(&path) {
            let raw: [u8; SECRET_KEY_LENGTH] =
                bytes.as_slice().try_into().map_err(|_| Error::Malformed {
                    path: path.display().to_string(),
                })?;
            return Ok(Identity {
                key: SigningKey::from_bytes(&raw),
                path,
            });
        }

        std::fs::create_dir_all(dir).map_err(|source| Error::Io {
            path: dir.display().to_string(),
            source,
        })?;

        // 32 bytes from the OS, then straight into a key. Avoids pinning a
        // second rand_core version alongside the one dalek links.
        let mut seed = [0u8; SECRET_KEY_LENGTH];
        rand::fill(&mut seed);
        let key = SigningKey::from_bytes(&seed);
        write_private(&path, &key.to_bytes())?;
        Ok(Identity { key, path })
    }

    /// The public key as the contract's lower-case hex.
    pub fn public_hex(&self) -> String {
        hex::encode(self.key.verifying_key().to_bytes())
    }

    /// Where the key lives, for a status surface to show.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Sign a document in place of its `sig`.
    pub fn sign(&self, doc: &Value) -> Result<Value, Error> {
        Ok(sign::sign(doc, &self.key)?)
    }
}

/// Write the key owner-readable only. It is the module's whole identity, and
/// the persistence directory is an ordinary directory on disk.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let io = |source: std::io::Error| Error::Io {
        path: path.display().to_string(),
        source,
    };
    std::fs::write(path, bytes).map_err(io)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(io)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema;

    /// A scratch directory that cleans itself up.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("collect-identity-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn creates_a_key_on_first_use() {
        let scratch = Scratch::new("create");
        let identity = Identity::load_or_create(&scratch.0).unwrap();
        assert!(identity.path().exists());
        // 64 lower-case hex, matching the contract's `key`.
        let hex = identity.public_hex();
        assert_eq!(hex.len(), 64);
        assert!(hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    }

    #[test]
    fn the_same_directory_gives_the_same_key() {
        // The creator must stay the creator across a relaunch, or its own
        // experiments stop being its own.
        let scratch = Scratch::new("stable");
        let first = Identity::load_or_create(&scratch.0).unwrap().public_hex();
        let second = Identity::load_or_create(&scratch.0).unwrap().public_hex();
        assert_eq!(first, second);
    }

    #[test]
    fn different_directories_give_different_keys() {
        let a = Scratch::new("a");
        let b = Scratch::new("b");
        assert_ne!(
            Identity::load_or_create(&a.0).unwrap().public_hex(),
            Identity::load_or_create(&b.0).unwrap().public_hex()
        );
    }

    #[test]
    fn creates_the_directory_if_it_is_missing() {
        let scratch = Scratch::new("nested");
        let deeper = scratch.0.join("not").join("there");
        assert!(Identity::load_or_create(&deeper).is_ok());
    }

    #[test]
    fn a_malformed_key_file_is_reported_not_replaced() {
        // Silently generating a new key would change who the creator is, and
        // orphan every experiment already attributed to the old one.
        let scratch = Scratch::new("malformed");
        std::fs::write(scratch.0.join(KEY_FILE), b"too short").unwrap();
        assert!(matches!(
            Identity::load_or_create(&scratch.0),
            Err(Error::Malformed { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn the_key_is_owner_readable_only() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::new("perms");
        let identity = Identity::load_or_create(&scratch.0).unwrap();
        let mode = std::fs::metadata(identity.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "mode was {:o}", mode & 0o777);
    }

    #[test]
    fn a_signed_document_verifies_and_validates() {
        let scratch = Scratch::new("sign");
        let identity = Identity::load_or_create(&scratch.0).unwrap();

        let mut doc: Value =
            serde_json::from_str(include_str!("../../../../../examples/experiment.json")).unwrap();
        // The document must claim this identity as its creator, since
        // verify() reads the key out of the document.
        doc["creator"]["key"] = Value::String(identity.public_hex());

        let signed = identity.sign(&doc).unwrap();
        assert_eq!(
            sign::verify(&signed).unwrap(),
            hex::decode(identity.public_hex()).unwrap()[..]
        );
        assert!(schema::is_valid(&signed), "{:?}", schema::errors(&signed));
    }

    #[test]
    fn signing_replaces_a_placeholder_signature() {
        let scratch = Scratch::new("replace");
        let identity = Identity::load_or_create(&scratch.0).unwrap();
        let doc = serde_json::json!({
            "schema": "collect/experiment@1",
            "creator": { "key": identity.public_hex() },
            "sig": "00".repeat(64)
        });
        let signed = identity.sign(&doc).unwrap();
        assert_ne!(signed["sig"], doc["sig"]);
        assert!(sign::verify(&signed).is_ok());
    }
}
