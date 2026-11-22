//! Time source for every wait, cooldown and reset, so tests never sleep for real.

use std::sync::Mutex;
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use jiff::{SignedDuration, Timestamp};

pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()>;
}

/// Wall clock + tokio timers.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
        tokio::time::sleep(d).boxed()
    }
}

/// Test clock: `sleep` returns at once and moves `now` forward; every wait is recorded.
pub struct FakeClock {
    now: Mutex<Timestamp>,
    slept: Mutex<Vec<Duration>>,
}

impl FakeClock {
    pub fn new(start: Timestamp) -> Self {
        Self {
            now: Mutex::new(start),
            slept: Mutex::new(Vec::new()),
        }
    }
    pub fn advance(&self, d: Duration) {
        if let Ok(mut n) = self.now.lock() {
            *n = n
                .saturating_add(SignedDuration::try_from(d).unwrap_or(SignedDuration::MAX))
                .unwrap_or(*n);
        }
    }
    /// Every duration passed to `sleep`, in order.
    pub fn slept(&self) -> Vec<Duration> {
        self.slept.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Timestamp {
        self.now.lock().map_or_else(|_| Timestamp::now(), |n| *n)
    }
    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
        if let Ok(mut s) = self.slept.lock() {
            s.push(d);
        }
        self.advance(d);
        futures::future::ready(()).boxed()
    }
}

/// Time from `now` until `t` (zero if `t` is in the past).
pub fn until(now: Timestamp, t: Timestamp) -> Duration {
    Duration::try_from(t.duration_since(now)).unwrap_or(Duration::ZERO)
}

/// `now + d`, saturating.
pub fn after(now: Timestamp, d: Duration) -> Timestamp {
    now.saturating_add(SignedDuration::try_from(d).unwrap_or(SignedDuration::MAX))
        .unwrap_or(now)
}
