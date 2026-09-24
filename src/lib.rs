//! Frame-ribbon: an offline SMPTE timecode / drop-frame / multi-source
//! alignment review workbench.
//!
//! Every fact in the system is either an immutable imported *material* or an
//! append-only *event* on the timeline.  Derived values are recomputable from
//! that history and carry an algorithm version, a parameter fingerprint and a
//! direct source list, so reopening a project replays the exact same state.

pub mod archive;
pub mod clock;
pub mod derive;
pub mod domain;
pub mod hash;
pub mod ingest;
pub mod project;
pub mod state;
pub mod storage;
pub mod timecode;

pub use domain::*;
pub use project::Project;
