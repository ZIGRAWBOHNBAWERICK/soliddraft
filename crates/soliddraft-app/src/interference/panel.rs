//! The Interference panel: which parts, interference or a clearance, and the results of the
//! last check, pair by pair. It runs the same check as the MCP tools.

use crate::interference::tools::length;
use crate::state::{AppState, NoteKind};
use crate::theme;
use soliddraft_assembly::{
    CheckReport, CheckRequest, CheckScope, ClearanceOutcome, HoleAlignmentReport, HoleAlignmentRequest, HoleOutcome,
    OccurrencePath, Outcome,
};
use soliddraft_document::ContentHash;

/// Static interference, clearance or hole alignment check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Interference,
    Clearance,
    HoleAlignment,
}

/// The Interference panel's fields and the last report.
pub struct Draft {
    pub open: bool,
    pub mode: Mode,
    /// Only the parts chosen in the tree, rather than the whole assembly.
    pub chosen_only: bool,
    pub include_hidden: bool,
    pub clearance: String,
    pub max_deviation: String,
    /// Last hole check and its assembly revision.
    pub hole_report: Option<(HoleAlignmentReport, ContentHash)>,
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
            max_deviation: "1 mm".into(),
            hole_report: None,
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
    if state.assembly.interference.mode == Mode::HoleAlignment {
        check_holes(state);
        return;
    }
    let d = &state.assembly.interference;
    let clearance = match d.mode {
        Mode::Interference | Mode::HoleAlignment => None,
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
    // Wrapping can move a hovered choice to another row before the click releases.
    ui.vertical(|ui| {
        ui.selectable_value(&mut d.mode, Mode::Interference, "Detect interference");
        ui.selectable_value(&mut d.mode, Mode::Clearance, "Verify clearance");
        ui.selectable_value(&mut d.mode, Mode::HoleAlignment, "Hole alignment");
    });
    ui.horizontal(|ui| {
        ui.selectable_value(&mut d.chosen_only, false, "Whole assembly");
        ui.selectable_value(&mut d.chosen_only, true, format!("Chosen parts ({chosen})"));
    });
    if d.mode == Mode::Clearance {
        ui.add(egui::TextEdit::singleline(&mut d.clearance).hint_text("Clearance, such as 0.5 mm"))
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Clearance"));
    }
    if d.mode == Mode::HoleAlignment {
        ui.add(egui::TextEdit::singleline(&mut d.max_deviation).hint_text("Maximum deviation, such as 0.5 mm"))
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Maximum deviation"));
    }
    ui.checkbox(&mut d.include_hidden, "Include hidden parts");
    if ui.button("Check").clicked() {
        actions.push(InterferenceAction::Check);
    }
    if d.mode == Mode::HoleAlignment {
        ui.label("Exact internal cylinder axes at pinned revisions. Checks parallel axes within the maximum deviation, not diameter or fit.");
    } else {
        ui.label(
            egui::RichText::new(
                "Proven on certified meshes of each part at its pinned revision. Pairs closer than the meshes' bounds \
             stay touching or undecided.",
            )
            .color(theme::muted()),
        );
    }
    if let Some(e) = &d.error {
        ui.colored_label(theme::error(), e);
    }
    if d.mode == Mode::HoleAlignment {
        if let Some((report, at)) = &d.hole_report {
            show_holes(ui, report, *at != revision, &mut actions);
        }
    } else if let Some((report, at)) = &d.report {
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
            ui.label(all_clear(report));
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

/// What an empty result list proves: every pair, or with anything skipped, only the pairs
/// of the parts that were checked.
fn all_clear(report: &CheckReport) -> &'static str {
    match (report.clearance_mm, report.complete) {
        (Some(_), true) => "Every pair is proven clear of the clearance",
        (None, true) => "Every pair is proven clear",
        (Some(_), false) => {
            "Every pair of the checked parts is proven clear of the clearance; the parts not checked are unknown"
        }
        (None, false) => "Every pair of the checked parts is proven clear; the parts not checked are unknown",
    }
}

fn check_holes(state: &mut AppState) {
    let d = &state.assembly.interference;
    let max_deviation_mm = match length(&d.max_deviation) {
        Ok(v) => v,
        Err(e) => {
            state.assembly.interference.error = Some(e);
            return;
        }
    };
    let Some(s) = state.assembly.session.as_mut() else { return };
    let scope = if d.chosen_only { CheckScope::Occurrences(s.chosen.clone()) } else { CheckScope::Assembly };
    let request = HoleAlignmentRequest { scope, include_hidden: d.include_hidden, max_deviation_mm };
    let revision = s.doc.revision();
    match s.doc.check_hole_alignment(&request) {
        Ok(report) => {
            state.assembly.interference.error = None;
            state.assembly.interference.hole_report = Some((report, revision));
        }
        Err(e) => {
            state.assembly.interference.error = Some(e.to_string());
        }
    }
}

fn show_holes(ui: &mut egui::Ui, report: &HoleAlignmentReport, stale: bool, actions: &mut Vec<InterferenceAction>) {
    theme::section_label(ui, "Hole results");
    if stale {
        ui.colored_label(theme::warn(), "The assembly changed since this check; check again");
    }
    ui.label(format!(
        "{} aligned, {} misaligned within {} mm",
        report.aligned, report.misaligned, report.max_deviation_mm
    ));
    ui.label(format!(
        "{} parts inspected, {} bore axes, {} nonparallel pairs, {} beyond the maximum",
        report.parts,
        report.holes.len(),
        report.nonparallel,
        report.beyond
    ));
    for part in &report.skipped {
        ui.colored_label(theme::error(), format!("Not checked: {}. {}", part.label, part.message));
    }
    if !report.complete {
        ui.colored_label(theme::warn(), "Incomplete check; unchecked parts may contain other holes");
    }
    if report.results.is_empty() {
        ui.label("No parallel bore-axis pairs found within the maximum deviation in the checked parts");
    }
    for pair in &report.results {
        let (a, b) = (&report.holes[pair.a], &report.holes[pair.b]);
        let words = if pair.outcome == HoleOutcome::Aligned { "Aligned holes" } else { "Misaligned holes" };
        let color = if pair.outcome == HoleOutcome::Aligned { theme::ok() } else { theme::warn() };
        let row = ui.add(
            egui::Label::new(egui::RichText::new(format!("{words}: {} and {}", a.label, b.label)).color(color))
                .sense(egui::Sense::click()),
        );
        if row.clicked() {
            actions.push(InterferenceAction::Show(a.path.clone(), b.path.clone()));
        }
        ui.label(format!(
            "{} and {}. Deviation approximately {} mm; exact square {} mm²",
            a.face, b.face, pair.deviation_mm, pair.deviation_squared
        ));
    }
}
