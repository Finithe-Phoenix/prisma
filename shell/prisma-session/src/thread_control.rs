//! Cooperative host cancellation for a threaded guest run.

use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// Wakes the workers of a threaded guest run when the host requests shutdown.
///
/// Create a new control for each run. Share it with the cancelling thread by
/// reference or through an [`std::sync::Arc`]. Once cancelled, the control
/// stays cancelled, including when cancellation precedes the start of a wait.
#[derive(Debug, Default)]
pub struct ThreadRunControl {
    cancelled: Mutex<bool>,
    changed: Condvar,
}

impl ThreadRunControl {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Request shutdown and wake every worker waiting for a turn or timer.
    pub fn cancel(&self) {
        let mut cancelled = self
            .cancelled
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *cancelled = true;
        self.changed.notify_all();
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        *self
            .cancelled
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Notify workers after updating a scheduling predicate under the process
    /// lock. Taking the same signal lock as `wait` covers the handoff from
    /// observing that predicate to parking on the condition variable.
    pub(crate) fn notify_all(&self) {
        let _signal = self
            .cancelled
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.changed.notify_all();
    }

    /// Release the process guard and wait for a scheduler update, cancellation
    /// or timeout. The caller reacquires the process lock and rechecks its
    /// predicates after every return, including spurious wakeups.
    pub(crate) fn wait<T>(&self, process_guard: MutexGuard<'_, T>, duration: Option<Duration>) {
        let cancelled = self
            .cancelled
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        drop(process_guard);
        if *cancelled {
            return;
        }
        if let Some(timeout) = duration {
            drop(
                self.changed
                    .wait_timeout(cancelled, timeout)
                    .unwrap_or_else(PoisonError::into_inner),
            );
        } else {
            drop(
                self.changed
                    .wait(cancelled)
                    .unwrap_or_else(PoisonError::into_inner),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ThreadRunControl;
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn scheduling_update_wakes_waiter_without_cancelling() {
        let control = Arc::new(ThreadRunControl::new());
        let process = Arc::new(Mutex::new(false));
        let (started, entered) = mpsc::sync_channel(0);
        let (done, finished) = mpsc::channel();
        let worker_control = Arc::clone(&control);
        let worker_process = Arc::clone(&process);
        let worker = thread::spawn(move || {
            let mut guard = worker_process.lock().unwrap();
            started.send(()).unwrap();
            while !*guard {
                worker_control.wait(guard, None);
                guard = worker_process.lock().unwrap();
            }
            done.send(()).unwrap();
        });
        entered.recv().unwrap();
        {
            let mut ready = process.lock().unwrap();
            *ready = true;
            control.notify_all();
        }
        finished.recv_timeout(Duration::from_secs(5)).unwrap();
        worker.join().unwrap();
        assert!(!control.is_cancelled());
    }

    #[test]
    fn host_cancellation_wakes_timed_and_untimed_waits() {
        for duration in [None, Some(Duration::from_secs(3600))] {
            let control = Arc::new(ThreadRunControl::new());
            let process = Arc::new(Mutex::new(()));
            let (ready, entered) = mpsc::sync_channel(0);
            let (done, finished) = mpsc::channel();
            let worker_control = Arc::clone(&control);
            let worker_process = Arc::clone(&process);
            let worker = thread::spawn(move || {
                let mut guard = worker_process.lock().unwrap();
                ready.send(()).unwrap();
                while !worker_control.is_cancelled() {
                    worker_control.wait(guard, duration);
                    guard = worker_process.lock().unwrap();
                }
                done.send(()).unwrap();
            });
            entered.recv().unwrap();
            // Acquiring the process lock proves that the worker has begun the
            // wait handoff. Cancellation can race with entering the condvar.
            drop(process.lock().unwrap());
            control.cancel();
            finished.recv_timeout(Duration::from_secs(5)).unwrap();
            worker.join().unwrap();

            // A cancellation requested before the next wait is also retained.
            control.wait(process.lock().unwrap(), None);
            assert!(control.is_cancelled());
        }
    }
}
