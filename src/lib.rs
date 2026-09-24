//! Frame Ribbon — offline SMPTE timecode / drop-frame / multi-source alignment workbench.
//!
//! Crate layout:
//! - [`rat`]     exact rational frame-rate / duration arithmetic
//! - [`hashing`] deterministic stable identities (FNV-1a based)
//! - [`timecode`] SMPTE LTC/VITC bit codec, labels, drop-frame numbering
//! - [`model`]   preserved raw inputs, derived values, machine-readable events
//! - [`import`]  copy-paste sample parser with explicit format diagnostics
//! - [`clock`]   virtual / system clocks (replay uses fixed-seed virtual time)
//! - [`storage`] SQLite persistence, immutability, as-of queries
//! - [`pipeline`] derivation / replay / provenance / counterexample archive
//! - [`casepack`] the bundled "frame ribbon" offline domain case pack
//! - [`archive`]  export / import preserving identity, order and provenance
//! - [`server`]  Axum JSON API + static review UI

pub mod clock;
pub mod error;
pub mod hashing;
pub mod import;
pub mod model;
pub mod pipeline;
pub mod rat;
pub mod server;
pub mod storage;
pub mod timecode;

pub mod archive;
pub mod casepack;

pub use error::{DomainError, DomainResult};
