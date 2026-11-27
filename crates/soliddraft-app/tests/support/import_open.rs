use std::time::{Duration, Instant};

use egui_kittest::{Harness, kittest::Queryable};

/// Drives the frame that starts the open, then waits for the worker's window to close.
pub fn settle<T>(app: &mut Harness<'_, T>) {
    let started = Instant::now();
    app.run_steps(3);
    while app.query_by_label("Checking STEP file…").is_some() {
        assert!(started.elapsed() < Duration::from_secs(60), "STEP open worker did not answer");
        std::thread::sleep(Duration::from_millis(1));
        app.run_steps(1);
    }
    app.run_steps(3);
}
