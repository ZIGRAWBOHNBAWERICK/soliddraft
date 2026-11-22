//! Configurations and the exploded view of the open assembly, in the window and over MCP.
//! What a configuration holds and how an exploded view moves parts are
//! `soliddraft-assembly`'s (see `docs/assemblies.md`); this module holds the
//! Configurations panel, the ribbon commands and the tools.

pub(crate) mod registry;
pub(crate) mod tools;

use crate::state::{AppState, NoteKind};
use crate::theme;
use glam::DVec3;
use soliddraft_assembly::OccurrencePath;

/// The Configurations panel's fields.
pub struct Draft {
    pub open: bool,
    /// The name a new configuration gets.
    pub name: String,
    /// How far a new explode step or Auto explode moves, in millimetres.
    pub distance: f64,
    /// The last refusal, shown until something succeeds.
    pub error: Option<String>,
}

impl Default for Draft {
    fn default() -> Draft {
        Draft { open: false, name: String::new(), distance: 50.0, error: None }
    }
}

/// What the panel and the ribbon ask for.
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigureAction {
    Toggle,
    /// Adds a configuration with the panel's name.
    Add,
    Activate(u32),
    Delete(u32),
    /// Shows the exploded view, or the assembled positions.
    Explode(bool),
    /// Replaces the exploded view with one step per shown component.
    AutoExplode,
    /// Moves the chosen occurrences along a direction by the panel's distance.
    Step(DVec3),
    EditStep(u32, f64),
    DeleteStep(u32),
}

/// The six directions a step can take from the panel.
pub const DIRECTIONS: [(&str, DVec3); 6] = [
    ("+X", DVec3::X),
    ("-X", DVec3::NEG_X),
    ("+Y", DVec3::Y),
    ("-Y", DVec3::NEG_Y),
    ("+Z", DVec3::Z),
    ("-Z", DVec3::NEG_Z),
];

pub fn apply(state: &mut AppState, action: ConfigureAction) {
    if action == ConfigureAction::Toggle {
        let d = &mut state.assembly.configure;
        d.open = !d.open;
        if d.open {
            state.assembly.mate.open = false;
            state.assembly.interference.open = false;
        }
        // Where the property area is a sheet, as in the browser, the panel opens it.
        state.options_details = d.open;
        return;
    }
    let distance = state.assembly.configure.distance;
    let name = state.assembly.configure.name.trim().to_owned();
    let Some(s) = state.assembly.session.as_mut() else { return };
    let chosen: Vec<OccurrencePath> = match (s.chosen.is_empty(), &s.selected) {
        (false, _) => s.chosen.clone(),
        (true, Some(p)) => vec![p.clone()],
        (true, None) => Vec::new(),
    };
    let result: Result<String, String> = match action {
        ConfigureAction::Toggle => unreachable!("handled above"),
        ConfigureAction::Add => s.doc.add_configuration(&name).map(|_| format!("Added configuration {name}")),
        ConfigureAction::Activate(id) => s.doc.activate_configuration(id).map(|()| {
            let now = s.doc.assembly().active_configuration().map(|c| c.name.clone()).unwrap_or_default();
            format!("Activated {now}")
        }),
        ConfigureAction::Delete(id) => s.doc.delete_configuration(id).map(|()| "Deleted the configuration".into()),
        ConfigureAction::Explode(on) => {
            s.doc.set_exploded(on).map(|()| if on { "Exploded".into() } else { "Collapsed".into() })
        }
        ConfigureAction::AutoExplode => s.doc.auto_explode(distance).and_then(|ids| {
            s.doc.set_exploded(true)?;
            Ok(format!("Exploded {} components", ids.len()))
        }),
        ConfigureAction::Step(direction) => {
            if chosen.is_empty() {
                Err(soliddraft_assembly::EditError::Invalid("choose the components to move first".into()))
            } else {
                s.doc.add_explode_step(chosen, direction, distance).and_then(|_| {
                    s.doc.set_exploded(true)?;
                    Ok("Added an explode step".into())
                })
            }
        }
        ConfigureAction::EditStep(id, mm) => s.doc.edit_explode_step(id, None, Some(mm)).map(|()| String::new()),
        ConfigureAction::DeleteStep(id) => s.doc.delete_explode_step(id).map(|()| "Deleted the step".into()),
    }
    .map_err(|e| e.to_string());
    match result {
        Ok(said) => {
            state.assembly.configure.error = None;
            if action == ConfigureAction::Add {
                state.assembly.configure.name.clear();
            }
            if !said.is_empty() {
                state.note(said, NoteKind::Ok);
            }
        }
        Err(e) => {
            state.assembly.configure.error = Some(e.clone());
            state.note(e, NoteKind::Error);
        }
    }
}

