//! Work policy at the bridge's numerical request boundaries.

use std::cell::Cell;

use catallax_numbers::{
    OperationAllowance, OperationTask, OperationWork, RecordedWork, Refusal, current_operation_work,
    operation_budget_active, with_operation_budget,
};
use soliddraft_kernel_api::Interrupt;

/// The application export ceiling and the legacy default for naming a raw kernel allowance.
pub const APP_REQUEST_WORK: u64 = 100_000_000;

/// The work target for one measured edit. Exceeding it does not refuse work.
pub const APP_WORK_TARGET: u64 = 20_000_000;

/// The most work an edit may do while the window waits for it. A stopgap until every edit
/// builds where the user can cancel it: an edit that can be cancelled has no work limit.
pub const EDIT_STOPGAP_WORK: u64 = 1_000_000_000;

/// The refusal of an edit that reached [`EDIT_STOPGAP_WORK`].
pub const EDIT_STOPGAP_REFUSAL: &str = concat!(
    "edit.work-stopgap: the edit did not finish within 1,000,000,000 work units, ",
    "the most an edit may do while the window waits for it"
);

/// No work limit, as large as the phase 5 census used for one.
const UNLIMITED_WORK: u64 = 1 << 62;

/// The deterministic work charged by a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestWork {
    /// Units charged by the call and its nested operations, excluding earlier work in the caller.
    pub used: u64,
    /// The enclosing request's allowance, when known, or the application's default.
    pub limit: u64,
    /// The allowance ran out; the request must not publish an incomplete result.
    pub exhausted: bool,
}

/// Runs an explicitly limited request. Inside an existing kernel allowance,
/// reports the work charged to that allowance without installing another one.
pub fn with_request_allowance<T>(limit: u64, run: impl FnOnce() -> T) -> (T, RequestWork) {
    let (result, work) = RequestPolicy::Limited(limit).run(run);
    (result, work.request)
}

#[derive(Clone, Copy)]
pub(crate) enum RequestPolicy {
    Open,
    Edit,
    Display,
    Query,
    Measured(u64),
    Limited(u64),
    Export,
}

pub(crate) struct ScopedWork {
    pub(crate) request: RequestWork,
    /// Total charged to the active allowance, including work before this call.
    pub(crate) total_used: u64,
    inherited: bool,
}

impl ScopedWork {
    /// Some operations translate exhaustion only when they own the allowance.
    pub(crate) fn exhausted_here(&self) -> bool {
        !self.inherited && self.request.exhausted
    }
}

impl RequestPolicy {
    fn limit(self) -> u64 {
        match self {
            Self::Open | Self::Query => 100_000_000,
            // A worker runs an edit under an interrupt when the user can cancel it, so it
            // needs no limit. One the window waits for keeps the stopgap.
            Self::Edit if Interrupt::current().is_some() => UNLIMITED_WORK,
            Self::Edit => EDIT_STOPGAP_WORK,
            Self::Display => catallax_app::DEFAULT_BUILD_WORK,
            Self::Measured(limit) | Self::Limited(limit) => limit,
            Self::Export => APP_REQUEST_WORK,
        }
    }

    pub(crate) fn run<T>(self, run: impl FnOnce() -> T) -> (T, ScopedWork) {
        let inherited = active();
        let (result, before, after, limit) = if inherited {
            let before = current_operation_work().used;
            let result = run();
            (result, before, current_operation_work(), request_limit())
        } else {
            let limit = self.limit();
            let (result, after) = with_reported_limit(limit, || with_operation_budget(limit, || stoppable(run)));
            (result, 0, after, limit)
        };
        let request = RequestWork { used: after.used.saturating_sub(before), limit, exhausted: after.exhausted };
        (result, ScopedWork { request, total_used: after.used, inherited })
    }
}

pub(crate) fn active() -> bool {
    operation_budget_active()
}

/// Keeps interactive proof per-build; a caller-owned request prepays both phases.
pub(crate) fn reserve_edit(preview: u64) -> Result<(ReservedWork, Option<ReservedWork>), Refusal> {
    if !active() {
        return Ok((ReservedWork::reserve(preview)?, None));
    }
    let total = catallax_app::DEFAULT_BUILD_WORK;
    let proof = total.checked_sub(preview).expect("constant split is within the application total");
    let mut display = ReservedWork::reserve(total)?;
    let proof = display.split_off(proof).expect("validated phase split");
    Ok((display, Some(proof)))
}

thread_local! {
    static REQUEST_LIMIT: Cell<Option<u64>> = const { Cell::new(None) };
}

pub(crate) fn request_limit() -> u64 {
    REQUEST_LIMIT.get().unwrap_or(APP_REQUEST_WORK)
}

fn with_reported_limit<T>(limit: u64, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<u64>);
    impl Drop for Restore {
        fn drop(&mut self) {
            REQUEST_LIMIT.set(self.0);
        }
    }
    let _restore = Restore(REQUEST_LIMIT.replace(Some(limit)));
    run()
}

/// A single-use share of the caller's work, with its refusal context for a worker.
/// Catallax charges the reservation at its origin, not again when the worker runs it.
pub(crate) struct ReservedWork {
    allowance: OperationAllowance,
    request_limit: u64,
}

