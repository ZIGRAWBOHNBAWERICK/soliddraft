//! The Manufacturing information panel: the part's items as a drawing writes them, and
//! a form that puts a datum, a dimension, a slot, a geometric tolerance or a surface
//! finish on the faces picked in the viewport.

use soliddraft_kernel_api::pmi::{
    Anchor, At, DatumRef, Feature, GdtCharacteristic, MaterialCondition, PartNotes, PmiItem, PmiKind, SizeKind,
    SurfaceProcess, Tolerance,
};
use soliddraft_kernel_api::{FaceId, SurfaceKind};
use soliddraft_model::FaceRef;
use soliddraft_model::pmi::{PartPmi, PmiRow};

use crate::document::{Document, Selection};
use crate::state::AppState;
use crate::theme;

/// What the form adds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Datum,
    Diameter,
    Radius,
    Distance,
    Slot,
    Tolerance,
    Finish,
}

impl Kind {
    const ALL: [Kind; 7] =
        [Kind::Datum, Kind::Diameter, Kind::Radius, Kind::Distance, Kind::Slot, Kind::Tolerance, Kind::Finish];

    fn label(self) -> &'static str {
        match self {
            Kind::Datum => "Datum",
            Kind::Diameter => "Diameter",
            Kind::Radius => "Radius",
            Kind::Distance => "Distance",
            Kind::Slot => "Slot",
            Kind::Tolerance => "Geometric tolerance",
            Kind::Finish => "Surface finish",
        }
    }

    /// What to pick for this kind of item.
    pub fn prompt(self) -> &'static str {
        match self {
            Kind::Datum => "Pick the face that is the datum.",
            Kind::Diameter | Kind::Radius => "Pick the round face, or each round face of a pattern.",
            Kind::Distance => "Pick the faces it starts on, press From, then pick the faces it ends on.",
            Kind::Slot => "Pick the slot's two flat sides and its two round ends.",
            Kind::Tolerance => "Pick the face it applies to, or each face of a pattern.",
            Kind::Finish => "Pick the face the finish applies to.",
        }
    }
}

const ENDS: [(At, &str); 4] =
    [(At::Feature, "Face or axis"), (At::Middle, "Middle"), (At::Inner, "Near side"), (At::Outer, "Far side")];

/// The rows last listed, and what they were listed from.
struct Listed {
    geometry: soliddraft_document::ContentHash,
    pmi: PartPmi,
    rows: Vec<PmiRow>,
    /// The rows as a browser check reads them; see
    /// `web::shown_manufacturing_information`. Empty on the desktop.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    shown: String,
}

/// The listed rows for a browser check: each item with its callout and its faces, a
/// face by its name, its surface and its centroid to a micrometre.
fn shown(doc: &Document, rows: &[PmiRow]) -> String {
    let face = |face: &FaceId| -> String {
        let name = crate::command_tools::name_of_face(doc, *face).unwrap_or_default();
        match doc.named_face(*face) {
            Some(named) => {
                let c = [named.centroid.x, named.centroid.y, named.centroid.z]
                    .map(|v| (f64::from(v) * 1000.0).round() / 1000.0 + 0.0);
                format!("{name} @ {:?} ({}, {}, {})", named.surface, c[0], c[1], c[2])
            }
            None => name,
        }
    };
    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let faces: Vec<Vec<String>> = row.on_body.as_ref().map_or_else(Vec::new, |on_body| {
                on_body.features().into_iter().map(|feature| feature.iter().map(face).collect()).collect()
            });
            let mut item = serde_json::json!({ "name": row.item.id, "callout": row.callout(), "faces": faces });
            if let Some(problem) = &row.problem {
                item["problem"] = serde_json::json!(problem);
            }
            item
        })
        .collect();
    serde_json::json!({ "items": items, "notes": doc.model().part_notes() }).to_string()
}

pub struct State {
    pub open: bool,
    /// The faces picked for the item being made, in order.
    pub picks: Vec<FaceId>,
    /// The faces a distance starts on.
    from: Vec<FaceId>,
    /// The selection seen last frame: a new face selection is a pick.
    seen: Option<Selection>,
    pub kind: Kind,
    name: String,
    letter: String,
    tolerance: String,
    through: bool,
    /// Several picked faces are one feature each, such as four equal holes.
    pattern: bool,
    from_at: At,
    to_at: At,
    characteristic: GdtCharacteristic,
    value: String,
    diameter: bool,
    material: MaterialCondition,
    datums: String,
    roughness: String,
    process: SurfaceProcess,
    /// The item the form replaces, when it was filled from one.
    replaces: Option<String>,
    error: Option<String>,
    notes: Option<NotesDraft>,
    listed: Option<Listed>,
}

