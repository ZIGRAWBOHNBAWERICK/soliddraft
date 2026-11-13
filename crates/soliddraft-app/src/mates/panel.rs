//! The Mate panel: picked faces held parallel, at right angles, on one plane, at a
//! distance or within limits, on one axis, tangent, hinged, mirrored, coupled, centred as
//! profiles, as a tab in a groove or a pin in a slot, two components locked together, the
//! assembly's values, what the solver says, and the conflicts a refusal named. Everything
//! goes through the same commands as the MCP tools.

use crate::assembly::AssemblyState;
use crate::mates::tools::rows;
use crate::state::{AppState, NoteKind};
use crate::theme;
use soliddraft_assembly::{
    Alignment, ComponentId, EndpointSelector, FaceSelector, MateEdit, MateRefusal, OccurrencePath, SlotInput,
    WidthInput,
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
    Gear,
    RackAndPinion,
    Screw,
    Angle,
    LimitAngle,
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
    /// An entity, its mirror image, then the symmetry plane.
    Symmetric,
    /// Two planes or axes whose slides are coupled by a ratio.
    LinearCoupler,
    /// A distance held between a least and a greatest value.
    LimitDistance,
    /// Two profiles, rectangles, regular polygons or circles, centred on each other.
    ProfileCenter,
    /// A wall or round end of a slot, then the pin's cylinder.
    Slot,
}

impl Family {
    const ALL: [Family; 19] = [
        Family::Gear,
        Family::RackAndPinion,
        Family::Screw,
        Family::Angle,
        Family::LimitAngle,
        Family::Coincident,
        Family::Distance,
        Family::Parallel,
        Family::Perpendicular,
        Family::Width,
        Family::Lock,
        Family::Concentric,
        Family::Tangent,
        Family::Hinge,
        Family::Symmetric,
        Family::LinearCoupler,
        Family::LimitDistance,
        Family::ProfileCenter,
        Family::Slot,
    ];

    fn label(self) -> &'static str {
        match self {
            Family::Gear => "Gear",
            Family::RackAndPinion => "Rack and Pinion",
            Family::Screw => "Screw",
            Family::Angle => "Angle",
            Family::LimitAngle => "Limit Angle",
            Family::Coincident => "Coincident",
            Family::Distance => "Distance",
            Family::Parallel => "Parallel",
            Family::Perpendicular => "Perpendicular",
            Family::Width => "Width",
            Family::Lock => "Lock",
            Family::Concentric => "Concentric",
            Family::Tangent => "Tangent",
            Family::Hinge => "Hinge",
            Family::Symmetric => "Symmetric",
            Family::LinearCoupler => "Linear Coupler",
            Family::LimitDistance => "Limit Distance",
            Family::ProfileCenter => "Profile Center",
            Family::Slot => "Slot",
        }
    }

    /// How many faces it needs.
    fn picks(self) -> usize {
        match self {
            Family::Gear | Family::Screw => 0,
            Family::RackAndPinion => 1,
            Family::Width | Family::Hinge => 4,
            Family::Angle | Family::LimitAngle | Family::Symmetric => 3,
            _ => 2,
        }
    }

    fn has_alignment(self) -> bool {
        !matches!(
            self,
            Family::Gear
                | Family::RackAndPinion
                | Family::Screw
                | Family::Angle
                | Family::LimitAngle
                | Family::Perpendicular
                | Family::Lock
                | Family::LinearCoupler
        )
    }
}

/// Where the panel's Width puts the tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidthMode {
    Free,
    Centered,
    Dimension,
    Percent,
}

