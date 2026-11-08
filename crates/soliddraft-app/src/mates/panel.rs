//! The Mate panel: picked faces held parallel, at right angles, on one plane, at a
//! distance, on one axis, tangent, hinged or as a tab in a groove, two components locked
//! together, the assembly's values, what the solver says, and the conflicts a refusal
//! named. Everything goes through the same commands as the MCP tools.

use crate::assembly::AssemblyState;
use crate::mates::tools::rows;
use crate::state::{AppState, NoteKind};
use crate::theme;
use soliddraft_assembly::{
    Alignment, ComponentId, EndpointSelector, FaceSelector, MateEdit, MateRefusal, OccurrencePath, WidthInput,
};
use soliddraft_model::FaceRef;

/// A picked face.
#[derive(Clone, Debug, PartialEq)]
pub struct PickedFace {
    pub component: ComponentId,
    pub face: FaceRef,
}

/// Which mate the panel adds between its picked faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Coincident,
    Distance,
    Parallel,
    Perpendicular,
    /// The first two picks are the groove's faces, the last two the tab's.
    Width,
    /// Locks the components of the two picked faces together.
    Lock,
    /// Two cylinders, cones or spheres on one axis or centre.
    Concentric,
    /// A plane, then a cylinder or sphere laid against it.
    Tangent,
    /// The first two picks are the axes, the last two the planes.
    Hinge,
}

impl Family {
    const ALL: [Family; 9] = [
        Family::Coincident,
        Family::Distance,
        Family::Parallel,
        Family::Perpendicular,
        Family::Width,
        Family::Lock,
        Family::Concentric,
        Family::Tangent,
        Family::Hinge,
    ];

    fn label(self) -> &'static str {
        match self {
            Family::Coincident => "Coincident",
            Family::Distance => "Distance",
            Family::Parallel => "Parallel",
            Family::Perpendicular => "Perpendicular",
            Family::Width => "Width",
            Family::Lock => "Lock",
            Family::Concentric => "Concentric",
            Family::Tangent => "Tangent",
            Family::Hinge => "Hinge",
        }
    }

    /// How many faces it needs.
    fn picks(self) -> usize {
        if matches!(self, Family::Width | Family::Hinge) { 4 } else { 2 }
    }

    fn has_alignment(self) -> bool {
        !matches!(self, Family::Perpendicular | Family::Lock)
    }
}

/// Where the panel's Width puts the tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidthMode {
    Centered,
    Dimension,
    Percent,
}

/// The Mate panel's fields.
pub struct Draft {
    pub open: bool,
    pub a: Option<PickedFace>,
    pub b: Option<PickedFace>,
    /// A Width's tab faces.
    pub c: Option<PickedFace>,
    pub d: Option<PickedFace>,
    pub family: Family,
    pub alignment: Alignment,
    /// A Hinge's planes' alignment.
    pub plane_alignment: Alignment,
    /// A Concentric holds the turn about its axis.
    pub lock_rotation: bool,
    pub name: String,
    pub distance: String,
    pub width_mode: WidthMode,
    pub percent: String,
    pub flipped: bool,
    pub value_name: String,
    pub value_expression: String,
    /// The last refusal, shown until something works.
    pub error: Option<String>,
    /// The last refusal that named mates or freedoms, for the Conflicts list.
    pub conflict: Option<MateRefusal>,
}

impl Default for Draft {
    fn default() -> Draft {
        Draft {
            open: false,
            a: None,
            b: None,
            c: None,
            d: None,
            family: Family::Distance,
            alignment: Alignment::Opposed,
            plane_alignment: Alignment::Opposed,
            lock_rotation: false,
            name: String::new(),
            distance: "0 mm".into(),
            width_mode: WidthMode::Centered,
            percent: "50".into(),
            flipped: false,
            value_name: String::new(),
            value_expression: String::new(),
            error: None,
            conflict: None,
        }
    }
}

impl Draft {
    fn slots(&mut self) -> [&mut Option<PickedFace>; 4] {
        [&mut self.a, &mut self.b, &mut self.c, &mut self.d]
    }
}

/// What the panel asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum MateAction {
    Toggle,
    /// A face was clicked: fills the first free slot.
    Pick(OccurrencePath, String),
    Clear,
    /// Adds the drafted mate between the picked faces.
    Add,
    AddOrigin(ComponentId),
    SetValue,
    Drive(String, String),
    /// Removes a mate the Conflicts list names.
    Delete(String),
    /// Turns a mate's alignment the other way.
    Flip(String, Alignment),
    /// Hides the Conflicts list.
    Dismiss,
}