/// The panel: the configurations with the active one marked, then the exploded view's
/// steps. Nothing shows while it is closed.
pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    if !state.assembly.configure.open {
        return;
    }
    let mut actions = Vec::new();
    let Some(s) = state.assembly.session.as_ref() else { return };
    let truth = s.doc.assembly();
    let chosen = if s.chosen.is_empty() { usize::from(s.selected.is_some()) } else { s.chosen.len() };
    let exploded = s.doc.is_exploded();
    let d = &mut state.assembly.configure;
    ui.add_space(8.0);
    theme::section_label(ui, "Configurations");
    if truth.configurations.is_empty() {
        ui.label(egui::RichText::new("Only the default state. Add one to keep another.").color(theme::muted()));
    }
    for c in &truth.configurations {
        ui.horizontal(|ui| {
            if c.is_active() {
                ui.label(egui::RichText::new(format!("● {}", c.name)).strong());
                ui.label(egui::RichText::new("active").color(theme::muted()));
            } else {
                ui.label(format!("○ {}", c.name));
                let activate = ui.small_button("Activate").on_hover_text(format!("Show {}", c.name));
                #[cfg(target_arch = "wasm32")]
                crate::web::show_control(format!("Activate {}", c.name), activate.rect);
                if activate.clicked() {
                    actions.push(ConfigureAction::Activate(c.id));
                }
                if ui.small_button("Delete").clicked() {
                    actions.push(ConfigureAction::Delete(c.id));
                }
            }
        });
    }
    ui.horizontal(|ui| {
        let field = ui.add(egui::TextEdit::singleline(&mut d.name).hint_text("New configuration").desired_width(130.0));
        field.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Configuration name"));
        let add = ui.add_enabled(!d.name.trim().is_empty(), egui::Button::new("Add"));
        #[cfg(target_arch = "wasm32")]
        {
            crate::web::show_control("Configuration name", field.rect);
            crate::web::show_control("Add configuration", add.rect);
        }
        if add.clicked() {
            actions.push(ConfigureAction::Add);
        }
    });
    ui.label(
        egui::RichText::new(
            "Each keeps its own suppression, placements, values and exploded view. Edits change the active one.",
        )
        .size(11.0)
        .color(theme::muted()),
    );

    ui.add_space(8.0);
    theme::section_label(ui, "Exploded view");
    ui.horizontal(|ui| {
        let label = if exploded { "Collapse" } else { "Explode" };
        let toggle = ui.add_enabled(!truth.explode.is_empty(), egui::Button::new(label));
        let auto = ui.button("Auto explode");
        #[cfg(target_arch = "wasm32")]
        {
            crate::web::show_control("Explode or collapse", toggle.rect);
            crate::web::show_control("Auto explode", auto.rect);
        }
        if toggle.clicked() {
            actions.push(ConfigureAction::Explode(!exploded));
        }
        if auto.clicked() {
            actions.push(ConfigureAction::AutoExplode);
        }
    });
    ui.horizontal(|ui| {
        ui.label("Distance");
        ui.add(egui::DragValue::new(&mut d.distance).range(-1e6..=1e6).speed(1.0).max_decimals(3).suffix(" mm"))
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, "Explode distance"));
    });
    ui.label(format!("Move the {chosen} chosen along:"));
    ui.horizontal_wrapped(|ui| {
        for (label, direction) in DIRECTIONS {
            if ui.add_enabled(chosen > 0, egui::Button::new(label)).clicked() {
                actions.push(ConfigureAction::Step(direction));
            }
        }
    });
    for (i, step) in truth.explode.iter().enumerate() {
        let what: Vec<String> = step
            .occurrences
            .iter()
            .map(|p| s.doc.table().find(p).map(|r| s.doc.label(r)).unwrap_or_else(|| p.to_string()))
            .collect();
        ui.horizontal(|ui| {
            ui.label(format!("{}. {}", i + 1, what.join(", ")));
            let mut mm = step.distance;
            // Typed, not dragged: each change is an undo step.
            let r = ui.add(egui::DragValue::new(&mut mm).speed(0.0).max_decimals(3).suffix(" mm"));
            if r.changed() {
                actions.push(ConfigureAction::EditStep(step.id, mm));
            }
            if ui.small_button("Delete").on_hover_text("Delete the step").clicked() {
                actions.push(ConfigureAction::DeleteStep(step.id));
            }
        });
    }
    ui.label(
        egui::RichText::new("Display only: placements do not move, and any edit collapses the view.")
            .size(11.0)
            .color(theme::muted()),
    );
    if let Some(e) = &d.error {
        ui.colored_label(theme::error(), e);
    }
    for a in actions {
        state.push(crate::state::AppAction::Assembly(crate::assembly::AssemblyAction::Configure(a)));
    }
}
