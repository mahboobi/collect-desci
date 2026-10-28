//! `collect_core_logic` — everything Collect does, with no Logos dependency.
//!
//! The QML module is sandboxed (no network, no filesystem outside the module),
//! so every document, crypto, media and network operation lives here and the
//! UI reaches it through `logos.callModule`. See `CLAUDE.md`.
//!
//! Kept separate from the module crate in `..` on purpose. The
//! builder derives the `.lidl` by scanning that crate's `lib.rs` for the trait,
//! and the trait's signatures name types from a scaffold that only exists
//! inside a Nix build — so that crate cannot be compiled by `cargo test`.
//! Everything worth testing is here, and `api` is the whole boundary the shim
//! forwards to.

pub mod api;
pub mod canonical;
pub mod fields;
pub mod geo;
pub mod identity;
pub mod ids;
pub mod indexer;
pub mod media;
pub mod schema;
pub mod session;
pub mod sign;

