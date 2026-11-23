//! Part, drawing and assembly docks.

use crate::document::Selection;
use crate::icons::{self, Icon};
use crate::panels::Pane;
use crate::state::{AppAction, AppState};
use crate::theme;
use crate::tree;
use crate::viewport::Overlay;
use egui::{Align2, CornerRadius, Rect, Sense, pos2, vec2};
use egui_dock::{DockArea, DockState, TabViewer};

use crate::app::dispatch::selection_of;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Viewport,
    /// The drawing document, opened beside the part by Make drawing.
    Drawing,
    /// The assembly document, opened beside the part by New assembly or Open.
    Assembly,
}

impl Tab {
    fn title(&self) -> &'static str {
        match self {
            Tab::Viewport => "Part",
            Tab::Drawing => "Drawing",
            Tab::Assembly => "Assembly",
        }
    }
}

/// The middle: the part, and the drawing and assembly tabs beside it once open.
pub(in crate::app) fn center_dock() -> DockState<Tab> {
    DockState::new(vec![Tab::Viewport])
}

/// The D1 Clear Bench single side column: stacks every toggled pane (`Tree`, `History`,
/// `Values`, `Props`) vertically top-to-bottom with draggable 7px horizontal splitters and
/// a close `×` per pane.
pub(in crate::app) fn pane(ui: &mut egui::Ui, state: &mut AppState) {
    // While a feature is created or edited the side column is its property panel, with the
    // tree kept below it: planes, axes and features are still picked from the tree. The
    // toggled panes come back when the feature closes.
    if crate::feature_panel::active(state) {
        feature_pane(ui, state);
        return;
    }
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    let mut active = state.layout.panes.active_list();
    // An assembly has no part parameters: its column leaves Values out and keeps its tree.
    if state.assembly.is_active() && active.contains(&Pane::Values) {
        active.retain(|p| *p != Pane::Values);
        if active.is_empty() {
            active.push(Pane::Tree);
        }
    }
    if active.is_empty() {
        return;
    }

    let full = ui.available_rect_before_wrap();
    let splitter_h = theme::spacing().splitter_height;
    let total_splitters = (active.len().saturating_sub(1)) as f32 * splitter_h;
    let usable_h = (full.height() - total_splitters).max(64.0 * active.len() as f32);
    let min_pane_h = (usable_h / active.len() as f32).min(96.0);

    // Compute pixel heights from stored weights.
    let weight_sum: f32 = active.iter().map(|p| state.layout.split_weights[p.index()].max(0.1)).sum();
    let mut heights: Vec<f32> =
        active.iter().map(|p| (state.layout.split_weights[p.index()].max(0.1) / weight_sum) * usable_h).collect();
    // Enforce minimum height per pane and re-normalize so total matches `usable_h`.
    for h in &mut heights {
        if *h < min_pane_h {
            *h = min_pane_h;
        }
    }
    let h_sum: f32 = heights.iter().sum();
    if h_sum > 0.0 {
        let scale = usable_h / h_sum;
        for h in &mut heights {
            *h *= scale;
        }
    }

    // An assembly and a drawing have no history steps yet. Their History pane only says
    // so, and takes the height of that note instead of its share of the column.
    let note_only = state.assembly.is_active() || state.drawing.is_active();
    if let Some(history) = active.iter().position(|p| *p == Pane::History).filter(|_| note_only && active.len() > 1) {
        let note = (theme::spacing().pane_header_height + 76.0).min(heights[history]);
        let others: f32 = heights.iter().sum::<f32>() - heights[history];
        let scale = if others > 0.0 { (usable_h - note) / others } else { 1.0 };
        for (i, h) in heights.iter_mut().enumerate() {
            *h = if i == history { note } else { *h * scale };
        }
    }

    let mut close_request: Option<Pane> = None;
    let mut split_drag: Option<(Pane, Pane, f32)> = None;

    for (i, &p) in active.iter().enumerate() {
        let h = heights[i];
        let pane_rect = ui.allocate_exact_size(vec2(full.width(), h), Sense::hover()).0;
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("d1_pane", p))
                .max_rect(pane_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.set_clip_rect(pane_rect);
        child.spacing_mut().item_spacing = vec2(6.0, 4.0);

        if pane_header(&mut child, state, p) {
            close_request = Some(p);
        }
        // A part's Browser header offers the part's appearance and material, as the
        // tree's own header does. The close button keeps its clicks.
        if p == Pane::Tree && !state.assembly.is_active() && !state.drawing.is_active() {
            let head =
                Rect::from_min_size(pane_rect.min, vec2(pane_rect.width() - 34.0, theme::spacing().pane_header_height));
            let response = child.interact(head, child.id().with("part_header_menu"), Sense::click());
            tree::part_menu(&response, state);
        }

        let body_rect = Rect::from_min_max(
            pos2(pane_rect.left(), pane_rect.top() + theme::spacing().pane_header_height),
            pane_rect.max,
        );
        let mut body = child
            .new_child(egui::UiBuilder::new().max_rect(body_rect).layout(egui::Layout::top_down(egui::Align::Min)));
        body.set_clip_rect(body_rect);
        match p {
            Pane::Tree if state.assembly.is_active() => crate::assembly::tree(&mut body, state),
            Pane::Tree if state.drawing.is_active() => crate::drawing::tree_drawing::show(&mut body, state),
            Pane::Tree => tree::show_contents(&mut body, state),
            Pane::History => crate::history_panel::show(&mut body, state),
            Pane::Values => {
                // Mount point for the Parameters lane.
                egui::Frame::new()
                    .inner_margin(egui::Margin { left: 10, right: 6, top: 4, bottom: 4 })
                    .show(&mut body, |ui| crate::values_tab::show(ui, state));
            }
            Pane::Props => {
                // Mount point for the Feature Panel lane.
                props_pane(&mut body, state);
            }
        }

        // Draggable splitter between pane `i` and pane `i + 1`.
        if i + 1 < active.len() {
            let (srect, sresp) = ui.allocate_exact_size(vec2(full.width(), splitter_h), Sense::drag());
            let sresp = sresp.on_hover_cursor(egui::CursorIcon::ResizeVertical);
            sresp.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Other,
                    true,
                    format!("Resize {:?} and {:?}", p, active[i + 1]),
                )
            });
            let hot = sresp.hovered() || sresp.dragged();
            ui.painter().rect_filled(
                srect,
                CornerRadius::ZERO,
                if hot { theme::accent_soft() } else { theme::panel_header() },
            );
            ui.painter().hline(srect.x_range(), srect.top() + 0.5, egui::Stroke::new(1.0, theme::line()));
            ui.painter().hline(srect.x_range(), srect.bottom() - 0.5, egui::Stroke::new(1.0, theme::line()));
            let grip = Rect::from_center_size(srect.center(), vec2(30.0, 2.0));
            ui.painter().rect_filled(
                grip,
                CornerRadius::same(2),
                if hot { theme::accent() } else { theme::line_strong() },
            );
            if sresp.dragged() {
                let dy = ui.input(|inp| inp.pointer.delta().y);
                if dy.abs() > 0.0 {
                    split_drag = Some((p, active[i + 1], dy));
                }
            }
        }
    }

    if let Some((upper, lower, dy)) = split_drag {
        let delta_w = (dy / usable_h) * weight_sum;
        let min_w = (min_pane_h / usable_h) * weight_sum;
        let u_idx = upper.index();
        let l_idx = lower.index();
        let pair_sum = state.layout.split_weights[u_idx] + state.layout.split_weights[l_idx];
        let next_u = (state.layout.split_weights[u_idx] + delta_w).clamp(min_w, (pair_sum - min_w).max(min_w));
        state.layout.split_weights[u_idx] = next_u;
        state.layout.split_weights[l_idx] = (pair_sum - next_u).max(min_w);
    }

    if let Some(p) = close_request {
        state.close_pane(p);
    }

    // Subtle right divider separating the side column from the viewport.
    ui.painter().vline(full.right() - 0.5, full.y_range(), egui::Stroke::new(1.0, theme::line()));
}

