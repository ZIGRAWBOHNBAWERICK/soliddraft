//! A temporary half-space cut. Only its tessellation reaches the viewport.

use crate::document::{DisplayMesh, Document, Selection, preview_mesh};
use crate::theme;
use soliddraft_kernel_api::{
    BodyId, BodyKind, BooleanOp, Extrude, FaceId, Kernel, OpTag, Plane, Point2, Profile, SurfaceKind, Vec3,
};
use std::sync::{Arc, mpsc};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SectionPlane {
    #[default]
    XY,
    YZ,
    ZX,
    Face(FaceId),
}

impl SectionPlane {
    fn resolve(self, doc: &Document) -> Option<Plane> {
        match self {
            Self::XY => Some(Plane::XY),
            Self::YZ => Some(Plane::YZ),
            Self::ZX => Some(Plane::ZX),
            Self::Face(face) => {
                doc.faces.iter().find(|f| f.id == face && f.surface == SurfaceKind::Plane)?;
                doc.face_plane(face)
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Request {
    document: soliddraft_document::DocumentId,
    body: BodyId,
    mesh_version: u64,
    plane: Plane,
    flip: bool,
}

type Answer = Result<Arc<DisplayMesh>, String>;

struct Pending {
    request: Request,
    answer: mpsc::Receiver<Answer>,
}

#[derive(Default)]
pub struct State {
    pub enabled: bool,
    pub plane: SectionPlane,
    pub offset: f64,
    pub flip: bool,
    cached: Option<(Request, Answer)>,
    pending: Option<Pending>,
}

impl State {
    pub fn toggle(&mut self, doc: &Document) {
        self.enabled = !self.enabled;
        if self.enabled {
            if let Some(Selection::Face(face)) = doc.selection
                && SectionPlane::Face(face).resolve(doc).is_some()
            {
                self.plane = SectionPlane::Face(face);
                self.offset = 0.0;
            } else if self.cached.is_none() {
                self.centre(doc);
            }
        } else {
            self.cached = None;
        }
    }

    fn centre(&mut self, doc: &Document) {
        if let Some(plane) = self.plane.resolve(doc)
            && let Some(body) = doc.display_body()
            && let Ok(topology) = doc.kernel().topology(body)
        {
            self.offset = (topology.bbox.center() - plane.origin).dot(plane.normal);
        }
    }

    fn request(&self, doc: &Document) -> Option<Request> {
        if !self.enabled || !doc.proof_state().current_proved_display() {
            return None;
        }
        let mut plane = self.plane.resolve(doc)?;
        plane.origin = plane.origin + plane.normal * self.offset;
        Some(Request {
            document: doc.model().id(),
            body: doc.display_body()?,
            mesh_version: doc.mesh_version,
            plane,
            flip: self.flip,
        })
    }

    /// One in-flight boolean per window. Dragging coalesces to the latest plane.
    pub fn refresh(&mut self, doc: &Document, ctx: &egui::Context) {
        let wanted = self.request(doc);
        if let Some(pending) = &self.pending {
            match pending.answer.try_recv() {
                Ok(answer) => {
                    if Some(pending.request) == wanted {
                        self.cached = Some((pending.request, answer));
                    }
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    if Some(pending.request) == wanted {
                        self.cached = Some((pending.request, Err("Could not create the section. Try again.".into())));
                    }
                    self.pending = None;
                }
            }
        }
        if self.cached.as_ref().is_some_and(|(request, _)| Some(*request) != wanted) {
            self.cached = None;
        }
        let Some(request) = wanted else { return };
        if self.pending.is_some() || self.cached.is_some() {
            return;
        }
        let model = doc.model().fork();
        let params = doc.tess_params;
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (sender, answer) = mpsc::channel();
            let ctx = ctx.clone();
            let job = std::thread::Builder::new().name("section-view".into()).spawn(move || {
                // The fork keeps the source body alive through edits and document switches.
                let (result, work) = soliddraft_kernel_catallax::with_request_allowance(200_000_000, || {
                    cut_mesh(model.kernel().as_ref(), request.body, request.plane, request.flip, &params)
                });
                let result =
                    if work.exhausted { Err("This section is too complex. Try another plane.".into()) } else { result };
                let _ = sender.send(result);
                ctx.request_repaint();
            });
            match job {
                Ok(_) => self.pending = Some(Pending { request, answer }),
                Err(_) => self.cached = Some((request, Err("Could not start the section. Try again.".into()))),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.cached =
                Some((request, cut_mesh(model.kernel().as_ref(), request.body, request.plane, request.flip, &params)));
        }
    }

    pub fn mesh(&self, doc: &Document) -> Option<Arc<DisplayMesh>> {
        let (request, answer) = self.cached.as_ref()?;
        (Some(*request) == self.request(doc)).then(|| answer.as_ref().ok().cloned()).flatten()
    }

    pub fn show(&mut self, ctx: &egui::Context, doc: &Document) {
        if !self.enabled {
            return;
        }
        let mut open = true;
        let mut close = false;
        egui::Window::new("Section view").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label("Display only. The part and exports stay whole.");
            ui.horizontal(|ui| {
                for (plane, label) in [(SectionPlane::XY, "XY"), (SectionPlane::YZ, "YZ"), (SectionPlane::ZX, "ZX")] {
                    if ui.selectable_value(&mut self.plane, plane, label).clicked() {
                        self.centre(doc);
                    }
                }
            });
            let label = match self.plane {
                SectionPlane::Face(face) => {
                    doc.faces.iter().find(|f| f.id == face).map(|f| format!("Face: {}", f.label))
                }
                _ => None,
            };
            egui::ComboBox::from_label("Face plane")
                .selected_text(label.unwrap_or_else(|| "Choose a planar face".into()))
                .show_ui(ui, |ui| {
                    for face in doc.faces.iter().filter(|f| f.surface == SurfaceKind::Plane) {
                        if ui.selectable_value(&mut self.plane, SectionPlane::Face(face.id), &face.label).clicked() {
                            self.offset = 0.0;
                        }
                    }
                });
            ui.horizontal(|ui| {
                ui.label("Offset");
                ui.add(egui::DragValue::new(&mut self.offset).speed(0.25).suffix(" mm"))
                    .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, "Section offset"));
            });
            if ui.button("Flip side").clicked() {
                self.flip = !self.flip;
            }
            ui.label(if self.flip { "Keep positive side" } else { "Keep negative side" });
            if self.request(doc).is_none() {
                ui.label(if matches!(self.plane, SectionPlane::Face(_)) {
                    "Choose a planar face on the current part."
                } else {
                    "A solid part is needed for a section."
                });
            } else if let Some((_, Err(error))) = &self.cached {
                ui.colored_label(theme::error(), error);
            } else if self.mesh(doc).is_none() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Updating section...");
                });
            }
            close = ui.button("Close section").clicked();
        });
        if !open || close {
            self.enabled = false;
            self.cached = None;
        }
    }
}

