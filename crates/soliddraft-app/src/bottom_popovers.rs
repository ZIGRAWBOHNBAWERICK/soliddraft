//! Zoo-model (`27.png`–`29.png`) bottom bar right-side popovers:
//! 1. Physical analysis (mass, volume, surface area, centre of mass, unit pickers, density)
//! 2. Selection filter (All, Faces, Edges & vertices, Bodies)
//! 3. Default units for the current file (in, ft, yd, mm, cm, m)

use crate::icons::{self, Icon};
use crate::state::{AppState, DisplayUnit};
use crate::theme;
use egui::{Align2, CornerRadius, Rect, RectAlign, Sense, Stroke, StrokeKind, pos2, vec2};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LengthUnit {
    #[default]
    Mm,
    Cm,
    M,
    In,
    Ft,
}

impl LengthUnit {
    pub const ALL: [Self; 5] = [Self::Mm, Self::Cm, Self::M, Self::In, Self::Ft];

    pub fn label(self) -> &'static str {
        match self {
            Self::Mm => "mm",
            Self::Cm => "cm",
            Self::M => "m",
            Self::In => "in",
            Self::Ft => "ft",
        }
    }

    pub fn from_mm(self, mm: f64) -> f64 {
        match self {
            Self::Mm => mm,
            Self::Cm => mm / 10.0,
            Self::M => mm / 1000.0,
            Self::In => mm / 25.4,
            Self::Ft => mm / 304.8,
        }
    }

    pub fn from_display(unit: DisplayUnit) -> Self {
        match unit {
            DisplayUnit::Mm => Self::Mm,
            DisplayUnit::Cm => Self::Cm,
            DisplayUnit::M => Self::M,
            DisplayUnit::In => Self::In,
            DisplayUnit::Ft | DisplayUnit::Yd => Self::Ft,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AreaUnit {
    #[default]
    Mm2,
    Cm2,
    M2,
    In2,
    Ft2,
}

impl AreaUnit {
    pub const ALL: [Self; 5] = [Self::Mm2, Self::Cm2, Self::M2, Self::In2, Self::Ft2];

    pub fn label(self) -> &'static str {
        match self {
            Self::Mm2 => "mm²",
            Self::Cm2 => "cm²",
            Self::M2 => "m²",
            Self::In2 => "in²",
            Self::Ft2 => "ft²",
        }
    }

    pub fn from_mm2(self, mm2: f64) -> f64 {
        match self {
            Self::Mm2 => mm2,
            Self::Cm2 => mm2 / 100.0,
            Self::M2 => mm2 / 1_000_000.0,
            Self::In2 => mm2 / (25.4 * 25.4),
            Self::Ft2 => mm2 / (304.8 * 304.8),
        }
    }

    pub fn from_display(unit: DisplayUnit) -> Self {
        match unit {
            DisplayUnit::Mm => Self::Mm2,
            DisplayUnit::Cm => Self::Cm2,
            DisplayUnit::M => Self::M2,
            DisplayUnit::In => Self::In2,
            DisplayUnit::Ft | DisplayUnit::Yd => Self::Ft2,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VolumeUnit {
    #[default]
    Mm3,
    Cm3,
    M3,
    In3,
    Ft3,
}

impl VolumeUnit {
    pub const ALL: [Self; 5] = [Self::Mm3, Self::Cm3, Self::M3, Self::In3, Self::Ft3];

    pub fn label(self) -> &'static str {
        match self {
            Self::Mm3 => "mm³",
            Self::Cm3 => "cm³",
            Self::M3 => "m³",
            Self::In3 => "in³",
            Self::Ft3 => "ft³",
        }
    }

    pub fn from_mm3(self, mm3: f64) -> f64 {
        match self {
            Self::Mm3 => mm3,
            Self::Cm3 => mm3 / 1_000.0,
            Self::M3 => mm3 / 1_000_000_000.0,
            Self::In3 => mm3 / (25.4 * 25.4 * 25.4),
            Self::Ft3 => mm3 / (304.8 * 304.8 * 304.8),
        }
    }

    pub fn from_display(unit: DisplayUnit) -> Self {
        match unit {
            DisplayUnit::Mm => Self::Mm3,
            DisplayUnit::Cm => Self::Cm3,
            DisplayUnit::M => Self::M3,
            DisplayUnit::In => Self::In3,
            DisplayUnit::Ft | DisplayUnit::Yd => Self::Ft3,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MassUnit {
    #[default]
    G,
    Kg,
    Mg,
    Lb,
    Oz,
}

impl MassUnit {
    pub const ALL: [Self; 5] = [Self::G, Self::Kg, Self::Mg, Self::Lb, Self::Oz];

    pub fn label(self) -> &'static str {
        match self {
            Self::G => "g",
            Self::Kg => "kg",
            Self::Mg => "mg",
            Self::Lb => "lb",
            Self::Oz => "oz",
        }
    }

    pub fn from_kg(self, kg: f64) -> f64 {
        match self {
            Self::Kg => kg,
            Self::G => kg * 1000.0,
            Self::Mg => kg * 1_000_000.0,
            Self::Lb => kg * 2.204_622_621_848_775_7,
            Self::Oz => kg * 35.273_961_949_580_41,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DensityUnit {
    #[default]
    KgM3,
    GCm3,
    LbIn3,
}

impl DensityUnit {
    pub const ALL: [Self; 3] = [Self::KgM3, Self::GCm3, Self::LbIn3];

    pub fn label(self) -> &'static str {
        match self {
            Self::KgM3 => "kg/m³",
            Self::GCm3 => "g/cm³",
            Self::LbIn3 => "lb/in³",
        }
    }

    /// Converts a density value in `self` to `kg/m³`.
    pub fn to_kg_m3(self, value: f64) -> f64 {
        match self {
            Self::KgM3 => value,
            Self::GCm3 => value * 1000.0,
            Self::LbIn3 => value * 27_679.904_710_191,
        }
    }

    /// Converts a density value in `kg/m³` into `self`.
    pub fn from_kg_m3(self, kg_m3: f64) -> f64 {
        match self {
            Self::KgM3 => kg_m3,
            Self::GCm3 => kg_m3 / 1000.0,
            Self::LbIn3 => kg_m3 / 27_679.904_710_191,
        }
    }
}

/// Persistent session state for the Physical Analysis popover (`28.png`).
#[derive(Clone, Copy, Debug)]
pub struct PhysicalState {
    pub length: LengthUnit,
    pub area: AreaUnit,
    pub volume: VolumeUnit,
    pub mass: MassUnit,
    pub density: f64,
    pub density_unit: DensityUnit,
    pub user_overrode_density: bool,
    pub requested_body: Option<u64>,
}

impl Default for PhysicalState {
    fn default() -> Self {
        Self {
            length: LengthUnit::Mm,
            area: AreaUnit::Mm2,
            volume: VolumeUnit::Mm3,
            mass: MassUnit::G,
            density: 7850.0,
            density_unit: DensityUnit::KgM3,
            user_overrode_density: false,
            requested_body: None,
        }
    }
}

/// Computes mass (in `mass_unit`) from volume in `mm³` and density in `density_unit`.
pub fn compute_mass(volume_mm3: f64, density: f64, density_unit: DensityUnit, mass_unit: MassUnit) -> f64 {
    let volume_m3 = volume_mm3 * 1e-9;
    let density_kg_m3 = density_unit.to_kg_m3(density);
    mass_unit.from_kg(volume_m3 * density_kg_m3)
}

/// Renders the three Zoo-model right-side bottom bar controls:
/// Default units (`29.png`), Selection filter (`27.png`), Physical analysis (`28.png`).
/// Called inside a `right_to_left` layout so the leftmost call draws rightmost.
pub fn show_right_controls(ui: &mut egui::Ui, state: &mut AppState) {
    units_control(ui, state);
    selection_filter_control(ui, state);
    physical_analysis_control(ui, state);
}

fn bar_pill(
    ui: &mut egui::Ui,
    icon: Option<Icon>,
    text: &str,
    accessible: &str,
    active: bool,
    caret: bool,
) -> egui::Response {
    let font = theme::font_medium(11.5);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, egui::Color32::PLACEHOLDER);
    let icon_pad = if icon.is_some() { 18.0 } else { 0.0 };
    let text_pad = if text.is_empty() { 0.0 } else { galley.size().x + 4.0 };
    let caret_pad = if caret { 12.0 } else { 0.0 };
    let width = (12.0 + icon_pad + text_pad + caret_pad).max(28.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 22.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, accessible));
    let (fill, stroke, ink) = if active {
        (theme::accent_soft(), theme::accent(), theme::accent())
    } else if resp.hovered() {
        (theme::hover(), theme::line_strong(), theme::ink())
    } else {
        (theme::chip(), theme::line(), theme::ink())
    };
    ui.painter().rect(rect, CornerRadius::same(theme::RADIUS), fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
    let mut x = rect.left() + 7.0;
    if let Some(ic) = icon {
        let ir = if text.is_empty() && !caret {
            Rect::from_center_size(rect.center(), vec2(14.0, 14.0))
        } else {
            Rect::from_center_size(pos2(x + 7.0, rect.center().y), vec2(13.0, 13.0))
        };
        icons::paint(ui.painter(), ic, ir, ink);
        x += 17.0;
    }
    if !text.is_empty() {
        ui.painter().galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, ink);
    }
    if caret {
        let cx = rect.right() - 9.0;
        let cy = rect.center().y;
        let s = Stroke::new(1.3, theme::muted());
        ui.painter().line_segment([pos2(cx - 3.0, cy + 1.5), pos2(cx, cy - 1.5)], s);
        ui.painter().line_segment([pos2(cx, cy - 1.5), pos2(cx + 3.0, cy + 1.5)], s);
    }
    resp
}

fn physical_analysis_control(ui: &mut egui::Ui, state: &mut AppState) {
    let id = egui::Id::new("d1_physical_analysis_state");
    let mut phys = ui.data(|d| d.get_temp::<PhysicalState>(id).unwrap_or_default());

    if !phys.user_overrode_density
        && let Some(density) = state.doc.material().and_then(|m| m.density_kg_m3)
        && density > 0.0
    {
        phys.density = phys.density_unit.from_kg_m3(density);
    }

    let resp = bar_pill(ui, Some(Icon::Mass), "", "Physical analysis", false, false)
        .on_hover_text("Physical analysis (mass, volume, area, centre of mass)");

    egui::Popup::menu(&resp)
        .align(RectAlign::TOP_END)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(330.0);
            ui.horizontal(|ui| {
                let (ir, _) = ui.allocate_exact_size(vec2(15.0, 15.0), Sense::hover());
                icons::paint(ui.painter(), Icon::Mass, ir, theme::accent());
                ui.label(egui::RichText::new("Physical analysis").family(theme::semibold()).size(13.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("× Close").clicked() {
                        ui.close();
                    }
                });
            });
            ui.separator();

            egui::Grid::new("physical_analysis_units").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("Length").size(11.0).color(theme::muted()));
                    egui::ComboBox::from_id_salt("pa_length").width(140.0).selected_text(phys.length.label()).show_ui(
                        ui,
                        |ui| {
                            for u in LengthUnit::ALL {
                                ui.selectable_value(&mut phys.length, u, u.label());
                            }
                        },
                    );
                });
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("Area").size(11.0).color(theme::muted()));
                    egui::ComboBox::from_id_salt("pa_area").width(140.0).selected_text(phys.area.label()).show_ui(
                        ui,
                        |ui| {
                            for u in AreaUnit::ALL {
                                ui.selectable_value(&mut phys.area, u, u.label());
                            }
                        },
                    );
                });
                ui.end_row();

                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("Volume").size(11.0).color(theme::muted()));
                    egui::ComboBox::from_id_salt("pa_volume").width(140.0).selected_text(phys.volume.label()).show_ui(
                        ui,
                        |ui| {
                            for u in VolumeUnit::ALL {
                                ui.selectable_value(&mut phys.volume, u, u.label());
                            }
                        },
                    );
                });
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("Mass").size(11.0).color(theme::muted()));
                    egui::ComboBox::from_id_salt("pa_mass").width(140.0).selected_text(phys.mass.label()).show_ui(
                        ui,
                        |ui| {
                            for u in MassUnit::ALL {
                                ui.selectable_value(&mut phys.mass, u, u.label());
                            }
                        },
                    );
                });
                ui.end_row();

                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("Material density").size(11.0).color(theme::muted()));
                    let dv = ui.add(
                        egui::DragValue::new(&mut phys.density).range(0.0001..=100_000.0).speed(10.0).max_decimals(4),
                    );
                    if dv.changed() {
                        phys.user_overrode_density = true;
                    }
                });
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("Density unit").size(11.0).color(theme::muted()));
                    let prev_unit = phys.density_unit;
                    egui::ComboBox::from_id_salt("pa_density_unit")
                        .width(140.0)
                        .selected_text(phys.density_unit.label())
                        .show_ui(ui, |ui| {
                            for u in DensityUnit::ALL {
                                ui.selectable_value(&mut phys.density_unit, u, u.label());
                            }
                        });
                    if phys.density_unit != prev_unit {
                        let kg_m3 = prev_unit.to_kg_m3(phys.density);
                        phys.density = phys.density_unit.from_kg_m3(kg_m3);
                    }
                });
                ui.end_row();
            });

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);

            if let Some(body) = state.doc.display_body() {
                let rev = state.doc.revision();
                if let Some(mp) = crate::measure::mass::cached(&state.doc, body) {
                    let mass = compute_mass(mp.volume, phys.density, phys.density_unit, phys.mass);
                    let volume = phys.volume.from_mm3(mp.volume);
                    let area = phys.area.from_mm2(mp.area);
                    let cx = phys.length.from_mm(mp.centroid.x);
                    let cy = phys.length.from_mm(mp.centroid.y);
                    let cz = phys.length.from_mm(mp.centroid.z);
                    egui::Grid::new("pa_results").num_columns(2).spacing([14.0, 5.0]).show(ui, |ui| {
                        ui.label(egui::RichText::new("Mass").color(theme::muted()));
                        ui.label(
                            egui::RichText::new(format!("{mass:.3} {}", phys.mass.label()))
                                .font(theme::mono(12.0))
                                .color(theme::ink()),
                        );
                        ui.end_row();

                        ui.label(egui::RichText::new("Volume").color(theme::muted()));
                        ui.label(
                            egui::RichText::new(format!("{volume:.3} {}", phys.volume.label()))
                                .font(theme::mono(12.0))
                                .color(theme::ink()),
                        );
                        ui.end_row();

                        ui.label(egui::RichText::new("Surface area").color(theme::muted()));
                        ui.label(
                            egui::RichText::new(format!("{area:.3} {}", phys.area.label()))
                                .font(theme::mono(12.0))
                                .color(theme::ink()),
                        );
                        ui.end_row();

                        ui.label(egui::RichText::new("Centre of mass").color(theme::muted()));
                        ui.label(
                            egui::RichText::new(format!("({cx:.2}, {cy:.2}, {cz:.2}) {}", phys.length.label()))
                                .font(theme::mono(11.5))
                                .color(theme::ink()),
                        );
                        ui.end_row();
                    });
                } else {
                    let target_feature = state.doc.features.iter().rev().find(|f| f.body == Some(body)).map(|f| f.id);
                    if phys.requested_body != Some(rev)
                        && state.doc.proof_state().current_proved_display()
                        && let Some(feat_id) = target_feature
                    {
                        phys.requested_body = Some(rev);
                        let _ = state.mass.request(&state.doc, feat_id, ui.ctx());
                    }
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(egui::RichText::new("Computing exact physical properties…").color(theme::muted()));
                    });
                    if let Some(feat_id) = target_feature
                        && ui.small_button("Compute now").clicked()
                    {
                        let _ = state.mass.request(&state.doc, feat_id, ui.ctx());
                    }
                }
            } else {
                ui.label(egui::RichText::new("No solid body in the current part.").color(theme::muted()));
            }
        });

    ui.data_mut(|d| d.insert_temp(id, phys));
}

