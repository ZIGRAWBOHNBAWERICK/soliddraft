//! The Interference panel: which parts, interference or a clearance, and the results of the
//! last check, pair by pair. It runs the same check as the MCP tools.

use crate::interference::tools::length;
use crate::state::{AppState, NoteKind};
use crate::theme;
use soliddraft_assembly::{CheckReport, CheckRequest, CheckScope, ClearanceOutcome, OccurrencePath, Outcome};
use soliddraft_document::ContentHash;

/// Interference Detection or Clearance Verification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Interference,
    Clearance,
}

/// The Interference panel's fields and the last report.
pub struct Draft {
    pub open: bool,
    pub mode: Mode,
    /// Only the parts chosen in the tree, rather than the whole assembly.
    pub chosen_only: bool,
    pub include_hidden: bool,
    pub clearance: String,
    /// The last check's report, with the assembly revision it was run on.
    pub report: Option<(CheckReport, ContentHash)>,
    /// The last refusal, shown until a check runs.
    pub error: Option<String>,
}

impl Default for Draft {
    fn default() -> Draft {
        Draft {
            open: false,
            mode: Mode::Interference,
            chosen_only: false,
            include_hidden: false,
            clearance: "1 mm".into(),
            report: None,
            error: None,
        }
    }
}

/// What the panel asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum InterferenceAction {
    Toggle,
    /// Runs the check the panel describes.
    Check,
    /// Selects a result's two parts.
    Show(OccurrencePath, OccurrencePath),
}

pub fn apply(state: &mut AppState, action: InterferenceAction) {
    match action {
        InterferenceAction::Toggle => {
            let d = &mut state.assembly.interference;
            d.open = !d.open;
            if d.open {
                state.assembly.mate.open = false;
            }
        }
        InterferenceAction::Check => check(state),
        InterferenceAction::Show(a, b) => {
            let Some(s) = state.assembly.session.as_mut() else { return };
            s.selected = Some(a.clone());
            s.chosen = vec![a, b];
        }
    }
}

fn check(state: &mut AppState) {
    let d = &state.assembly.interference;
    let clearance = match d.mode {
        Mode::Interference => None,
        Mode::Clearance => match length(&d.clearance) {
            Ok(c) => Some(c),
            Err(e) => {
                state.assembly.interference.error = Some(e);
                return;
            }
        },
    };
    let (chosen_only, include_hidden) = (d.chosen_only, d.include_hidden);
    let Some(s) = state.assembly.session.as_mut() else { return };
    let scope = if chosen_only { CheckScope::Occurrences(s.chosen.clone()) } else { CheckScope::Assembly };
    let request = CheckRequest { scope, clearance, include_hidden, ..CheckRequest::default() };
    let revision = s.doc.revision();
    match s.doc.check_interference(&request) {
        Ok(report) => {
            let said = summary(&report);
            let d = &mut state.assembly.interference;
            d.error = None;
            d.report = Some((report, revision));
            state.note(said, NoteKind::Ok);
        }
        Err(r) => {
            state.assembly.interference.error = Some(r.to_string());
            state.note(r.to_string(), NoteKind::Error);
        }
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The report in a line.
fn summary(r: &CheckReport) -> String {
    let pairs = plural(r.pairs, "pair", "pairs");
    match (r.below, r.met, r.undecided) {
        (Some(below), Some(met), Some(undecided)) => {
            format!("{pairs}: {below} at or under the clearance, {undecided} undecided, {met} clear of it")
        }
        _ => format!(
            "{pairs}: {} interfering, {} touching or undecided, {} clear",
            r.interferes, r.touching_or_undecided, r.clear
        ),
    }
}

fn colour(outcome: Outcome, clearance: Option<ClearanceOutcome>) -> egui::Color32 {
    match (clearance, outcome) {
        (_, Outcome::Interferes) | (Some(ClearanceOutcome::Below), _) => theme::error(),
        (Some(ClearanceOutcome::Met), _) | (None, Outcome::Clear) => theme::ok(),
        _ => theme::warn(),
    }
}

/// The panel. Nothing shows while it is closed.
pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    if !state.assembly.interference.open {
        return;
    }
    let mut actions = Vec::new();
    let Some(s) = state.assembly.session.as_ref() else { return };
    let chosen = s.chosen.len();
    let revision = s.doc.revision();
    let d = &mut state.assembly.interference;
    ui.add_space(8.0);
    theme::section_label(ui, "Interference");
    ui.horizontal(|ui| {
        ui.selectable_value(&mut d.mode, Mode::Interference, "Detect interference");
        ui.selectable_value(&mut d.mode, Mode::Clearance, "Verify clearance");
    });
    ui.horizontal(|ui| {
        ui.selectable_value(&mut d.chosen_only, false, "Whole assembly");
        ui.selectable_value(&mut d.chosen_only, true, format!("Chosen parts ({chosen})"));
    });
    if d.mode == Mode::Clearance {
        ui.add(egui::TextEdit::singleline(&mut d.clearance).hint_text("Clearance, such as 0.5 mm"))
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Clearance"));
    }
    ui.checkbox(&mut d.include_hidden, "Include hidden parts");
    if ui.button("Check").clicked() {
        actions.push(InterferenceAction::Check);
    }
    ui.label(
        egui::RichText::new(
            "Proven on certified meshes of each part at its pinned revision. Pairs closer than the meshes' bounds \
             stay touching or undecided.",
        )
        .color(theme::muted()),
    );
    if let Some(e) = &d.error {
        ui.colored_label(theme::error(), e);
    }
    if let Some((report, at)) = &d.report {
        ui.add_space(6.0);
        theme::section_label(ui, "Results");
        if *at != revision {
            ui.colored_label(theme::warn(), "The assembly changed since this check; check again");
        }
        ui.label(summary(report));
        ui.label(egui::RichText::new(format!("Chord bound {} mm per part", report.chord_mm)).color(theme::muted()));
        for part in &report.skipped {
            ui.colored_label(theme::error(), format!("Not checked: {}. {}", part.label, part.message));
        }
        if report.results.is_empty() {
            ui.label(match report.clearance_mm {
                Some(_) => "Every pair is proven clear of the clearance",
                None => "Every pair is proven clear",
            });
        }
        for r in &report.results {
            let words = match r.clearance {
                Some(c) => c.words(),
                None => r.outcome.words(),
            };
            let head = ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("{words}: {} and {}", r.a_label, r.b_label))
                        .color(colour(r.outcome, r.clearance)),
                )
                .sense(egui::Sense::click()),
            );
            if head.clicked() {
                actions.push(InterferenceAction::Show(r.a.clone(), r.b.clone()));
            }
            ui.label(egui::RichText::new(&r.why).color(theme::muted()));
            if let Some(f) = r.faces.first() {
                ui.label(egui::RichText::new(format!("Nearest faces: {} and {}", f.a, f.b)).color(theme::muted()));
            }
        }
        for why in &report.refused {
            let text = why.split_once(": ").map_or(*why, |(_, t)| t);
            ui.label(egui::RichText::new(format!("Not reported: {text}")).color(theme::muted()));
        }
    }
    for a in actions {
        apply(state, a);
    }
}
