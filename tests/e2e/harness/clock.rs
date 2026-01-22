use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Controllable time for stale session testing.
///
/// This clock can be passed to CtxRepo via `with_time_provider()` to control
/// time during tests, enabling testing of stale session detection.
#[derive(Clone)]
pub struct MockClock {
    current: Arc<AtomicI64>,
}

impl MockClock {
    /// Creates a time provider function suitable for passing to CtxRepo.
    pub fn as_provider(&self) -> impl Fn() -> i64 + Send + Sync + 'static {
        let current = self.current.clone();
        move || current.load(Ordering::SeqCst)
    }
}

impl MockClock {
    /// Create a new mock clock starting at current time
    pub fn new() -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        Self {
            current: Arc::new(AtomicI64::new(now)),
        }
    }

    /// Get current timestamp
    pub fn now(&self) -> i64 {
        self.current.load(Ordering::SeqCst)
    }

    /// Advance time by duration
    pub fn advance(&self, duration: Duration) {
        let seconds = duration.as_secs() as i64;
        self.current.fetch_add(seconds, Ordering::SeqCst);
    }

    /// Advance time by hours
    pub fn advance_hours(&self, hours: u64) {
        self.advance(Duration::from_secs(hours * 3600));
    }

    /// Advance time by days
    pub fn advance_days(&self, days: u64) {
        self.advance(Duration::from_secs(days * 86400));
    }
}

impl Default for MockClock {
    fn default() -> Self {
        Self::new()
    }
}
