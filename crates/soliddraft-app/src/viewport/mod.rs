//! The 3D viewport: camera input, picking, overlays (heads-up toolbar, view
//! cube, origin axes, plane names, grid readout, hover label) and the hand-off to the wgpu
//! renderer.

mod annotation_drag;
pub mod assembly;
pub mod camera;
pub mod dimensions;
pub mod environment;
pub mod grid;
pub mod look;
mod navigation_bar;
pub mod offscreen;
pub mod pick;
pub mod planes;
pub mod render;
pub mod scenes;

use crate::dimension_edit::DimensionAction;
use crate::document::{AxisKind, DisplayMesh, Document, Reference, Selection};
use crate::icons::{self, Icon};
use crate::navigation::{Action, Motion, NavContext, NavigationSettings, Outcome, Routed, Router, ZoomAnchor};
use crate::settings::ReferenceDisplay;
use crate::sketch::Sketch;
use crate::state::{AppAction, SketchSession};
use crate::theme;
use crate::tree_visibility::Visible;
use camera::{Camera, StandardView};
use egui::{Align2, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use glam::Vec3;
use grid::{GridFrame, GridPlane, GridStyle, Spacing};
use look::Preset;
use planes::ReferenceScene;
use render::{DisplayMode, FrameInput, PickSample, PickView, Picked, ViewportCallback};
use soliddraft_kernel_api::Plane;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct ViewportEvents {
    /// A primary click happened; the payload is what was under the cursor.
    pub clicked: Option<Option<Picked>>,
    /// With a click on no model: the reference it found.
    pub clicked_reference: Option<Reference>,
    pub set_view: Option<StandardView>,
    pub fit: bool,
    pub set_display: Option<DisplayMode>,
    pub toggle_projection: bool,
    /// Actions the sketcher raised.
    pub actions: Vec<AppAction>,
    /// For an assembly: the id-buffer texel under the cursor, from the last readback.
    pub hover_texel: Option<[u32; 2]>,
    /// For an assembly: a primary click happened, with the texel under the cursor.
    pub clicked_texel: Option<Option<[u32; 2]>>,
}

/// The part meshes version an assembly frame hands the renderer: none, so the part's
/// are uploaded again when its tab comes back.
const NO_PART: u64 = u64::MAX - 1;

/// What the viewport draws besides the document this frame.
pub struct Overlay<'a> {
    /// Material a feature adds or removes, with whether it removes.
    pub ghost: Option<(Arc<DisplayMesh>, bool)>,
    /// A body drawn in place of the document's while a feature is previewed.
    pub preview: Option<Arc<DisplayMesh>>,
    /// The open sketch, which takes the pointer.
    pub sketch: Option<&'a mut SketchSession>,
    /// Closed sketches to show quietly on their planes.
    pub passive_sketches: Vec<(Plane, &'a Sketch)>,
    /// The feature value being typed at its dimension.
    pub dimension_edit: Option<&'a mut crate::dimension_edit::DimensionEdit>,
    /// Which of the part's items show.
    pub visible: Visible,
    /// An open Fillet or Chamfer, whose preview the viewport names and marks.
    pub rounding: Option<RoundingOverlay<'a>>,
}

/// What an open Fillet or Chamfer adds to the viewport: a label naming its size and
/// picks, and the picked edges themselves while no rounded body stands for them.
pub struct RoundingOverlay<'a> {
    /// "Fillet preview" or "Chamfer preview": the label's accessible name.
    pub title: &'static str,
    /// What the label reads, such as "R 5 mm · 2 edges".
    pub text: String,
    /// Whether a rounded body is shown in place of the part.
    pub shown: bool,
    /// The picked edges on the body being rounded.
    pub edges: &'a [Vec<Vec3>],
}

impl<'a> RoundingOverlay<'a> {
    /// The overlay of an open editor, when it is a Fillet or Chamfer holding an edge.
    pub fn of(editor: &'a crate::state::FeatureEditor) -> Option<Self> {
        use crate::state::EditorParams;
        let (title, size) = match &editor.params {
            EditorParams::Fillet(f) => ("Fillet preview", format!("R {} mm", crate::document::fmt_mm(f.radius))),
            EditorParams::Chamfer(c) => ("Chamfer preview", format!("{} mm", crate::document::fmt_mm(c.distance))),
            _ => return None,
        };
        let count = editor.edges().len();
        if count == 0 {
            return None;
        }
        let shown = editor.preview.result.is_some();
        let edges = format!("{count} edge{}", if count == 1 { "" } else { "s" });
        let text = if shown { format!("{size} · {edges}") } else { format!("No preview · {edges}") };
        Some(Self { title, text, shown, edges: &editor.marks })
    }
}

pub struct ViewportState {
    pub camera: Camera,
    pub display: DisplayMode,
    pub preset: Preset,
    /// Named background in Edit Suite; classic keeps the theme fade.
    pub scene: Option<scenes::Scene>,
    /// What the viewport shows of the part's reference geometry without being asked.
    pub references: ReferenceDisplay,
    /// How the grid looks, as the user chose.
    pub grid_style: GridStyle,
    pub hover: Option<Picked>,
    /// The reference under the pointer, when no model is.
    pub hover_reference: Option<Reference>,
    pub hover_point: Option<Vec3>,
    pub rect: Rect,
    /// The dimensions a selected body showed last frame.
    pub dimensions: Vec<dimensions::ShownDimension>,
    annotation_drag: Option<annotation_drag::AnnotationDrag>,
    /// Turns the pointer, the wheel and held keys into camera moves, by the user's preset.
    pub router: Router,
    pick: Arc<Mutex<PickSample>>,
    cube_hover: Option<StandardView>,
    navigation_bar: navigation_bar::NavigationBar,
    /// Shows feature dimensions without selecting the body.
    pub all_dimensions: bool,
}