fn next_name(state: &AssemblyState, prefix: &str) -> String {
    let taken = |n: &str| state.session.as_ref().is_some_and(|s| s.doc.assembly().known_mates().any(|m| m.name() == n));
    (1..).map(|i| format!("{prefix}{i}")).find(|n| !taken(n)).expect("an unused name")
}

/// Shows what a command did. A refusal that names mates or freedoms also fills the
/// Conflicts list; a success clears it.
pub(crate) fn report(state: &mut AppState, done: Result<(), MateRefusal>, said: &str) {
    match done {
        Ok(()) => {
            state.assembly.mate.error = None;
            state.assembly.mate.conflict = None;
            state.note(said.to_owned(), NoteKind::Ok);
        }
        Err(r) => {
            state.assembly.mate.error = Some(r.to_string());
            state.note(r.to_string(), NoteKind::Error);
            state.assembly.mate.conflict = (!r.mates.is_empty() || !r.free.is_empty()).then_some(r);
        }
    }
}

fn pick(state: &mut AppState, path: &OccurrencePath, name: &str) {
    let [step] = path.steps() else {
        state.assembly.mate.error = Some("mates reach the parts placed directly for now".into());
        return;
    };
    let Ok(face) = serde_json::from_str::<FaceRef>(name) else {
        state.assembly.mate.error = Some("this face has no persistent name to mate".into());
        return;
    };
    let picked = PickedFace { component: step.component, face };
    let d = &mut state.assembly.mate;
    d.error = None;
    let wanted = d.family.picks();
    let mut slots = d.slots();
    match slots[..wanted].iter_mut().find(|s| s.is_none()) {
        Some(slot) => **slot = Some(picked),
        None => *slots[wanted - 1] = Some(picked),
    }
}

fn width_input(d: &Draft) -> Result<WidthInput, String> {
    Ok(match d.width_mode {
        WidthMode::Centered => WidthInput::Centered,
        WidthMode::Dimension => WidthInput::Dimension { distance: d.distance.clone(), flipped: d.flipped },
        WidthMode::Percent => {
            let percent = d.percent.trim().trim_end_matches('%').trim().parse::<f64>();
            WidthInput::Percent { percent: percent.map_err(|_| "the percentage is a number")?, flipped: d.flipped }
        }
    })
}

fn add(state: &mut AppState) {
    let family = state.assembly.mate.family;
    let name = match state.assembly.mate.name.trim() {
        "" => next_name(&state.assembly, family.label()),
        n => n.to_owned(),
    };
    let (picked, distance, alignment, width) = {
        let d = &state.assembly.mate;
        let picked: Vec<PickedFace> =
            [&d.a, &d.b, &d.c, &d.d].into_iter().take(family.picks()).flatten().cloned().collect();
        (picked, d.distance.clone(), d.alignment, (family == Family::Width).then(|| width_input(d)))
    };
    let (plane_alignment, lock_rotation) = (state.assembly.mate.plane_alignment, state.assembly.mate.lock_rotation);
    if picked.len() < family.picks() {
        let wanted = match family {
            Family::Width => "Pick the groove's two faces, then the tab's",
            Family::Hinge => "Pick the two axes' faces, then the two planes",
            _ => "Pick two faces first",
        };
        state.assembly.mate.error = Some(wanted.into());
        return;
    }
    let width = match width.transpose() {
        Ok(w) => w,
        Err(e) => {
            state.assembly.mate.error = Some(e);
            return;
        }
    };
    let end = |p: &PickedFace| EndpointSelector { component: p.component, face: FaceSelector::Picked(p.face.clone()) };
    let ends: Vec<EndpointSelector> = picked.iter().map(end).collect();
    let Some(s) = state.assembly.session.as_mut() else { return };
    let (a, b) = (&ends[0], &ends[1]);
    let done = match family {
        Family::Coincident => s.doc.mate_coincident(&name, a, b, alignment, false),
        Family::Distance => s.doc.mate_distance(&name, a, b, &distance, alignment, false),
        Family::Parallel => s.doc.mate_parallel(&name, a, b, alignment, false),
        Family::Perpendicular => s.doc.mate_perpendicular(&name, a, b, false),
        Family::Width => {
            let width = width.expect("made above");
            s.doc.mate_width(&name, [a, b], [&ends[2], &ends[3]], alignment, &width, false)
        }
        Family::Lock => s.doc.mate_lock(&name, picked[0].component, picked[1].component, false),
        Family::Concentric => s.doc.mate_concentric(&name, a, b, alignment, lock_rotation, false),
        Family::Tangent => s.doc.mate_tangent(&name, a, b, alignment, false),
        Family::Hinge => s.doc.mate_hinge(&name, [a, b], [&ends[2], &ends[3]], alignment, plane_alignment, false),
    };
    report(state, done.map(|_| ()), "Mate added");
}

