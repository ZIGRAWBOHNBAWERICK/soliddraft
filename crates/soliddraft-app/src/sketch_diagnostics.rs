//! Session-local, read-only inspection of profile warnings.

use crate::{
    sketch::{
        Sketch,
        profile::diagnostics::{self, Diagnostic},
    },
    state::{AppAction, RelationFilter, SketchSel, SketchSession},
    theme,
};

/// A report retains its source so stale locations cannot highlight edited geometry.
pub struct Inspection {
    /// The unmodified sketch used to make the report.
    pub source: Sketch,
    /// The warnings in deterministic model order.
    pub issues: Vec<Diagnostic>,
    /// The warning currently displayed.
    pub current: usize,
    /// Reveal the report after opening or refreshing it.
    pub reveal: bool,
}

impl Inspection {
    /// Analyze once, on open or explicit refresh.
    pub fn new(sketch: &Sketch) -> Self {
        Self { source: sketch.clone(), issues: diagnostics::inspect(sketch), current: 0, reveal: true }
    }
}

/// The current warning, only while its saved locations still describe this sketch.
pub fn selected(s: &SketchSession) -> Option<&Diagnostic> {
    let report = s.diagnostics.as_ref()?;
    (report.source == s.sketch && s.drag.is_none() && s.trim_drag.is_none())
        .then(|| report.issues.get(report.current))
        .flatten()
}

/// The inspector uses the existing sketch property panel and document commands.
pub fn panel(ui: &mut egui::Ui, s: &mut SketchSession, actions: &mut Vec<AppAction>) {
    let Some(mut report) = s.diagnostics.take() else { return };
    if report.reveal {
        ui.scroll_to_cursor(Some(egui::Align::TOP));
        report.reveal = false;
    }
    theme::section_label(ui, "Profile diagnostics");
    let stale = report.source != s.sketch || s.drag.is_some() || s.trim_drag.is_some();
    if stale {
        ui.label(egui::RichText::new("Sketch changed. Refresh diagnostics.").size(11.0).color(theme::error()));
    } else if let Some(issue) = report.issues.get(report.current) {
        ui.label(
            egui::RichText::new(format!("Fault {} of {}", report.current + 1, report.issues.len()))
                .size(11.0)
                .color(theme::muted()),
        );
        ui.label(egui::RichText::new(issue.kind.label()).color(theme::error()));
        let owners = issue
            .entities
            .iter()
            .map(|id| {
                s.sketch
                    .entity(*id)
                    .map(|e| format!("{} {}", e.kind_name(), e.id))
                    .unwrap_or_else(|| format!("missing entity {id}"))
            })
            .collect::<Vec<_>>()
            .join(", ");
        ui.label(egui::RichText::new(owners).size(11.0).color(theme::muted()));
        if let Some(id) = issue.relation
            && let Some(relation) = s.sketch.constraint(id)
        {
            ui.label(egui::RichText::new(format!("Relation {id}: {}", relation.name())).size(11.0));
        }
        if let Some(message) = &issue.message {
            ui.label(egui::RichText::new(message).size(11.0).color(theme::error()));
        }
    } else {
        let curves = s.sketch.drawn().any(|e| !e.construction && !e.is_point());
        ui.label(if curves { "No listed profile defects found." } else { "No profile curves to inspect." });
    }
    let active = !stale && !report.issues.is_empty();
    let mut close = false;
    ui.horizontal_wrapped(|ui| {
        if let Some(id) = report.issues.get(report.current).and_then(|i| i.relation)
            && ui.add_enabled(active, egui::Button::new("Inspect relation")).clicked()
        {
            s.selection = vec![SketchSel::Constraint(id)];
            s.relations_filter = RelationFilter::Selected;
        }
        if ui.add_enabled(active, egui::Button::new("Previous fault")).clicked() {
            report.current = (report.current + report.issues.len() - 1) % report.issues.len();
        }
        if ui.add_enabled(active, egui::Button::new("Next fault")).clicked() {
            report.current = (report.current + 1) % report.issues.len();
        }
        let located = report.issues.get(report.current).is_some_and(|i| !i.locations.is_empty());
        if ui.add_enabled(active && located, egui::Button::new("Locate fault")).clicked() {
            actions.push(AppAction::LocateSketchDiagnostic);
        }
        if ui.button("Refresh diagnostics").clicked() {
            report = Inspection::new(&s.sketch);
        }
        close = ui.button("Close diagnostics").clicked();
    });
    ui.label(
        egui::RichText::new(
            "Read-only warnings. Intentional open paths may be valid. Not automatic repair or a kernel check.",
        )
        .size(11.0)
        .color(theme::muted()),
    );
    ui.add_space(8.0);
    if !close {
        s.diagnostics = Some(report);
    }
}
