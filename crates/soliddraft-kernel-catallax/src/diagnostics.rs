//! Opt-in observations of the kernel calls the application actually executes.

use std::{cell::RefCell, collections::HashMap, sync::Mutex, thread::ThreadId, time::Instant};

use catallax_numbers::{
    BoundKind, MarkLog, OperationEntry, current_operation_work, operation_budget_active, with_operation_budget,
    with_operation_log,
};
use serde_json::{Value, json};
use soliddraft_kernel_api::{KernelError, KernelResult};

thread_local! {
    static ACTIVE: RefCell<Option<String>> = const { RefCell::new(None) };
}

struct Scope;

impl Drop for Scope {
    fn drop(&mut self) {
        ACTIVE.with(|active| *active.borrow_mut() = None);
    }
}

pub(crate) struct Diagnostics {
    max_work: Option<u64>,
    calls: Mutex<Vec<Value>>,
    work: Mutex<HashMap<ThreadId, u64>>,
    capture_thread: std::thread::ThreadId,
}

impl Diagnostics {
    pub(crate) fn new(max_work: Option<u64>) -> Self {
        Self {
            max_work,
            calls: Mutex::new(Vec::new()),
            work: Mutex::new(HashMap::new()),
            capture_thread: std::thread::current().id(),
        }
    }

    /// Completed outer calls on this thread, independent of draining the evidence log.
    pub(crate) fn work_used(&self) -> Option<u64> {
        self.max_work
            .map(|_| *self.work.lock().expect("diagnostic work lock").get(&std::thread::current().id()).unwrap_or(&0))
    }

    pub(crate) fn take(&self) -> Vec<Value> {
        std::mem::take(&mut *self.calls.lock().expect("diagnostics lock"))
    }

    pub(crate) fn observe<T>(&self, operation: &str, run: impl FnOnce() -> KernelResult<T>) -> KernelResult<T> {
        self.scoped(operation, self.max_work, run)
    }

    /// Observes one application export. A diagnostic allowance above the application's
    /// limit does not reach it.
    pub(crate) fn observe_export<T>(&self, operation: &str, run: impl FnOnce() -> KernelResult<T>) -> KernelResult<T> {
        self.scoped(operation, self.max_work.map(|limit| limit.min(APP_REQUEST_WORK)), run)
    }

    fn scoped<T>(
        &self,
        operation: &str,
        max_work: Option<u64>,
        run: impl FnOnce() -> KernelResult<T>,
    ) -> KernelResult<T> {
        let start = Instant::now();
        let on_capture_thread = std::thread::current().id() == self.capture_thread;
        if let Some(parent) = ACTIVE.with(|active| active.borrow().clone()) {
            let result = run();
            self.calls.lock().expect("diagnostics lock").push(json!({
                "operation":operation,"log_scope":parent,"nested":true,
                "on_capture_thread":on_capture_thread,
                "status":if result.is_ok() {"answered"} else {"refused"},
                "error":result.as_ref().err().map(ToString::to_string),
                "max_work":null,"work":null,"wall_ms":start.elapsed().as_secs_f64()*1000.0,
                "operation_log":{"format":"catallax-operation-log/1","operations":[]}
            }));
            return result;
        }
        ACTIVE.with(|active| *active.borrow_mut() = Some(operation.to_owned()));
        let _scope = Scope;
        // Background mass requests carry their own allowance, separate from edit and export limits.
        let caller_mass_allowance = operation == "mass_properties" && operation_budget_active();
        let before = current_operation_work();
        let ((mut result, work), entries) = with_operation_log(|| match (max_work, caller_mass_allowance) {
            (_, true) => {
                let result = run();
                let after = current_operation_work();
                (result, Some(Spent { used: after.used - before.used, exhausted: after.exhausted }))
            }
            (Some(limit), false) => {
                // A request allowance, so a refusal inside names this limit, not the default.
                let (result, work) = with_request_allowance(limit, run);
                (result, Some(Spent { used: work.used, exhausted: work.exhausted }))
            }
            (None, false) => (run(), None),
        });
        if work.is_some_and(|w| w.exhausted) && !matches!(result, Err(KernelError::DisplayLimit { .. })) {
            result = Err(
                if matches!(
                    operation,
                    "propose_import" | "source_solids" | "import_step" | "import_iges" | "import_mesh"
                ) {
                    KernelError::InvalidInput(crate::import::TOO_COMPLEX.into())
                } else {
                    KernelError::Unavailable("numbers.operation-budget: operation work allowance exhausted".into())
                },
            );
        }
        if let Some(work) = work {
            let mut totals = self.work.lock().expect("diagnostic work lock");
            let total = totals.entry(std::thread::current().id()).or_default();
            *total = total.saturating_add(work.used);
        }
        self.calls.lock().expect("diagnostics lock").push(json!({
            "operation":operation, "log_scope":operation,"nested":false, "status":if result.is_ok() {"answered"} else {"refused"},
            "on_capture_thread":on_capture_thread,
            "error":result.as_ref().err().map(ToString::to_string),
            "max_work":if caller_mass_allowance {None} else {max_work},
            "work":work.map(|w| json!({"used":w.used,"exhausted":w.exhausted,
                "target":APP_WORK_TARGET,"over_target":w.used > APP_WORK_TARGET})),
            "wall_ms":start.elapsed().as_secs_f64()*1000.0,
            "operation_log":{"format":"catallax-operation-log/1", "operations":entries.iter().enumerate()
                .map(|(sequence, e)| entry(sequence, e)).collect::<Vec<_>>()}
        }));
        result
    }
}

