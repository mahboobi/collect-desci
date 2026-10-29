//! Publishing documents to Logos Storage, behind a trait.
//!
//! The trait exists because of the crate split. `collect_core_logic` has no
//! Logos dependency — that is what lets all of it be tested outside a Nix
//! build — but publishing is the one thing it cannot do by itself. So the
//! logic names the capability and the module shim supplies it
//! (`rust-lib/src/storage.rs`, over the real `storage_module` proxy).
//!
//! ## What a CID is taken over
//!
//! The canonical JSON (RFC 8785) of the **signed** document, `sig` included.
//! That matters: a CID has to be reproducible by anyone holding the document,
//! and canonical form is the only representation two nodes are guaranteed to
//! agree on. Uploading `serde_json`'s output instead would make the CID depend
//! on this crate's key order, so the same document would land at two
//! addresses.
//!
//! ## What is deliberately not here
//!
//! No retraction. Storage's `remove()` is local-only — the DHT record is not
//! withdrawn and any node that fetched the bytes keeps them — so nothing
//! published is recallable. `hide` therefore means "drop from the index",
//! never "delete", and `rounded_1km` has to happen before upload rather than
//! after (SPEC 2, docs/api-verification.md Q2).

use crate::canonical;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Mutex;

/// Why a publish did not happen.
///
/// Separated from a bare string so the caller can tell a configuration
/// problem (no Storage at all) from a transport one (it was there and timed
/// out) — the first is permanent until the module is rebuilt, the second is
/// worth retrying.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// No Storage module is attached. Nothing was attempted.
    Unavailable(String),
    /// The upload was accepted but no completion arrived in time.
    Timeout(String),
    /// Storage answered, and the answer was no.
    Rejected(String),
    /// The document could not be put into canonical form.
    NotCanonical(String),
}

impl StoreError {
    /// The stable code that reaches the UI. Matches the `api::code` style:
    /// snake_case, never localized, safe to branch on.
    pub fn code(&self) -> &'static str {
        match self {
            StoreError::Unavailable(_) => "storage_unavailable",
            StoreError::Timeout(_) => "storage_timeout",
            StoreError::Rejected(_) => "storage_rejected",
            StoreError::NotCanonical(_) => "invalid_document",
        }
    }

    /// Whether trying the same upload again could plausibly work.
    pub fn retryable(&self) -> bool {
        matches!(self, StoreError::Timeout(_) | StoreError::Rejected(_))
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Unavailable(m) => write!(f, "Logos Storage is not available: {m}"),
            StoreError::Timeout(m) => write!(f, "Logos Storage did not answer in time: {m}"),
            StoreError::Rejected(m) => write!(f, "Logos Storage refused the upload: {m}"),
            StoreError::NotCanonical(m) => write!(f, "the document is not canonicalizable: {m}"),
        }
    }
}

/// Somewhere a signed document can be put and fetched back by CID.
///
/// `Send` because the shim's implementation talks to another module and may
/// end up driven from a listener thread; nothing here assumes a thread.
pub trait DocumentStore: Send {
    /// Upload `bytes` under a display filename, and return its CID.
    ///
    /// `filename` is Storage's manifest label (it shows up in `manifests()`),
    /// not an address — the CID is content-derived and the only identifier
    /// that matters.
    fn put(&self, filename: &str, bytes: &[u8]) -> Result<String, StoreError>;

    /// Fetch the bytes at `cid`. `local` asks for a cache-only read: true
    /// re-reads something this node already holds without a network round
    /// trip, which is the common case for our own documents.
    fn get(&self, cid: &str, local: bool) -> Result<Vec<u8>, StoreError>;

    /// A one-line description for `session.capabilities`, so the UI can say
    /// which Storage it is talking to rather than guessing.
    fn describe(&self) -> String {
        "a document store".to_string()
    }
}