/// The part's notes while they are typed.
struct NotesDraft {
    of: PartNotes,
    general_tolerance: String,
    material: String,
    notes: String,
}

impl NotesDraft {
    fn of(notes: &PartNotes) -> NotesDraft {
        NotesDraft {
            of: notes.clone(),
            general_tolerance: notes.general_tolerance.clone().unwrap_or_default(),
            material: notes.material.clone().unwrap_or_default(),
            notes: notes.notes.join("\n"),
        }
    }

    fn typed(&self) -> PartNotes {
        let text = |value: &str| Some(value.trim().to_owned()).filter(|value| !value.is_empty());
        PartNotes {
            general_tolerance: text(&self.general_tolerance),
            material: text(&self.material),
            notes: self.notes.lines().filter_map(text).collect(),
        }
    }
}

impl Default for State {
    fn default() -> Self {
        State {
            open: false,
            picks: Vec::new(),
            from: Vec::new(),
            seen: None,
            kind: Kind::Datum,
            name: String::new(),
            letter: String::new(),
            tolerance: String::new(),
            through: false,
            pattern: true,
            from_at: At::Feature,
            to_at: At::Feature,
            characteristic: GdtCharacteristic::Position,
            value: String::new(),
            diameter: false,
            material: MaterialCondition::Regardless,
            datums: String::new(),
            roughness: String::new(),
            process: SurfaceProcess::Any,
            replaces: None,
            error: None,
            notes: None,
            listed: None,
        }
    }
}

impl State {
    /// Empties the form for the next item, keeping its kind.
    fn clear(&mut self) {
        let (open, kind) = (self.open, self.kind);
        let (seen, notes, listed) = (self.seen, self.notes.take(), self.listed.take());
        *self = State { open, kind, seen, notes, listed, ..State::default() };
    }

    /// Fills the form from an item of the part, to change it.
    fn load(&mut self, row: &PmiRow) {
        self.clear();
        self.replaces = Some(row.item.id.clone());
        self.name.clone_from(&row.item.id);
        let Some(on_body) = &row.on_body else { return };
        let all = |features: &[Feature<FaceId>]| features.iter().flatten().copied().collect::<Vec<_>>();
        match &on_body.kind {
            PmiKind::Datum { letter, feature } => {
                self.kind = Kind::Datum;
                self.letter.clone_from(letter);
                self.picks.clone_from(feature);
            }
            PmiKind::Size { size, features, tolerance, through, .. } => {
                self.kind = if *size == SizeKind::Diameter { Kind::Diameter } else { Kind::Radius };
                self.tolerance = tolerance.suffix().trim().to_owned();
                self.through = *through;
                self.pattern = features.len() > 1;
                self.picks = all(features);
            }
            PmiKind::Location { from, to, tolerance, .. } => {
                self.kind = Kind::Distance;
                self.tolerance = tolerance.suffix().trim().to_owned();
                self.from.clone_from(&from.feature);
                self.from_at = from.at;
                self.picks.clone_from(&to.feature);
                self.to_at = to.at;
            }
            PmiKind::Slot { flats, ends, tolerance } => {
                self.kind = Kind::Slot;
                self.tolerance = tolerance.suffix().trim().to_owned();
                self.picks = flats.iter().chain(ends).flatten().copied().collect();
            }
            PmiKind::Gdt { characteristic, tolerance, diameter, material, datums, features } => {
                self.kind = Kind::Tolerance;
                self.characteristic = *characteristic;
                self.value = soliddraft_kernel_api::pmi::length_text(*tolerance);
                self.diameter = *diameter;
                self.material = *material;
                self.datums = datums.iter().map(|datum| datum.letter.as_str()).collect::<Vec<_>>().join(" ");
                self.pattern = features.len() > 1;
                self.picks = all(features);
            }
            PmiKind::SurfaceFinish { text, process, feature, .. } => {
                self.kind = Kind::Finish;
                self.roughness.clone_from(text);
                self.process = *process;
                self.picks.clone_from(feature);
            }
        }
    }
}

fn face_title(doc: &Document, face: FaceId) -> String {
    crate::command_tools::name_of_face(doc, face).unwrap_or_else(|| "A face".to_owned())
}

fn feature(doc: &Document, faces: &[FaceId]) -> Result<Feature<FaceRef>, String> {
    doc.model().pmi_feature(faces).map_err(|error| error.to_string())
}

