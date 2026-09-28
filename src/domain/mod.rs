//! 领域核心：帧率有理数、SMPTE 时码（含 drop-frame 跳号）、LTC/VITC 比特。
pub mod ratio;
pub mod timecode;
pub mod ltc;
pub mod vitc;
pub mod quantity;
pub mod seeds;

pub use ratio::Rational;
pub use timecode::{Fps, Timecode};
pub use quantity::{Coord, Quantity, Unit};
