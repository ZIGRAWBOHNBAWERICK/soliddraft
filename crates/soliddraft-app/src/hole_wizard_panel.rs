//! Self-contained Hole Wizard property panel widget.
//!
//! Designed with two tabs (**Type** and **Positions**) and collapsible sections so it
//! mounts cleanly in the current Edit Suite layout and can be re-homed into the D1
//! Clear Bench feature panel without untangling from the top options bar.

use crate::document::{HoleParams, HolePickTarget, HoleWizardTab, fmt_mm};
use crate::state::{AppAction, AppState, EditorParams, NoteKind, Tool};
use crate::theme;
use egui::{CornerRadius, RichText, Stroke, vec2};
use soliddraft_kernel_api::{HoleDepth, HoleHead, HoleTip};
use soliddraft_model::hole_wizard::{
    CalloutParam, CalloutTolerance, CountersinkOption, HoleEndCondition, HoleFit, HoleStandard, HoleType,
    ToleranceMode, fastener_tables,
};

/// Sentinel feature id used when an inline Hole Wizard position sketch is open in the
/// viewport sketcher before the Hole Wizard is committed.
pub const INLINE_POSITION_SKETCH_ID: u64 = u64::MAX - 17;

/// Mounts the Hole Wizard property panel in the current frame layout whenever a Hole
/// feature editor is active.
pub fn panel(root: &mut egui::Ui, state: &mut AppState) {
    let active = !state.assembly.is_active()
        && !state.drawing.is_active()
        && matches!(state.editor.as_ref().map(|e| &e.params), Some(EditorParams::Hole(_)));
    if !active {
        return;
    }
    egui::Panel::left("hole_wizard_panel")
        .resizable(true)
        .default_size(304.0)
        .size_range(260.0..=420.0)
        .frame(egui::Frame::new().fill(theme::panel()).inner_margin(egui::Margin::symmetric(10, 8)))
        .show(root, |ui| show_contents(ui, state));
}

/// Pure self-contained Hole Wizard widget suitable for mounting in any feature panel slot.
pub fn show_contents(ui: &mut egui::Ui, state: &mut AppState) {
    let is_inline_sketch = state.sketch.as_ref().is_some_and(|s| s.feature == INLINE_POSITION_SKETCH_ID);
    let mut actions: Vec<AppAction> = Vec::new();
    let mut changed = false;
    let mut note: Option<(String, NoteKind)> = None;

    let Some(editor) = state.editor.as_mut() else { return };
    let EditorParams::Hole(params) = &mut editor.params else { return };

    // Header row: title + Apply / Cancel actions.
    ui.horizontal(|ui| {
        let title = match params.wizard_tab {
            HoleWizardTab::Type => "Hole Specification",
            HoleWizardTab::Positions => "Hole Positions",
        };
        ui.label(RichText::new(title).family(theme::semibold()).size(13.5).color(theme::ink()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let cancel_btn = ui.button("✕").on_hover_text("Cancel Hole Wizard");
            cancel_btn.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Cancel Hole Wizard"));
            if cancel_btn.clicked() {
                actions.push(AppAction::CancelEditor);
            }
            let apply_btn = ui
                .add(
                    egui::Button::new(RichText::new("✓ Apply").color(theme::on_accent()))
                        .fill(theme::accent())
                        .corner_radius(CornerRadius::same(4)),
                )
                .on_hover_text("Commit Hole Wizard feature");
            apply_btn.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Apply Hole Wizard"));
            if apply_btn.clicked() {
                actions.push(AppAction::CommitEditor);
            }
        });
    });

    ui.add_space(4.0);

    // Tab strip: Type | Positions.
    let point_count = params.points.len();
    ui.horizontal(|ui| {
        let type_selected = params.wizard_tab == HoleWizardTab::Type;
        let pos_selected = params.wizard_tab == HoleWizardTab::Positions;
        let type_btn = ui.selectable_label(type_selected, "Type");
        type_btn.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Hole Wizard Type tab"));
        if type_btn.clicked() && !type_selected {
            params.wizard_tab = HoleWizardTab::Type;
            params.picking_target = None;
            if is_inline_sketch {
                actions.push(AppAction::CloseHolePositionSketch);
            }
        }
        let pos_btn = ui.selectable_label(pos_selected, format!("Positions ({point_count})"));
        pos_btn
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Hole Wizard Positions tab"));
        if pos_btn.clicked() && !pos_selected {
            params.wizard_tab = HoleWizardTab::Positions;
            params.picking_target = None;
            params.ensure_position_sketch();
        }
    });
    ui.separator();

    if let Some(err) = &editor.error {
        egui::Frame::new()
            .fill(theme::error().gamma_multiply(0.15))
            .stroke(Stroke::new(1.0, theme::error()))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(egui::Margin::symmetric(8, 5))
            .show(ui, |ui| {
                ui.label(RichText::new(err).size(11.5).color(theme::error()));
            });
        ui.add_space(4.0);
    }

    egui::ScrollArea::vertical().id_salt("hole_wizard_scroll").auto_shrink([false, false]).show(ui, |ui| {
        match params.wizard_tab {
            HoleWizardTab::Type => {
                changed |= show_type_tab(ui, params, &mut note);
            }
            HoleWizardTab::Positions => {
                changed |= show_positions_tab(ui, params, is_inline_sketch, &mut actions, &mut note);
            }
        }
    });

    if changed {
        if let Some(ed) = state.editor.as_mut() {
            ed.refresh(&state.doc);
        }
    }
    if let Some((msg, kind)) = note {
        state.note(msg, kind);
    }
    for action in actions {
        state.push(action);
    }
}