/// One feature per picked face for a pattern, else all picked faces as one feature.
fn features(doc: &Document, faces: &[FaceId], pattern: bool) -> Result<Vec<Feature<FaceRef>>, String> {
    if pattern && faces.len() > 1 {
        faces.iter().map(|face| feature(doc, &[*face])).collect()
    } else {
        Ok(vec![feature(doc, faces)?])
    }
}

fn typed_tolerance(text: &str) -> Result<Tolerance, String> {
    if text.trim().is_empty() {
        return Ok(Tolerance::None);
    }
    Tolerance::parse(text).ok_or_else(|| "Write the tolerance as H7, \u{b1}0.1 or +0.1/0.".to_owned())
}

/// The item the form describes, on the part's faces.
fn item(doc: &Document, form: &State) -> Result<PmiItem<FaceRef>, String> {
    let pmi = doc.model().pmi();
    if form.picks.is_empty() {
        return Err(form.kind.prompt().to_owned());
    }
    let kind = match form.kind {
        Kind::Datum => PmiKind::Datum {
            letter: Some(form.letter.trim().to_ascii_uppercase())
                .filter(|letter| !letter.is_empty())
                .unwrap_or_else(|| pmi.next_datum_letter()),
            feature: feature(doc, &form.picks)?,
        },
        Kind::Diameter | Kind::Radius => PmiKind::Size {
            size: if form.kind == Kind::Diameter { SizeKind::Diameter } else { SizeKind::Radius },
            features: features(doc, &form.picks, form.pattern)?,
            tolerance: typed_tolerance(&form.tolerance)?,
            through: form.kind == Kind::Diameter && form.through,
            nominal: None,
        },
        Kind::Distance => {
            if form.from.is_empty() {
                return Err("Pick the faces it starts on and press From.".to_owned());
            }
            PmiKind::Location {
                from: Anchor { feature: feature(doc, &form.from)?, at: form.from_at },
                to: Anchor { feature: feature(doc, &form.picks)?, at: form.to_at },
                tolerance: typed_tolerance(&form.tolerance)?,
                nominal: None,
            }
        }
        Kind::Slot => {
            let of = |kind: SurfaceKind| -> Vec<FaceId> {
                form.picks
                    .iter()
                    .copied()
                    .filter(|face| doc.named_face(*face).is_some_and(|named| named.surface == kind))
                    .collect()
            };
            let (flats, ends) = (of(SurfaceKind::Plane), of(SurfaceKind::Cylinder));
            if flats.len() != 2 || ends.len() != 2 || form.picks.len() != 4 {
                return Err("A slot is two flat sides and two round ends.".to_owned());
            }
            PmiKind::Slot {
                flats: [feature(doc, &flats[..1])?, feature(doc, &flats[1..])?],
                ends: [feature(doc, &ends[..1])?, feature(doc, &ends[1..])?],
                tolerance: typed_tolerance(&form.tolerance)?,
            }
        }
        Kind::Tolerance => PmiKind::Gdt {
            characteristic: form.characteristic,
            tolerance: form
                .value
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .ok_or("Write the tolerance as a number, such as 0.05.")?,
            diameter: form.diameter,
            material: form.material,
            datums: form
                .datums
                .split(|c: char| c == '|' || c == ',' || c.is_whitespace())
                .filter(|letter| !letter.is_empty())
                .map(|letter| DatumRef::new(&letter.to_ascii_uppercase()))
                .collect(),
            features: features(doc, &form.picks, form.pattern)?,
        },
        Kind::Finish => PmiKind::SurfaceFinish {
            text: form.roughness.trim().to_owned(),
            process: form.process,
            method: String::new(),
            lay: None,
            allowance: String::new(),
            all_around: false,
            feature: feature(doc, &form.picks)?,
        },
    };
    let mut item = PmiItem { id: String::new(), kind };
    item.id = match (form.name.trim(), &form.replaces) {
        ("", Some(old)) => old.clone(),
        ("", None) => pmi.free_name(&match &item.kind {
            PmiKind::Datum { letter, .. } => format!("Datum {letter}"),
            _ => item.kind_label().to_owned(),
        }),
        (name, _) => name.to_owned(),
    };
    Ok(item)
}

/// Follows the viewport while the panel is open: a face selected since last frame joins
/// the picks.
pub fn follow_selection(state: &mut AppState) {
    if !state.pmi.open {
        return;
    }
    let selection = state.doc.selection;
    let form = &mut state.pmi;
    if selection == form.seen {
        return;
    }
    form.seen = selection;
    if let Some(Selection::Face(face)) = selection
        && !form.picks.contains(&face)
    {
        form.picks.push(face);
        form.error = None;
    }
}