/// Where the panel's Slot puts the pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotMode {
    Free,
    Center,
    Distance,
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
    pub joint_a: String,
    pub joint_b: String,
    pub travel: String,
    pub alignment: Alignment,
    /// A Hinge's planes' alignment.
    pub plane_alignment: Alignment,
    pub hinge_limits: bool,
    /// A Concentric holds the turn about its axis.
    pub lock_rotation: bool,
    pub name: String,
    pub distance: String,
    pub width_mode: WidthMode,
    pub slot_mode: SlotMode,
    pub percent: String,
    pub flipped: bool,
    /// A Limit Distance's least and greatest distances.
    pub min: String,
    pub max: String,
    /// A Linear Coupler's ratio: the first part slides this much ...
    pub ratio_a: String,
    /// ... while the second slides this much.
    pub ratio_b: String,
    /// A Linear Coupler's second part slides the other way.
    pub reverse: bool,
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
            joint_a: String::new(),
            joint_b: String::new(),
            travel: "10".into(),
            alignment: Alignment::Opposed,
            plane_alignment: Alignment::Opposed,
            hinge_limits: false,
            lock_rotation: false,
            name: String::new(),
            distance: "0 mm".into(),
            width_mode: WidthMode::Centered,
            slot_mode: SlotMode::Free,
            percent: "50".into(),
            flipped: false,
            min: "0 mm".into(),
            max: "10 mm".into(),
            ratio_a: "1".into(),
            ratio_b: "1".into(),
            reverse: false,
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

fn percent(d: &Draft) -> Result<f64, String> {
    d.percent.trim().trim_end_matches('%').trim().parse::<f64>().map_err(|_| "the percentage is a number".into())
}

fn width_input(d: &Draft) -> Result<WidthInput, String> {
    Ok(match d.width_mode {
        WidthMode::Free => WidthInput::Free,
        WidthMode::Centered => WidthInput::Centered,
        WidthMode::Dimension => WidthInput::Dimension { distance: d.distance.clone(), flipped: d.flipped },
        WidthMode::Percent => WidthInput::Percent { percent: percent(d)?, flipped: d.flipped },
    })
}

fn slot_input(d: &Draft) -> Result<SlotInput, String> {
    Ok(match d.slot_mode {
        SlotMode::Free => SlotInput::Free,
        SlotMode::Center => SlotInput::Center,
        SlotMode::Distance => SlotInput::Distance { distance: d.distance.clone(), flipped: d.flipped },
        SlotMode::Percent => SlotInput::Percent { percent: percent(d)?, flipped: d.flipped },
    })
}

fn ratio(d: &Draft) -> Result<[f64; 2], String> {
    let number = |t: &str| t.trim().parse::<f64>().map_err(|_| "each side of the ratio is a number".to_owned());
    Ok([number(&d.ratio_a)?, number(&d.ratio_b)?])
}

