//! Ordered sketch and face selection for the loft property page.

use soliddraft_kernel_api::LoftShape;
use soliddraft_model::{LoftFeature, LoftProfile};

pub(crate) fn page(ui: &mut egui::Ui, loft: &mut LoftFeature, choices: &[(LoftProfile, String)]) {
    ui.label("Add at least two profiles in the order the loft follows.");
    let mut remove = None;
    let mut swap = None;
    for (index, source) in loft.profiles.iter().enumerate() {
        let label =
            choices.iter().find(|(p, _)| p == source).map(|(_, label)| label.as_str()).unwrap_or_else(
                || match source {
                    LoftProfile::Sketch { .. } => "Missing sketch",
                    LoftProfile::Face { face } => &face.label,
                },
            );
        ui.push_id(index, |ui| {
            ui.label(format!("{}. {label}", index + 1));
            ui.horizontal(|ui| {
                if ui.add_enabled(index > 0, egui::Button::new("Up")).on_hover_text("Move profile earlier").clicked() {
                    swap = Some((index, index - 1));
                }
                if ui
                    .add_enabled(index + 1 < loft.profiles.len(), egui::Button::new("Down"))
                    .on_hover_text("Move profile later")
                    .clicked()
                {
                    swap = Some((index, index + 1));
                }
                if ui.small_button("Remove").clicked() {
                    remove = Some(index);
                }
            });
        });
    }
    if let Some(index) = remove {
        loft.profiles.remove(index);
    }
    if let Some((a, b)) = swap {
        loft.profiles.swap(a, b);
    }
    ui.menu_button("Add profile", |ui| {
        for (source, label) in choices {
            if !loft.profiles.contains(source) && ui.button(label).clicked() {
                loft.profiles.push(source.clone());
                ui.close();
            }
        }
    });
    ui.add_space(8.0);
    crate::theme::section_label(ui, "Sides");
    ui.radio_value(&mut loft.shape, LoftShape::Ruled, "Ruled");
    ui.radio_value(&mut loft.shape, LoftShape::Smooth, "Smooth");
}