/// The part's rows, listed again only when the part or its information changed.
fn rows(state: &mut AppState) -> Vec<PmiRow> {
    let geometry = state.doc.model().geometry_hash();
    let pmi = state.doc.model().pmi();
    let current = state.pmi.listed.as_ref().is_some_and(|listed| listed.geometry == geometry && &listed.pmi == pmi);
    if !current || state.doc.proof_pending() {
        let rows = state.doc.model().pmi_rows();
        let shown = if cfg!(target_arch = "wasm32") { shown(&state.doc, &rows) } else { String::new() };
        let listed = Listed { geometry, pmi: pmi.clone(), rows, shown };
        state.pmi.listed = Some(listed);
    }
    state.pmi.listed.as_ref().map(|listed| listed.rows.clone()).unwrap_or_default()
}

fn labelled(response: egui::Response, label: &'static str) -> egui::Response {
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, label));
    response
}

fn picked(ui: &mut egui::Ui, doc: &Document, title: &str, faces: &mut Vec<FaceId>) {
    if faces.is_empty() {
        return;
    }
    ui.label(egui::RichText::new(title).color(theme::muted()));
    let mut removed = None;
    for (index, face) in faces.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(face_title(doc, *face));
            if ui.small_button("Remove").on_hover_text("Take this face out").clicked() {
                removed = Some(index);
            }
        });
    }
    if let Some(index) = removed {
        faces.remove(index);
    }
}

fn form(ui: &mut egui::Ui, state: &mut AppState) {
    let (doc, form) = (&mut state.doc, &mut state.pmi);
    ui.horizontal_wrapped(|ui| {
        for kind in Kind::ALL {
            ui.selectable_value(&mut form.kind, kind, kind.label());
        }
    });
    ui.label(egui::RichText::new(form.kind.prompt()).color(theme::muted()));
    if form.kind == Kind::Distance {
        picked(ui, doc, "From", &mut form.from);
        if !form.from.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for (at, label) in ENDS {
                    ui.selectable_value(&mut form.from_at, at, label);
                }
            });
        }
        if ui.button("From").on_hover_text("The picked faces are where the distance starts").clicked() {
            form.from = std::mem::take(&mut form.picks);
        }
    }
    picked(ui, doc, if form.kind == Kind::Distance { "To" } else { "Picked" }, &mut form.picks);
    match form.kind {
        Kind::Datum => {
            let hint = doc.model().pmi().next_datum_letter();
            labelled(
                ui.add(egui::TextEdit::singleline(&mut form.letter).hint_text(format!("Letter, {hint} if empty"))),
                "Datum letter",
            );
        }
        Kind::Diameter | Kind::Radius | Kind::Distance | Kind::Slot => {
            if form.kind == Kind::Distance && !form.picks.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for (at, label) in ENDS {
                        ui.selectable_value(&mut form.to_at, at, label);
                    }
                });
            }
            labelled(
                ui.add(egui::TextEdit::singleline(&mut form.tolerance).hint_text("Tolerance: H7, \u{b1}0.1 or +0.1/0")),
                "Tolerance",
            );
            if form.kind == Kind::Diameter {
                ui.checkbox(&mut form.through, "Through hole");
            }
            if matches!(form.kind, Kind::Diameter | Kind::Radius) && form.picks.len() > 1 {
                ui.checkbox(&mut form.pattern, "Each picked face is one of a pattern");
            }
        }
        Kind::Tolerance => {
            egui::ComboBox::from_id_salt("pmi_characteristic")
                .selected_text(form.characteristic.name())
                .show_ui(ui, |ui| {
                    for characteristic in GdtCharacteristic::ALL {
                        ui.selectable_value(&mut form.characteristic, characteristic, characteristic.name());
                    }
                })
                .response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Characteristic"));
            labelled(
                ui.add(egui::TextEdit::singleline(&mut form.value).hint_text("Tolerance in mm, such as 0.05")),
                "Tolerance value",
            );
            ui.checkbox(&mut form.diameter, "Cylindrical zone (\u{d8})");
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut form.material, MaterialCondition::Regardless, "No modifier");
                ui.selectable_value(&mut form.material, MaterialCondition::Maximum, "Maximum material");
                ui.selectable_value(&mut form.material, MaterialCondition::Least, "Least material");
            });
            labelled(
                ui.add(egui::TextEdit::singleline(&mut form.datums).hint_text("Datums, primary first: A B C")),
                "Datums",
            );
            if form.picks.len() > 1 {
                ui.checkbox(&mut form.pattern, "Each picked face is one of a pattern");
            }
        }
        Kind::Finish => {
            labelled(
                ui.add(egui::TextEdit::singleline(&mut form.roughness).hint_text("Roughness, such as Ra 1.6")),
                "Roughness",
            );
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut form.process, SurfaceProcess::Any, "Any process");
                ui.selectable_value(&mut form.process, SurfaceProcess::Removal, "Material removed");
                ui.selectable_value(&mut form.process, SurfaceProcess::NoRemoval, "No material removed");
            });
        }
    }
    labelled(ui.add(egui::TextEdit::singleline(&mut form.name).hint_text("Name, made up if empty")), "Item name");
    ui.horizontal(|ui| {
        let action = if form.replaces.is_some() { "Change" } else { "Add" };
        if ui.button(action).clicked() {
            match item(doc, form).and_then(|item| doc.put_pmi(item, form.replaces.clone())) {
                Ok(()) => form.clear(),
                Err(error) => form.error = Some(error),
            }
        }
        if ui.button("Clear").on_hover_text("Empty the form and the picked faces").clicked() {
            form.clear();
        }
    });
    if let Some(error) = &form.error {
        ui.colored_label(theme::error(), error);
    }
}

