//! Interior section offsets must survive a large dimension parallel to the plane.

use soliddraft_app::{
    document::{Command, Document, ExtrudeParams, ProfileKind},
    section::State,
};
use std::time::{Duration, Instant};

#[test]
fn scaled_part_interior_sections_stop_at_the_requested_plane() {
    let mut doc = Document::new("Wide section plate", soliddraft_app::kernel());
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams {
            profile: ProfileKind::Rectangle { width: 1_000_000_000.0, height: 40.0 },
            depth: 20.0,
            ..Default::default()
        },
        via_assistant: false,
    })
    .unwrap();
    let ctx = egui::Context::default();
    let mut section = State::default();
    section.toggle(&doc);
    let mut bounds = Vec::new();
    for offset in [0.5, 19.5] {
        section.offset = offset;
        let started = Instant::now();
        let mesh = loop {
            section.refresh(&doc, &ctx);
            if let Some(mesh) = section.mesh(&doc) {
                break mesh;
            }
            assert!(started.elapsed() < Duration::from_secs(60), "section at {offset} did not finish");
            std::thread::sleep(Duration::from_millis(1));
        };
        bounds.push((offset, mesh.bounds()));
    }
    for (offset, bounds) in bounds {
        let (min, max) = bounds.unwrap_or_else(|| panic!("the retained slab at {offset} must be nonempty"));
        assert!(min.z.abs() < 1e-4, "the slab at {offset} starts at z=0, got {min:?}");
        assert!((f64::from(max.z) - offset).abs() < 1e-4, "the slab ends at {offset}, got {max:?}");
    }
}