/// The feature property panel over the feature tree, filling the side column. The panel
/// takes what its open sections need, up to two thirds; the tree the rest.
fn feature_pane(ui: &mut egui::Ui, state: &mut AppState) {
    ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
    let full = ui.available_rect_before_wrap();
    let layout = egui::Layout::top_down(egui::Align::Min);
    // A hole is set up in the Hole Wizard, which takes the tree's place as the SOLIDWORKS
    // PropertyManager does. The hole's own form (OK, depths, thread) sits under it and
    // takes what its open sections need, up to half the column.
    if crate::hole_wizard_panel::active(state) {
        let most = (full.height() * 0.5).max(160.0).min(full.height());
        let wanted = crate::feature_panel::needed_height(ui.ctx()).unwrap_or(most);
        let split = full.bottom() - wanted.clamp(160.0_f32.min(most), most);
        let top = Rect::from_min_max(full.min, pos2(full.right(), split));
        let bottom = Rect::from_min_max(pos2(full.left(), split), full.max);
        let mut wizard = ui.new_child(
            egui::UiBuilder::new().id_salt("hole_wizard_panel").max_rect(top.shrink2(vec2(10.0, 8.0))).layout(layout),
        );
        wizard.set_clip_rect(top.intersect(ui.clip_rect()));
        crate::hole_wizard_panel::show_contents(&mut wizard, state);
        ui.painter().hline(full.x_range(), split, egui::Stroke::new(1.0, theme::line_strong()));
        let mut panel =
            ui.new_child(egui::UiBuilder::new().id_salt("feature_panel_area").max_rect(bottom).layout(layout));
        panel.set_clip_rect(bottom.intersect(ui.clip_rect()));
        crate::feature_panel::show(&mut panel, state);
        ui.allocate_rect(full, Sense::hover());
        ui.painter().vline(full.right() - 0.5, full.y_range(), egui::Stroke::new(1.0, theme::line()));
        return;
    }
    let most = (full.height() * 0.68).max(160.0).min(full.height());
    let wanted = crate::feature_panel::needed_height(ui.ctx()).unwrap_or(most);
    let split = full.top() + wanted.clamp(160.0_f32.min(most), most);
    let top = Rect::from_min_max(full.min, pos2(full.right(), split));
    let bottom = Rect::from_min_max(pos2(full.left(), split), full.max);
    let mut panel = ui.new_child(egui::UiBuilder::new().id_salt("feature_panel_area").max_rect(top).layout(layout));
    panel.set_clip_rect(top.intersect(ui.clip_rect()));
    crate::feature_panel::show(&mut panel, state);
    ui.painter().hline(full.x_range(), split, egui::Stroke::new(1.0, theme::line_strong()));
    let mut rows = ui.new_child(egui::UiBuilder::new().id_salt("feature_panel_tree").max_rect(bottom).layout(layout));
    rows.set_clip_rect(bottom.intersect(ui.clip_rect()));
    tree::show(&mut rows, state);
    ui.allocate_rect(full, Sense::hover());
    // The same right divider the toggled panes draw.
    ui.painter().vline(full.right() - 0.5, full.y_range(), egui::Stroke::new(1.0, theme::line()));
}