fn list(ui: &mut egui::Ui, state: &mut AppState, rows: &[PmiRow]) {
    if rows.is_empty() {
        ui.label(egui::RichText::new("The part has no manufacturing information yet.").color(theme::muted()));
        return;
    }
    for row in rows {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(row.callout()).family(theme::semibold()).color(theme::ink()));
            ui.label(egui::RichText::new(&row.item.id).color(theme::muted()));
        });
        if !row.located.is_empty() {
            let located: Vec<String> = row
                .located
                .iter()
                .map(|one| {
                    let value = soliddraft_kernel_api::pmi::length_text(one.value);
                    format!("{value} from {}", one.datum)
                })
                .collect();
            ui.label(egui::RichText::new(format!("Located: {}", located.join(", "))).color(theme::muted()));
        }
        if let Some(problem) = &row.problem {
            ui.colored_label(theme::error(), problem);
        }
        ui.horizontal(|ui| {
            if ui.small_button("Edit").on_hover_text(format!("Change {}", row.item.id)).clicked() {
                state.pmi.load(row);
            }
            if ui.small_button("Delete").on_hover_text(format!("Remove {}", row.item.id)).clicked() {
                state.pmi.error = state.doc.remove_pmi(&row.item.id).err();
            }
        });
    }
}

fn notes(ui: &mut egui::Ui, state: &mut AppState) {
    let current = state.doc.model().pmi().part.clone();
    if state.pmi.notes.as_ref().is_none_or(|draft| draft.of != current) {
        state.pmi.notes = Some(NotesDraft::of(&current));
    }
    let Some(draft) = state.pmi.notes.as_mut() else { return };
    labelled(
        ui.add(egui::TextEdit::singleline(&mut draft.general_tolerance).hint_text("General tolerance: ISO 2768-m")),
        "General tolerance",
    );
    labelled(
        ui.add(egui::TextEdit::singleline(&mut draft.material).hint_text("Material, as the drawing names it")),
        "Material note",
    );
    labelled(
        ui.add(egui::TextEdit::multiline(&mut draft.notes).desired_rows(2).hint_text("Other notes, one a line")),
        "Part notes",
    );
    let typed = draft.typed();
    if typed != current && ui.button("Apply notes").clicked() {
        state.pmi.error = state.doc.set_part_notes(typed).err();
    }
}

/// The panel, in the property panel while it is open.
pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    if !state.pmi.open {
        return;
    }
    follow_selection(state);
    theme::section_label(ui, "Manufacturing information");
    // Fields as wide as the sheet, so their hints read in full.
    ui.spacing_mut().text_edit_width = ui.available_width();
    form(ui, state);
    ui.separator();
    notes(ui, state);
    ui.separator();
    // The items take what is left of the sheet and scroll in it.
    let rows = rows(state);
    #[cfg(target_arch = "wasm32")]
    if let Some(listed) = &state.pmi.listed {
        crate::web::show_manufacturing_information(&listed.shown);
    }
    let height = ui.available_height().max(120.0);
    egui::ScrollArea::vertical().id_salt("pmi_items").auto_shrink([false, true]).max_height(height).show(ui, |ui| {
        list(ui, state, &rows);
        if let Some(report) = &state.doc.pmi_import
            && !report.left_out.is_empty()
        {
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Not read from the file").color(theme::muted()));
            for (name, reason) in &report.left_out {
                ui.label(format!("{name}: {reason}"));
            }
        }
    });
}
