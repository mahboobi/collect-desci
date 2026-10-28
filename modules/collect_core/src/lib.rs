//! `collect_core` — the non-UI half of Collect.
//!
//! The QML module is sandboxed (no network, no filesystem outside the module),
//! so every document, crypto, media and network operation lives here and the
//! UI reaches it through `logos.callModule`. See `CLAUDE.md`.

pub mod canonical;
pub mod ids;
pub mod schema;
pub mod sign;
