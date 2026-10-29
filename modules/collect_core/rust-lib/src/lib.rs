//! The Logos module surface for Collect.
//!
//! Rust-first authoring: the trait below IS the contract. The builder derives
//! the `.lidl` from it (`codegen.rust.trait` in metadata.json), generates the
//! C-ABI scaffold around it, and compiles this crate to a staticlib — no
//! `build.rs`, no committed `.lidl`.
//!
//! **The trait has to be in this file.** The generator extracts the contract by
//! scanning `src/lib.rs`, and fails with "trait ... not found in source" if it
//! lives anywhere else. That is also why this crate is a shim: the trait's
//! `on_context_ready` names `RustModuleContext`, which the generated scaffold
//! defines, so nothing here compiles outside a Nix build. All the logic — and
//! all 222 tests — sit in `collect_core_logic`, which has no Logos dependency.
//!
//! Three more constraints come from the template (`docs/api-verification.md`
//! Q1):
//!
//! - The trait name is derived, not chosen: `collect_core` -> `CollectCoreModule`.
//! - It must be `Send + 'static`; the generated dispatch stores the impl as a
//!   `Box<dyn Any + Send>`.
//! - Only `String`, `i64`, `u64`, `f64`, `bool`, `Vec<u8>` and `Option<T>`
//!   cross the ABI. No structs, no maps — which is why every method takes and
//!   returns a JSON string, as SPEC section 6 describes.

use collect_core_logic::{api, session::Session, store::DocumentStore};

// The real Logos Storage client. Kept in its own file because it is the only
// thing here that talks to another module, and because it is the one part of
// this crate that can be compiled outside the Nix build (it does not touch
// the generated scaffold) — see its own docs.
mod storage;

/// Collect's core: documents, signing, validation, media and indexing.
pub trait CollectCoreModule: Send + 'static {
    /// What this build can actually do, including whether it can sign.
    fn session_capabilities(&mut self) -> String;

    /// This node's signing identity: the public key documents are attributed
    /// to, and where the private key is kept.
    fn identity_me(&mut self) -> String;

    /// Canonical JSON (RFC 8785) of a document with `sig` removed: the exact
    /// bytes a signature covers.
    fn documents_canonicalize(&mut self, request: String) -> String;

    /// Validate a document against the Collect data contract.
    fn documents_validate(&mut self, request: String) -> String;

    /// Verify a document's contract and signature. Pass `key` for a protocol
    /// or index page, which carry no author key of their own.
    fn documents_verify(&mut self, request: String) -> String;

    /// Validate a protocol draft and report what changed against the published
    /// version, and whether that change is breaking.
    fn protocols_validate_draft(&mut self, request: String) -> String;

    /// Check a submission's values against a protocol. Returns missing fields
    /// and per-field errors.
    fn submissions_validate(&mut self, request: String) -> String;

    /// Apply each field's store rule — notably rounded_1km — before any value
    /// reaches a document.
    fn submissions_prepare_values(&mut self, request: String) -> String;

    /// Read a photo's metadata, then return the cleaned bytes to upload and
    /// the values to prefill from it. The original is never returned.
    fn submissions_process_media(&mut self, request: String) -> String;

    /// The creator's verdict on a received submission: contract, signature,
    /// window, repeat and rate limits, hidden keys, then field validation.
    fn index_accept(&mut self, request: String) -> String;

    /// Lay accepted entries out across index pages.
    fn index_page(&mut self, request: String) -> String;

    /// Snap a coordinate to the centre of its 0.01 degree cell.
    fn geo_round(&mut self, request: String) -> String;

    /// Pull the experiment id, manifest CID and Delivery topic out of a link.
    fn experiments_parse_link(&mut self, request: String) -> String;

    /// The view model for an experiment opened from a link: the manifest and
    /// the protocol documents it names. Needs COLLECT_FIXTURES=1 until
    /// Storage is wired.
    fn experiments_open(&mut self, request: String) -> String;

    /// One protocol document by id.
    fn protocols_get(&mut self, request: String) -> String;

    /// Every experiment this node knows, as dashboard cards, split into the
    /// ones this identity created and the ones it only submits to.
    fn experiments_list(&mut self) -> String;

    /// One experiment manifest plus the protocol documents this node holds.
    fn experiments_get(&mut self, request: String) -> String;

    /// Create an experiment from the details form. Saved on this node only —
    /// publishing needs Logos Storage.
    fn experiments_create(&mut self, request: String) -> String;

    /// Attach or replace a protocol form on an experiment, bumping its
    /// version. Saved on this node only.
    fn protocols_save_draft(&mut self, request: String) -> String;

    /// The creator's results table for one form: columns from the protocol's
    /// fields, rows from the submissions this node holds.
    fn results_list(&mut self, request: String) -> String;

    /// Try again to put an experiment manifest on Logos Storage, after a
    /// `storage_timeout` or `storage_rejected`.
    fn experiments_publish(&mut self, request: String) -> String;

    /// Call any method by its SPEC section 6 name, e.g. `submissions.validate`.
    /// Lets the UI hold one call site, and lets `logoscore call` reach a method
    /// this trait has not named yet.
    fn call(&mut self, method: String, request: String) -> String;

    /// Called once the module is wired to the host. `ctx` carries the instance
    /// id and the per-instance persistence path.
    fn on_context_ready(&mut self, _ctx: &RustModuleContext) {}
}

