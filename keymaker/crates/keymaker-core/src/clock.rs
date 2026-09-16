//! Time, injectable so expiry logic is testable without sleeping.

use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

pub trait Clock: std::fmt::Debug {
    /// Seconds since the unix epoch.
    fn now(&self) -> u64;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// A clock the test drives by hand.
#[derive(Debug)]
pub struct FixedClock(Cell<u64>);

impl FixedClock {
    pub fn new(start: u64) -> Self {
        FixedClock(Cell::new(start))
    }
    pub fn advance(&self, secs: u64) {
        self.0.set(self.0.get() + secs);
    }
    pub fn set(&self, secs: u64) {
        self.0.set(secs);
    }
}

impl Clock for FixedClock {
    fn now(&self) -> u64 {
        self.0.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_clock_advances_only_when_told() {
        let c = FixedClock::new(100);
        assert_eq!(c.now(), 100);
        assert_eq!(c.now(), 100, "reading the clock must not move it");
        c.advance(30);
        assert_eq!(c.now(), 130);
        c.set(7);
        assert_eq!(c.now(), 7);
    }

    #[test]
    fn system_clock_is_after_2020() {
        // 2020-01-01. Guards against a clock returning 0 on error.
        assert!(SystemClock.now() > 1_577_836_800);
    }
}
