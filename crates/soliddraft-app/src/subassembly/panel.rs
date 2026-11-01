//! The Form subassembly panel: the parts chosen in the tree, in the order they were chosen,
//! the new file's name, and Apply. Apply runs the same command as the MCP tool.

use crate::assembly::AssemblyAction;
use crate::state::{AppAction, AppState, NoteKind};
use crate::theme;
use soliddraft_assembly::{FormRequest, OccurrencePath};
use soliddraft_document::DocumentId;
use std::path::PathBuf;

/// The panel's fields.
#[derive(Default)]
pub struct Draft {
    pub open: bool,
    /// The assembly the panel was opened on: Apply forms nothing in another.
    pub document: Option<DocumentId>,
    /// The parts to form, in order; the first gives the new file its frame.
    pub paths: Vec<OccurrencePath>,
    /// The new file, relative to the open assembly's folder unless absolute.
    pub file: String,
    /// The last refusal, shown until something works.
    pub error: Option<String>,
}

/// What the panel asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum FormAction {
    /// Opens the panel on the parts chosen in the tree.
    Open,
    /// Forms them into the named file.
    Apply,
    Cancel,
}

/// The tree's context menu entry for the parts chosen in it.
pub fn context_menu(ui: &mut egui::Ui, actions: &mut Vec<AppAction>) {
    if ui.button("Form subassembly…").clicked() {
        actions.push(AppAction::Assembly(AssemblyAction::Form(FormAction::Open)));
        ui.close();
    }
}

pub fn apply(state: &mut AppState, action: FormAction) {
    match action {
        FormAction::Open => {
            let Some(s) = state.assembly.session.as_ref() else { return };
            let chosen = if s.chosen.is_empty() { s.selected.iter().cloned().collect() } else { s.chosen.clone() };
            state.assembly.form = Draft { open: true, document: Some(s.doc.id()), paths: chosen, ..Draft::default() };
            state.assembly.mate.open = false;
        }
        FormAction::Cancel => state.assembly.form.open = false,
        FormAction::Apply => {
            let d = &state.assembly.form;
            let (Some(origin), Some(s)) = (d.paths.first(), state.assembly.session.as_ref()) else { return };
            if d.document != Some(s.doc.id()) {
                state.assembly.form = Draft::default();
                state.note("The assembly changed; choose the parts again", NoteKind::Info);
                return;
            }
            let named = PathBuf::from(d.file.trim());
            let path = match s.doc.path().and_then(|p| p.parent()) {
                Some(folder) if named.is_relative() => folder.join(named),
                _ => named,
            };
            let request = FormRequest { paths: d.paths.clone(), path, origin: origin.clone() };
            match super::tools::form(state, &request) {
                Ok(formed) => {
                    state.assembly.form = Draft::default();
                    let name = formed.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    state.note(format!("Formed {name}"), NoteKind::Ok);
                }
                Err(r) => {
                    state.note(r.to_string(), NoteKind::Error);
                    state.assembly.form.error = Some(r.to_string());
                }
            }
        }
    }
}

/// The panel. Nothing shows while it is closed.
pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    if !state.assembly.form.open {
        return;
    }
    let labels: Vec<String> = {
        let Some(s) = state.assembly.session.as_ref() else { return };
        let table = s.doc.table();
        let d = &state.assembly.form;
        d.paths.iter().map(|p| table.find(p).map_or_else(|| p.to_string(), |r| s.doc.label(r))).collect()
    };
    let mut actions = Vec::new();
    let d = &mut state.assembly.form;
    ui.add_space(8.0);
    theme::section_label(ui, "Form subassembly");
    ui.label(
        egui::RichText::new("The parts move into a new file placed where they were. Nothing moves.")
            .size(11.0)
            .color(theme::muted()),
    );
    for (i, label) in labels.iter().enumerate() {
        ui.label(if i == 0 { format!("{label}  (frame)") } else { label.clone() });
    }
    ui.add(egui::TextEdit::singleline(&mut d.file).hint_text("modules/name.dwasm"))
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Subassembly file"));
    ui.horizontal(|ui| {
        if ui.button("Apply").clicked() {
            actions.push(FormAction::Apply);
        }
        if ui.button("Cancel").clicked() {
            actions.push(FormAction::Cancel);
        }
    });
    if let Some(e) = &d.error {
        ui.label(egui::RichText::new(e).color(theme::error()));
    }
    for a in actions {
        state.push(AppAction::Assembly(AssemblyAction::Form(a)));
    }
}