impl Default for ViewportState {
    fn default() -> Self {
        Self {
            camera: Camera::default(),
            display: DisplayMode::default(),
            preset: Preset::default(),
            scene: None,
            references: ReferenceDisplay::default(),
            grid_style: GridStyle::default(),
            hover: None,
            hover_reference: None,
            hover_point: None,
            rect: Rect::NOTHING,
            dimensions: Vec::new(),
            annotation_drag: None,
            router: Router::new(&NavigationSettings::new(crate::settings::InstallKind::Existing), false),
            pick: Arc::new(Mutex::new(PickSample::default())),
            cube_hover: None,
            navigation_bar: navigation_bar::NavigationBar::default(),
            all_dimensions: false,
        }
    }
}

/// The zoom of one point of a drag-zoom: the natural log of its factor.
const DRAG_ZOOM: f32 = 0.005;

/// Moves `camera` as one routed navigation event says, scaled by the user's camera
/// settings. `pointer` anchors a cursor zoom. Returns whether the event asked to fit the
/// view, which the caller does with the document's bounds.
pub fn apply_navigation(
    camera: &mut Camera,
    r: &Routed,
    router: &Router,
    nav: &NavigationSettings,
    rect: Rect,
    pointer: Option<Pos2>,
) -> bool {
    let aspect = rect.width() / rect.height().max(1.0);
    let ndc =
        |p: Pos2| ((p.x - rect.left()) / rect.width() * 2.0 - 1.0, 1.0 - (p.y - rect.top()) / rect.height() * 2.0);
    let Outcome::Action(action) = r.outcome else { return false };
    match (action, r.motion) {
        (Action::Orbit, Motion::Drag { delta, .. }) => {
            let k = nav.camera.orbit_sensitivity;
            camera.orbit(delta.x * k, delta.y * k);
        }
        (Action::Pan, Motion::Drag { delta, .. }) => {
            let k = nav.camera.pan_sensitivity;
            camera.pan(delta.x * k, delta.y * k, rect.height());
        }
        (Action::DragZoom, Motion::Drag { delta, origin }) => {
            let anchor = r.binding.and_then(|id| router.binding(id)).map(|b| b.anchor);
            let at = (anchor == Some(ZoomAnchor::Cursor)).then(|| ndc(origin));
            // Dragging up zooms in.
            camera.zoom((-delta.y * DRAG_ZOOM).exp(), at, aspect);
        }
        (Action::WheelZoom, Motion::Zoom { factor }) => {
            let at = match nav.camera.zoom_anchor {
                ZoomAnchor::Cursor => pointer.filter(|p| rect.contains(*p)).map(ndc),
                ZoomAnchor::Center => None,
            };
            camera.zoom(factor, at, aspect);
        }
        (Action::Fit, _) => return true,
        _ => {}
    }
    false
}

/// A projected view-cube face: view, normal, label, screen polygon, depth, facing.
type CubeFace<'a> = (StandardView, Vec3, &'a str, Vec<Pos2>, f32, f32);

const CUBE_FACES: [(StandardView, Vec3, &str); 6] = [
    (StandardView::Front, Vec3::NEG_Y, "FRONT"),
    (StandardView::Back, Vec3::Y, "BACK"),
    (StandardView::Right, Vec3::X, "RIGHT"),
    (StandardView::Left, Vec3::NEG_X, "LEFT"),
    (StandardView::Top, Vec3::Z, "TOP"),
    (StandardView::Bottom, Vec3::NEG_Z, "BOTTOM"),
];

impl ViewportState {
    /// An annotation owns the pointer until its primary-button gesture ends.
    pub(crate) fn moving_dimension(&self) -> bool {
        self.annotation_drag.is_some()
    }

    /// How many physical pixels a millimetre covers in the last frame's view, for the
    /// display level choice.
    pub fn projection(&self, pixels_per_point: f32) -> soliddraft_tessellation::Projection {
        let c = &self.camera;
        let height_px = f64::from((self.rect.height() * pixels_per_point).max(1.0));
        let fov_y = f64::from(c.fov_y);
        if c.orthographic {
            let view_height = 2.0 * f64::from(c.distance) * (fov_y / 2.0).tan();
            soliddraft_tessellation::Projection::Orthographic { pixels_per_mm: height_px / view_height.max(1e-9) }
        } else {
            let e = c.eye();
            let eye = soliddraft_kernel_api::Vec3::new(e.x.into(), e.y.into(), e.z.into());
            soliddraft_tessellation::Projection::Perspective { eye, fov_y, height_px }
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        doc: &Document,
        overlay: Overlay<'_>,
        selection: Option<Selection>,
        hover_label: Option<&str>,
        nav: &NavigationSettings,
    ) -> ViewportEvents {
        let Overlay { ghost, preview, sketch, passive_sketches, dimension_edit, visible, rounding } = overlay;
        let pick_revision = Some((doc.model().id(), doc.mesh_version, doc.display_body()));
        let provisional = doc.provisional_mesh();
        let is_provisional = |shown: &Arc<DisplayMesh>| provisional.as_ref().is_some_and(|b| Arc::ptr_eq(shown, b));
        // While its proof is pending the provisional body is the part's own, so it hides with
        // the body; an open tool's preview still shows.
        let preview = preview.filter(|shown| visible.body || !is_provisional(shown));
        let preview_pickable = preview.as_ref().is_some_and(is_provisional);
        let sketching = sketch.is_some();
        let sketch_dimensions = sketch.as_ref().map(|s| s.show_dimensions);
        let mut events = ViewportEvents::default();
        let rect = ui.available_rect_before_wrap();
        self.rect = rect;
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "3D viewport"));
        let ppp = ui.ctx().pixels_per_point();
        let annotation_gesture = if !sketching && dimension_edit.is_none() {
            annotation_drag::input(
                &mut self.annotation_drag,
                &self.dimensions,
                &self.camera,
                rect,
                ui,
                doc,
                &mut events,
            )
        } else {
            self.annotation_drag = None;
            false
        };
        if !annotation_gesture {
            events.fit = self.navigate(ui, &response, rect, sketching, nav);
        }
        let scene = ReferenceScene::new(&visible, sketching, doc.bounds(), &self.camera);

