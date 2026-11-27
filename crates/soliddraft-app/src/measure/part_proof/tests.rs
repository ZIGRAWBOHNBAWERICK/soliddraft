use super::*;
use crate::{app_tools::AppTools, state::AppState};
use egui::{Event, Modifiers, PointerButton, Pos2, Shape};
use serde_json::Value;
use soliddraft_assistant::{Caller, tools::call_checked};
use soliddraft_kernel_api::EdgeProof;
use std::sync::Arc;
use std::time::{Duration, Instant};

const P1: &str = include_str!("../../../../../ci/scenarios/canonical-p1.json");
const FITTED: &str = include_str!("../../../../../ci/scenarios/ring-fillet.json");
const BOUND: f64 = 0.000_000_03125;
const TIMEOUT: Duration = Duration::from_secs(10);

fn replay(source: &str, last: &str) -> Document {
    let scenario: Value = serde_json::from_str(source).unwrap();
    let mut state = AppState::new(Document::new(scenario["name"].as_str().unwrap(), crate::startup::kernel()));
    let caller = Caller::Mcp { client: "part-proof".into() };
    for step in scenario["steps"].as_array().unwrap() {
        let tool = step["tool"].as_str().unwrap();
        let result = call_checked(&mut AppTools { state: &mut state }, tool, step["arguments"].clone(), &caller);
        assert!(!result.is_error, "{tool}: {}", result.content);
        if step["label"] == last {
            return state.doc;
        }
    }
    panic!("missing scenario checkpoint {last}");
}

struct Screen {
    ctx: egui::Context,
    texts: Vec<(String, Pos2)>,
}

impl Screen {
    fn new() -> Self {
        Self { ctx: egui::Context::default(), texts: Vec::new() }
    }

