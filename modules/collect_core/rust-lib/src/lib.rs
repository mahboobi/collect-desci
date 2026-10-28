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

use collect_core_logic::api;

/// Collect's core: documents, signing, validation, media and indexing.
pub trait CollectCoreModule: Send + 'static {
    /// What this build can actually do. Worth calling first — `storage` and
    /// `delivery` report false until SPEC Q9 and Q12 are settled.
    fn session_capabilities(&mut self) -> String;

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
    /// Where durable state belongs, stamped by the host.
    ///
    /// `None` until `on_context_ready`, which is why nothing opens state before
    /// then. This is the only correct location: everything under basecamp's
    /// `module_root` is scrubbed by `lgs basecamp launch` on every invocation
    /// by design, so a creator keeping its identity key or index state there
    /// would stop being the indexer after one relaunch (SPEC Q8).
    persistence_path: Option<String>,
}

impl CollectCoreModule for Collect {
    fn session_capabilities(&mut self) -> String {
        api::session_capabilities("{}")
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

    fn call(&mut self, method: String, request: String) -> String {
        api::dispatch(&method, &request)
    }

    fn on_context_ready(&mut self, ctx: &RustModuleContext) {
        self.persistence_path = Some(ctx.instance_persistence_path.clone());
    }
}

#[no_mangle]
pub extern "Rust" fn logos_module_install() {
    install::<Collect>();
}