        // Hover pick from the last id-buffer readback. The readback lags one
        // frame, so keep repainting briefly while the pointer rests on the view.
        let hover_pos =
            if response.hovered() && !response.dragged() && !sketching { response.hover_pos() } else { None };
        let cursor_px = hover_pos
            .map(|p| [((p.x - rect.left()) * ppp).max(0.0) as u32, ((p.y - rect.top()) * ppp).max(0.0) as u32]);
        self.hover = None;
        self.hover_reference = None;
        self.hover_point = None;
        let mut can_pick_references = false;
        if let (Some(px), Some(pos)) = (cursor_px, hover_pos) {
            let sample = self.pick.lock().unwrap();
            self.hover = (sample.revision == pick_revision).then(|| decode_pick(&sample, px[0], px[1])).flatten();
            let current = planes::sample_is_current(&sample, pick_revision, self.pick_view(rect, ppp), px);
            can_pick_references = current;
            drop(sample);
            if let Some(picked) = self.hover {
                let (o, d) = self.camera.ray(pos, rect);
                self.hover_point = pick::hit_point(doc, picked, o, d);
            } else if current {
                self.hover_reference = scene.hit(&self.camera, rect, pos);
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
        }
        if response.clicked_by(egui::PointerButton::Primary) && !sketching {
            events.clicked = Some(self.hover);
            events.clicked_reference = self.hover_reference;
        }

        // Hand the frame to the GPU.
        // Sent every frame: egui may discard a pass before its paint callbacks run,
        // so the renderer decides what to upload by comparing versions.
        // A hidden body leaves the scene and the pick pass; the version changed with it.
        let shown = if visible.body { doc.meshes.iter().map(|(id, m)| (*id, m.clone())).collect() } else { Vec::new() };
        let meshes = (doc.mesh_version, shown);
        // The shader compares face pick ids; edges and vertices are highlighted by the painter.
        let hover_id = match self.hover {
            Some(Picked::Face(p)) => p,
            _ => 0,
        };
        let selected_id = match selection {
            Some(Selection::Face(f)) => doc.pick_of_face(f).unwrap_or(0),
            _ => 0,
        };
        // A selected feature lights the faces it made; the body is one merged solid now.
        let selected_feature = match selection {
            Some(Selection::Feature(id)) if sketch.is_none() => id as u32,
            _ => 0,
        };
        let mut input = self.frame_input(rect, ppp, cursor_px);
        input.meshes = meshes;
        input.hover_id = hover_id;
        input.selected_id = selected_id;
        input.selected_feature = selected_feature;
        input.selected_body = match selection {
            Some(Selection::Body) => doc.display_body(),
            _ => None,
        };
        input.ghost = ghost;
        input.preview = preview;
        input.preview_pickable = preview_pickable;
        input.pick_revision = pick_revision;
        input.planes = scene.frame(theme::accent(), self.hover_reference, selection);
        if let Some(session) = &sketch {
            input.grid = self.grid(rect, &GridPlane::of_sketch(&session.plane));
        }
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, ViewportCallback(input)));

        let painter = ui.painter().with_clip_rect(rect);
        for (plane, sk) in passive_sketches {
            crate::sketcher::paint_passive(&painter, &self.camera, rect, plane, sk, theme::muted().gamma_multiply(0.9));
        }
        if let Some(session) = sketch {
            let spacing = Spacing::for_scale(self.camera.mm_per_point(rect.height()), &self.grid_style);
            events.actions = crate::sketcher::run(
                ui,
                session,
                doc,
                &self.camera,
                rect,
                &response,
                spacing,
                theme::is_edit_suite() && self.scene.is_some_and(scenes::Scene::is_dark),
            );
            ui.ctx().request_repaint();
        }
        if doc.model().active_field().is_some_and(|f| doc.model().status(f.id).is_some_and(|s| s.is_ok())) {
            ui.put(
                Rect::from_min_size(rect.left_top() + vec2(12.0, 12.0), vec2(160.0, 24.0)),
                egui::Label::new(egui::RichText::new("unproved preview").color(theme::muted())),
            );
        }
        if scene.axes.contains(&true) {
            self.paint_origin_axes(&painter, scene.reach, rect, scene.axes);
        }
        self.paint_plane_names(ui, &scene, rect);
        if !sketching {
            self.paint_reference_planes(ui, doc, rect, can_pick_references, &response, &mut events);
        }
        if visible.body {
            self.paint_highlights(&painter, doc, selection, rect);
        }
        if let Some(rounding) = &rounding {
            self.paint_rounding(ui, &painter, rect, rounding);
        }
        // Label gestures take precedence over body picking and camera navigation.
        let pointer = ui.input(|i| i.pointer.interact_pos()).filter(|p| rect.contains(*p));
        let dims = if !sketching {
            dimensions::paint(
                &painter,
                &self.camera,
                rect,
                doc,
                pointer,
                (self.all_dimensions || selection == Some(Selection::Body)) && visible.body,
                &visible.sketches,
                self.annotation_drag.as_ref().map(|d| (d.key, d.position)),
            )
        } else {
            Vec::new()
        };
        if annotation_gesture {
            events.clicked = None;
            events.clicked_reference = None;
        }
        if let Some(d) = pointer.and_then(|p| dims.iter().rev().find(|d| d.rect.contains(p))) {
            events.clicked = None;
            if self.annotation_drag.is_none() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            if response.double_clicked_by(egui::PointerButton::Primary) {
                events.actions.push(d.edit_action());
            }
            response.clone().on_hover_text_at_pointer(format!("{} · drag to move · double-click to edit", d.tip));
        } else if response.double_clicked_by(egui::PointerButton::Primary)
            && !sketching
            && self.hover.is_some()
            && doc.display_body().is_some()
        {
            events.clicked = None;
            events.actions.push(AppAction::Select(Some(Selection::Body)));
        }
        for (i, d) in dims.iter().enumerate() {
            d.expose(ui, i);
        }
        if let Some(edit) = dimension_edit {
            // The value goes when its dimension does, or when the part changes under it.
            match dims.iter().find(|d| d.is_field(edit.feature, edit.field)) {
                Some(d) if edit.revision == doc.revision() => {
                    dimensions::value_editor(ui, &painter, rect, d.rect, edit, &mut events.actions);
                }
                _ => events.actions.push(AppAction::Dimension(DimensionAction::Cancel)),
            }
        }
        self.dimensions = dims;
        self.paint_hud(ui, rect, &mut events, sketch_dimensions, true);
        self.paint_view_cube(ui, rect, &mut events);
        self.paint_pills(ui, rect, hover_label);
        events
    }

    /// Shows an assembly with the part's camera controls, heads-up toolbar and view cube:
    /// `frame`'s scene in place of the part's meshes, with the camera filled in here. The
    /// events carry the id-buffer texels under the cursor for `Scene::decode`.
    pub fn show_assembly(
        &mut self,
        ui: &mut egui::Ui,
        mut frame: assembly::AssemblyFrame,
        hover_label: Option<&str>,
        nav: &NavigationSettings,
    ) -> ViewportEvents {
        let mut events = ViewportEvents::default();
        let rect = ui.available_rect_before_wrap();
        self.rect = rect;
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Assembly viewport"));
        let ppp = ui.ctx().pixels_per_point();
        events.fit = self.navigate(ui, &response, rect, false, nav);
        let hover_pos = if response.hovered() && !response.dragged() { response.hover_pos() } else { None };
        let cursor_px = hover_pos
            .map(|p| [((p.x - rect.left()) * ppp).max(0.0) as u32, ((p.y - rect.top()) * ppp).max(0.0) as u32]);
        self.hover = None;
        self.hover_point = None;
        if let Some(px) = cursor_px {
            events.hover_texel = self.pick.lock().unwrap().texel(px[0], px[1]).filter(|t| t[0] != 0);
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
        }
        if response.clicked_by(egui::PointerButton::Primary) {
            events.clicked_texel = Some(events.hover_texel);
        }
        let aspect = rect.width() / rect.height().max(1.0);
        frame.view_proj = (self.camera.projection(aspect) * self.camera.view()).as_dmat4();
        frame.eye = self.camera.eye().as_dvec3();
        let mut input = self.frame_input(rect, ppp, cursor_px);
        input.assembly = Some(frame);
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, ViewportCallback(input)));
        if self.references.always_show_origin_axes {
            let painter = ui.painter().with_clip_rect(rect);
            self.paint_origin_axes(&painter, self.camera.distance * 0.25, rect, [true; 3]);
        }
        self.paint_hud(ui, rect, &mut events, None, false);
        self.paint_view_cube(ui, rect, &mut events);
        let label = hover_label.unwrap_or("Hover a component");
        self.paint_pills(ui, rect, Some(label));
        events
    }

    /// The grid's spacing at this frame's zoom, when the grid shows.
    fn grid_spacing(&self, rect: Rect) -> Option<Spacing> {
        Spacing::for_scale(self.camera.mm_per_point(rect.height()), &self.grid_style)
            .filter(|_| self.references.show_grid)
    }

    fn grid(&self, rect: Rect, plane: &GridPlane) -> Option<GridFrame> {
        let ink = self.grid_style.color.unwrap_or_else(theme::ink);
        let opacity = self.grid_style.opacity_in(theme::mode().is_dark());
        GridFrame::new(plane, self.grid_spacing(rect)?, &self.camera, ink, opacity)
    }

    /// Closes a viewport dialog before the app handles Escape underneath it.
    pub fn overlay_input(&mut self, ctx: &egui::Context) {
        self.navigation_bar.consume_escape(ctx);
    }

    /// A copy of the latest id-buffer sample.
    pub fn pick_sample(&self) -> PickSample {
        self.pick.lock().unwrap().clone()
    }

    /// The camera and render size of a frame of `rect` drawn now.
    pub fn pick_view(&self, rect: Rect, ppp: f32) -> PickView {
        let aspect = rect.width() / rect.height().max(1.0);
        PickView {
            view: self.camera.view(),
            proj: self.camera.projection(aspect),
            size: [(rect.width() * ppp).round().max(1.0) as u32, (rect.height() * ppp).round().max(1.0) as u32],
        }
    }

    /// What every frame hands the renderer from the camera and the look, with nothing in
    /// the scene yet. The grid lies on the ground until a sketch moves it.
    fn frame_input(&self, rect: Rect, ppp: f32, cursor_px: Option<[u32; 2]>) -> FrameInput {
        let PickView { view, proj, size: size_px } = self.pick_view(rect, ppp);
        FrameInput {
            view,
            proj,
            eye: self.camera.eye(),
            basis: [self.camera.right(), self.camera.up(), self.camera.direction()],
            orthographic: self.camera.orthographic,
            distance: self.camera.distance,
            size_px,
            pixels_per_point: ppp,
            display: self.display,
            preset: self.preset,
            backdrop: self.scene.map_or_else(|| theme::viewport_backdrop().into(), scenes::Scene::backdrop),
            hover_id: 0,
            selected_id: 0,
            selected_feature: 0,
            selected_body: None,
            cursor_px,
            meshes: (NO_PART, Vec::new()),
            ghost: None,
            preview: None,
            preview_pickable: false,
            pick_revision: None,
            pick_out: self.pick.clone(),
            assembly: None,
            grid: self.grid(rect, &GridPlane::GROUND),
            planes: None,
        }
    }

    /// Moves the camera as the user's navigation preset says: the router decides what
    /// the pointer, the wheel and held keys do, and this applies it. A drag captured here
    /// keeps going over panels and overlays until its buttons are up. Returns whether a
    /// gesture asked to fit the view.
    fn navigate(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: Rect,
        sketching: bool,
        nav: &NavigationSettings,
    ) -> bool {
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        self.camera.transition_seconds = nav.camera.transition_seconds;
        if self.camera.tick(dt) {
            ui.ctx().request_repaint();
        }
        let mac = ui.ctx().os() == egui::os::OperatingSystem::Mac;
        self.router.configure(nav, mac);
        self.router.context = if sketching { NavContext::Sketch } else { NavContext::Model };
        let hovered = response.hovered();
        let events =
            ui.input(|i| crate::navigation::gather(&i.events, i.time, |p| hovered && rect.contains(p), hovered));
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let mut routed: Vec<_> = events.into_iter().map(|e| self.router.handle(e)).collect();
        // A release outside the window may never arrive; the buttons egui sees down decide.
        let (time, last) = ui.input(|i| (i.time, i.pointer.latest_pos()));
        for b in self.router.buttons().to_vec() {
            if !ui.input(|i| i.pointer.button_down(crate::navigation::egui_button(b))) {
                let pos = last.unwrap_or(rect.center());
                routed.push(self.router.handle(crate::navigation::NavEvent::Release { button: b, pos, time }));
            }
        }
        let mut fit = false;
        for r in routed {
            fit |= apply_navigation(&mut self.camera, &r, &self.router, nav, rect, pointer);
        }
        if self.router.dragging() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            ui.ctx().request_repaint();
        }
        fit
    }

    /// A Fillet or Chamfer preview's label over the top of the view, and its picked edges
    /// in the accent while no rounded body shows them.
    fn paint_rounding(&self, ui: &egui::Ui, painter: &egui::Painter, rect: Rect, rounding: &RoundingOverlay<'_>) {
        if !rounding.shown {
            for path in rounding.edges {
                let points: Vec<Pos2> = path.iter().filter_map(|p| self.camera.project(*p, rect)).collect();
                if points.len() < 2 {
                    continue;
                }
                painter.add(egui::Shape::line(points.clone(), Stroke::new(5.0, theme::hud_bg())));
                painter.add(egui::Shape::line(points.clone(), Stroke::new(2.5, theme::accent())));
                for end in [points[0], points[points.len() - 1]] {
                    painter.circle(end, 3.5, theme::accent(), Stroke::new(1.5, theme::hud_bg()));
                }
            }
        }
        let galley = painter.layout_no_wrap(rounding.text.clone(), theme::font_medium(12.0), theme::ink());
        let size = vec2(galley.size().x + 40.0, 28.0);
        // Under the heads-up toolbar, centred like it.
        let pill = Rect::from_center_size(pos2(rect.center().x, rect.top() + 58.0 + size.y / 2.0), size);
        let (fill, edge) =
            if rounding.shown { (theme::hud_bg(), theme::accent()) } else { (theme::hud_bg(), theme::line()) };
        painter.rect(pill, CornerRadius::same(14), fill, Stroke::new(1.0, edge), StrokeKind::Inside);
        let icon = Rect::from_center_size(pos2(pill.left() + 17.0, pill.center().y), vec2(14.0, 14.0));
        let glyph = if rounding.title.starts_with("Chamfer") { Icon::Chamfer } else { Icon::Fillet };
        icons::paint(painter, glyph, icon, if rounding.shown { theme::accent() } else { theme::muted() });
        painter.galley(pos2(icon.right() + 7.0, pill.center().y - galley.size().y / 2.0), galley, theme::ink());
        let response = ui.interact(pill, ui.id().with("rounding preview"), Sense::hover());
        response.widget_info(|| {
            let mut info = egui::WidgetInfo::labeled(egui::WidgetType::Other, true, rounding.title);
            info.current_text_value = Some(rounding.text.clone());
            info
        });
    }

    fn paint_highlights(&self, painter: &egui::Painter, doc: &Document, selection: Option<Selection>, rect: Rect) {
        let draw_edge = |pick: u32, stroke: Stroke| {
            for m in doc.meshes.values() {
                if let Some(e) = m.edges.iter().find(|e| e.pick == pick) {
                    for seg in e.segments.as_chunks::<2>().0 {
                        if let (Some(a), Some(b)) =
                            (self.camera.project(seg[0], rect), self.camera.project(seg[1], rect))
                        {
                            painter.line_segment([a, b], stroke);
                        }
                    }
                }
            }
        };
        let draw_vertex = |pos: Vec3, radius: f32, color: Color32| {
            if let Some(p) = self.camera.project(pos, rect) {
                painter.circle(p, radius, color, Stroke::new(1.5, Color32::WHITE));
            }
        };
        let selected_edge = match selection {
            Some(Selection::Edge(e)) => doc.pick_of_edge(e),
            _ => None,
        };
        let selected_vertex = match selection {
            Some(Selection::Vertex(k)) => doc.vertex_position(k),
            _ => None,
        };
        if let Some(p) = selected_edge {
            draw_edge(p, Stroke::new(2.5, theme::accent()));
        }
        if let Some(pos) = selected_vertex {
            draw_vertex(pos, 5.0, theme::accent());
        }
        match self.hover {
            Some(Picked::Edge(p)) if selected_edge != Some(p) => {
                draw_edge(p, Stroke::new(2.0, theme::accent().gamma_multiply(0.7)))
            }
            Some(Picked::Vertex(p)) => {
                if let Some(pos) = pick::vertex_position(doc, p)
                    && selected_vertex != Some(pos)
                {
                    draw_vertex(pos, 4.5, theme::accent().gamma_multiply(0.7));
                }
            }
            _ => {}
        }
    }

    /// The origin axes through the part origin that `shown` names, in [`AxisKind::ALL`]
    /// order, `reach` long on both sides. The positive half is solid and labelled; the
    /// negative half is fainter.
    fn paint_origin_axes(&self, painter: &egui::Painter, reach: f32, rect: Rect, shown: [bool; 3]) {
        let Some(o) = self.camera.project(Vec3::ZERO, rect) else { return };
        // Labels stay inside the view, so an axis that runs off the edge is still named.
        let inside = rect.shrink(10.0);
        for (axis, _) in AxisKind::ALL.into_iter().zip(shown).filter(|(_, shown)| *shown) {
            let color = theme::axis_color(axis);
            let dir = axis.direction();
            if let Some(tail) = self.camera.project(-dir * reach, rect) {
                painter.line_segment([tail, o], Stroke::new(1.0, color.gamma_multiply(0.45)));
            }
            let Some(tip) = self.camera.project(dir * reach, rect) else { continue };
            painter.line_segment([o, tip], Stroke::new(1.5, color.gamma_multiply(0.85)));
            // An axis seen end-on projects to a point: no line to label.
            let along = tip - o;
            if along.length() > 12.0 && inside.contains(o) {
                let end = tip + along.normalized() * 8.0;
                let t = last_inside(o, end, inside);
                // At the edge the label moves beside the line instead of covering it.
                let beside = if t < 1.0 { along.normalized().rot90() * 9.0 } else { vec2(0.0, 0.0) };
                painter.text(
                    o + (end - o) * t + beside,
                    Align2::CENTER_CENTER,
                    &axis.label()[..1],
                    theme::font_semibold(10.0),
                    color,
                );
            }
        }
    }

    fn paint_reference_planes(
        &self,
        ui: &mut egui::Ui,
        doc: &Document,
        rect: Rect,
        can_pick: bool,
        response: &egui::Response,
        events: &mut ViewportEvents,
    ) {
        let bounds = doc.bounds();
        for f in &doc.features {
            if !matches!(f.kind, crate::document::FeatureKind::ReferencePlane(_))
                || doc.view_choice(crate::document::ViewItem::Feature(f.id)) == Some(false)
            {
                continue;
            }
            let Some(plane) = f.plane else { continue };
            let v = |p: soliddraft_kernel_api::Vec3| Vec3::new(p.x as f32, p.y as f32, p.z as f32);
            let origin = v(plane.origin);
            let x = v(plane.x_dir);
            let y = v(plane.normal).cross(x);
            // Center the guide over the part without moving the plane's sketch origin.
            let (center, half) = bounds.map_or((origin, 50.0), |(lo, hi)| {
                let relative = (lo + hi) * 0.5 - origin;
                (origin + x * relative.dot(x) + y * relative.dot(y), (hi - lo).length() * 0.625)
            });
            let u = x * half;
            let w = y * half;
            let corners: Option<Vec<_>> = [center - u - w, center + u - w, center + u + w, center - u + w]
                .into_iter()
                .map(|p| self.camera.project(p, rect))
                .collect();
            let Some(corners) = corners else { continue };
            let ink =
                theme::accent().gamma_multiply(if doc.selection == Some(Selection::Feature(f.id)) { 1.0 } else { 0.6 });
            let painter = ui.painter().with_clip_rect(rect);
            painter.add(egui::Shape::closed_line(corners.clone(), egui::Stroke::new(1.25, ink)));
            let label =
                painter.text(corners[2], egui::Align2::LEFT_BOTTOM, &f.name, egui::FontId::proportional(11.0), ink);
            let response_label = ui.interact(label, ui.id().with(("reference plane label", f.id)), Sense::hover());
            response_label.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("{} in the view", f.name))
            });
            if can_pick
                && self.hover.is_none()
                && response.clicked_by(egui::PointerButton::Primary)
                && let Some(pos) = response.interact_pointer_pos()
                && corners.iter().zip(corners.iter().cycle().skip(1)).any(|(a, b)| {
                    let d = *b - *a;
                    let t = ((pos - *a).dot(d) / d.length_sq().max(1e-6)).clamp(0.0, 1.0);
                    pos.distance(*a + d * t) < 6.0
                })
            {
                events.clicked = None;
                events.clicked_reference = None;
                events.actions.push(AppAction::Select(Some(Selection::Feature(f.id))));
                break;
            }
        }
    }

    /// Each visible plane's name, near its far corner. The tree rows hold the plain names,
    /// so what a screen reader finds here says where it is.
    fn paint_plane_names(&self, ui: &mut egui::Ui, scene: &ReferenceScene, rect: Rect) {
        for (plane, at) in scene.labels(&self.camera, rect) {
            let name = plane.label();
            let painter = ui.painter().with_clip_rect(rect);
            let shown = painter.text(at, Align2::RIGHT_TOP, name, theme::font(11.0), theme::muted());
            let response = ui.interact(shown, ui.id().with(("plane name", name)), Sense::hover());
            let description = format!("{name} in the view");
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &description));
        }
    }

    fn paint_hud(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        events: &mut ViewportEvents,
        sketch_dimensions: Option<bool>,
        allow_dimensions: bool,
    ) {
        if theme::is_edit_suite() {
            self.navigation_bar.show(
                ui,
                rect,
                &mut self.camera,
                self.display,
                self.scene.unwrap_or_default(),
                &mut self.references,
                &mut self.all_dimensions,
                sketch_dimensions,
                allow_dimensions,
                events,
            );
            return;
        }
        let height = 30.0;
        let items: Vec<HudItem> = vec![
            HudItem::Icon(Icon::Home, "Isometric view (I)"),
            HudItem::Icon(Icon::Fit, "Zoom to fit (Z)"),
            HudItem::Text("F", "Front view (F)"),
            HudItem::Text("T", "Top view (T)"),
            HudItem::Text("R", "Right view (R)"),
            HudItem::Sep,
            HudItem::Icon(Icon::Display, "Display style"),
            HudItem::Icon(Icon::Section, "Coming soon"),
            HudItem::Icon(Icon::Measure, "Coming soon"),
        ];
        let width: f32 = items.iter().map(|i| i.width()).sum::<f32>() + 10.0;
        let hud = Rect::from_center_size(pos2(rect.center().x, rect.top() + 12.0 + height / 2.0), vec2(width, height));
        ui.painter().rect(
            hud,
            CornerRadius::same(9),
            theme::hud_bg(),
            Stroke::new(1.0, theme::line()),
            StrokeKind::Inside,
        );
        let mut x = hud.left() + 5.0;
        for (idx, item) in items.iter().enumerate() {
            let w = item.width();
            let r = Rect::from_min_size(pos2(x, hud.top() + 3.0), vec2(w, height - 6.0));
            x += w;
            match item {
                HudItem::Sep => {
                    ui.painter().vline(r.center().x, r.y_range().shrink(5.0), Stroke::new(1.0, theme::line()));
                    continue;
                }
                HudItem::Icon(icon, tip) | HudItem::IconText(icon, tip) => {
                    let enabled = !matches!(icon, Icon::Section | Icon::Measure);
                    let resp = hud_button(ui, r, idx, enabled, tip, |p, c| icons::paint(p, *icon, r.shrink(5.0), c));
                    if resp.clicked() {
                        match idx {
                            0 => events.set_view = Some(StandardView::Isometric),
                            1 => events.fit = true,
                            6 => {
                                let i = DisplayMode::ALL.iter().position(|m| *m == self.display).unwrap_or(0);
                                events.set_display = Some(DisplayMode::ALL[(i + 1) % DisplayMode::ALL.len()]);
                            }
                            _ => {}
                        }
                    }
                }
                HudItem::Text(label, tip) => {
                    let resp = hud_button(ui, r, idx, true, tip, |p, c| {
                        p.text(r.center(), Align2::CENTER_CENTER, label, theme::font_semibold(12.0), c);
                    });
                    if resp.clicked() {
                        events.set_view = Some(match *label {
                            "F" => StandardView::Front,
                            "T" => StandardView::Top,
                            _ => StandardView::Right,
                        });
                    }
                }
            }
        }
    }

    fn paint_view_cube(&mut self, ui: &mut egui::Ui, rect: Rect, events: &mut ViewportEvents) {
        let size = 84.0;
        let center = pos2(rect.right() - 18.0 - size / 2.0, rect.top() + 20.0 + size / 2.0);
        let area = Rect::from_center_size(center, Vec2::splat(size + 16.0));
        let resp = ui.interact(area, ui.id().with("view_cube"), Sense::click());
        let view = self.camera.view();
        let half = size * 0.31;
        let project = |p: Vec3| {
            let v = view.transform_vector3(p);
            (center + vec2(v.x, -v.y) * half, v.z)
        };
        let painter = ui.painter();
        let mut faces: Vec<CubeFace<'_>> = Vec::new();
        for (sv, n, label) in CUBE_FACES {
            let u = if n.z.abs() > 0.5 { Vec3::X } else { Vec3::Z.cross(n).normalize() };
            let v = n.cross(u);
            let corners = [n + u + v, n - u + v, n - u - v, n + u - v];
            let projected: Vec<(Pos2, f32)> = corners.iter().map(|c| project(*c)).collect();
            let depth = projected.iter().map(|(_, z)| z).sum::<f32>() / 4.0;
            let facing = view.transform_vector3(n).z;
            faces.push((sv, n, label, projected.iter().map(|(p, _)| *p).collect(), depth, facing));
        }
        faces.sort_by(|a, b| a.4.total_cmp(&b.4));
        let pointer = resp.hover_pos();
        let mut hovered = None;
        for (sv, _, label, poly, _, facing) in &faces {
            if *facing <= 0.02 {
                continue;
            }
            let is_hover = pointer.is_some_and(|p| point_in_convex(p, poly));
            if is_hover {
                hovered = Some(*sv);
            }
            let shade = 0.82 + 0.18 * facing.clamp(0.0, 1.0);
            let base = if is_hover { theme::selection_bg() } else { theme::chip().gamma_multiply(shade) };
            painter.add(egui::Shape::convex_polygon(
                poly.clone(),
                base,
                Stroke::new(1.0, theme::line().gamma_multiply(1.2)),
            ));
            let c = poly.iter().fold(Vec2::ZERO, |a, p| a + p.to_vec2()) / 4.0;
            let alpha = (facing.clamp(0.0, 1.0) * 1.6).min(1.0);
            let col = if is_hover { theme::selection_ink() } else { theme::muted().gamma_multiply(alpha) };
            let font = theme::font_semibold(6.5 + 1.5 * facing.clamp(0.0, 1.0));
            painter.text(c.to_pos2(), Align2::CENTER_CENTER, label, font, col);
        }
        self.cube_hover = hovered;
        if let Some(sv) = hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if resp.clicked() {
                events.set_view = Some(sv);
            }
        }
    }

    fn paint_pills(&self, ui: &mut egui::Ui, rect: Rect, hover_label: Option<&str>) {
        let rect = if theme::is_edit_suite() { Rect::from_min_max(rect.min, rect.max - vec2(0.0, 44.0)) } else { rect };
        let proj = if self.camera.orthographic { "Orthographic" } else { "Perspective" };
        let text =
            format!("mm · {} · {} · {} · {}", self.camera.view_name, self.display.label(), self.preset.label(), proj);
        let painter = ui.painter();
        let galley = painter.layout_no_wrap(text, theme::font(11.0), theme::muted());
        let size = galley.size() + vec2(18.0, 7.0);
        let r = Rect::from_min_size(pos2(rect.left() + 82.0, rect.bottom() - 12.0 - size.y), size);
        painter.rect(r, CornerRadius::same(99), theme::hud_bg(), Stroke::new(1.0, theme::line()), StrokeKind::Inside);
        painter.galley(r.center() - galley.size() / 2.0, galley, theme::muted());

        if let Some(spacing) = self.grid_spacing(rect) {
            let galley = painter.layout_no_wrap(spacing.label(), theme::font(11.0), theme::muted());
            let size = galley.size() + vec2(18.0, 7.0);
            let r = Rect::from_min_size(pos2(r.right() + 6.0, r.top()), size);
            painter.rect(
                r,
                CornerRadius::same(99),
                theme::hud_bg(),
                Stroke::new(1.0, theme::line()),
                StrokeKind::Inside,
            );
            painter.galley(r.center() - galley.size() / 2.0, galley, theme::muted());
            let response = ui.interact(r, ui.id().with("grid readout"), Sense::hover());
            let description = spacing.description();
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &description));
            response.on_hover_text(&description);
        }

        let painter = ui.painter();
        let (label, color) = match hover_label {
            Some(l) => (l.to_owned(), theme::ink()),
            None => ("Hover a face, edge or vertex".to_owned(), theme::muted()),
        };
        let galley = painter.layout_no_wrap(label, theme::font(11.0), color);
        let size = galley.size() + vec2(18.0, 7.0);
        let r = Rect::from_min_size(pos2(rect.right() - 14.0 - size.x, rect.bottom() - 12.0 - size.y), size);
        painter.rect(r, CornerRadius::same(99), theme::hud_bg(), Stroke::new(1.0, theme::line()), StrokeKind::Inside);
        painter.galley(r.center() - galley.size() / 2.0, galley, color);
    }
}