fn add(state: &mut AppState) {
    let family = state.assembly.mate.family;
    if matches!(family, Family::Gear | Family::RackAndPinion | Family::Screw) {
        add_mechanical(state);
        return;
    }
    let name = match state.assembly.mate.name.trim() {
        "" => next_name(&state.assembly, family.label()),
        n => n.to_owned(),
    };
    let (picked, distance, alignment) = {
        let d = &state.assembly.mate;
        let picked: Vec<PickedFace> =
            [&d.a, &d.b, &d.c, &d.d].into_iter().take(family.picks()).flatten().cloned().collect();
        (picked, d.distance.clone(), d.alignment)
    };
    let d = &state.assembly.mate;
    let (plane_alignment, lock_rotation, reverse) = (d.plane_alignment, d.lock_rotation, d.reverse);
    let hinge_limits = d.hinge_limits;
    let (limits, width, slot, ratio) = (
        [d.min.clone(), d.max.clone()],
        (family == Family::Width).then(|| width_input(d)),
        (family == Family::Slot).then(|| slot_input(d)),
        (family == Family::LinearCoupler).then(|| ratio(d)),
    );
    if picked.len() < family.picks() {
        let wanted = match family {
            Family::Width => "Pick the groove's two faces, then the tab's",
            Family::Hinge => "Pick the two axes' faces, then the two planes",
            Family::Symmetric => "Pick the face, its mirror image, then the symmetry plane",
            Family::Slot => "Pick a wall or round end of the slot, then the pin",
            _ => "Pick two faces first",
        };
        state.assembly.mate.error = Some(wanted.into());
        return;
    }
    let (width, slot, ratio) = match (width.transpose(), slot.transpose(), ratio.transpose()) {
        (Ok(w), Ok(s), Ok(r)) => (w, s, r),
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => {
            state.assembly.mate.error = Some(e);
            return;
        }
    };
    let end = |p: &PickedFace| EndpointSelector { component: p.component, face: FaceSelector::Picked(p.face.clone()) };
    let ends: Vec<EndpointSelector> = picked.iter().map(end).collect();
    let Some(s) = state.assembly.session.as_mut() else { return };
    let (a, b) = (&ends[0], &ends[1]);
    let done = match family {
        Family::Gear | Family::RackAndPinion | Family::Screw => unreachable!("handled before face pairs"),
        Family::Angle => match distance.parse::<f64>() {
            Ok(v) => s.doc.mate_angle(&name, a, b, &ends[2], v, false),
            Err(_) => {
                Err(MateRefusal::new(soliddraft_assembly::RefusalCode::InvalidValue, "Angle takes signed degrees"))
            }
        },
        Family::LimitAngle => match (limits[0].parse::<f64>(), limits[1].parse::<f64>()) {
            (Ok(min), Ok(max)) => s.doc.mate_limit_angle(&name, a, b, &ends[2], [min, max], false),
            _ => Err(MateRefusal::new(
                soliddraft_assembly::RefusalCode::InvalidValue,
                "Limit Angle takes signed degrees",
            )),
        },
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
        Family::Hinge => {
            let stops = if hinge_limits {
                match (limits[0].parse::<f64>(), limits[1].parse::<f64>()) {
                    (Ok(lo), Ok(hi)) => Some([lo, hi]),
                    _ => {
                        state.assembly.mate.error = Some("Hinge stops take signed degrees".into());
                        return;
                    }
                }
            } else {
                None
            };
            s.doc.mate_hinge_limited(&name, [a, b], [&ends[2], &ends[3]], [alignment, plane_alignment], stops, false)
        }
        Family::Symmetric => s.doc.mate_symmetric(&name, [a, b, &ends[2]], alignment, false),
        Family::LinearCoupler => {
            let ratio = ratio.expect("made above");
            s.doc.mate_linear_coupler(&name, a, b, [None, None], ratio, reverse, false)
        }
        Family::LimitDistance => {
            s.doc.mate_limit_distance(&name, a, b, [limits[0].as_str(), limits[1].as_str()], alignment, false)
        }
        Family::ProfileCenter => s.doc.mate_profile_center(&name, a, b, alignment, &distance, lock_rotation, false),
        Family::Slot => {
            let slot = slot.expect("made above");
            s.doc.mate_slot(&name, a, b, alignment, &slot, lock_rotation, false)
        }
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
        C::UnsupportedReference => "An axis, point, profile or slot cannot be proved for this mate",
        _ => "Refused",
    }
}

fn labelled(ui: &mut egui::Ui, value: &mut String, hint: &str, name: &'static str) {
    ui.add(egui::TextEdit::singleline(value).hint_text(hint))
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, name));
}

fn choose_family(draft: &mut Draft, family: Family) {
    draft.family = family;
    if family == Family::Angle {
        draft.distance = "30".into();
    }
    if matches!(family, Family::LimitAngle | Family::Hinge) {
        draft.min = "-30".into();
        draft.max = "30".into();
    }
    if family == Family::LimitDistance {
        draft.min = "0 mm".into();
        draft.max = "10 mm".into();
    }
}

