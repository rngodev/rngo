use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// A thread-safe handle for stopping a `Proxy`; once stopped, `send` sends nothing and any
/// realtime wait ends immediately. The proxy's owner still calls `finish`.
#[derive(Clone, Debug, Default)]
pub struct StopHandle(Arc<(Mutex<bool>, Condvar)>);

impl StopHandle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stop(&self) {
        *self.lock() = true;
        self.0.1.notify_all();
    }

    pub fn is_stopped(&self) -> bool {
        *self.lock()
    }

    /// Blocks for up to `timeout`, returning early if stopped; returns whether it is stopped.
    pub(crate) fn wait_timeout(&self, timeout: Duration) -> bool {
        let (guard, _) = self
            .0
            .1
            .wait_timeout_while(self.lock(), timeout, |stopped| !*stopped)
            .unwrap_or_else(PoisonError::into_inner);
        *guard
    }

    fn lock(&self) -> MutexGuard<'_, bool> {
        self.0.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
