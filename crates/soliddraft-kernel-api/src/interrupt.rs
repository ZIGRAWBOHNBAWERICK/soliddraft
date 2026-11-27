//! Stopping a kernel request part way, from another thread.
//!
//! A worker thread runs its kernel requests inside [`Interrupt::run`]. A backend that
//! can stop a request before it ends registers how with [`Interrupt::register`] when the
//! request starts. [`Interrupt::stop`] then ends the request running under it, and every
//! later request under it as soon as that one starts. A stopped request refuses; it never
//! answers with a partial result. A backend that registers nothing runs its requests to
//! the end, and stopping only drops their answers.

use std::cell::RefCell;
use std::sync::{Arc, Mutex, PoisonError};

/// How a backend stops one running request. It returns at once; the request ends at
/// the backend's next check.
pub type Stopper = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct Shared {
    stopped: bool,
    /// How to stop each request now running under this interrupt, by key.
    running: Vec<(u64, Stopper)>,
    next: u64,
}

/// Stops the kernel requests a worker runs under it. Clones share one state, and are
/// equal to each other and to nothing else.
#[derive(Clone, Default)]
pub struct Interrupt(Arc<Mutex<Shared>>);

impl PartialEq for Interrupt {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Interrupt {}

thread_local! {
    static CURRENT: RefCell<Option<Interrupt>> = const { RefCell::new(None) };
}

impl Interrupt {
    /// An interrupt that has not stopped anything yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The interrupt over the requests this thread makes now, if any.
    pub fn current() -> Option<Self> {
        CURRENT.with(|current| current.borrow().clone())
    }

    /// Runs `work` with this interrupt over each kernel request it makes on this thread.
    pub fn run<T>(&self, work: impl FnOnce() -> T) -> T {
        struct Restore(Option<Interrupt>);
        impl Drop for Restore {
            fn drop(&mut self) {
                CURRENT.with(|current| *current.borrow_mut() = self.0.take());
            }
        }
        let _restore = Restore(CURRENT.with(|current| current.borrow_mut().replace(self.clone())));
        work()
    }

    /// Stops the request running under this interrupt and each later one.
    pub fn stop(&self) {
        let running = {
            let mut shared = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            shared.stopped = true;
            std::mem::take(&mut shared.running)
        };
        for (_, stop) in running {
            stop();
        }
    }

    /// Whether [`Self::stop`] was called.
    pub fn stopped(&self) -> bool {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).stopped
    }

    /// Whether a kernel request has registered a stop callback.
    #[cfg(feature = "counting")]
    pub fn request_running(&self) -> bool {
        !self.0.lock().unwrap_or_else(PoisonError::into_inner).running.is_empty()
    }

    /// Keeps `stop` for the request starting now, until the returned registration is
    /// dropped as the request ends. Calls it at once when this interrupt was stopped.
    pub fn register(&self, stop: Stopper) -> Registration {
        let mut shared = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if shared.stopped {
            drop(shared);
            stop();
            return Registration { interrupt: self.clone(), key: None };
        }
        let key = shared.next;
        shared.next += 1;
        shared.running.push((key, stop));
        Registration { interrupt: self.clone(), key: Some(key) }
    }
}

/// A running request's way to be stopped. Dropping it forgets that way.
pub struct Registration {
    interrupt: Interrupt,
    key: Option<u64>,
}

impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(key) = self.key {
            let mut shared = self.interrupt.0.lock().unwrap_or_else(PoisonError::into_inner);
            shared.running.retain(|(running, _)| *running != key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn stop_reaches_the_running_request_and_each_later_one() {
        let stops = Arc::new(AtomicUsize::new(0));
        let counter = |stops: &Arc<AtomicUsize>| -> Stopper {
            let stops = stops.clone();
            Box::new(move || {
                stops.fetch_add(1, Ordering::SeqCst);
            })
        };
        let interrupt = Interrupt::new();
        assert!(Interrupt::current().is_none());
        let running = interrupt.run(|| Interrupt::current().expect("run sets the interrupt").register(counter(&stops)));
        assert!(Interrupt::current().is_none(), "run restores the thread's interrupt");
        let ended = interrupt.register(counter(&stops));
        drop(ended);
        interrupt.stop();
        assert_eq!(stops.load(Ordering::SeqCst), 1, "only the request still running is stopped");
        drop(running);
        let _later = interrupt.register(counter(&stops));
        assert_eq!(stops.load(Ordering::SeqCst), 2, "a request starting after the stop stops at once");
        assert!(interrupt.stopped());
    }
}