/// Renders a 30px D1 pane header with icon, title, status badge, and close `×` button.
/// Returns true if the close `×` button was clicked.
fn pane_header(ui: &mut egui::Ui, state: &AppState, pane: Pane) -> bool {
    let h = theme::spacing().pane_header_height;
    let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(head, CornerRadius::ZERO, theme::panel_header());
    painter.hline(head.x_range(), head.bottom() - 0.5, egui::Stroke::new(1.0, theme::line()));

    let (icon, title, badge_text, badge_fg, badge_bg) = match pane {
        Pane::Tree => {
            let title = if state.assembly.is_active() {
                "Assembly"
            } else if state.drawing.is_active() {
                "Drawing"
            } else {
                "Browser"
            };
            let errors = state.doc.error_count();
            let repairs = state.doc.repair_count();
            let (b, fg, bg) = if errors > 0 {
                (format!("{errors} err"), theme::error(), theme::error_soft())
            } else if repairs > 0 {
                (format!("{repairs} fix"), theme::warn(), theme::warn_soft())
            } else {
                ("OK".to_owned(), theme::ok(), theme::ok_soft())
            };
            (Icon::Tree, title, Some(b), fg, bg)
        }
        Pane::History => {
            (Icon::History, "History", Some(crate::history_panel::badge(state)), theme::muted(), theme::chip())
        }
        Pane::Values => {
            let active_cfg =
                state.doc.model().configuration_summary()["active"].as_str().unwrap_or("Default").to_owned();
            (Icon::Formula, "Parameters", Some(active_cfg), theme::muted(), theme::chip())
        }
        Pane::Props => {
            let badge = if state.doc.selection.is_some() { "Selected" } else { "Part" };
            (Icon::Props, "Props", Some(badge.to_owned()), theme::muted(), theme::chip())
        }
    };

    icons::paint(
        painter,
        icon,
        Rect::from_center_size(pos2(head.left() + 16.0, head.center().y), vec2(14.0, 14.0)),
        theme::muted(),
    );
    painter.text(
        pos2(head.left() + 28.0, head.center().y),
        Align2::LEFT_CENTER,
        title,
        theme::font_semibold(12.0),
        theme::ink(),
    );

    // Close `×` button at far right of the pane header
    let close_rect = Rect::from_center_size(pos2(head.right() - 15.0, head.center().y), vec2(20.0, 20.0));
    let close_resp = ui.interact(close_rect, ui.id().with(("close_pane", pane)), Sense::click());
    close_resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Close {title}")));
    if close_resp.hovered() {
        ui.painter().rect_filled(close_rect, CornerRadius::same(4), theme::hover());
    }
    icons::paint(
        ui.painter(),
        Icon::Close,
        close_rect.shrink(4.5),
        if close_resp.hovered() { theme::ink() } else { theme::muted() },
    );

    if let Some(text) = badge_text {
        let galley = ui.painter().layout_no_wrap(text, theme::font_semibold(10.0), badge_fg);
        let pill_w = galley.size().x + 12.0;
        let pill_rect =
            Rect::from_center_size(pos2(close_rect.left() - 6.0 - pill_w / 2.0, head.center().y), vec2(pill_w, 17.0));
        ui.painter().rect_filled(pill_rect, CornerRadius::same(theme::radii().pill), badge_bg);
        ui.painter().galley(
            pos2(pill_rect.center().x - galley.size().x / 2.0, pill_rect.center().y - galley.size().y / 2.0),
            galley,
            badge_fg,
        );
    }

    close_resp.on_hover_text(format!("Close {title}")).clicked()
}

