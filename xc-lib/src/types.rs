//! XC wire types.
//!
//! Canonical definitions live in the dependency-light `oversample-ipc::xc` crate
//! so they can be shared with the WASM frontend (which can't depend on `xc-lib`,
//! since it pulls `reqwest`). Re-exported here so existing `crate::types::…` and
//! `xc_lib::…` paths keep working.
pub use oversample_ipc::xc::*;