/// Canonicalize a signed document and publish it, returning a CID in the form
/// the contract requires inside a document.
///
/// The single place the bytes-for-a-CID rule is applied, so a document cannot
/// be uploaded one way here and verified another way there.
pub fn publish(store: &dyn DocumentStore, kind: &str, doc: &Value) -> Result<String, StoreError> {
    let bytes =
        canonical::to_bytes(doc).map_err(|e| StoreError::NotCanonical(e.to_string()))?;
    let id = doc
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("document");
    as_document_cid(&store.put(&format!("{kind}-{id}.json"), &bytes)?)
}

/// Put a raw Storage address into the form a document may carry.
///
/// The contract's CID pattern is `^logos:[A-Za-z0-9]+$`, so an address with a
/// hyphen or underscore in it — which some multibase encodings produce —
/// cannot go into a document at all. Checking here turns that into one clear
/// error at publish time instead of a schema failure later, with the raw
/// address in the message so it is obvious what came back.
pub fn as_document_cid(raw: &str) -> Result<String, StoreError> {
    let bare = bare_cid(raw);
    if bare.is_empty() || !bare.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(StoreError::Rejected(format!(
            "Storage returned `{raw}`, which the contract's CID pattern does not allow"
        )));
    }
    Ok(format!("logos:{bare}"))
}

/// The address Storage itself uses: the document form without the scheme.
/// Idempotent, so it is safe on either form.
pub fn bare_cid(cid: &str) -> &str {
    cid.strip_prefix("logos:").unwrap_or(cid)
}

/// An in-process store, for tests and for `COLLECT_FIXTURES=1`.
///
/// The CID is **not** a real Storage CID and does not pretend to be: it is
/// `mem1` followed by a SHA-256 of the bytes. A fake that looked like a CIDv1
/// would be worse than one that does not, because it could be mistaken for a
/// published address and copied into a link.
pub struct MemoryStore {
    blobs: Mutex<BTreeMap<String, Vec<u8>>>,
    /// Set to make every `put` fail, to exercise the error path.
    fail_with: Option<StoreError>,
}

impl Default for MemoryStore {
    fn default() -> Self {
        MemoryStore::new()
    }
}

impl MemoryStore {
    pub fn new() -> Self {
        MemoryStore { blobs: Mutex::new(BTreeMap::new()), fail_with: None }
    }

    /// A store that refuses everything, so callers can be tested against a
    /// failing Storage without one.
    pub fn failing(error: StoreError) -> Self {
        MemoryStore { blobs: Mutex::new(BTreeMap::new()), fail_with: Some(error) }
    }