pub fn apply(state: &mut AppState, action: MateAction) {
    match action {
        MateAction::Toggle => {
            state.assembly.mate.open = !state.assembly.mate.open;
            if state.assembly.mate.open {
                state.assembly.interference.open = false;
            }
        }
        MateAction::Pick(path, name) => pick(state, &path, &name),
        MateAction::Clear => state.assembly.mate.slots().into_iter().for_each(|s| *s = None),
        MateAction::Add => add(state),
        MateAction::AddOrigin(component) => {
            let name = next_name(&state.assembly, "Origin");
            let Some(s) = state.assembly.session.as_mut() else { return };
            let done = s.doc.mate_origin(&name, component, true, false).map(|_| ());
            report(state, done, "Origin mate added");
        }
        MateAction::SetValue => {
            let (name, expression) =
                (state.assembly.mate.value_name.clone(), state.assembly.mate.value_expression.clone());
            let Some(s) = state.assembly.session.as_mut() else { return };
            let done = s.doc.set_value(name.trim(), &expression);
            report(state, done, "Value set");
        }
        MateAction::Drive(mate, expression) => {
            let Some(s) = state.assembly.session.as_mut() else { return };
            let done = s.doc.drive_mate(&mate, &expression, true);
            report(state, done, "Mate driven");
        }
        MateAction::Delete(mate) => {
            let Some(s) = state.assembly.session.as_mut() else { return };
            let done = s.doc.delete_mate(&mate);
            report(state, done, "Mate deleted");
        }
        MateAction::Flip(mate, to) => {
            let Some(s) = state.assembly.session.as_mut() else { return };
            let done = s.doc.edit_mates(&[(mate, MateEdit::Alignment(to))]);
            report(state, done, "Mate flipped");
        }
        MateAction::Dismiss => state.assembly.mate.conflict = None,
    }
}

/// The refusal's code in words.
fn headline(r: &MateRefusal) -> &'static str {
    use soliddraft_assembly::RefusalCode as C;
    match r.code {
        C::OverConstrained => "These mates cannot all hold",
        C::UnderConstrained => "Freedom remains",
        C::DanglingReference => "A mated face is gone",
        C::UnsupportedPlacement => "A part is turned off the quarter turns",
        C::UnsupportedPlane | C::NotParallel => "A face cannot be proved for this mate",
        C::UnsupportedReference => "An axis or point cannot be proved for this mate",
        _ => "Refused",
    }
}

fn labelled(ui: &mut egui::Ui, value: &mut String, hint: &str, name: &'static str) {
    ui.add(egui::TextEdit::singleline(value).hint_text(hint))
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, name));
}

