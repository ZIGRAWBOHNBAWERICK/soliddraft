//! Browser worker entry points. Trunk builds these into a separate module.

#[cfg(target_arch = "wasm32")]
mod bindings {
    use soliddraft_app::kernel_worker::{worker_hand_over, worker_run};
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub fn kernel_worker_commit() -> String {
        env!("SOLIDDRAFT_COMMIT").to_owned()
    }

    /// Runs job `id`; a checked build posts its rebuilt part to the page before answering.
    #[wasm_bindgen]
    pub fn kernel_worker_run(header: &str, part: &[u8], id: f64) -> String {
        worker_run(header, part, id)
    }

    #[wasm_bindgen]
    pub fn kernel_worker_hand_over() -> Vec<u8> {
        worker_hand_over()
    }
}

fn main() {}
