//! Scan reference review and fitting workspace.

use crate::document::{Command, Document, FeatureId, FeatureKind, Selection};
use crate::state::{AppState, NoteKind};
use egui::{Color32, Stroke};
use glam::Vec3;
use soliddraft_kernel_api::scan::{ScanFit, ScanMapPoint, ScanReference, ScanSource};
use std::sync::{Arc, mpsc};

pub struct Workspace {
    review: Option<Review>,
    fitting: Option<Pending>,
    fitted: Option<Fitted>,
    cached: Option<(String, Arc<ScanReference>)>,
    pub open: bool,
    pub show_reference: bool,
    pub direction: Direction,
    error: Option<String>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            review: None,
            fitting: None,
            fitted: None,
            cached: None,
            open: false,
            show_reference: true,
            direction: Direction::default(),
            error: None,
        }
    }
}

struct Review {
    source: ScanSource,
    reference: Arc<ScanReference>,
    revision: u64,
}
struct Pending {
    feature: FeatureId,
    revision: u64,
    result: mpsc::Receiver<Result<ScanFit, String>>,
}
struct Fitted {
    feature: FeatureId,
    revision: u64,
    fit: ScanFit,
}

#[derive(Clone, Copy, Default, PartialEq)]
pub enum Direction {
    #[default]
    ScanToPart,
    PartToScan,
}

pub fn selected(doc: &Document) -> Option<FeatureId> {
    if let Some(Selection::Feature(id)) = doc.selection
        && doc.feature(id).is_some_and(|f| matches!(f.kind, FeatureKind::Scan(_)))
    {
        return Some(id);
    }
    doc.features.iter().find(|f| matches!(f.kind, FeatureKind::Scan(_)) && !f.suppressed).map(|f| f.id)
}

pub fn can_fit(state: &AppState) -> bool {
    !state.in_sketch()
        && !state.doc.proof_pending()
        && state.scan.fitting.is_none()
        && selected(&state.doc).is_some_and(|id| {
            state.doc.feature(id).is_some_and(|f| matches!(&f.kind, FeatureKind::Scan(s) if s.fit.is_none()))
        })
        && state.doc.display_body().is_none()
}

pub fn review(state: &mut AppState, name: String, bytes: Vec<u8>) -> Result<(), String> {
    let source = ScanSource { name, sha256: soliddraft_model::sha256_hex(&bytes), bytes };
    let reference = Arc::new(state.doc.kernel().scan_reference(&source).map_err(plain)?);
    state.scan.review = Some(Review { source, reference, revision: state.doc.revision() });
    state.scan.error = None;
    state.scan.open = true;
    Ok(())
}

fn plain(e: soliddraft_kernel_api::KernelError) -> String {
    match e {
        soliddraft_kernel_api::KernelError::InvalidInput(s) => s,
        soliddraft_kernel_api::KernelError::Unsupported(_) => "Coming soon".into(),
        other => other.to_string(),
    }
}

pub fn start_fit(state: &mut AppState) {
    if !can_fit(state) {
        return;
    }
    let feature = selected(&state.doc).expect("scan selected");
    let FeatureKind::Scan(scan) = &state.doc.feature(feature).expect("scan exists").kind else { return };
    let source = scan.source.clone();
    let kernel = state.doc.kernel().clone();
    let (tx, result) = mpsc::channel();
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(move || {
        let _ = tx.send(kernel.fit_scan(&source).map_err(plain));
    });
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (kernel, source);
        let _ = tx.send(Err("Coming soon".into()));
    }
    state.scan.fitting = Some(Pending { feature, revision: state.doc.revision(), result });
    state.scan.fitted = None;
    state.scan.error = None;
    state.scan.open = true;
}