impl ReservedWork {
    pub(crate) fn reserve(units: u64) -> Result<Self, Refusal> {
        Ok(Self { allowance: OperationAllowance::reserve(units)?, request_limit: request_limit() })
    }

    pub(crate) fn split_off(&mut self, units: u64) -> Option<Self> {
        Some(Self { allowance: self.allowance.split_off(units)?, request_limit: self.request_limit })
    }

    pub(crate) fn run<T>(self, run: impl FnOnce() -> T) -> (T, OperationWork) {
        with_reported_limit(self.request_limit, || self.allowance.run(|| stoppable(run)))
    }
}

/// Runs a request that owns its allowance so that the thread's [`Interrupt`] can stop it
/// part way. A nested request shares its caller's allowance, so stopping the caller stops it.
fn stoppable<T>(run: impl FnOnce() -> T) -> T {
    let _registration = Interrupt::current().map(|interrupt| {
        let task = OperationTask::capture();
        interrupt.register(Box::new(move || exhaust(task)))
    });
    run()
}

/// Ends the request `task` was captured from at its next work charge. Catallax spends a
/// request's whole allowance when one of its operation tasks panics, whoever catches the
/// panic, and a request with no allowance left refuses instead of answering. The unwind
/// skips the panic hook, so nothing reports it as a crash. The task runs in an allowance
/// of its own, which puts back whatever this thread was running when it ends.
fn exhaust(task: OperationTask) {
    struct Stopped;
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: RecordedWork<()> = task.run(|| std::panic::resume_unwind(Box::new(Stopped)));
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    /// Spends one unit at a time, as a long kernel build does, until the allowance runs
    /// out or a minute has passed. Returns the units spent.
    fn spend() -> u64 {
        let give_up = Instant::now() + Duration::from_secs(60);
        let mut spent = 0;
        while Instant::now() < give_up && OperationAllowance::reserve(1).is_ok() {
            spent += 1;
        }
        spent
    }

    #[test]
    fn an_edit_that_can_be_cancelled_has_no_work_limit() {
        assert_eq!(RequestPolicy::Edit.limit(), EDIT_STOPGAP_WORK);
        assert!(Interrupt::new().run(|| RequestPolicy::Edit.limit()) > 1_000 * EDIT_STOPGAP_WORK);
        assert_eq!(RequestPolicy::Display.limit(), catallax_app::DEFAULT_BUILD_WORK);
    }

    /// The Phase 5 later cut k10 through the P5 housing (Catallax
    /// `crates/catallax-features/tests/fixtures/p5-later-cuts/k10-axis-section.json` at
    /// the pinned revision). With no work limit Catallax spends about 21 billion units on
    /// it, two minutes in a release build, before it refuses. An edit the user can cancel
    /// runs it with no limit, so a stop must end it part way.
    #[test]
    fn an_interrupt_stops_a_doomed_later_cut_part_way() {
        let recipe =
            catallax_recipe::Recipe::parse(include_str!("../tests/fixtures/m8-k10-axis-section.json")).unwrap();
        let interrupt = Interrupt::new();
        let worker = {
            let interrupt = interrupt.clone();
            std::thread::spawn(move || {
                interrupt.run(|| {
                    let (built, work) = RequestPolicy::Edit
                        .run(|| catallax_features::evaluate(&recipe, &mut catallax_numbers::Decider::default()));
                    (built.is_ok(), work.request)
                })
            })
        };
        std::thread::sleep(Duration::from_secs(3));
        assert!(!worker.is_finished(), "the later cut ended on its own within three seconds");
        let asked = Instant::now();
        interrupt.stop();
        let (built, work) = worker.join().unwrap();
        let stopped = asked.elapsed();
        eprintln!("the later cut stopped {stopped:?} after the stop");
        assert!(stopped < Duration::from_secs(10), "the later cut ran on for {stopped:?} after the stop");
        assert!(!built && work.exhausted, "a stopped cut must refuse: {work:?}");
    }

    #[test]
    fn an_interrupt_ends_the_running_request_and_refuses_the_next() {
        let interrupt = Interrupt::new();
        let (started, running) = mpsc::channel();
        let worker = {
            let interrupt = interrupt.clone();
            std::thread::spawn(move || {
                interrupt.run(|| {
                    let (_, first) = RequestPolicy::Limited(u64::MAX / 2).run(|| {
                        started.send(()).unwrap();
                        spend()
                    });
                    let (next_spent, next) = RequestPolicy::Edit.run(spend);
                    (first.request, next_spent, next.request)
                })
            })
        };
        running.recv().unwrap();
        std::thread::sleep(Duration::from_millis(50));
        let asked = Instant::now();
        interrupt.stop();
        let (first, next_spent, next) = worker.join().unwrap();
        assert!(asked.elapsed() < Duration::from_secs(10), "the request ran on after the stop");
        assert!(first.exhausted, "the stopped request must refuse, not answer: {first:?}");
        assert!(next.exhausted && next_spent == 0, "a request after the stop must not run: {next:?}");
        // The thread's own requests are not stopped once the interrupt is gone.
        let (_, after) = RequestPolicy::Limited(10).run(spend);
        assert!(after.request.exhausted && after.request.used == 10, "{:?}", after.request);
    }
}
