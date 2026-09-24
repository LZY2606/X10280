//! Pure domain layer: SMPTE timecode, raw records, events, derived state.

pub mod timecode;
pub mod raw;
pub mod events;
pub mod state;
pub mod align;
pub mod archive;

pub use timecode::{FrameRate, TcLabel, label_to_frames, frames_to_label, delta_with_wrap, WrapDelta};
