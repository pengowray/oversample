//! XC wire types.
//!
//! Canonical definitions live in the dependency-light `oversample-ipc::xc` crate,
//! which the WASM frontend also uses. Re-exported here so existing
//! `crate::types::…` and `xc_lib::…` paths keep working.
pub use oversample_ipc::annota::*;
pub use oversample_ipc::xc::*;
