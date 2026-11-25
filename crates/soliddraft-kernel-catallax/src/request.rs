//! Work policy at the bridge's numerical request boundaries.

use std::cell::Cell;

use catallax_numbers::{
    OperationAllowance, OperationWork, Refusal, current_operation_work, operation_budget_active, with_operation_budget,
};

/// The application export ceiling and the legacy default for naming a raw kernel allowance.
pub const APP_REQUEST_WORK: u64 = 100_000_000;

/// The work target for one measured edit. Exceeding it does not refuse work.
pub const APP_WORK_TARGET: u64 = 20_000_000;

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
            Self::Edit | Self::Display => catallax_app::DEFAULT_BUILD_WORK,
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
            let (result, after) = with_reported_limit(limit, || with_operation_budget(limit, run));
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
    let total = RequestPolicy::Edit.limit();
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
        with_reported_limit(self.request_limit, || self.allowance.run(run))
    }
}