/// Mate uses the feature form's fixed actions and overflow sheet in Edit Suite.
pub fn show_suite(ui: &mut egui::Ui, state: &mut AppState) {
    use crate::form_bar::{Field, Form};
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    enum Key {
        Type,
        Picks,
        Alignment,
        Value,
        Details,
    }
    let draft = &state.assembly.mate;
    let mut fields = vec![Field::new(Key::Type, "Type", "Mate", 240.0), Field::new(Key::Picks, "Faces", "Mate", 140.0)];
    if draft.family.has_alignment() {
        fields.push(Field::new(Key::Alignment, "", "Mate", 90.0));
    }
    if matches!(draft.family, Family::Distance | Family::ProfileCenter | Family::Angle) {
        fields.push(Field::new(
            Key::Value,
            if draft.family == Family::Angle { "Degrees" } else { "Value" },
            "Mate",
            180.0,
        ));
    }
    fields.push(Field::new(Key::Details, "", "Mate", 0.0).sheet());
    let error = draft.error.clone();
    let mut bar = std::mem::take(&mut state.form_bar);
    let response = crate::form_bar::show(
        ui,
        &mut bar,
        Form {
            key: egui::Id::new("assembly_mate_form"),
            title: "Mate",
            icon: crate::icons::Icon::Tree,
            fields: &fields,
            can_confirm: true,
            error: error.as_deref(),
        },
        |ui, key| {
            if key == Key::Details {
                details(ui, state);
                return false;
            }
            let draft = &mut state.assembly.mate;
            match key {
                Key::Type => {
                    egui::ComboBox::from_id_salt("mate_family").selected_text(draft.family.label()).show_ui(ui, |ui| {
                        for family in Family::ALL {
                            if ui.selectable_label(draft.family == family, family.label()).clicked()
                                && draft.family != family
                            {
                                choose_family(draft, family);
                            }
                        }
                    });
                }
                Key::Picks => {
                    let count =
                        [&draft.a, &draft.b, &draft.c, &draft.d].into_iter().filter(|pick| pick.is_some()).count();
                    return ui
                        .button(format!("{count} picked"))
                        .on_hover_text("Pick faces in the model; open the sheet to inspect or clear them")
                        .clicked();
                }
                Key::Alignment => {
                    let mut flipped = draft.alignment == Alignment::Aligned;
                    if ui.checkbox(&mut flipped, "Flip").changed() {
                        draft.alignment = if flipped { Alignment::Aligned } else { Alignment::Opposed };
                    }
                }
                Key::Value => {
                    let (cell, _) =
                        ui.allocate_exact_size(egui::vec2(ui.available_width().min(130.0), 24.0), egui::Sense::hover());
                    let response = crate::form_bar::value_cell(ui, cell, "", |ui, inner| {
                        ui.put(inner, egui::TextEdit::singleline(&mut draft.distance).frame(egui::Frame::NONE))
                    });
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Mate value"));
                }
                Key::Details => unreachable!(),
            }
            false
        },
    );
    state.form_bar = bar;
    let shortcuts = !egui::Popup::is_any_open(ui.ctx())
        && !ui.ctx().egui_wants_keyboard_input()
        && !state.command_line.session_open;
    let confirm_key = shortcuts && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let cancel_key = shortcuts && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    if response.cancel || cancel_key {
        state.form_bar.reset();
        state.push(crate::state::AppAction::Assembly(crate::assembly::AssemblyAction::Mate(MateAction::Toggle)));
        state.options_details = false;
    } else if response.confirm || confirm_key {
        state.push(crate::state::AppAction::Assembly(crate::assembly::AssemblyAction::Mate(MateAction::Add)));
    }
}

