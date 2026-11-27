//! Thread CPU time for the serial whole-edge prover.

pub(super) fn proof_cpu_ns() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        use rustix::time::{ClockId, clock_gettime};
        let clock = clock_gettime(ClockId::ThreadCPUTime);
        u64::try_from(clock.tv_sec).ok()?.checked_mul(1_000_000_000)?.checked_add(u64::try_from(clock.tv_nsec).ok()?)
    }
    #[cfg(not(target_os = "linux"))]
    None
}