fn filter_summary(f: crate::viewport::PickFilter) -> &'static str {
    match (f.faces, f.edges, f.bodies) {
        (true, true, true) => "All",
        (true, false, false) => "Faces",
        (false, true, false) => "Edges",
        (false, false, true) => "Bodies",
        (true, true, false) => "Faces, Edges",
        (true, false, true) => "Faces, Bodies",
        (false, true, true) => "Edges, Bodies",
        (false, false, false) => "None",
    }
}

fn selection_filter_control(ui: &mut egui::Ui, state: &mut AppState) {
    let summary = filter_summary(state.pick_filter);
    let custom = state.pick_filter != crate::viewport::PickFilter::default();
    let resp = bar_pill(ui, Some(Icon::Filter), summary, "Selection filter", custom, true)
        .on_hover_text("Selection filter (faces, edges, bodies)");

    egui::Popup::menu(&resp)
        .align(RectAlign::TOP_END)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(175.0);
            ui.label(egui::RichText::new("Selection filter").family(theme::semibold()).size(12.0));
            ui.separator();
            let mut all = state.pick_filter.faces && state.pick_filter.edges && state.pick_filter.bodies;
            if ui.checkbox(&mut all, "All").changed() {
                state.pick_filter.faces = all;
                state.pick_filter.edges = all;
                state.pick_filter.bodies = all;
            }
            ui.separator();
            ui.checkbox(&mut state.pick_filter.faces, "Faces");
            ui.checkbox(&mut state.pick_filter.edges, "Edges & vertices");
            ui.checkbox(&mut state.pick_filter.bodies, "Bodies");
        });
}