/// The panel. Nothing shows while it is closed.
pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    if !state.assembly.mate.open {
        return;
    }
    let mut actions = Vec::new();
    let selected = state.assembly.session.as_ref().and_then(|s| {
        let path = s.selected.as_ref()?;
        match path.steps() {
            [step] if step.member.is_none() => Some(step.component),
            _ => None,
        }
    });
    let Some(s) = state.assembly.session.as_mut() else { return };
    let (mates, values, solver) = rows(s);
    let warnings: Vec<String> = s
        .components()
        .into_iter()
        .filter_map(|row| crate::assembly::warning(s.doc.store().get(s.doc.table().definition(row))))
        .collect();
    let d = &mut state.assembly.mate;
    ui.add_space(8.0);
    theme::section_label(ui, "Mate");
    let face = |p: &Option<PickedFace>| {
        p.as_ref().map_or("Pick a face".to_owned(), |p| format!("{}: {}", p.component, p.face.label))
    };
    let slots: &[&str] = match d.family {
        Family::Width => &["Groove face", "Groove face", "Tab face", "Tab face"],
        Family::Lock => &["First component", "Second component"],
        Family::Tangent => &["Plane", "Cylinder or sphere"],
        Family::Hinge => &["First axis", "Second axis", "First plane", "Second plane"],
        _ => &["First face", "Second face"],
    };
    for (label, slot) in slots.iter().zip([&d.a, &d.b, &d.c, &d.d]) {
        ui.label(format!("{label}  {}", face(slot)));
    }
    ui.horizontal_wrapped(|ui| {
        for f in Family::ALL {
            ui.selectable_value(&mut d.family, f, f.label());
        }
    });
    if d.family.has_alignment() {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut d.alignment, Alignment::Opposed, "Opposed");
            ui.selectable_value(&mut d.alignment, Alignment::Aligned, "Aligned");
        });
    }
    if d.family == Family::Hinge {
        ui.horizontal(|ui| {
            ui.label("Planes");
            ui.selectable_value(&mut d.plane_alignment, Alignment::Opposed, "Planes opposed");
            ui.selectable_value(&mut d.plane_alignment, Alignment::Aligned, "Planes aligned");
        });
    }
    if d.family == Family::Concentric {
        ui.checkbox(&mut d.lock_rotation, "Lock rotation");
    }
    labelled(ui, &mut d.name, "Name", "Mate name");
    if d.family == Family::Width {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut d.width_mode, WidthMode::Centered, "Centred");
            ui.selectable_value(&mut d.width_mode, WidthMode::Dimension, "Gap");
            ui.selectable_value(&mut d.width_mode, WidthMode::Percent, "Percent");
        });
        match d.width_mode {
            WidthMode::Centered => {}
            WidthMode::Dimension => labelled(ui, &mut d.distance, "Gap, such as 1 mm", "Mate distance"),
            WidthMode::Percent => labelled(ui, &mut d.percent, "Percent of the play", "Mate percent"),
        }
        if d.width_mode != WidthMode::Centered {
            ui.checkbox(&mut d.flipped, "From the second groove face");
        }
    }
    if d.family == Family::Distance {
        labelled(ui, &mut d.distance, "Distance, such as 5 mm", "Mate distance");
    }
    ui.horizontal(|ui| {
        if ui.button("Add mate").clicked() {
            actions.push(MateAction::Add);
        }
        if ui.button("Clear faces").clicked() {
            actions.push(MateAction::Clear);
        }
        if let Some(c) = selected
            && ui.button("Origin mate").clicked()
        {
            actions.push(MateAction::AddOrigin(c));
        }
    });
    // What the last command refused, under the button that ran it.
    if let Some(e) = &d.error {
        ui.colored_label(theme::error(), e);
    }
    if let Some(c) = &d.conflict {
        conflicts(ui, c, &mates, &mut actions);
    }
    ui.add_space(6.0);
    theme::section_label(ui, "Values");
    for v in values.as_array().into_iter().flatten() {
        ui.label(format!(
            "{} = {} ({} {})",
            v["name"].as_str().unwrap_or(""),
            v["expression"].as_str().unwrap_or(""),
            v["value"],
            v["unit"].as_str().unwrap_or("")
        ));
    }
    labelled(ui, &mut d.value_name, "Name", "Value name");
    labelled(ui, &mut d.value_expression, "Expression, such as 1.2 cm", "Value expression");
    if ui.button("Set value").clicked() {
        actions.push(MateAction::SetValue);
    }
    ui.add_space(6.0);
    theme::section_label(ui, "Mates");
    let named: Vec<String> = d.conflict.as_ref().map(|c| c.mates.clone()).unwrap_or_default();
    for m in mates.as_array().into_iter().flatten() {
        let name = m["name"].as_str().unwrap_or("?").to_owned();
        let alignment = m["alignment"].as_str().unwrap_or("?");
        let gap = |m: &serde_json::Value| m["measurement"]["value"].as_f64();
        let shown = |v: Option<f64>, unit: &str| {
            v.map_or("unproved".to_owned(), |v| format!("{} mm{unit}", crate::document::fmt_mm(v)))
        };
        let line = match m["kind"].as_str() {
            Some("distance") => {
                let mm = gap(m).or(m["distance"]["value"].as_f64());
                ui.horizontal(|ui| {
                    show_row(ui, &named, &name, format!("{name}: {}", shown(mm, "")));
                    let wanted = d.value_name.trim();
                    if !wanted.is_empty() && m["distance"]["expression"].as_str() != Some(wanted) {
                        let drive = ui.button(format!("Drive {name}"));
                        if drive.clicked() {
                            actions.push(MateAction::Drive(name.clone(), wanted.to_owned()));
                        }
                    }
                });
                continue;
            }
            Some("coincident") => format!("{name}: coincident, {alignment}, {}", shown(gap(m), " apart")),
            Some("parallel") => format!("{name}: parallel, {alignment}, {}", shown(gap(m), " apart")),
            Some("perpendicular") => format!("{name}: perpendicular"),
            Some("width") => {
                let how = match m["width"]["mode"].as_str() {
                    Some("dimension") => shown(m["width"]["distance"]["value"].as_f64(), " gap"),
                    Some("percent") => {
                        format!("{}% of the play", m["width"]["percent"].as_f64().unwrap_or(f64::NAN))
                    }
                    _ => "centred".to_owned(),
                };
                format!("{name}: width, {alignment}, {how}")
            }
            Some("lock") => format!("{name}: locked"),
            Some("concentric") => {
                let locked = if m["lock_rotation"].as_bool() == Some(true) { ", rotation locked" } else { "" };
                match m["alignment"].as_str() {
                    Some(a) => format!("{name}: concentric, {a}{locked}"),
                    None => format!("{name}: concentric{locked}"),
                }
            }
            Some("tangent") => format!("{name}: tangent, {alignment}"),
            Some("hinge") => {
                format!("{name}: hinge, axes {alignment}, planes {}", m["plane_alignment"].as_str().unwrap_or("?"))
            }
            Some("origin") => format!("{name}: on the origin"),
            _ => {
                ui.colored_label(theme::error(), format!("{name}: this version cannot solve it"));
                continue;
            }
        };
        show_row(ui, &named, &name, line);
    }
    ui.label(match solver["state"].as_str() {
        Some("solved") => "Fully constrained".to_owned(),
        Some("error") => "The mates cannot be solved".to_owned(),
        _ => format!("{} freedoms remain", solver["remaining_dof"]),
    });
    for w in warnings {
        ui.colored_label(theme::error(), w);
    }
    for a in actions {
        apply(state, a);
    }
}