fn marks(log: &MarkLog, step: &str) -> Vec<Value> {
    log.iter()
        .map(|m| {
            json!({"code":m.code,"detail":m.detail,"step":step,
            "bound":m.bound.to_decimal_bounds(40).1, "kind":match m.kind {
                BoundKind::Distance=>"distance",BoundKind::Angle=>"angle",BoundKind::Value=>"value"
            }})
        })
        .collect()
}

fn entry(sequence: usize, e: &OperationEntry) -> Value {
    let decisions = e.after.as_ref().map(|a| {
        let b = &e.before;
        json!({"total":a.decisions.saturating_sub(b.decisions),
            "filter":a.filter.saturating_sub(b.filter), "refined":a.refined.saturating_sub(b.refined),
            "exact":a.exact.saturating_sub(b.exact),
            "coincident_exact":a.coincident_exact.saturating_sub(b.coincident_exact),
            "coincident_separation":a.coincident_separation.saturating_sub(b.coincident_separation),
            "coincident_fact":a.coincident_fact.saturating_sub(b.coincident_fact),
            "not_exact":a.not_exact.saturating_sub(b.not_exact), "refused":a.refused.saturating_sub(b.refused),
            "max_bits_high_water":a.max_bits})
    });
    json!({"sequence":sequence,"kernel":e.kernel,"id":e.id,"operation":e.operation,
        "status":if e.after.is_none() {"interrupted"} else if e.refusal.is_some() {"refused"} else {"answered"},
        "decisions":decisions,"marks":marks(&e.marks,&e.id),"local_tolerances":null,
        "refusal":e.refusal.as_ref().map(|r| json!({"code":r.code,"detail":r.detail})),
        "work":{"before":e.work_before,"after":e.work_after,"used":e.work_after-e.work_before},"wall_ms":e.wall_ms})
}

/// The deterministic work one application request may use. Kernel replay and the
/// scoreboard keep their own limits. Until the speed phase it is five times
/// [`APP_WORK_TARGET`], so slow parts still build; a request over the target is
/// recorded as such.
pub const APP_REQUEST_WORK: u64 = 100_000_000;

/// The work one edit should fit in. A request that uses more is marked
/// `over_target` in the diagnostics, with its work per stage, for the speed phase.
pub const APP_WORK_TARGET: u64 = 20_000_000;

/// The work one observed call charged to its allowance.
#[derive(Clone, Copy)]
struct Spent {
    used: u64,
    exhausted: bool,
}

/// The deterministic work one whole request used: opening, rebuilding and writing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestWork {
    /// Units every kernel call in the request used together.
    pub used: u64,
    /// The request's allowance.
    pub limit: u64,
    /// The allowance ran out; nothing the request computed may be published.
    pub exhausted: bool,
}

