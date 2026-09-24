//! Virtual clock used by every state mutation.
//!
//! Time is a monotonically increasing virtual millisecond counter anchored at
//! a fixed epoch, so replay on any machine produces identical timestamps.
//! Tests construct clocks with a fixed seed; the production binary never reads
//! the wall clock either (the demo is deterministic by design).

#[derive(Clone, Debug)]
pub struct VirtualClock {
    epoch_ms: i64,
    counter: u64,
}

pub const FIXED_EPOCH_MS: i64 = 1_767_225_600_000; // 2026-01-01T00:00:00Z
pub const TICK_MS: i64 = 7;

impl VirtualClock {
    pub fn seeded(seed: u64) -> Self {
        Self {
            epoch_ms: FIXED_EPOCH_MS,
            counter: seed,
        }
    }

    pub fn now_ms(&mut self) -> i64 {
        let value = self.epoch_ms + (self.counter as i64) * TICK_MS;
        self.counter = self.counter.wrapping_add(1);
        value
    }

    pub fn ticks(&self) -> u64 {
        self.counter
    }
}