/// A mate's row, in the error colour when the Conflicts list names it.
fn show_row(ui: &mut egui::Ui, named: &[String], name: &str, line: String) {
    if named.iter().any(|n| n == name) {
        ui.colored_label(theme::error(), line);
    } else {
        ui.label(line);
    }
}

/// The Conflicts list: what the last refusal named, each mate with what can be done to it,
/// and the motions left free.
fn conflicts(ui: &mut egui::Ui, c: &MateRefusal, mates: &serde_json::Value, actions: &mut Vec<MateAction>) {
    ui.add_space(6.0);
    theme::section_label(ui, "Conflicts");
    ui.colored_label(theme::error(), headline(c));
    for name in &c.mates {
        let row = mates.as_array().into_iter().flatten().find(|m| m["name"].as_str() == Some(name.as_str()));
        // The mate the refused command would have added is not in the list.
        let kind = row.and_then(|m| m["kind"].as_str()).unwrap_or("not added");
        ui.horizontal(|ui| {
            ui.label(format!("{name} ({kind})"));
            // A sub-assembly's mate is named with its path and is edited in its own file.
            let own = row.is_some();
            if own && ui.button(format!("Delete {name}")).clicked() {
                actions.push(MateAction::Delete(name.clone()));
            }
            let flip = match row.and_then(|m| m["alignment"].as_str()) {
                Some("aligned") => Some(Alignment::Opposed),
                Some("opposed") => Some(Alignment::Aligned),
                _ => None,
            };
            if let Some(to) = flip
                && ui.button(format!("Flip {name}")).clicked()
            {
                actions.push(MateAction::Flip(name.clone(), to));
            }
        });
    }
    if let Some(dof) = c.remaining_dof {
        ui.label(format!("{dof} freedoms would remain"));
    }
    for m in &c.free {
        ui.label(m.describe());
    }
    if ui.button("Dismiss").clicked() {
        actions.push(MateAction::Dismiss);
    }
}