/// Runs a whole request on one deterministic allowance that every kernel call in it
/// shares on this thread, with no fresh allowance for a nested call.
pub fn with_request_allowance<T>(limit: u64, run: impl FnOnce() -> T) -> (T, RequestWork) {
    struct Restore(Option<u64>);
    impl Drop for Restore {
        fn drop(&mut self) {
            REQUEST_LIMIT.set(self.0);
        }
    }
    let _restore = Restore(REQUEST_LIMIT.replace(Some(limit)));
    let (value, work) = with_operation_budget(limit, run);
    (value, RequestWork { used: work.used, limit, exhausted: work.exhausted })
}

thread_local! {
    static REQUEST_LIMIT: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// The allowance of the request running on this thread, for a refusal to name: the
/// caller's when it set one through [`with_request_allowance`], otherwise the
/// application's.
pub(crate) fn request_limit() -> u64 {
    REQUEST_LIMIT.get().unwrap_or(APP_REQUEST_WORK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use catallax_numbers::{Decider, OperationAllowance, OperationSpan};

    #[test]
    fn nested_queries_share_one_log_and_work_scope() {
        let diagnostics = Diagnostics::new(Some(100));
        diagnostics
            .observe("view_curves", || {
                diagnostics.observe("tessellate", || {
                    let decider = Decider::default();
                    OperationSpan::begin("catallax", &"mesh", "mesh", &decider).finish(&decider, None, None);
                    Ok(())
                })
            })
            .unwrap();
        let calls = diagnostics.take();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["operation"], "tessellate");
        assert_eq!(calls[0]["log_scope"], "view_curves");
        assert!(calls[0]["work"].is_null());
        assert_eq!(calls[1]["operation_log"]["operations"][0]["operation"], "mesh");
        diagnostics.observe("next_request", || Ok(())).unwrap();
        assert_eq!(diagnostics.take()[0]["nested"], false);
    }

    #[test]
    fn an_export_stops_at_the_application_limit_under_a_larger_diagnostic_allowance() {
        let diagnostics = Diagnostics::new(Some(5 * APP_REQUEST_WORK));
        let spend = || {
            OperationAllowance::reserve(APP_REQUEST_WORK + 1)
                .map(|_| ())
                .map_err(|e| KernelError::Unavailable(e.to_string()))
        };
        diagnostics.observe("extrude", spend).unwrap();
        let refused = diagnostics.observe_export("export_step", spend).unwrap_err();
        assert!(refused.to_string().contains("numbers.operation-budget"), "{refused}");
        let calls = diagnostics.take();
        assert_eq!(calls[0]["max_work"], 5 * APP_REQUEST_WORK);
        assert_eq!(calls[1]["max_work"], APP_REQUEST_WORK);
        assert_eq!(calls[1]["work"]["exhausted"], true);
    }

    #[test]
    fn a_request_over_the_work_target_is_marked_and_still_answered() {
        let diagnostics = Diagnostics::new(Some(APP_REQUEST_WORK));
        let spend = |units| {
            move || OperationAllowance::reserve(units).map(|_| ()).map_err(|e| KernelError::Unavailable(e.to_string()))
        };
        diagnostics.observe("fillet", spend(APP_WORK_TARGET)).unwrap();
        diagnostics.observe("fillet", spend(APP_WORK_TARGET + 1)).unwrap();
        let calls = diagnostics.take();
        assert_eq!(calls[0]["work"]["target"], APP_WORK_TARGET);
        assert_eq!(calls[0]["work"]["over_target"], false);
        assert_eq!(calls[1]["status"], "answered");
        assert_eq!(calls[1]["work"]["used"], APP_WORK_TARGET + 1);
        assert_eq!(calls[1]["work"]["over_target"], true);
    }

    #[test]
    fn a_request_names_its_own_allowance_until_it_ends() {
        assert_eq!(request_limit(), APP_REQUEST_WORK);
        assert_eq!(with_request_allowance(APP_WORK_TARGET, request_limit).0, APP_WORK_TARGET);
        assert_eq!(request_limit(), APP_REQUEST_WORK);
        let ended = std::panic::catch_unwind(|| with_request_allowance(7, || panic!("refused")));
        assert!(ended.is_err());
        assert_eq!(request_limit(), APP_REQUEST_WORK, "a request that panics leaves no limit behind");
    }

    #[test]
    fn exhausted_display_keeps_its_named_quantified_refusal() {
        let diagnostics = Diagnostics::new(Some(5));
        let result = diagnostics.observe("display_bound", || {
            OperationAllowance::reserve(5).unwrap();
            assert!(OperationAllowance::reserve(1).is_err());
            Err::<(), _>(KernelError::DisplayLimit { used: 5, limit: 5 })
        });
        assert!(matches!(result, Err(KernelError::DisplayLimit { used: 5, limit: 5 })));
        let calls = diagnostics.take();
        assert_eq!(calls[0]["work"]["exhausted"], true);
        assert!(calls[0]["error"].as_str().unwrap().contains("SD-DISPLAY-01"));
    }

    #[test]
    fn an_explicit_lower_allowance_is_the_limit_a_refusal_names() {
        let diagnostics = Diagnostics::new(Some(APP_WORK_TARGET));
        let named = diagnostics.observe("display_bound", || Ok(request_limit())).unwrap();
        assert_eq!(named, APP_WORK_TARGET);
        let named = diagnostics.observe_export("export_step", || Ok(request_limit())).unwrap();
        assert_eq!(named, APP_WORK_TARGET);
        assert_eq!(request_limit(), APP_REQUEST_WORK, "the allowance ends with the call");
        let higher = Diagnostics::new(Some(5 * APP_REQUEST_WORK));
        assert_eq!(higher.observe_export("export_step", || Ok(request_limit())).unwrap(), APP_REQUEST_WORK);
    }

    #[test]
    fn background_calls_remain_distinct_from_the_capture_thread() {
        let diagnostics = Diagnostics::new(None);
        std::thread::scope(|scope| {
            scope.spawn(|| diagnostics.observe("tessellate", || Ok(()))).join().unwrap().unwrap();
        });
        diagnostics.observe("extrude", || Ok(())).unwrap();
        let calls = diagnostics.take();
        assert_eq!(calls[0]["on_capture_thread"], false);
        assert_eq!(calls[1]["on_capture_thread"], true);
    }

    #[test]
    fn an_explicit_mass_allowance_governs_the_observed_query() {
        let diagnostics = Diagnostics::new(Some(1));
        let spend = || OperationAllowance::reserve(4).map(|_| ()).map_err(|e| KernelError::Unavailable(e.to_string()));
        let (answer, work) = with_request_allowance(10, || diagnostics.observe("mass_properties", spend));
        answer.unwrap();
        assert_eq!(work.used, 4);
        assert!(!work.exhausted);
        let (answer, work) = with_request_allowance(3, || diagnostics.observe("mass_properties", spend));
        assert!(answer.is_err());
        assert!(work.exhausted);
        let calls = diagnostics.take();
        assert_eq!(calls[0]["work"]["used"], 4);
        assert_eq!(calls[1]["work"]["exhausted"], true);
    }
}

#[cfg(test)]
mod feature_work_tests {
    use crate::CatallaxKernel;
    use catallax_numbers::OperationAllowance;
    use soliddraft_kernel_api::Kernel;

    #[test]
    fn feature_work_uses_diagnostic_allowances_without_nesting_them() {
        let kernel = CatallaxKernel::with_diagnostics(100);
        let work = kernel
            .measure_work(&mut || {
                for cost in [3, 4] {
                    kernel
                        .observe("feature-work", || {
                            OperationAllowance::reserve(cost).unwrap();
                            Ok(())
                        })
                        .unwrap();
                }
            })
            .unwrap();
        assert_eq!(work, Some(7));
        assert_eq!(kernel.diagnostics.as_ref().unwrap().take().len(), 2);
        let work = kernel
            .measure_work(&mut || {
                kernel
                    .observe("next-feature", || {
                        OperationAllowance::reserve(5).unwrap();
                        Ok(())
                    })
                    .unwrap();
            })
            .unwrap();
        assert_eq!(work, Some(5));
    }
}