fn show_type_tab(ui: &mut egui::Ui, params: &mut HoleParams, note: &mut Option<(String, NoteKind)>) -> bool {
    let mut changed = false;

    // -----------------------------------------------------------------------
    // 1. Favorite section
    // -----------------------------------------------------------------------
    egui::CollapsingHeader::new(RichText::new("Favorite").family(theme::semibold()))
        .id_salt("hw_favorite")
        .default_open(true)
        .show(ui, |ui| {
            let mut favorites = crate::settings::hole_favorites();
            let mut apply_idx: Option<usize> = None;
            let mut delete_idx: Option<usize> = None;

            let selected_text = if favorites.is_empty() {
                "No favorites saved".to_owned()
            } else {
                "Apply a favorite…".to_owned()
            };
            egui::ComboBox::from_id_salt("hw_fav_combo")
                .width(ui.available_width())
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    for (i, fav) in favorites.iter().enumerate() {
                        let label = format!(
                            "{} ({} {} {})",
                            fav.name,
                            fav.spec.standard.label(),
                            fav.spec.size,
                            fav.spec.hole_type.short_label()
                        );
                        if ui.selectable_label(false, label).clicked() {
                            apply_idx = Some(i);
                        }
                    }
                })
                .response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Wizard Favorites"));

            if let Some(i) = apply_idx
                && let Some(fav) = favorites.get(i)
            {
                params.apply_favorite(fav);
                params.favorite_name = fav.name.clone();
                changed = true;
                *note = Some((format!("Applied favorite \"{}\"", fav.name), NoteKind::Info));
            }

            ui.horizontal(|ui| {
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut params.favorite_name)
                        .desired_width(ui.available_width() - 108.0)
                        .hint_text("Preset name…"),
                );
                edit.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Favorite name"));

                let save_btn = ui.button("Save").on_hover_text("Save or update this hole preset");
                save_btn.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Save favorite"));
                if save_btn.clicked() {
                    let name = params.favorite_name.trim().to_owned();
                    if name.is_empty() {
                        *note = Some(("Enter a favorite name first".into(), NoteKind::Error));
                    } else {
                        let fav = params.to_favorite(name.clone());
                        if let Some(existing) = favorites.iter_mut().find(|f| f.name == name) {
                            *existing = fav;
                        } else {
                            favorites.push(fav);
                        }
                        match crate::settings::save_hole_favorites(&favorites) {
                            Ok(()) => *note = Some((format!("Saved favorite \"{name}\""), NoteKind::Ok)),
                            Err(e) => *note = Some((format!("Could not save favorite: {e}"), NoteKind::Error)),
                        }
                    }
                }

                let existing = favorites.iter().position(|f| f.name == params.favorite_name.trim());
                let del_btn = ui
                    .add_enabled(existing.is_some(), egui::Button::new("Delete"))
                    .on_hover_text("Delete the named favorite");
                del_btn.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Delete favorite"));
                if del_btn.clicked() {
                    delete_idx = existing;
                }
            });

            if let Some(i) = delete_idx {
                let removed = favorites.remove(i);
                match crate::settings::save_hole_favorites(&favorites) {
                    Ok(()) => {
                        params.favorite_name.clear();
                        *note = Some((format!("Deleted favorite \"{}\"", removed.name), NoteKind::Info));
                    }
                    Err(e) => *note = Some((format!("Could not delete favorite: {e}"), NoteKind::Error)),
                }
            }
        });

    ui.add_space(4.0);

    // -----------------------------------------------------------------------
    // 2. Hole Type + Standard + Fastener Type
    // -----------------------------------------------------------------------
    egui::CollapsingHeader::new(RichText::new("Hole Type").family(theme::semibold()))
        .id_salt("hw_type")
        .default_open(true)
        .show(ui, |ui| {
            let current_type = params.wizard.as_ref().map(|w| w.hole_type).unwrap_or_else(|| {
                if params.thread.is_some() {
                    HoleType::StraightTap
                } else if matches!(params.head, Some(HoleHead::Counterbore { .. })) {
                    HoleType::Counterbore
                } else if matches!(params.head, Some(HoleHead::Countersink { .. })) {
                    HoleType::Countersink
                } else {
                    HoleType::Simple
                }
            });

            // 2 rows x 4 columns of hole type tiles.
            let tile_w = ((ui.available_width() - 12.0) / 4.0).max(54.0);
            for row in HoleType::ALL.chunks(4) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for &ty in row {
                        let selected = params.wizard.is_some() && current_type == ty;
                        let soon = ty.coming_soon_reason().is_some();
                        let label = if soon {
                            format!("{}\n(soon)", ty.short_label())
                        } else {
                            ty.short_label().to_owned()
                        };
                        let fill = if selected { theme::accent_soft() } else { theme::field_bg() };
                        let stroke = if selected {
                            Stroke::new(1.5, theme::accent())
                        } else {
                            Stroke::new(1.0, theme::line_strong())
                        };
                        let btn = ui
                            .add_sized(
                                vec2(tile_w, 34.0),
                                egui::Button::new(
                                    RichText::new(label)
                                        .size(10.5)
                                        .color(if selected { theme::accent() } else { theme::ink() }),
                                )
                                .fill(fill)
                                .stroke(stroke)
                                .corner_radius(CornerRadius::same(4)),
                            )
                            .on_hover_text(match ty.coming_soon_reason() {
                                Some(reason) => format!("{}: {reason}", ty.label()),
                                None => ty.label().to_owned(),
                            });
                        btn.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                true,
                                format!("Hole type: {}", ty.label()),
                            )
                        });
                        if btn.clicked() {
                            params.ensure_wizard().set_hole_type(ty);
                            params.apply_wizard_defaults();
                            changed = true;
                        }
                    }
                });
                ui.add_space(2.0);
            }

            if let Some(reason) = params.wizard.as_ref().and_then(|w| w.hole_type.coming_soon_reason()) {
                ui.label(RichText::new(reason).size(11.0).color(theme::warn()));
            }

            ui.add_space(4.0);

            // Standard combo box.
            let current_std = params.wizard.as_ref().map(|w| w.standard).unwrap_or(HoleStandard::Iso);
            let mut chosen_std = current_std;
            ui.label(RichText::new("Standard").size(11.0).color(theme::muted()));
            egui::ComboBox::from_id_salt("hw_standard")
                .width(ui.available_width())
                .selected_text(chosen_std.label())
                .show_ui(ui, |ui| {
                    for &std in &HoleStandard::ALL {
                        ui.selectable_value(&mut chosen_std, std, std.label()).on_hover_text(std.source_summary());
                    }
                })
                .response
                .on_hover_text(chosen_std.source_summary())
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Wizard Standard"));
            if chosen_std != current_std {
                params.ensure_wizard().set_standard(chosen_std);
                params.apply_wizard_defaults();
                changed = true;
            }

            // Fastener type combo box.
            let active_type = params.wizard.as_ref().map(|w| w.hole_type).unwrap_or(current_type);
            let tables = fastener_tables(chosen_std, active_type);
            let current_fastener = params
                .wizard
                .as_ref()
                .map(|w| w.fastener.clone())
                .unwrap_or_else(|| tables[0].id.to_owned());
            let current_table = tables.iter().find(|t| t.id == current_fastener).unwrap_or(&tables[0]);
            let mut chosen_fastener = current_table.id.to_owned();
            ui.label(RichText::new("Type").size(11.0).color(theme::muted()));
            egui::ComboBox::from_id_salt("hw_fastener")
                .width(ui.available_width())
                .selected_text(current_table.label)
                .show_ui(ui, |ui| {
                    for t in tables {
                        ui.selectable_value(&mut chosen_fastener, t.id.to_owned(), t.label)
                            .on_hover_text(format!("Source: {}", t.citation));
                    }
                })
                .response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Wizard Fastener Type"));
            if chosen_fastener != current_table.id {
                params.ensure_wizard().set_fastener(&chosen_fastener);
                params.apply_wizard_defaults();
                changed = true;
            }
            ui.label(RichText::new(format!("Ref: {}", current_table.citation)).size(10.0).color(theme::muted()));
        });

    ui.add_space(4.0);

    // -----------------------------------------------------------------------
    // 3. Hole Specifications (Size, Fit, Slot Length/Angle, Custom Sizing)
    // -----------------------------------------------------------------------
    egui::CollapsingHeader::new(RichText::new("Hole Specifications").family(theme::semibold()))
        .id_salt("hw_specs")
        .default_open(true)
        .show(ui, |ui| {
            let (std, htype) = params
                .wizard
                .as_ref()
                .map(|w| (w.standard, w.hole_type))
                .unwrap_or((HoleStandard::Iso, HoleType::Simple));
            let table = params
                .wizard
                .as_ref()
                .map(|w| w.fastener_table())
                .unwrap_or_else(|| &fastener_tables(std, htype)[0]);
            let current_size = params
                .wizard
                .as_ref()
                .map(|w| w.size.clone())
                .unwrap_or_else(|| table.sizes[0].name.to_owned());
            let mut chosen_size = current_size.clone();

            ui.label(RichText::new("Size").size(11.0).color(theme::muted()));
            egui::ComboBox::from_id_salt("hw_size")
                .width(ui.available_width())
                .selected_text(&chosen_size)
                .show_ui(ui, |ui| {
                    for row in table.sizes {
                        ui.selectable_value(&mut chosen_size, row.name.to_owned(), row.name);
                    }
                })
                .response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Wizard Size"));
            if chosen_size != current_size {
                let w = params.ensure_wizard();
                w.size = chosen_size;
                params.apply_wizard_defaults();
                changed = true;
            }

            if htype.uses_fit() {
                let current_fit = params.wizard.as_ref().map(|w| w.fit).unwrap_or(HoleFit::Normal);
                let mut chosen_fit = current_fit;
                ui.label(RichText::new("Fit").size(11.0).color(theme::muted()));
                egui::ComboBox::from_id_salt("hw_fit")
                    .width(ui.available_width())
                    .selected_text(chosen_fit.label())
                    .show_ui(ui, |ui| {
                        for &fit in &HoleFit::ALL {
                            ui.selectable_value(&mut chosen_fit, fit, fit.label());
                        }
                    })
                    .response
                    .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Wizard Fit"));
                if chosen_fit != current_fit {
                    params.ensure_wizard().fit = chosen_fit;
                    params.apply_wizard_defaults();
                    changed = true;
                }
            }

            if htype.is_slot() {
                let mut length = params
                    .wizard
                    .as_ref()
                    .and_then(|w| w.slot_length)
                    .unwrap_or_else(|| table.sizes[0].default_slot_length_mm());
                let mut angle = params.wizard.as_ref().map(|w| w.slot_angle_deg).unwrap_or(0.0);
                ui.horizontal(|ui| {
                    ui.label("Slot length");
                    let r = ui.add(
                        egui::DragValue::new(&mut length).speed(0.5).range(0.1..=1000.0).suffix(" mm"),
                    );
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, "Slot length"));
                    if r.changed() {
                        params.ensure_wizard().slot_length = Some(length);
                        changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Slot angle");
                    let r = ui.add(egui::DragValue::new(&mut angle).speed(1.0).range(-360.0..=360.0).suffix("°"));
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, "Slot angle"));
                    if r.changed() {
                        params.ensure_wizard().slot_angle_deg = angle;
                        changed = true;
                    }
                });
            }

            ui.add_space(2.0);
            ui.checkbox(&mut params.show_custom_sizing, "Show custom sizing");

            if params.show_custom_sizing {
                let dia_label = if htype.is_tapped() { "Tap drill" } else { "Through dia" };
                ui.horizontal(|ui| {
                    ui.label(dia_label);
                    let r = ui.add(
                        egui::DragValue::new(&mut params.diameter)
                            .speed(0.1)
                            .range(0.1..=500.0)
                            .fixed_decimals(3)
                            .suffix(" mm"),
                    );
                    r.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, "Wizard custom diameter")
                    });
                    if r.changed() {
                        params.ensure_wizard().custom_sizing = true;
                        changed = true;
                    }
                });

                let mut mark_custom = false;
                match &mut params.head {
                    Some(HoleHead::Counterbore { diameter, depth }) => {
                        ui.horizontal(|ui| {
                            ui.label("CBore dia");
                            let r = ui.add(
                                egui::DragValue::new(diameter)
                                    .speed(0.1)
                                    .range(0.2..=600.0)
                                    .fixed_decimals(3)
                                    .suffix(" mm"),
                            );
                            r.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::DragValue,
                                    true,
                                    "Wizard counterbore diameter",
                                )
                            });
                            if r.changed() {
                                mark_custom = true;
                                changed = true;
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("CBore depth");
                            let r = ui.add(
                                egui::DragValue::new(depth)
                                    .speed(0.1)
                                    .range(0.1..=500.0)
                                    .fixed_decimals(3)
                                    .suffix(" mm"),
                            );
                            r.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::DragValue,
                                    true,
                                    "Wizard counterbore depth",
                                )
                            });
                            if r.changed() {
                                mark_custom = true;
                                changed = true;
                            }
                        });
                    }
                    Some(HoleHead::Countersink { diameter, angle_deg }) => {
                        ui.horizontal(|ui| {
                            ui.label("CSink dia");
                            let r = ui.add(
                                egui::DragValue::new(diameter)
                                    .speed(0.1)
                                    .range(0.2..=600.0)
                                    .fixed_decimals(3)
                                    .suffix(" mm"),
                            );
                            r.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::DragValue,
                                    true,
                                    "Wizard countersink diameter",
                                )
                            });
                            if r.changed() {
                                mark_custom = true;
                                changed = true;
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("CSink angle");
                            let r = ui.add(
                                egui::DragValue::new(angle_deg).speed(1.0).range(10.0..=175.0).suffix("°"),
                            );
                            r.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::DragValue,
                                    true,
                                    "Wizard countersink angle",
                                )
                            });
                            if r.changed() {
                                mark_custom = true;
                                changed = true;
                            }
                        });
                    }
                    None => {}
                }

                if !htype.is_slot() {
                    let mut cone = matches!(params.tip, HoleTip::Cone { .. });
                    if ui.checkbox(&mut cone, "Drill-point tip").changed() {
                        params.tip = if cone { HoleTip::Cone { angle_deg: 118.0 } } else { HoleTip::Flat };
                        mark_custom = true;
                        changed = true;
                    }
                    if let HoleTip::Cone { angle_deg } = &mut params.tip {
                        ui.horizontal(|ui| {
                            ui.label("Tip angle");
                            let r = ui.add(egui::DragValue::new(angle_deg).speed(1.0).range(30.0..=179.0).suffix("°"));
                            if r.changed() {
                                mark_custom = true;
                                changed = true;
                            }
                        });
                    }
                }
                if mark_custom {
                    params.ensure_wizard().custom_sizing = true;
                }

                let restore = ui.button("Restore Default Values");
                restore.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Restore Default Values")
                });
                if restore.clicked() {
                    params.ensure_wizard();
                    params.restore_wizard_defaults();
                    changed = true;
                }
            }
        });

    ui.add_space(4.0);

    // -----------------------------------------------------------------------
    // 4. End Condition (6 modes + Flip direction)
    // -----------------------------------------------------------------------
    egui::CollapsingHeader::new(RichText::new("End Condition").family(theme::semibold()))
        .id_salt("hw_end")
        .default_open(true)
        .show(ui, |ui| {
            let current_end = params.wizard.as_ref().map(|w| w.end_condition.clone()).unwrap_or_else(|| match params.depth {
                HoleDepth::Through => HoleEndCondition::ThroughAll,
                HoleDepth::Blind { depth } => HoleEndCondition::Blind { depth },
            });
            let end_kind = match &current_end {
                HoleEndCondition::Blind { .. } => 0,
                HoleEndCondition::ThroughAll => 1,
                HoleEndCondition::UpToNext => 2,
                HoleEndCondition::UpToVertex { .. } => 3,
                HoleEndCondition::UpToSurface { .. } => 4,
                HoleEndCondition::OffsetFromSurface { .. } => 5,
            };
            let mut chosen_kind = end_kind;
            let labels = [
                "Blind",
                "Through All",
                "Up To Next",
                "Up To Vertex",
                "Up To Surface",
                "Offset From Surface",
            ];
            egui::ComboBox::from_id_salt("hw_end_combo")
                .width(ui.available_width())
                .selected_text(labels[chosen_kind])
                .show_ui(ui, |ui| {
                    for (i, label) in labels.iter().enumerate() {
                        ui.selectable_value(&mut chosen_kind, i, *label);
                    }
                })
                .response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Wizard End Condition"));

            if chosen_kind != end_kind {
                let next_end = match chosen_kind {
                    0 => HoleEndCondition::Blind { depth: 10.0 },
                    1 => HoleEndCondition::ThroughAll,
                    2 => HoleEndCondition::UpToNext,
                    3 => {
                        params.picking_target = Some(HolePickTarget::Vertex);
                        if let Some(f) = params.face.clone() {
                            HoleEndCondition::UpToVertex {
                                vertex: soliddraft_model::VertexRef {
                                    faces: vec![f],
                                    position: soliddraft_kernel_api::Vec3::ZERO,
                                    label: String::new(),
                                },
                            }
                        } else {
                            HoleEndCondition::ThroughAll
                        }
                    }
                    4 => {
                        params.picking_target = Some(HolePickTarget::Surface);
                        if let Some(f) = params.face.clone() {
                            HoleEndCondition::UpToSurface { face: f }
                        } else {
                            HoleEndCondition::ThroughAll
                        }
                    }
                    _ => {
                        params.picking_target = Some(HolePickTarget::Surface);
                        if let Some(f) = params.face.clone() {
                            HoleEndCondition::OffsetFromSurface { face: f, offset: 5.0, reverse_offset: false }
                        } else {
                            HoleEndCondition::ThroughAll
                        }
                    }
                };
                params.depth = match &next_end {
                    HoleEndCondition::Blind { depth } => HoleDepth::Blind { depth: *depth },
                    _ => HoleDepth::Through,
                };
                params.ensure_wizard().end_condition = next_end;
                changed = true;
            }

            if let Some(w) = params.wizard.as_mut() {
                match &mut w.end_condition {
                    HoleEndCondition::Blind { depth } => {
                        ui.horizontal(|ui| {
                            ui.label("Depth");
                            let r = ui.add(
                                egui::DragValue::new(depth).speed(0.5).range(0.1..=5000.0).suffix(" mm"),
                            );
                            r.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, "Wizard Blind Depth")
                            });
                            if r.changed() {
                                params.depth = HoleDepth::Blind { depth: *depth };
                                changed = true;
                            }
                        });
                    }
                    HoleEndCondition::UpToVertex { vertex } => {
                        let picking = params.picking_target == Some(HolePickTarget::Vertex);
                        if ui
                            .selectable_label(
                                picking,
                                if vertex.label.is_empty() {
                                    "Click a vertex in viewport…"
                                } else {
                                    &vertex.label
                                },
                            )
                            .clicked()
                        {
                            params.picking_target = if picking { None } else { Some(HolePickTarget::Vertex) };
                        }
                    }
                    HoleEndCondition::UpToSurface { face } => {
                        let picking = params.picking_target == Some(HolePickTarget::Surface);
                        if ui
                            .selectable_label(
                                picking,
                                if picking { "Click target planar face…" } else { &face.label },
                            )
                            .clicked()
                        {
                            params.picking_target = if picking { None } else { Some(HolePickTarget::Surface) };
                        }
                    }
                    HoleEndCondition::OffsetFromSurface { face, offset, reverse_offset } => {
                        let picking = params.picking_target == Some(HolePickTarget::Surface);
                        if ui
                            .selectable_label(
                                picking,
                                if picking { "Click target planar face…" } else { &face.label },
                            )
                            .clicked()
                        {
                            params.picking_target = if picking { None } else { Some(HolePickTarget::Surface) };
                        }
                        ui.horizontal(|ui| {
                            ui.label("Offset");
                            if ui
                                .add(egui::DragValue::new(offset).speed(0.5).range(0.0..=5000.0).suffix(" mm"))
                                .changed()
                            {
                                changed = true;
                            }
                        });
                        if ui.checkbox(reverse_offset, "Reverse offset").changed() {
                            changed = true;
                        }
                    }
                    HoleEndCondition::ThroughAll | HoleEndCondition::UpToNext => {}
                }

                if ui.checkbox(&mut w.flip_direction, "Flip direction").changed() {
                    changed = true;
                }
            }
        });

    ui.add_space(4.0);

    // -----------------------------------------------------------------------
    // 5. Options (Head clearance, Near / Under / Far CSK, Cosmetic Thread)
    // -----------------------------------------------------------------------
    egui::CollapsingHeader::new(RichText::new("Options").family(theme::semibold()))
        .id_salt("hw_options")
        .default_open(true)
        .show(ui, |ui| {
            let htype = params.wizard.as_ref().map(|w| w.hole_type).unwrap_or(HoleType::Simple);

            // Head clearance
            if htype.has_counterbore() || htype.has_countersink() {
                let mut hc_on = params.wizard.as_ref().and_then(|w| w.options.head_clearance).is_some();
                if ui.checkbox(&mut hc_on, "Head clearance").changed() {
                    params.ensure_wizard().options.head_clearance = hc_on.then_some(0.5);
                    changed = true;
                }
                if let Some(w) = params.wizard.as_mut()
                    && let Some(hc) = &mut w.options.head_clearance
                {
                    ui.horizontal(|ui| {
                        ui.label("Clearance");
                        if ui.add(egui::DragValue::new(hc).speed(0.1).range(0.0..=100.0).suffix(" mm")).changed() {
                            changed = true;
                        }
                    });
                    if htype.has_countersink() {
                        ui.label(
                            RichText::new("Head clearance on a countersink is coming soon.")
                                .size(10.5)
                                .color(theme::warn()),
                        );
                    }
                }
            }

            // Near side countersink
            let mut near_on = params.wizard.as_ref().and_then(|w| w.options.near_side_csk).is_some();
            if ui.checkbox(&mut near_on, "Near side countersink").changed() {
                let default_dia = ((params.diameter + 1.5) * 10.0).round() / 10.0;
                params.ensure_wizard().options.near_side_csk =
                    near_on.then_some(CountersinkOption { diameter: default_dia, angle_deg: 90.0 });
                changed = true;
            }
            if let Some(w) = params.wizard.as_mut()
                && let Some(csk) = &mut w.options.near_side_csk
            {
                changed |= csk_fields(ui, "near", csk);
                if htype.has_counterbore() {
                    ui.label(
                        RichText::new("Near side countersink on a counterbore is coming soon.")
                            .size(10.5)
                            .color(theme::warn()),
                    );
                }
            }

            // Under head countersink (coming soon)
            if htype.has_counterbore() {
                let mut under_on = params.wizard.as_ref().and_then(|w| w.options.under_head_csk).is_some();
                if ui.checkbox(&mut under_on, "Under head countersink (coming soon)").changed() {
                    let default_dia = ((params.diameter + 1.0) * 10.0).round() / 10.0;
                    params.ensure_wizard().options.under_head_csk =
                        under_on.then_some(CountersinkOption { diameter: default_dia, angle_deg: 90.0 });
                    changed = true;
                }
                if let Some(w) = params.wizard.as_mut()
                    && let Some(csk) = &mut w.options.under_head_csk
                {
                    changed |= csk_fields(ui, "under", csk);
                    ui.label(
                        RichText::new("Under head countersink is coming soon.").size(10.5).color(theme::warn()),
                    );
                }
            }

            // Far side countersink (coming soon)
            let mut far_on = params.wizard.as_ref().and_then(|w| w.options.far_side_csk).is_some();
            if ui.checkbox(&mut far_on, "Far side countersink (coming soon)").changed() {
                let default_dia = ((params.diameter + 1.5) * 10.0).round() / 10.0;
                params.ensure_wizard().options.far_side_csk =
                    far_on.then_some(CountersinkOption { diameter: default_dia, angle_deg: 90.0 });
                changed = true;
            }
            if let Some(w) = params.wizard.as_mut()
                && let Some(csk) = &mut w.options.far_side_csk
            {
                changed |= csk_fields(ui, "far", csk);
                ui.label(RichText::new("Far side countersink is coming soon.").size(10.5).color(theme::warn()));
            }

            // Cosmetic thread controls for tapped holes.
            if htype.is_tapped() {
                ui.separator();
                let mut cos = params.wizard.as_ref().map_or(true, |w| w.options.cosmetic_thread);
                if ui.checkbox(&mut cos, "Cosmetic thread").changed() {
                    params.ensure_wizard().options.cosmetic_thread = cos;
                    if cos {
                        let row = params.wizard.as_ref().unwrap().size_row();
                        params.thread = row.hole_thread(None);
                    } else {
                        params.thread = None;
                    }
                    changed = true;
                }
                if let Some(thread) = &mut params.thread {
                    ui.label(
                        RichText::new(format!("Thread: {}", thread.designation()))
                            .size(11.5)
                            .color(theme::accent()),
                    );
                    let mut full = thread.length.is_none();
                    let cb = ui.checkbox(&mut full, "Full thread depth");
                    cb.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Checkbox, true, "Wizard Full Thread Depth")
                    });
                    if cb.changed() {
                        thread.length = (!full).then(|| (2.0 * thread.size).round());
                        changed = true;
                    }
                    if let Some(len) = &mut thread.length {
                        ui.horizontal(|ui| {
                            ui.label("Thread depth");
                            let r = ui.add(egui::DragValue::new(len).speed(0.5).range(0.5..=500.0).suffix(" mm"));
                            r.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, "Wizard Thread Depth")
                            });
                            if r.changed() {
                                changed = true;
                            }
                        });
                    }
                }
            }
        });

    ui.add_space(4.0);

    // -----------------------------------------------------------------------
    // 6. Tolerance / Precision callout values
    // -----------------------------------------------------------------------
    let has_tols = params.wizard.as_ref().is_some_and(|w| !w.tolerances.is_empty());
    egui::CollapsingHeader::new(RichText::new("Tolerance / Precision").family(theme::semibold()))
        .id_salt("hw_tolerance")
        .default_open(has_tols)
        .show(ui, |ui| {
            let all_params = [
                CalloutParam::Diameter,
                CalloutParam::Depth,
                CalloutParam::CounterboreDiameter,
                CalloutParam::CounterboreDepth,
                CalloutParam::CountersinkDiameter,
                CalloutParam::CountersinkAngle,
                CalloutParam::ThreadDepth,
                CalloutParam::SlotLength,
            ];
            egui::ComboBox::from_id_salt("hw_tol_param")
                .width(ui.available_width())
                .selected_text(params.selected_callout_param.label())
                .show_ui(ui, |ui| {
                    for p in all_params {
                        ui.selectable_value(&mut params.selected_callout_param, p, p.label());
                    }
                });

            let target_param = params.selected_callout_param;
            let mut entry = params
                .wizard
                .as_ref()
                .and_then(|w| w.tolerance_for(target_param).cloned())
                .unwrap_or(CalloutTolerance { param: target_param, mode: ToleranceMode::None, precision: 2 });

            let mut entry_changed = false;
            let mode_idx = match &entry.mode {
                ToleranceMode::None => 0,
                ToleranceMode::Bilateral { .. } => 1,
                ToleranceMode::Symmetric { .. } => 2,
                ToleranceMode::Limit { .. } => 3,
                ToleranceMode::Fit { .. } => 4,
            };
            let mut chosen_mode = mode_idx;
            let mode_labels = ["None", "Bilateral", "Symmetric", "Limit", "Fit"];
            egui::ComboBox::from_id_salt("hw_tol_mode")
                .width(ui.available_width())
                .selected_text(mode_labels[chosen_mode])
                .show_ui(ui, |ui| {
                    for (i, l) in mode_labels.iter().enumerate() {
                        ui.selectable_value(&mut chosen_mode, i, *l);
                    }
                });
            if chosen_mode != mode_idx {
                entry.mode = match chosen_mode {
                    1 => ToleranceMode::Bilateral { upper: 0.1, lower: -0.05 },
                    2 => ToleranceMode::Symmetric { plus_minus: 0.05 },
                    3 => ToleranceMode::Limit { upper: 0.1, lower: -0.05 },
                    4 => ToleranceMode::Fit { class: "H13".into() },
                    _ => ToleranceMode::None,
                };
                entry_changed = true;
            }

            match &mut entry.mode {
                ToleranceMode::None => {}
                ToleranceMode::Bilateral { upper, lower } | ToleranceMode::Limit { upper, lower } => {
                    ui.horizontal(|ui| {
                        ui.label("Upper");
                        entry_changed |= ui.add(egui::DragValue::new(upper).speed(0.01).fixed_decimals(3)).changed();
                        ui.label("Lower");
                        entry_changed |= ui.add(egui::DragValue::new(lower).speed(0.01).fixed_decimals(3)).changed();
                    });
                }
                ToleranceMode::Symmetric { plus_minus } => {
                    ui.horizontal(|ui| {
                        ui.label("±");
                        entry_changed |= ui
                            .add(egui::DragValue::new(plus_minus).speed(0.01).range(0.0..=50.0).fixed_decimals(3))
                            .changed();
                    });
                }
                ToleranceMode::Fit { class } => {
                    ui.horizontal(|ui| {
                        ui.label("Fit class");
                        entry_changed |= ui.add(egui::TextEdit::singleline(class).desired_width(80.0)).changed();
                    });
                }
            }

            ui.horizontal(|ui| {
                ui.label("Decimals");
                entry_changed |= ui.add(egui::DragValue::new(&mut entry.precision).range(0..=4)).changed();
            });

            if entry_changed {
                if matches!(entry.mode, ToleranceMode::None) && entry.precision == 2 {
                    params.ensure_wizard().clear_tolerance(target_param);
                } else {
                    params.ensure_wizard().set_tolerance(entry);
                }
                changed = true;
            }

            // Live drawing callout preview.
            if let Ok(model_hole) = params.to_preview_model() {
                let symbols = &soliddraft_drawing::library::template("iso-a3").style_file().symbols;
                let count = if model_hole.points.len() > 1 {
                    format!("{}{} ", model_hole.points.len(), symbols.times)
                } else {
                    String::new()
                };
                let preview = crate::drawing::part::format_hole_callout(&model_hole, &count, symbols);
                ui.add_space(4.0);
                ui.label(RichText::new("Callout preview").size(10.5).color(theme::muted()));
                egui::Frame::new()
                    .fill(theme::field_bg())
                    .corner_radius(CornerRadius::same(4))
                    .inner_margin(egui::Margin::symmetric(6, 4))
                    .show(ui, |ui| {
                        ui.label(RichText::new(preview).font(theme::mono(11.0)));
                    });
            }
        });

    changed
}