    fn frame(&mut self, state: &mut State, doc: &Document, events: Vec<Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 900.0))),
            events,
            ..Default::default()
        };
        let output = self.ctx.run_ui(input, |ui| {
            state.poll(doc, ui.ctx());
            state.controls(ui, doc);
            state.dialogs(doc, ui.ctx());
        });
        fn text(shape: Shape, out: &mut Vec<(String, Pos2)>) {
            match shape {
                Shape::Text(shape) if shape.visual_bounding_rect().is_positive() => {
                    out.push((shape.galley.text().to_owned(), shape.visual_bounding_rect().center()));
                }
                Shape::Vec(shapes) => shapes.into_iter().for_each(|s| text(s, out)),
                _ => {}
            }
        }
        self.texts.clear();
        for shape in output.shapes {
            text(shape.shape, &mut self.texts);
        }
    }

    fn contains(&self, fragment: &str) -> bool {
        self.texts.iter().any(|(s, _)| s.contains(fragment))
    }

    fn click(&mut self, state: &mut State, doc: &Document, label: &str) {
        // A newly opened egui window or modal may complete its sizing on the next frame.
        self.frame(state, doc, Vec::new());
        let &(_, pos) = self.texts.iter().find(|(text, _)| text == label).unwrap_or_else(|| panic!("no {label:?}"));
        for pressed in [true, false] {
            self.frame(
                state,
                doc,
                vec![
                    Event::PointerMoved(pos),
                    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE },
                ],
            );
        }
    }

    fn until(&mut self, state: &mut State, doc: &Document, predicate: impl Fn(&Self) -> bool) {
        let end = Instant::now() + TIMEOUT;
        loop {
            self.frame(state, doc, Vec::new());
            if predicate(self) {
                break;
            }
            assert!(Instant::now() < end, "visible text: {:?}", self.texts);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

fn fake(miss_one: bool) -> TestProver {
    Arc::new(move |kernel, body, _, _| {
        let mut edges = kernel.topology(body).unwrap().edges;
        if miss_one {
            edges.pop(); // A capped kernel may return no row for an unvisited edge.
        }
        let edges = edges.into_iter().map(|edge| EdgeProof { edge: edge.id, bound_mm: Some(BOUND) }).collect();
        Ok(EdgeProofRun { edges, work_used: 123, cpu_ns: Some(5_000_000), threads_used: 2 })
    })
}

#[test]
fn button_proves_p1_and_reports_the_unproved_edge_on_a_fitted_part() {
    for (source, checkpoint, miss_one) in [(P1, "original", false), (FITTED, "ring-fillet build", true)] {
        let mut doc = replay(source, checkpoint);
        let mesh_version = doc.mesh_version;
        let display = doc.meshes.clone();
        assert!(!display.is_empty());
        let mut state = State { prover: Some(fake(miss_one)), ..Default::default() };
        let mut screen = Screen::new();
        screen.click(&mut state, &doc, "Prove part");
        screen.until(&mut state, &doc, |s| s.contains("Prove this complex part?") || s.contains("Open proof report"));
        if screen.contains("Prove this complex part?") {
            screen.click(&mut state, &doc, "Continue");
        }
        screen.until(&mut state, &doc, |s| s.contains("Open proof report"));
        assert_eq!(screen.contains("Fully proven"), !miss_one);
        screen.click(&mut state, &doc, "Open proof report");
        screen.until(&mut state, &doc, |screen| screen.contains("Part proof report") && screen.contains("Wall time"));
        for text in [
            "Proof CPU time: 0.005 s",
            "Proof threads used (peak): 2",
            "Kernel proof work units: 123",
            "Faces:",
            "Edges:",
            "Vertices:",
        ] {
            assert!(screen.contains(text), "missing {text:?}");
        }
        let report = state.report.as_ref().unwrap();
        let body = &report.bodies[0];
        let topology = doc.kernel().topology(doc.model().body().unwrap()).unwrap();
        assert_eq!(body.edges.len(), topology.edges.len());
        assert_eq!(body.surfaces.values().sum::<usize>(), topology.faces.len());
        assert_eq!(body.vertices, topology.vertex_count);
        assert_eq!(body.edges.iter().filter(|(_, bound)| bound.is_none()).count(), usize::from(miss_one));
        assert_eq!(body.edges[0].1, Some(BOUND));
        assert!(body.mass.as_ref().is_ok_and(|mass| mass.area > 0.0 && mass.volume > 0.0), "{:?}", body.mass);
        assert_eq!(doc.mesh_version, mesh_version, "proof changed the renderer's mesh version");
        assert_eq!(doc.meshes, display, "proof changed the displayed geometry");

        if !miss_one {
            let before_edit = doc.model().geometry_hash();
            doc.undo().unwrap();
            assert_ne!(doc.model().geometry_hash(), before_edit);
            screen.frame(&mut state, &doc, Vec::new());
            assert!(!screen.contains("Fully proven"));
            let bytes = doc.model().snapshot().encode().unwrap().1;
            let reopened = model::Document::from_bytes(&bytes, crate::startup::kernel()).unwrap();
            let reopened = Document::from_model(reopened);
            screen.frame(&mut state, &reopened, Vec::new());
            assert!(!screen.contains("Fully proven") && !screen.contains("Open proof report"));
        }
    }
}

#[test]
fn heavy_warning_waits_and_cancel_interrupts_the_prover() {
    let doc = replay(FITTED, "ring-fillet build");
    let (entered, waiting) = mpsc::channel();
    let (stopped, observed) = mpsc::channel();
    let prover: TestProver = Arc::new(move |_, _, _, interrupt| {
        entered.send(()).unwrap();
        let end = Instant::now() + TIMEOUT;
        while !interrupt.stopped() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(2));
        }
        stopped.send(interrupt.stopped()).unwrap();
        Err(CANCELLED.into())
    });
    let mut state = State { prover: Some(prover), ..Default::default() };
    let mut screen = Screen::new();
    screen.click(&mut state, &doc, "Prove part");
    screen.until(&mut state, &doc, |s| s.contains("Prove this complex part?"));
    assert!(screen.contains("may take a long time and use a lot of computing power"));
    assert_eq!(waiting.try_recv(), Err(TryRecvError::Empty));
    screen.click(&mut state, &doc, "Cancel");
    screen.until(&mut state, &doc, |s| s.contains(CANCELLED));
    assert_eq!(waiting.try_recv(), Err(TryRecvError::Empty));

    screen.click(&mut state, &doc, "Prove part");
    screen.until(&mut state, &doc, |s| s.contains("Prove this complex part?"));
    screen.click(&mut state, &doc, "Continue");
    screen.until(&mut state, &doc, |s| s.contains("Proving edges…"));
    waiting.recv_timeout(TIMEOUT).unwrap();
    screen.click(&mut state, &doc, "Cancel proof");
    assert!(observed.recv_timeout(TIMEOUT).unwrap(), "cancel must stop the worker, not just hide its answer");
    screen.until(&mut state, &doc, |s| s.contains(CANCELLED));
    assert!(!screen.contains("Open proof report"));
}