pub fn window(ctx: &egui::Context, state: &mut AppState) {
    if let Some(pending) = &state.scan.fitting {
        match pending.result.try_recv() {
            Ok(result) => {
                let pending = state.scan.fitting.take().expect("pending fit");
                if pending.revision != state.doc.revision() {
                    state.scan.error = Some("The part changed while fitting. Fit the scan again.".into());
                } else {
                    match result {
                        Ok(fit) => {
                            state.scan.show_reference = false;
                            state.scan.fitted =
                                Some(Fitted { feature: pending.feature, revision: pending.revision, fit })
                        }
                        Err(e) => state.scan.error = Some(e),
                    }
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                state.scan.fitting = None;
                state.scan.error = Some("The scan fit stopped before it returned a result. Try fitting again.".into());
            }
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
    }
    if !state.scan.open {
        return;
    }
    let mut open = true;
    let mut accept_reference = false;
    let mut accept_fit = false;
    let mut fit_now = false;
    let mut cancel = false;
    egui::Window::new("Scan workspace").open(&mut open).default_width(430.0).show(ctx, |ui| {
        ui.label("Reference coordinates are in millimetres. A scan has no volume.");
        if let Some(review) = &state.scan.review {
            ui.label(&review.source.name);
            counts(ui, &review.reference);
            ui.label("Holes, reversed faces and isolated points are kept as supplied.");
            accept_reference = ui.button("Accept reference").clicked();
            cancel = ui.button("Cancel").clicked();
        } else {
            if let Some(id) = selected(&state.doc)
                && let Some(feature) = state.doc.feature(id)
                && let FeatureKind::Scan(scan) = &feature.kind
            {
                ui.label(format!("{} · {}", feature.name, scan.source.name));
                ui.checkbox(&mut state.scan.show_reference, "Show scan reference");
                if let Some(fit) = &scan.fit { report(ui, fit, &mut state.scan.direction); }
                else {
                    ui.label("Fits block stock, bosses, bores and convex stock-edge rounds. At least 100 points; up to 1,000,000 points.");
                    fit_now = ui.add_enabled(can_fit(state), egui::Button::new("Fit scan to part")).clicked();
                }
            }
            if state.scan.fitting.is_some() { ui.spinner(); ui.label("Fitting scan…"); }
            if let Some(fitted) = &state.scan.fitted {
                report(ui, &fitted.fit, &mut state.scan.direction);
                accept_fit = ui.button("Accept fitted part").clicked();
                cancel = ui.button("Discard fit").clicked();
            }
            if let Some(error) = &state.scan.error { ui.colored_label(crate::theme::error(), error); }
        }
    });
    state.scan.open = open;
    if !open || cancel {
        state.scan.review = None;
        state.scan.fitted = None;
    }
    if accept_reference && let Some(review) = state.scan.review.take() {
        if review.revision != state.doc.revision() {
            state.scan.error = Some("The part changed during review. Import the scan again.".into());
        } else {
            let scan = soliddraft_model::ScanFeature { source: review.source, fit: None };
            match state.doc.apply(Command::AddFeature(FeatureKind::Scan(scan))) {
                Ok(Some(id)) => {
                    state.doc.selection = Some(Selection::Feature(id));
                    state.scan.show_reference = true;
                    state.viewport.camera.fit(bounds(&review.reference.points));
                    state.scan.cached = Some((
                        state
                            .doc
                            .feature(id)
                            .and_then(|f| {
                                if let FeatureKind::Scan(s) = &f.kind { Some(s.source.sha256.clone()) } else { None }
                            })
                            .unwrap(),
                        review.reference,
                    ));
                    state.note("Imported scan reference", NoteKind::Ok);
                }
                Ok(None) => {}
                Err(e) => state.scan.error = Some(e),
            }
        }
    }
    if fit_now {
        start_fit(state);
    }
    if accept_fit && let Some(fitted) = state.scan.fitted.take() {
        if fitted.revision != state.doc.revision() {
            state.scan.error = Some("The part changed during review. Fit the scan again.".into());
        } else if let Some(feature) = state.doc.feature(fitted.feature)
            && let FeatureKind::Scan(scan) = &feature.kind
        {
            let mut scan = scan.clone();
            scan.fit = Some(fitted.fit);
            match commit_fit(&mut state.doc, fitted.feature, scan) {
                Ok(_) => {
                    state.scan.show_reference = false;
                    state.viewport.camera.fit(state.doc.bounds());
                    state.note("Accepted fitted part", NoteKind::Ok);
                }
                Err(e) => state.scan.error = Some(e),
            }
        }
    }
}

pub fn commit_fit(doc: &mut Document, id: FeatureId, scan: soliddraft_model::ScanFeature) -> Result<(), String> {
    let command = Command::EditFeature { id, kind: FeatureKind::Scan(scan) };
    let mut trial = doc.scratch();
    trial.apply(command.clone())?;
    if let Some(feature) = trial.features.iter().find(|f| f.status != crate::document::FeatureStatus::Ok) {
        return Err(format!("{} could not be rebuilt. The original part is retained.", feature.name));
    }
    doc.reuse_builds_from(trial.model());
    doc.apply(command).map(|_| ())
}

fn counts(ui: &mut egui::Ui, r: &ScanReference) {
    ui.label(format!(
        "{} points · {} triangles · {} isolated points",
        r.points.len(),
        r.triangle_count,
        r.isolated_points
    ));
    ui.label(format!(
        "{} open edges · {} reversed edge pairs · {} nonmanifold edges",
        r.boundary_edges, r.inconsistent_edges, r.nonmanifold_edges
    ));
}

fn report(ui: &mut egui::Ui, fit: &ScanFit, direction: &mut Direction) {
    for (name, side) in [("Scan to part", &fit.scan_to_part), ("Part to scan", &fit.part_to_scan)] {
        ui.label(format!(
            "{name}: max {:.4} mm · RMS {:.4} mm · 95% {:.4} mm · {} samples",
            side.max_mm, side.rms_mm, side.p95_mm, side.samples
        ));
    }
    ui.label("Sampled distances, not a bound on the whole surface.");
    ui.label(format!("Display mesh chord bound: {:.4} mm", fit.chord_mm));
    for warning in &fit.warnings {
        ui.colored_label(crate::theme::error(), warning);
    }
    ui.horizontal(|ui| {
        ui.selectable_value(direction, Direction::ScanToPart, "Scan to part map");
        ui.selectable_value(direction, Direction::PartToScan, "Part to scan map");
    });
    let max = match direction {
        Direction::ScanToPart => fit.scan_to_part.max_mm,
        Direction::PartToScan => fit.part_to_scan.max_mm,
    };
    ui.horizontal(|ui| {
        ui.colored_label(colour(0.0), "0 mm");
        ui.colored_label(colour(0.5), format!("{:.4} mm", max * 0.5));
        ui.colored_label(colour(1.0), format!("{max:.4} mm"));
    });
    ui.label("The viewport shows up to 30,000 evenly spaced map points. The part retains every measured value.");
}

pub fn document_bounds(doc: &Document) -> Option<(Vec3, Vec3)> {
    doc.bounds().or_else(|| {
        let feature = doc.feature(selected(doc)?)?;
        let FeatureKind::Scan(scan) = &feature.kind else { return None };
        let reference = doc.kernel().scan_reference(&scan.source).ok()?;
        bounds(&reference.points)
    })
}

pub fn bounds(points: &[[f64; 3]]) -> Option<(Vec3, Vec3)> {
    let first = *points.first()?;
    let mut min = first;
    let mut max = first;
    for point in points {
        for i in 0..3 {
            min[i] = min[i].min(point[i]);
            max[i] = max[i].max(point[i]);
        }
    }
    Some((Vec3::from_array(min.map(|v| v as f32)), Vec3::from_array(max.map(|v| v as f32))))
}

fn colour(t: f64) -> Color32 {
    let t = t.clamp(0.0, 1.0) as f32;
    if t < 0.5 {
        Color32::from_rgb(35, (100.0 + t * 300.0) as u8, (240.0 - t * 320.0) as u8)
    } else {
        Color32::from_rgb((35.0 + (t - 0.5) * 440.0) as u8, (250.0 - (t - 0.5) * 400.0) as u8, 80)
    }
}

/// Draws the reference or colour deviation samples in the part's camera frame.
pub fn paint(ui: &egui::Ui, state: &mut AppState) {
    let Some(id) = selected(&state.doc) else { return };
    let Some(feature) = state.doc.feature(id) else { return };
    if feature.suppressed || feature.status != crate::document::FeatureStatus::Ok {
        return;
    }
    let FeatureKind::Scan(scan) = &feature.kind else { return };
    if scan.fit.is_none() && state.scan.fitted.is_none() && !state.scan.show_reference {
        return;
    }
    if !crate::tree_visibility::shown(state, crate::document::ViewItem::Feature(id)) {
        return;
    }
    let rect = state.viewport.rect;
    let painter = ui.painter().with_clip_rect(rect);
    let camera = &state.viewport.camera;
    let fitted = state
        .scan
        .fitted
        .as_ref()
        .filter(|p| p.feature == id && p.revision == state.doc.revision())
        .map(|p| &p.fit)
        .or(scan.fit.as_ref());
    if !state.scan.show_reference
        && let Some(fit) = fitted
    {
        let (map, max) = match state.scan.direction {
            Direction::ScanToPart => (&fit.scan_map, fit.scan_to_part.max_mm),
            Direction::PartToScan => (&fit.part_map, fit.part_to_scan.max_mm),
        };
        for ScanMapPoint { point, distance_mm } in map.iter().step_by(map.len().div_ceil(30_000).max(1)) {
            if let Some(at) = camera.project(Vec3::from_array(point.map(|v| v as f32)), rect)
                && rect.contains(at)
            {
                painter.circle_filled(at, 1.8, colour(if max > 0.0 { distance_mm / max } else { 0.0 }));
            }
        }
    } else {
        if state.scan.cached.as_ref().is_none_or(|(sha, _)| sha != &scan.source.sha256) {
            if let Ok(reference) = state.doc.kernel().scan_reference(&scan.source) {
                state.scan.cached = Some((scan.source.sha256.clone(), Arc::new(reference)));
            }
        }
        if let Some((_, r)) = &state.scan.cached {
            for point in r.points.iter().step_by(r.points.len().div_ceil(30_000).max(1)) {
                if let Some(at) = camera.project(Vec3::from_array(point.map(|v| v as f32)), rect)
                    && rect.contains(at)
                {
                    painter.circle_filled(at, 1.6, Color32::from_rgb(70, 190, 215));
                }
            }
            for triangle in r.triangles.iter().step_by(r.triangles.len().div_ceil(10_000).max(1)) {
                let p =
                    triangle.map(|v| camera.project(Vec3::from_array(r.points[v as usize].map(|v| v as f32)), rect));
                if let [Some(a), Some(b), Some(c)] = p {
                    for [from, to] in [[a, b], [b, c], [c, a]] {
                        painter.line_segment([from, to], Stroke::new(0.5, Color32::from_rgb(50, 130, 150)));
                    }
                }
            }
        }
    }
}