/// Mount point for the Feature Panel & Appearance controls inside the `Props` stacked pane.
fn props_pane(ui: &mut egui::Ui, state: &mut AppState) {
    egui::ScrollArea::vertical().id_salt("d1_props_scroll").auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
            if state.assembly.is_active() {
                crate::assembly::properties(ui, state);
                return;
            }
            if state.drawing.is_active() {
                crate::drawing::properties_drawing::show(ui, state);
                return;
            }
            if state.doc.selection.is_some() || state.sketch.is_some() || state.measure.open || state.pmi.open {
                crate::properties::show(ui, state);
            } else {
                ui.label(
                    egui::RichText::new("Select a face, edge, body or feature to inspect its properties.")
                        .size(11.5)
                        .color(theme::muted()),
                );
                ui.add_space(10.0);
                theme::section_label(ui, "PART APPEARANCE & MATERIAL");
                ui.add_space(4.0);
                let mat = state.doc.material().map_or("Unassigned", |m| m.name.as_str());
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Material:").color(theme::muted()));
                    ui.label(egui::RichText::new(mat).family(theme::medium()));
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Material…").clicked() {
                        state.push(AppAction::ShowMaterial);
                    }
                    if ui.button("Appearances…").clicked() {
                        state.push(AppAction::ShowAppearance);
                    }
                });
            }
        });
    });
}

