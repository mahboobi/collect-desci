//! The two files under test, pulled in by path.
//!
//! `#[path]` rather than copies: a copy would drift, and the point is to
//! typecheck the files that actually ship inside the module.
#[path = "../../../modules/collect_core/rust-lib/src/storage.rs"]
pub mod storage;

#[path = "../../../modules/collect_core/rust-lib/src/courier.rs"]
pub mod courier;

#[path = "../../../modules/collect_core/rust-lib/src/lez.rs"]
pub mod lez;