fn units_control(ui: &mut egui::Ui, state: &mut AppState) {
    let short = state.display_unit.short();
    let resp = bar_pill(ui, None, short, "Default units", false, true).on_hover_text("Default units for current file");

    egui::Popup::menu(&resp)
        .align(RectAlign::TOP_END)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let row_w = 196.0;
            ui.set_width(row_w);
            ui.label(egui::RichText::new("Default units for current file").size(11.0).color(theme::muted()));
            ui.separator();
            for unit in DisplayUnit::ALL {
                let is_current = state.display_unit == unit;
                let (rect, item) = ui.allocate_exact_size(vec2(row_w, 24.0), Sense::click());
                item.widget_info(|| {
                    egui::WidgetInfo::selected(egui::WidgetType::Button, true, is_current, unit.name())
                });
                if is_current {
                    ui.painter().rect_filled(rect, CornerRadius::same(4), theme::accent_soft());
                } else if item.hovered() {
                    ui.painter().rect_filled(rect, CornerRadius::same(4), theme::hover());
                }
                ui.painter().text(
                    pos2(rect.left() + 8.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    unit.name(),
                    if is_current { theme::font_semibold(12.0) } else { theme::font(12.0) },
                    if is_current { theme::accent() } else { theme::ink() },
                );
                if is_current {
                    ui.painter().text(
                        pos2(rect.right() - 8.0, rect.center().y),
                        Align2::RIGHT_CENTER,
                        "current",
                        theme::font(11.0),
                        theme::muted(),
                    );
                }
                if item.clicked() {
                    state.display_unit = unit;
                    // Sync Physical Analysis default length/area/volume units to match.
                    let id = egui::Id::new("d1_physical_analysis_state");
                    ui.data_mut(|d| {
                        let mut phys = d.get_temp::<PhysicalState>(id).unwrap_or_default();
                        phys.length = LengthUnit::from_display(unit);
                        phys.area = AreaUnit::from_display(unit);
                        phys.volume = VolumeUnit::from_display(unit);
                        d.insert_temp(id, phys);
                    });
                    ui.close();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_analysis_unit_conversions_are_exact() {
        // 1000 mm³ = 1 cm³ = 1e-6 m³; at 7850 kg/m³ (steel), mass = 7.85 g = 0.00785 kg.
        let mass_g = compute_mass(1000.0, 7850.0, DensityUnit::KgM3, MassUnit::G);
        assert!((mass_g - 7.85).abs() < 1e-9);
        let mass_kg = compute_mass(1000.0, 7.85, DensityUnit::GCm3, MassUnit::Kg);
        assert!((mass_kg - 0.00785).abs() < 1e-9);
        assert!((LengthUnit::In.from_mm(25.4) - 1.0).abs() < 1e-12);
        assert!((AreaUnit::In2.from_mm2(645.16) - 1.0).abs() < 1e-9);
        assert!((VolumeUnit::Cm3.from_mm3(1000.0) - 1.0).abs() < 1e-12);
        assert!((DensityUnit::GCm3.to_kg_m3(2.7) - 2700.0).abs() < 1e-9);
    }
}
