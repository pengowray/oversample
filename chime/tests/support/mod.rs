//! Code shared by the integration tests of the chime crate. Each test file uses
//! it with `mod support;`.

#![allow(dead_code)]

pub mod channel;

// The encoder is the crate's own `encode` module (feature `std`, turned on for the tests by
// the crate's dev-dependency on itself). Tests write `support::generator`.
pub use chime::encode as generator;