fn cut_mesh(
    kernel: &dyn Kernel,
    body: BodyId,
    plane: Plane,
    flip: bool,
    params: &soliddraft_kernel_api::TessellationParams,
) -> Answer {
    let build = || -> Result<Arc<DisplayMesh>, soliddraft_kernel_api::KernelError> {
        if !plane.is_well_formed() {
            return Err(soliddraft_kernel_api::KernelError::InvalidInput("invalid section plane".into()));
        }
        let topology = kernel.topology(body)?;
        if topology.kind != BodyKind::Solid {
            return Err(soliddraft_kernel_api::KernelError::Unsupported("section of a sheet".into()));
        }
        let bbox = topology.bbox;
        let size = bbox.diagonal().max(1.0);
        let normal = if flip { plane.normal * -1.0 } else { plane.normal };
        let at = plane.origin.dot(normal);
        let mut near = f64::INFINITY;
        let mut far = f64::NEG_INFINITY;
        for i in 0..8 {
            let corner = Vec3::new(
                if i & 1 == 0 { bbox.min.x } else { bbox.max.x },
                if i & 2 == 0 { bbox.min.y } else { bbox.max.y },
                if i & 4 == 0 { bbox.min.z } else { bbox.max.z },
            );
            near = near.min(corner.dot(normal));
            far = far.max(corner.dot(normal));
        }
        if at <= near + 1e-9 * size {
            return Ok(Arc::new(DisplayMesh::default()));
        }
        if at >= far - 1e-9 * size {
            return preview_mesh(kernel, body, params).map_err(soliddraft_kernel_api::KernelError::InvalidInput);
        }
        let centre = bbox.center();
        let tool_plane = Plane { origin: centre - normal * (centre.dot(normal) - at), normal, x_dir: plane.x_dir };
        let profile = Profile::rectangle(tool_plane, Point2::new(0.0, 0.0), 2.0 * size + 2.0, 2.0 * size + 2.0);
        let sketch = kernel.create_body_from_profile(&OpTag::new("viewport_section_profile"), &profile)?;
        let tool = kernel.extrude(
            &OpTag::new("viewport_section_tool"),
            sketch.body,
            &Extrude { distance: far - at + size + 1.0, direction: None },
        );
        let _ = kernel.release(sketch.body);
        let tool = tool?;
        let cut = kernel.boolean(&OpTag::new("viewport_section"), BooleanOp::Subtract, body, tool.body);
        let _ = kernel.release(tool.body);
        let cut = cut?;
        let mesh = preview_mesh(kernel, cut.body, params);
        let _ = kernel.release(cut.body);
        mesh.map_err(soliddraft_kernel_api::KernelError::InvalidInput)
    };
    build().map_err(|error| {
        log::warn!("Section view: {error}");
        "Could not cut this part on that plane. Try another plane or offset.".into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Command, ExtrudeParams, ProfileKind};

    fn block() -> Document {
        let mut doc = Document::new("Section block", crate::kernel());
        doc.apply(Command::AddExtrude {
            params: ExtrudeParams {
                profile: ProfileKind::Rectangle { width: 60.0, height: 40.0 },
                depth: 20.0,
                ..Default::default()
            },
            via_assistant: false,
        })
        .unwrap();
        doc
    }

    fn volume(mesh: &DisplayMesh) -> f64 {
        mesh.indices
            .chunks_exact(3)
            .map(|t| {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.positions[i as usize]);
                f64::from(a.dot(b.cross(c))) / 6.0
            })
            .sum::<f64>()
            .abs()
    }

    #[test]
    fn principal_and_oblique_cuts_are_capped_and_leave_the_source_whole() {
        let doc = block();
        let body = doc.display_body().unwrap();
        let before = doc.model().to_bytes().unwrap();
        for plane in [
            Plane::XY.with_origin(Vec3::new(0.0, 0.0, 10.0)),
            Plane::YZ,
            Plane::ZX,
            Plane {
                origin: Vec3::new(0.0, 0.0, 10.0),
                normal: Vec3::new(1.0, 0.0, 1.0).normalized().unwrap(),
                x_dir: Vec3::Y,
            },
        ] {
            for flip in [false, true] {
                let mesh = cut_mesh(doc.kernel().as_ref(), body, plane, flip, &doc.tess_params).unwrap();
                assert!((volume(&mesh) - 24_000.0).abs() < 0.1, "a capped half has half the source volume");
                let cap_area: f64 = mesh
                    .indices
                    .chunks_exact(3)
                    .filter_map(|t| {
                        let p = [t[0], t[1], t[2]].map(|i| mesh.positions[i as usize]);
                        p.iter()
                            .all(|p| {
                                (Vec3::new(p.x as f64, p.y as f64, p.z as f64) - plane.origin).dot(plane.normal).abs()
                                    < 1e-4
                            })
                            .then(|| f64::from((p[1] - p[0]).cross(p[2] - p[0]).length()) / 2.0)
                    })
                    .sum();
                assert!(cap_area > 100.0, "the cut has triangles on its plane");
            }
        }
        assert_eq!(doc.model().to_bytes().unwrap(), before);
        assert!((doc.kernel().mass_properties(body).unwrap().volume - 48_000.0).abs() < 1e-4);
    }

    #[test]
    fn offsets_outside_the_part_show_whole_or_empty() {
        let doc = block();
        let body = doc.display_body().unwrap();
        for (z, flip, expected) in
            [(-10.0, false, 0.0), (30.0, false, 48_000.0), (-10.0, true, 48_000.0), (30.0, true, 0.0)]
        {
            let mesh = cut_mesh(
                doc.kernel().as_ref(),
                body,
                Plane::XY.with_origin(Vec3::new(0.0, 0.0, z)),
                flip,
                &doc.tess_params,
            )
            .unwrap();
            assert!((volume(&mesh) - expected).abs() < 0.1);
        }
    }

    #[test]
    fn an_old_answer_cannot_replace_a_new_plane_or_document() {
        let doc = block();
        let mut state = State::default();
        state.toggle(&doc);
        let old = state.request(&doc).unwrap();
        let (send, answer) = mpsc::channel();
        state.pending = Some(Pending { request: old, answer });
        state.offset += 2.0;
        send.send(Ok(Arc::new(DisplayMesh::default()))).unwrap();
        state.refresh(&doc, &egui::Context::default());
        assert!(state.mesh(&doc).is_none());
        assert!(state.cached.is_none());
        let other = block();
        assert!(state.mesh(&other).is_none());
        state.enabled = false;
        assert!(state.mesh(&doc).is_none());
    }
}