/// Typed events — the Rust analog of the C++ `logos_events:` section. Each
/// method becomes an `emit_<name>` free function.
pub trait CollectCoreModuleEvents {
    /// A submission this node accepted and indexed.
    fn submission_received(&self, experiment: String, protocol: String, cid: String);

    /// A receipt arrived for a submission this node made.
    fn receipt_received(&self, cid: String);

    /// A photo finished processing; the payload carries its media entry and
    /// prefills.
    fn media_processed(&self, draft_id: String, field_key: String, result: String);

    /// Node, storage or delivery state changed.
    fn sync_status_changed(&self, status: String);
}

// The builder injects the generated scaffold here — the `install` entry point,
// `RustModuleContext`, `context()`, `modules()` (typed dependency clients) and
// the `emit_*` event emitters. No build.rs, no OUT_DIR.
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/generated/provider_gen.rs"));

#[derive(Default)]
struct Collect {
    /// Experiments and protocols this node knows. Built lazily so it is
    /// created after the host has stamped the context.
    session: Option<Session>,

    /// Where durable state belongs, stamped by the host.
    ///
    /// `None` until `on_context_ready`, which is why nothing opens state before
    /// then. This is the only correct location: everything under basecamp's
    /// `module_root` is scrubbed by `lgs basecamp launch` on every invocation
    /// by design, so a creator keeping its identity key or index state there
    /// would stop being the indexer after one relaunch (SPEC Q8).
    persistence_path: Option<String>,
}

impl Collect {
    /// The live session, created on first use.
    fn session(&mut self) -> &mut Session {
        self.session.get_or_insert_with(Session::new)
    }
}

impl CollectCoreModule for Collect {
    fn session_capabilities(&mut self) -> String {
        // Reported rather than assumed: the UI can disable creating before
        // the key exists instead of failing at the call.
        let can_sign = self.session().can_sign();
        let can_publish = self.session().can_publish();
        let store = self.session().store_description();
        let base = api::session_capabilities("{}");
        match serde_json::from_str::<serde_json::Value>(&base) {
            Ok(mut v) => {
                v["data"]["signing"] = serde_json::Value::Bool(can_sign);
                v["data"]["publishing"] = serde_json::Value::Bool(can_publish);
                v["data"]["store"] = match store {
                    Some(d) => serde_json::Value::String(d),
                    None => serde_json::Value::Null,
                };
                v.to_string()
            }
            Err(_) => base,
        }
    }

    fn identity_me(&mut self) -> String {
        let key = self.session().creator_key().to_string();
        let can_sign = self.session().can_sign();
        serde_json::json!({
            "ok": true,
            "data": { "key": key, "signing": can_sign }
        })
        .to_string()
    }