fn details(ui: &mut egui::Ui, state: &mut AppState) {
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

    let face = |p: &Option<PickedFace>| {
        p.as_ref().map_or("Pick a face".to_owned(), |p| format!("{}: {}", p.component, p.face.label))
    };
    let slots: &[&str] = match d.family {
        Family::Gear | Family::Screw => &[],
        Family::RackAndPinion => &["Rack direction"],
        Family::Width => &["Groove face", "Groove face", "Tab face", "Tab face"],
        Family::Lock => &["First component", "Second component"],
        Family::Tangent => &["Plane or curved face", "Curved face"],
        Family::Hinge => &["First axis", "Second axis", "First plane", "Second plane"],
        Family::Angle | Family::LimitAngle => &["First direction", "Second direction", "Positive rotation direction"],
        Family::Symmetric => &["Face", "Mirror image", "Symmetry plane"],
        Family::Slot => &["Slot", "Pin"],
        _ => &["First face", "Second face"],
    };
    for (label, slot) in slots.iter().zip([&d.a, &d.b, &d.c, &d.d]) {
        ui.label(format!("{label}  {}", face(slot)));
    }

    if d.family == Family::Hinge {
        ui.checkbox(&mut d.hinge_limits, "Angular stops");
        ui.horizontal(|ui| {
            ui.label("Planes");
            ui.selectable_value(&mut d.plane_alignment, Alignment::Opposed, "Planes opposed");
            ui.selectable_value(&mut d.plane_alignment, Alignment::Aligned, "Planes aligned");
        });
    }
    if matches!(d.family, Family::Concentric | Family::ProfileCenter | Family::Slot) {
        ui.checkbox(&mut d.lock_rotation, "Lock rotation");
    }
    labelled(ui, &mut d.name, "Name", "Mate name");
    if d.family == Family::Width {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut d.width_mode, WidthMode::Free, "Free");
            ui.selectable_value(&mut d.width_mode, WidthMode::Centered, "Centred");
            ui.selectable_value(&mut d.width_mode, WidthMode::Dimension, "Gap");
            ui.selectable_value(&mut d.width_mode, WidthMode::Percent, "Percent");
        });
        match d.width_mode {
            WidthMode::Free | WidthMode::Centered => {}
            WidthMode::Dimension => labelled(ui, &mut d.distance, "Gap, such as 1 mm", "Mate distance"),
            WidthMode::Percent => labelled(ui, &mut d.percent, "Percent of the play", "Mate percent"),
        }
        if matches!(d.width_mode, WidthMode::Dimension | WidthMode::Percent) {
            ui.checkbox(&mut d.flipped, "From the second groove face");
        }
    }
    if d.family == Family::Slot {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut d.slot_mode, SlotMode::Free, "Free");
            ui.selectable_value(&mut d.slot_mode, SlotMode::Center, "Centre in slot");
            ui.selectable_value(&mut d.slot_mode, SlotMode::Distance, "Distance along slot");
            ui.selectable_value(&mut d.slot_mode, SlotMode::Percent, "Percent along slot");
        });
        match d.slot_mode {
            SlotMode::Free | SlotMode::Center => {}
            SlotMode::Distance => labelled(ui, &mut d.distance, "Distance, such as 4 mm", "Mate distance"),
            SlotMode::Percent => labelled(ui, &mut d.percent, "Percent of the run", "Mate percent"),
        }
        if matches!(d.slot_mode, SlotMode::Distance | SlotMode::Percent) {
            ui.checkbox(&mut d.flipped, "From the second end");
        }
    }

    if d.family == Family::LimitAngle || (d.family == Family::Hinge && d.hinge_limits) {
        labelled(ui, &mut d.min, "Least signed degrees", "Mate least degrees");
        labelled(ui, &mut d.max, "Greatest signed degrees", "Mate greatest degrees");
    }

    if d.family == Family::LimitDistance {
        labelled(ui, &mut d.min, "Least distance, such as 0 mm", "Mate least distance");
        labelled(ui, &mut d.max, "Greatest distance, such as 10 mm", "Mate greatest distance");
    }
    if matches!(d.family, Family::Gear | Family::RackAndPinion | Family::Screw) {
        labelled(ui, &mut d.joint_a, "Existing joint name", "First joint name");
        if d.family == Family::Gear {
            labelled(ui, &mut d.joint_b, "Second existing joint name", "Second joint name");
        } else {
            labelled(ui, &mut d.travel, "Millimetres per revolution", "Travel per turn");
        }
        if d.family != Family::Gear {
            ui.checkbox(&mut d.reverse, "Reverse motion");
        }
    }
    if matches!(d.family, Family::LinearCoupler | Family::Gear) {
        ui.horizontal(|ui| {
            let gear = d.family == Family::Gear;
            labelled(ui, &mut d.ratio_a, if gear { "First turns" } else { "First slides" }, "Mate ratio first");
            ui.label(":");
            labelled(ui, &mut d.ratio_b, if gear { "Second turns" } else { "Second slides" }, "Mate ratio second");
        });
        ui.checkbox(&mut d.reverse, "Reverse");
    }
    ui.horizontal(|ui| {
        if ui.button("Clear faces").clicked() {
            actions.push(MateAction::Clear);
        }
        if let Some(c) = selected
            && ui.button("Origin mate").clicked()
        {
            actions.push(MateAction::AddOrigin(c));
        }
    });
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
        if m["proof"]["certainty"].as_str() == Some("bounded") {
            ui.small(format!("{name}: bounded to {} mm and {} rad", m["proof"]["bound_mm"], m["proof"]["bound_angle"]));
        }
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
            Some("gear") => format!("{name}: gear ratio {} : {}", m["factor"][0], m["factor"][1]),
            Some("rack_and_pinion" | "screw") => format!("{name}: {} mm per turn", m["factor"][0]),
            Some("angle") => format!("{name}: {} degrees", m["degrees"]),
            Some("limit_angle") => format!("{name}: {} to {} degrees", m["min_degrees"], m["max_degrees"]),
            Some("width") => {
                let how = match m["width"]["mode"].as_str() {
                    Some("dimension") => shown(m["width"]["distance"]["value"].as_f64(), " gap"),
                    Some("percent") => {
                        format!("{}% of the play", m["width"]["percent"].as_f64().unwrap_or(f64::NAN))
                    }
                    Some("free") => "free".to_owned(),
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
            Some("symmetric") => match m["alignment"].as_str() {
                Some(a) => format!("{name}: symmetric, {a}"),
                None => format!("{name}: symmetric"),
            },
            Some("linear_coupler") => {
                let ratio = &m["ratio"];
                let reverse = if m["reverse"].as_bool() == Some(true) { ", reversed" } else { "" };
                format!("{name}: coupled {} : {}{reverse}", ratio[0], ratio[1])
            }
            Some("limit_distance") => format!(
                "{name}: from {} to {}, {}",
                shown(m["min"]["value"].as_f64(), ""),
                shown(m["max"]["value"].as_f64(), ""),
                shown(gap(m), " now")
            ),
            Some("profile_center") => {
                let locked = if m["lock_rotation"].as_bool() == Some(true) { ", rotation locked" } else { "" };
                format!(
                    "{name}: profile centre, {alignment}, {}{locked}",
                    shown(m["distance"]["value"].as_f64(), " apart")
                )
            }
            Some("slot") => {
                let how = match m["position"]["mode"].as_str() {
                    Some("center") => "centred".to_owned(),
                    Some("distance") => shown(m["position"]["distance"]["value"].as_f64(), " along"),
                    Some("percent") => format!("{}% along", m["position"]["percent"].as_f64().unwrap_or(f64::NAN)),
                    _ => "free".to_owned(),
                };
                let locked = if m["lock_rotation"].as_bool() == Some(true) { ", rotation locked" } else { "" };
                format!("{name}: in the slot, {how}{locked}")
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

fn add_mechanical(state: &mut AppState) {
    let d = &state.assembly.mate;
    let family = d.family;
    let name =
        if d.name.trim().is_empty() { next_name(&state.assembly, family.label()) } else { d.name.trim().to_owned() };
    let (a, b, reverse) = (d.joint_a.trim().to_owned(), d.joint_b.trim().to_owned(), d.reverse);
    let ratio = ratio(d);
    let travel = d.travel.trim().parse::<f64>();
    let rack =
        d.a.as_ref().map(|p| EndpointSelector { component: p.component, face: FaceSelector::Picked(p.face.clone()) });
    let Some(session) = state.assembly.session.as_mut() else { return };
    let invalid = |message| MateRefusal::new(soliddraft_assembly::RefusalCode::InvalidValue, message);
    let result = match family {
        Family::Gear => match ratio {
            Ok(ratio) => session.doc.mate_gear(&name, [&a, &b], ratio, reverse, false),
            Err(message) => Err(invalid(message)),
        },
        Family::Screw => match travel {
            Ok(pitch) => session.doc.mate_screw(&name, &a, pitch, reverse, false),
            Err(_) => Err(invalid("Pitch takes a positive number of millimetres".into())),
        },
        Family::RackAndPinion => match (rack, travel) {
            (Some(rack), Ok(travel)) => {
                session.doc.mate_rack_and_pinion(&name, &a, &rack, None, travel, reverse, false)
            }
            _ => Err(invalid("Pick the rack direction and enter millimetres per turn".into())),
        },
        _ => unreachable!(),
    };
    report(state, result.map(|_| ()), "Mechanical mate added");
}