/// A dock area in the app's style, without the buttons the shell does not use.
pub(in crate::app) fn show_dock(ui: &mut egui::Ui, dock: &mut DockState<Tab>, id: egui::Id, panels: &mut Panels<'_>) {
    let style = theme::dock_style(ui.ctx());
    DockArea::new(dock)
        .id(id)
        .style(style)
        .show_close_buttons(false)
        .show_add_buttons(false)
        .show_leaf_close_all_buttons(false)
        .show_leaf_collapse_buttons(false)
        .show_inside(ui, panels);
}

pub(in crate::app) struct Panels<'a> {
    pub(in crate::app) state: &'a mut AppState,
}

impl TabViewer for Panels<'_> {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Tab) -> egui::Id {
        egui::Id::new(("tab", tab.title()))
    }

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        let text = match tab {
            Tab::Viewport => self.state.doc.name.clone(),
            Tab::Assembly => match self.state.assembly.session.as_ref() {
                Some(s) => s.title(),
                None => "Assembly".to_owned(),
            },
            Tab::Drawing => match self.state.drawing.session.as_ref() {
                // A dot marks a drawing with stale views or unresolved annotations.
                Some(s) if s.has_issues() => format!("{} •", s.title()),
                Some(s) => s.title(),
                None => "Drawing".to_owned(),
            },
        };
        egui::RichText::new(text).family(theme::medium()).size(12.0).into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
        match tab {
            Tab::Assembly => crate::assembly::viewport(ui, self.state),
            Tab::Drawing => crate::drawing::canvas::show(ui, self.state),
            Tab::Viewport => {
                let st = &mut *self.state;
                let section_allowed = st.editor.is_none() && !st.in_sketch() && st.drag_preview.is_none();
                if section_allowed {
                    st.section.refresh(&st.doc, ui.ctx());
                    st.section.show(ui.ctx(), &st.doc);
                }
                st.viewport.section_active = section_allowed && st.section.enabled;
                let section_mesh = section_allowed.then(|| st.section.mesh(&st.doc)).flatten();
                let ghost = st.ghost();
                let preview = section_mesh.or_else(|| st.preview_body());
                let selection = st.doc.selection;
                let hover_label = st.hover_info.as_ref().map(|i| i.title.clone());
                // Shown sketches lie quietly on their planes, except the one open for editing.
                let visible = crate::tree_visibility::visible(st);
                let editing = st.sketch.as_ref().map(|s| s.feature);
                let passive = visible
                    .sketches
                    .iter()
                    .filter(|id| Some(**id) != editing)
                    .filter_map(|id| {
                        let f = st.doc.feature(*id)?;
                        Some((*id, f.plane?, &f.sketch()?.sketch))
                    })
                    .collect();
                let pick_filter = if st.viewport.section_active {
                    crate::viewport::PickFilter { faces: false, edges: false, bodies: false }
                } else {
                    st.selection_filter()
                };
                st.viewport.context_menus = true;
                let overlay = Overlay {
                    pick_filter,
                    dimensions_on_geometry: true,
                    ghost,
                    preview,
                    sketch: st.sketch.as_mut(),
                    passive_sketches: passive,
                    dimension_edit: st.dimension_edit.as_mut(),
                    visible,
                    rounding: st.editor.as_ref().and_then(crate::viewport::RoundingOverlay::of),
                    guides: st.editor.as_ref().map(|e| e.preview.guides.clone()).filter(|g| *g != Default::default()),
                    hole_ghost: match st.editor.as_ref().map(|e| &e.params) {
                        Some(crate::state::EditorParams::Hole(h)) if h.face.is_none() => Some((
                            h.diameter,
                            match h.depth {
                                soliddraft_kernel_api::HoleDepth::Blind { depth } => Some(depth),
                                _ => None,
                            },
                        )),
                        _ => None,
                    },
                };
                let events = st.viewport.show(ui, &st.doc, overlay, selection, hover_label.as_deref(), &st.navigation);
                crate::scan::paint(ui, st);
                st.after_viewport(ui, &events);
                if let Some(response) = &events.context_response {
                    let picked =
                        events.secondary_clicked.flatten().and_then(|p| crate::viewport::pick::selection(&st.doc, p));
                    st.doc.target_selection(picked);
                    let selection = pick_filter.selection(picked);
                    // A right-click on a relation glyph lists the relations on its geometry
                    // in place of the command ring.
                    let mut relations = false;
                    let target = if let Some(session) = st.sketch.as_mut() {
                        if let Some(pos) = events.context_position {
                            let picked =
                                crate::sketcher::context_pick(ui, session, &st.viewport.camera, st.viewport.rect, pos);
                            if !picked.is_some_and(|p| session.selection.contains(&p)) {
                                session.selection = picked.into_iter().collect();
                            }
                            let glyph = match picked {
                                Some(crate::state::SketchSel::Constraint(id))
                                    if session.sketch.constraint(id).is_some_and(|c| !c.is_dimension()) =>
                                {
                                    Some(id)
                                }
                                _ => None,
                            };
                            match glyph {
                                Some(id) => {
                                    crate::sketcher::relations::open(ui.ctx(), session, pos, id);
                                    relations = true;
                                }
                                None => crate::sketcher::relations::close(ui.ctx(), session),
                            }
                        }
                        let picks = session
                            .selection
                            .iter()
                            .filter(|pick| !matches!(pick, crate::state::SketchSel::Constraint(_)))
                            .count();
                        if session.selection.is_empty() {
                            crate::context_menu::Target::EmptySketch
                        } else if picks >= 2 {
                            // Two or more entities: the menu is for the whole selection.
                            crate::context_menu::Target::SketchSelection
                        } else {
                            crate::context_menu::Target::SketchLine
                        }
                    } else {
                        crate::context_menu::Target::of(selection)
                    };
                    if relations {
                        crate::context_menu::close(response);
                    } else {
                        crate::context_menu::show_at(
                            st,
                            response,
                            target,
                            selection,
                            events.context_position,
                            events.context_ring,
                        );
                    }
                    if let Some(session) = st.sketch.as_mut()
                        && let Some(action) = crate::sketcher::relations::show(ui, session, response.layer_id)
                    {
                        st.push(action);
                    }
                }
                let (dt, ppp) = ui.input(|i| (i.stable_dt.min(0.1), i.pixels_per_point));
                if st.doc.update_display(&st.viewport.projection(ppp), dt) {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
                }
                for a in events.actions {
                    st.push(a);
                }
                if let Some(id) = events.clicked_sketch.filter(|_| !st.viewport.section_active) {
                    st.push(AppAction::Select(Some(Selection::Feature(id))));
                }
                if let Some(picked) = events.clicked.filter(|_| !st.viewport.section_active) {
                    let reference = events.clicked_reference.map(Selection::Reference);
                    let sel = picked.and_then(|p| selection_of(&st.doc, p)).or(reference);
                    st.doc.target_selection(sel);
                    let sel = pick_filter.selection(sel);
                    st.click_point = sel.zip(events.clicked_point);
                    st.push(AppAction::Select(sel));
                }
                if let Some(v) = events.set_view {
                    st.push(AppAction::SetView(v));
                }
                if events.fit {
                    st.push(AppAction::ZoomFit);
                }
                if let Some(d) = events.set_display {
                    st.push(AppAction::SetDisplay(d));
                }
                if events.toggle_projection {
                    st.push(AppAction::ToggleProjection);
                }
            }
        }
    }

    fn is_closeable(&self, tab: &Tab) -> bool {
        matches!(tab, Tab::Drawing | Tab::Assembly)
    }

    fn on_close(&mut self, tab: &mut Tab) -> egui_dock::tab_viewer::OnCloseResponse {
        if matches!(tab, Tab::Drawing) {
            self.state.drawing.session = None;
            self.state.drawing.active = false;
        }
        if matches!(tab, Tab::Assembly) {
            self.state.assembly.session = None;
            self.state.assembly.active = false;
        }
        egui_dock::tab_viewer::OnCloseResponse::Close
    }

    fn clear_background(&self, _tab: &Tab) -> bool {
        true
    }

    fn scroll_bars(&self, _tab: &Tab) -> [bool; 2] {
        [false, false]
    }
}
