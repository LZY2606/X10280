//! Injectable clocks. Domain replay uses [`VirtualClock`] with a fixed seed so
//! tests never touch wall-clock and event timestamps are reproducible.

pub trait Clock: Send + Sync {
    fn now_ns(&self) -> i64;
}

pub struct SystemClock;
impl Clock for SystemClock {
    fn now_ns(&self) -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0)
    }
}

/// Deterministic monotonic clock: starts at a seed and advances by a fixed
/// quantum on every read.
pub struct VirtualClock {
    state: std::sync::Mutex<i64>,
    quantum_ns: i64,
}

impl VirtualClock {
    pub fn new(seed_ns: i64, quantum_ns: i64) -> Self {
        VirtualClock { state: std::sync::Mutex::new(seed_ns), quantum_ns }
    }
    /// fixed-seed virtual clock used by the bundled case pack and the tests
    pub fn fixed_seed() -> Self {
        VirtualClock::new(1_700_000_000_000_000_000, 1_000_000)
    }
}

impl Clock for VirtualClock {
    fn now_ns(&self) -> i64 {
        let mut g = self.state.lock().unwrap();
        let v = *g;
        *g += self.quantum_ns;
        v
    }
}
