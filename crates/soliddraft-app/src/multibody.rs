//! Body selection and the Combine dialog.

use crate::document::{Document, FeatureId};
use crate::state::AppState;
use soliddraft_kernel_api::BooleanOp;

pub fn operation_name(operation: BooleanOp) -> &'static str {
    match operation {
        BooleanOp::Union => "Union",
        BooleanOp::Subtract => "Subtract",
        BooleanOp::Intersect => "Intersect",
    }
}

pub struct CombineForm {
    pub(crate) editing: Option<FeatureId>,
    target: u32,
    tool: u32,
    operation: BooleanOp,
    pub(crate) error: Option<String>,
}

impl CombineForm {
    pub fn editing(id: FeatureId, target: u32, tool: u32, operation: BooleanOp) -> Self {
        Self { editing: Some(id), target, tool, operation, error: None }
    }

    pub fn new(doc: &Document) -> Option<Self> {
        let bodies = doc.model().live_bodies();
        let target = bodies.iter().find(|(key, _)| *key == doc.active_body).or(bodies.first())?.0;
        let tool = bodies.iter().find(|(key, _)| *key != target)?.0;
        Some(Self { editing: None, target, tool, operation: BooleanOp::Union, error: None })
    }
}

pub fn show(ctx: &egui::Context, state: &mut AppState) {
    let Some(mut form) = state.doc.combine_form.take() else { return };
    let bodies = match form.editing {
        Some(id) => state.doc.model().bodies_before(soliddraft_model::FeatureId(id as u32)),
        None => state.doc.model().live_bodies(),
    };
    let mut open = true;
    let mut apply = false;
    egui::Window::new("Combine bodies").collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
        ui.label("The result replaces the target and consumes the tool body.");
        for (label, selected) in [("Target body", &mut form.target), ("Tool body", &mut form.tool)] {
            egui::ComboBox::from_label(label).selected_text(format!("Body {}", *selected + 1)).show_ui(ui, |ui| {
                for &(key, _) in &bodies {
                    ui.add_enabled_ui(form.editing.is_none() || label != "Target body", |ui| {
                        ui.selectable_value(selected, key, format!("Body {}", key + 1));
                    });
                }
            });
        }
        egui::ComboBox::from_label("Operation").selected_text(operation_name(form.operation)).show_ui(ui, |ui| {
            for operation in [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect] {
                ui.selectable_value(&mut form.operation, operation, operation_name(operation));
            }
        });
        if let Some(error) = &form.error {
            ui.label(error);
        }
        apply = ui.button("Combine").clicked();
    });
    if apply {
        let result = if let Some(id) = form.editing {
            state.doc.edit_combine(id, form.tool, form.operation)
        } else {
            state.doc.combine(form.target, form.tool, form.operation)
        };
        match result {
            Ok(_) => open = false,
            Err(error) => form.error = Some(error),
        }
    }
    if open {
        state.doc.combine_form = Some(form);
    }
}
