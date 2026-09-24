//! Virtual clock used for event timestamps.
//!
//! Tests pin the seed so replay records are byte-for-byte reproducible; the
//! server seeds from a fixed constant too (the real wall clock is never used
//! to identify anything — device clocks from imports are preserved verbatim).
#[derive(Clone)]
pub struct VirtualClock {
    state: u64,
    epoch_ms: i64,
}

impl VirtualClock {
    /// Deterministic LCG (Knuth MMIX constants), seeded per project/test.
    pub fn new(seed: u64) -> Self {
        VirtualClock {
            state: if seed == 0 { 0x9e37_79b9_7f4a_7c15 } else { seed },
            epoch_ms: 946_684_800_000, // 2000-01-01T00:00:00Z, fixed virtual epoch
        }
    }

    /// Advance the virtual clock by a fixed 7 ms and return the timestamp.
    pub fn tick_ms(&mut self) -> i64 {
        // MMIX linear congruential generator, masked to 48 bits.
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let step = (self.state >> 16) % 37; // 0..37 jitter-free deterministic spread
        self.epoch_ms += 7 + (step as i64);
        self.epoch_ms
    }

    pub fn now_ms(&self) -> i64 {
        self.epoch_ms
    }
}