enum HudItem {
    Icon(Icon, &'static str),
    #[allow(dead_code)]
    IconText(Icon, &'static str),
    Text(&'static str, &'static str),
    Sep,
}

impl HudItem {
    fn width(&self) -> f32 {
        match self {
            HudItem::Sep => 9.0,
            _ => 28.0,
        }
    }
}

fn hud_button(
    ui: &mut egui::Ui,
    rect: Rect,
    idx: usize,
    enabled: bool,
    tip: &str,
    draw: impl FnOnce(&egui::Painter, Color32),
) -> egui::Response {
    let id = ui.id().with(("hud", idx));
    let resp = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tip));
    let painter = ui.painter();
    if enabled && resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(6), theme::hover());
    }
    let color = if enabled { theme::ink() } else { theme::muted().gamma_multiply(0.5) };
    draw(painter, color);
    resp.on_hover_text(tip)
}

fn point_in_convex(p: Pos2, poly: &[Pos2]) -> bool {
    let mut sign = 0.0_f32;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let cross = (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
        if sign == 0.0 {
            sign = cross;
        } else if sign * cross < 0.0 {
            return false;
        }
    }
    true
}

/// Priority: a vertex within 7 px, then an edge within 5 px, then the face
/// under the cursor.
fn decode_pick(sample: &PickSample, cx: u32, cy: u32) -> Option<Picked> {
    if sample.data.is_empty() {
        return None;
    }
    let mut best_vertex: Option<(i32, u32)> = None;
    let mut best_edge: Option<(i32, u32)> = None;
    for dy in -7i32..=7 {
        for dx in -7i32..=7 {
            let (x, y) = (cx as i32 + dx, cy as i32 + dy);
            if x < 0 || y < 0 {
                continue;
            }
            let Some(v) = sample.at(x as u32, y as u32) else { continue };
            let d2 = dx * dx + dy * dy;
            match v >> render::KIND_SHIFT {
                3 if d2 <= 49 && best_vertex.is_none_or(|(bd, _)| d2 < bd) => best_vertex = Some((d2, v)),
                2 if d2 <= 25 && best_edge.is_none_or(|(bd, _)| d2 < bd) => best_edge = Some((d2, v)),
                _ => {}
            }
        }
    }
    if let Some((_, v)) = best_vertex {
        return Picked::decode(v);
    }
    if let Some((_, v)) = best_edge {
        return Picked::decode(v);
    }
    sample.at(cx, cy).and_then(Picked::decode)
}

/// How far along the segment from `a`, which is inside `rect`, to `b` it stays inside,
/// from 0 to 1.
fn last_inside(a: Pos2, b: Pos2, rect: Rect) -> f32 {
    let d = b - a;
    let mut t: f32 = 1.0;
    for (start, delta, lo, hi) in [(a.x, d.x, rect.left(), rect.right()), (a.y, d.y, rect.top(), rect.bottom())] {
        if delta > 0.0 {
            t = t.min((hi - start) / delta);
        } else if delta < 0.0 {
            t = t.min((lo - start) / delta);
        }
    }
    t.max(0.0)
}
