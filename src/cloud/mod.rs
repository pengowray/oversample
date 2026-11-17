//! Optional copies of annotation (.batm) and project (.batproj) files in the
//! user's Google Drive. Browser build only: in Tauri none of this runs and
//! the settings block is not shown.
//!
//! - `rules`: the comparison of a local copy and a Drive copy (pure, tested).
//! - `google_auth`: sign-in state and access tokens from the Oversample API.
//! - `drive`: a thin client for the Drive REST API.
//! - `sync`: when to compare, upload and download, and the state the settings
//!   block shows.

pub mod drive;
pub mod google_auth;
pub mod http;
pub mod rules;
pub mod sync;