fn csk_fields(ui: &mut egui::Ui, salt: &str, csk: &mut CountersinkOption) -> bool {
    let mut changed = false;
    ui.push_id(salt, |ui| {
        ui.horizontal(|ui| {
            ui.label("Ø");
            changed |= ui.add(egui::DragValue::new(&mut csk.diameter).speed(0.1).range(0.2..=600.0).suffix(" mm")).changed();
            ui.label("∠");
            changed |= ui.add(egui::DragValue::new(&mut csk.angle_deg).speed(1.0).range(10.0..=175.0).suffix("°")).changed();
        });
    });
    changed
}

fn show_positions_tab(
    ui: &mut egui::Ui,
    params: &mut HoleParams,
    is_inline_sketch: bool,
    actions: &mut Vec<AppAction>,
    note: &mut Option<(String, NoteKind)>,
) -> bool {
    let mut changed = false;

    // 1. Placement face section.
    ui.label(RichText::new("Placement face").family(theme::semibold()).size(11.5));
    match &params.face {
        Some(f) => {
            ui.label(RichText::new(&f.label).color(theme::accent()));
            ui.label(
                RichText::new("Click anywhere on the face in the viewport to place holes, or edit the position sketch.")
                    .size(10.5)
                    .color(theme::muted()),
            );
        }
        None => {
            ui.label(
                RichText::new("Click a planar face in the viewport to place holes (non-planar faces are refused).")
                    .size(11.0)
                    .color(theme::warn()),
            );
        }
    }

    ui.add_space(6.0);

    // 2. Live inline position sketch editor (better than SolidWorks: full sketch tools inside the feature!).
    egui::CollapsingHeader::new(RichText::new("Position Sketch").family(theme::semibold()))
        .id_salt("hw_pos_sketch")
        .default_open(true)
        .show(ui, |ui| {
            let status_text = params
                .position_sketch
                .as_ref()
                .map(|sk| match &sk.status {
                    crate::sketch::SolveStatus::Empty => "Empty",
                    crate::sketch::SolveStatus::UnderDefined => "Under defined",
                    crate::sketch::SolveStatus::FullyDefined => "Fully defined",
                    crate::sketch::SolveStatus::OverDefined { .. } => "Over defined",
                    crate::sketch::SolveStatus::Failed(_) => "Solver failed",
                })
                .unwrap_or("Ready");
            ui.label(RichText::new(format!("Sketch status: {status_text}")).size(11.0).color(theme::muted()));

            let toggle_label = if is_inline_sketch {
                "Exit inline sketch tools"
            } else {
                "Edit position sketch (dimensions, relations, patterns)"
            };
            let btn = ui.add_enabled(params.face.is_some(), egui::Button::new(toggle_label));
            btn.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Toggle inline position sketch")
            });
            if btn.clicked() {
                params.ensure_position_sketch();
                if is_inline_sketch {
                    actions.push(AppAction::CloseHolePositionSketch);
                } else {
                    actions.push(AppAction::OpenHolePositionSketch);
                }
            }

            if is_inline_sketch {
                ui.add_space(4.0);
                ui.label(RichText::new("Sketch tools (live preview active)").size(10.5).color(theme::muted()));
                ui.horizontal_wrapped(|ui| {
                    for (label, tool) in [
                        ("Point", Tool::Point),
                        ("Select", Tool::Select),
                        ("Dimension", Tool::Dimension),
                        ("Centreline", Tool::Centreline),
                        ("Circle", Tool::Circle),
                        ("Linear Pat", Tool::LinearPattern),
                        ("Circular Pat", Tool::CircularPattern),
                    ] {
                        if ui.small_button(label).clicked() {
                            actions.push(AppAction::SketchTool(tool));
                        }
                    }
                });
            }
        });

    ui.add_space(6.0);

    // 3. Quick editable linear & circular point patterns (real sketch PatternDefinitions).
    egui::CollapsingHeader::new(RichText::new("Point Patterns").family(theme::semibold()))
        .id_salt("hw_pos_patterns")
        .default_open(false)
        .show(ui, |ui| {
            ui.label(RichText::new("Linear pattern (from last point)").size(11.0).color(theme::muted()));
            ui.horizontal(|ui| {
                ui.label("X");
                ui.add(egui::DragValue::new(&mut params.lin_pat.count_x).range(1..=50));
                ui.label("×");
                ui.add(egui::DragValue::new(&mut params.lin_pat.spacing_x).speed(1.0).suffix(" mm"));
            });
            ui.horizontal(|ui| {
                ui.label("Y");
                ui.add(egui::DragValue::new(&mut params.lin_pat.count_y).range(1..=50));
                ui.label("×");
                ui.add(egui::DragValue::new(&mut params.lin_pat.spacing_y).speed(1.0).suffix(" mm"));
            });
            let lin_btn = ui.button("Apply linear pattern");
            lin_btn.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Apply linear hole pattern")
            });
            if lin_btn.clicked() {
                match params.apply_linear_point_pattern() {
                    Ok(()) => {
                        changed = true;
                        actions.push(AppAction::SyncHolePositionSketch);
                    }
                    Err(e) => *note = Some((e, NoteKind::Error)),
                }
            }

            ui.separator();
            ui.label(RichText::new("Circular bolt circle").size(11.0).color(theme::muted()));
            ui.horizontal(|ui| {
                ui.label("Center U");
                ui.add(egui::DragValue::new(&mut params.circ_pat.center_u).speed(1.0).suffix(" mm"));
                ui.label("V");
                ui.add(egui::DragValue::new(&mut params.circ_pat.center_v).speed(1.0).suffix(" mm"));
            });
            ui.horizontal(|ui| {
                ui.label("R");
                ui.add(egui::DragValue::new(&mut params.circ_pat.radius).speed(0.5).range(0.5..=2000.0).suffix(" mm"));
                ui.label("N");
                ui.add(egui::DragValue::new(&mut params.circ_pat.count).range(2..=100));
            });
            let circ_btn = ui.button("Apply circular pattern");
            circ_btn.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Apply circular hole pattern")
            });
            if circ_btn.clicked() {
                match params.apply_circular_point_pattern() {
                    Ok(()) => {
                        changed = true;
                        actions.push(AppAction::SyncHolePositionSketch);
                    }
                    Err(e) => *note = Some((e, NoteKind::Error)),
                }
            }
        });

    ui.add_space(6.0);

    // 4. Direct point coordinates list.
    egui::CollapsingHeader::new(RichText::new(format!("Hole Points ({})", params.points.len())).family(theme::semibold()))
        .id_salt("hw_pos_list")
        .default_open(true)
        .show(ui, |ui| {
            let mut remove = None;
            let mut edits: Vec<(usize, crate::sketch::P2)> = Vec::new();
            for (i, pt) in params.points.iter().enumerate() {
                let mut u = pt.u;
                let mut v = pt.v;
                ui.horizontal(|ui| {
                    ui.label(format!("#{}", i + 1));
                    let ru = ui.add(egui::DragValue::new(&mut u).speed(0.5).prefix("u ").suffix(" mm"));
                    let rv = ui.add(egui::DragValue::new(&mut v).speed(0.5).prefix("v ").suffix(" mm"));
                    if ru.changed() || rv.changed() {
                        edits.push((i, crate::sketch::P2::new(u, v)));
                    }
                    if ui.add_enabled(params.points.len() > 1, egui::Button::new("×")).clicked() {
                        remove = Some(i);
                    }
                });
            }
            for (i, p) in edits {
                params.set_position_point(i, p);
                changed = true;
                actions.push(AppAction::SyncHolePositionSketch);
            }
            if let Some(i) = remove {
                params.remove_position_point(i);
                changed = true;
                actions.push(AppAction::SyncHolePositionSketch);
            }
            let add_btn = ui.button("+ Add hole point");
            add_btn.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Add hole point"));
            if add_btn.clicked() {
                let next = params.points.last().copied().unwrap_or_default();
                params.add_position_point(crate::sketch::P2::new(next.u + 15.0, next.v));
                changed = true;
                actions.push(AppAction::SyncHolePositionSketch);
            }
            if !params.points.is_empty() {
                ui.label(
                    RichText::new(format!("Hole Ø{} mm", fmt_mm(params.diameter))).size(10.5).color(theme::muted()),
                );
            }
        });

    changed
}