    /// How many distinct documents were accepted. Re-uploading identical
    /// bytes does not increase it, because the CID is the content.
    pub fn len(&self) -> usize {
        self.blobs.lock().map(|b| b.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn address(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("mem1{}", hex::encode(hasher.finalize()))
    }
}

impl DocumentStore for MemoryStore {
    fn put(&self, _filename: &str, bytes: &[u8]) -> Result<String, StoreError> {
        if let Some(e) = &self.fail_with {
            return Err(e.clone());
        }
        let cid = Self::address(bytes);
        match self.blobs.lock() {
            Ok(mut blobs) => {
                blobs.insert(cid.clone(), bytes.to_vec());
                Ok(cid)
            }
            Err(_) => Err(StoreError::Rejected("the in-memory store is poisoned".into())),
        }
    }

    fn get(&self, cid: &str, _local: bool) -> Result<Vec<u8>, StoreError> {
        match self.blobs.lock() {
            Ok(blobs) => blobs
                .get(cid)
                .cloned()
                .ok_or_else(|| StoreError::Rejected(format!("nothing at `{cid}`"))),
            Err(_) => Err(StoreError::Rejected("the in-memory store is poisoned".into())),
        }
    }

    fn describe(&self) -> String {
        "an in-process store (CIDs are not real Storage addresses)".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc() -> Value {
        json!({
            "schema": "collect/experiment@1",
            "id": "exp_1",
            "title": "Tide pools",
            "sig": "ab".repeat(64),
        })
    }

    #[test]
    fn a_cid_is_taken_over_canonical_bytes_including_the_signature() {
        let store = MemoryStore::new();
        let cid = publish(&store, "experiment", &doc()).expect("published");
        let bytes = store.get(bare_cid(&cid), true).expect("round trip");
        assert_eq!(bytes, canonical::to_bytes(&doc()).unwrap());
        // And the signature is inside those bytes, not stripped as it is for
        // the signing payload.
        assert!(String::from_utf8(bytes).unwrap().contains("\"sig\""));
    }

    #[test]
    fn key_order_in_the_input_does_not_move_the_address() {
        // The whole point of canonicalizing first: two orderings of the same
        // document are the same document, so they must land at one CID.
        let store = MemoryStore::new();
        let a: Value = serde_json::from_str(
            r#"{"id":"exp_1","schema":"collect/experiment@1","title":"Tide pools","sig":"ab"}"#,
        )
        .unwrap();
        let b: Value = serde_json::from_str(
            r#"{"title":"Tide pools","sig":"ab","schema":"collect/experiment@1","id":"exp_1"}"#,
        )
        .unwrap();
        assert_eq!(
            publish(&store, "experiment", &a).unwrap(),
            publish(&store, "experiment", &b).unwrap()
        );
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn a_changed_byte_moves_the_address() {
        let store = MemoryStore::new();
        let first = publish(&store, "experiment", &doc()).unwrap();
        let mut edited = doc();
        edited["title"] = json!("Tide pools ");
        let second = publish(&store, "experiment", &edited).unwrap();
        assert_ne!(first, second);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn the_filename_is_a_label_and_does_not_change_the_address() {
        let store = MemoryStore::new();
        let bytes = canonical::to_bytes(&doc()).unwrap();
        assert_eq!(
            store.put("experiment-exp_1.json", &bytes).unwrap(),
            store.put("something-else.json", &bytes).unwrap()
        );
    }

    #[test]
    fn a_failing_store_reports_a_code_and_whether_to_retry() {
        let store = MemoryStore::failing(StoreError::Timeout("no completion in 30s".into()));
        let error = publish(&store, "experiment", &doc()).expect_err("should fail");
        assert_eq!(error.code(), "storage_timeout");
        assert!(error.retryable());
        assert!(error.to_string().contains("did not answer in time"));
    }

    #[test]
    fn an_absent_store_is_not_retryable() {
        let store = MemoryStore::failing(StoreError::Unavailable("no dependency".into()));
        let error = publish(&store, "experiment", &doc()).expect_err("should fail");
        assert_eq!(error.code(), "storage_unavailable");
        assert!(!error.retryable());
    }

    #[test]
    fn a_published_cid_carries_the_scheme_the_contract_requires() {
        let store = MemoryStore::new();
        let cid = publish(&store, "experiment", &doc()).unwrap();
        assert!(cid.starts_with("logos:"), "{cid}");
        // The same check the contract applies, run here so the assertion is
        // about the pattern and not about our prefix alone.
        let pattern = regex_lite_match(&cid);
        assert!(pattern, "{cid} does not match ^logos:[A-Za-z0-9]+$");
    }

    /// `^logos:[A-Za-z0-9]+$` without pulling in a regex crate.
    fn regex_lite_match(cid: &str) -> bool {
        match cid.strip_prefix("logos:") {
            Some(rest) => !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric()),
            None => false,
        }
    }

    #[test]
    fn an_address_the_contract_cannot_express_is_refused_at_publish_time() {
        // A multibase encoding with a hyphen is a plausible thing for Storage
        // to hand back, and it can never go into a document — so it has to
        // fail here, loudly, rather than as a schema error three calls later.
        assert!(as_document_cid("zdj7W-g8h").is_err());
        assert!(as_document_cid("").is_err());
        assert!(as_document_cid("logos:").is_err());
        assert_eq!(as_document_cid("abc123").unwrap(), "logos:abc123");
        // Idempotent: already-prefixed input is not double-prefixed.
        assert_eq!(as_document_cid("logos:abc123").unwrap(), "logos:abc123");
    }

    #[test]
    fn fetching_an_unknown_cid_is_an_error_not_an_empty_document() {
        let store = MemoryStore::new();
        let error = store.get("mem1deadbeef", true).expect_err("should fail");
        assert!(error.to_string().contains("mem1deadbeef"));
    }
}