    fn documents_canonicalize(&mut self, request: String) -> String {
        api::documents_canonicalize(&request)
    }

    fn documents_validate(&mut self, request: String) -> String {
        api::documents_validate(&request)
    }

    fn documents_verify(&mut self, request: String) -> String {
        api::documents_verify(&request)
    }

    fn protocols_validate_draft(&mut self, request: String) -> String {
        api::protocols_validate_draft(&request)
    }

    fn submissions_validate(&mut self, request: String) -> String {
        api::submissions_validate(&request)
    }

    fn submissions_prepare_values(&mut self, request: String) -> String {
        api::submissions_prepare_values(&request)
    }

    fn submissions_process_media(&mut self, request: String) -> String {
        api::submissions_process_media(&request)
    }

    fn index_accept(&mut self, request: String) -> String {
        api::index_accept(&request)
    }

    fn index_page(&mut self, request: String) -> String {
        api::index_page(&request)
    }

    fn geo_round(&mut self, request: String) -> String {
        api::geo_round(&request)
    }

    fn experiments_parse_link(&mut self, request: String) -> String {
        api::experiments_parse_link(&request)
    }

    fn experiments_open(&mut self, request: String) -> String {
        api::experiments_open(&request)
    }

    fn protocols_get(&mut self, request: String) -> String {
        api::protocols_get(&request)
    }

    fn experiments_list(&mut self) -> String {
        let now = chrono::Utc::now();
        self.session().list(now)
    }

    fn experiments_get(&mut self, request: String) -> String {
        self.session().get(&request)
    }

    fn experiments_create(&mut self, request: String) -> String {
        let now = chrono::Utc::now();
        self.session().create(&request, now)
    }

    fn protocols_save_draft(&mut self, request: String) -> String {
        let now = chrono::Utc::now();
        self.session().save_protocol(&request, now)
    }

    fn results_list(&mut self, request: String) -> String {
        self.session().results(&request)
    }

    fn experiments_publish(&mut self, request: String) -> String {
        self.session().publish_now(&request)
    }

    fn call(&mut self, method: String, request: String) -> String {
        // The stateful methods live on Session, which api::dispatch cannot
        // reach — it only knows the pure functions. Routing them here is what
        // makes the dotted SPEC section 6 names work through one entry point;
        // without this they fell through to unknown_method.
        let now = chrono::Utc::now();
        match method.as_str() {
            "identity.me" => self.identity_me(),
            "experiments.list" => self.session().list(now),
            "experiments.get" => self.session().get(&request),
            "experiments.create" => self.session().create(&request, now),
            "protocols.saveDraft" => self.session().save_protocol(&request, now),
            "results.list" => self.session().results(&request),
            "experiments.publish" => self.session().publish_now(&request),
            _ => api::dispatch(&method, &request),
        }
    }

    fn on_context_ready(&mut self, ctx: &RustModuleContext) {
        self.persistence_path = Some(ctx.instance_persistence_path.clone());
        // First moment a path exists, so this is where the signing key is
        // opened. Without it `create` refuses rather than emitting an
        // unsigned document.
        match self.session().attach_identity(&ctx.instance_persistence_path) {
            Ok(public) => eprintln!("collect_core: signing as {public}"),
            Err(e) => eprintln!("collect_core: no signing key: {e}"),
        }

        // Storage is attached the same way, and separately: a node that can
        // sign but cannot publish still works — documents are kept locally
        // and the UI says they have no address yet — so a Storage failure
        // must not take signing down with it.
        let scratch = std::path::Path::new(&ctx.instance_persistence_path).join("uploads");
        match storage::LogosStorage::connect(&scratch) {
            Ok(store) => {
                eprintln!("collect_core: publishing to {}", store.describe());
                self.session().attach_store(Box::new(store));
            }
            Err(e) => eprintln!("collect_core: not publishing: {e}"),
        }
    }
}

#[no_mangle]
pub extern "Rust" fn logos_module_install() {
    install::<Collect>();
}
