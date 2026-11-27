//! The Manufacturing information panel: the part's items as a drawing writes them, and
//! a form that puts a datum, a dimension, a slot, a geometric tolerance or a surface
//! finish on the faces picked in the viewport.

use soliddraft_kernel_api::pmi::{
    Anchor, At, DatumRef, DatumTarget, Decimal, Feature, GdtCharacteristic, HoleHead, HoleHeadKind, HoleThread,
    MaterialCondition, PartNotes, PmiItem, PmiKind, PositionControl, PositionFrame, PositionSegment, ProjectedZone,
    SizeKind, SurfaceProcess, TargetArea, TargetKind, Tolerance,
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
    DatumTarget,
    Diameter,
    Radius,
    Hole,
    Distance,
    Slot,
    Width,
    Chamfer,
    EdgeBreak,
    BasicAngle,
    Tolerance,
    PositionControl,
    Finish,
}

impl Kind {
    const ALL: [Kind; 14] = [
        Kind::Datum,
        Kind::DatumTarget,
        Kind::Diameter,
        Kind::Radius,
        Kind::Hole,
        Kind::Distance,
        Kind::Slot,
        Kind::Width,
        Kind::Chamfer,
        Kind::EdgeBreak,
        Kind::BasicAngle,
        Kind::Tolerance,
        Kind::PositionControl,
        Kind::Finish,
    ];

    fn label(self) -> &'static str {
        match self {
            Kind::Datum => "Datum",
            Kind::DatumTarget => "Datum target",
            Kind::Diameter => "Diameter",
            Kind::Radius => "Radius",
            Kind::Hole => "Hole",
            Kind::Distance => "Distance",
            Kind::Slot => "Slot",
            Kind::Width => "Width",
            Kind::Chamfer => "Chamfer",
            Kind::EdgeBreak => "Edge break",
            Kind::BasicAngle => "Basic angle",
            Kind::Tolerance => "Geometric tolerance",
            Kind::PositionControl => "Position control",
            Kind::Finish => "Surface finish",
        }
    }

    /// What to pick for this kind of item.
    pub fn prompt(self) -> &'static str {
        match self {
            Kind::Datum => "Pick the face that is the datum.",
            Kind::DatumTarget => "Pick one face in the declared datum.",
            Kind::Diameter | Kind::Radius => "Pick the round face, or each round face of a pattern.",
            Kind::Hole => "Pick the head wall first if there is one, then pick the cylindrical pilot wall.",
            Kind::Distance => "Pick the faces it starts on, press From, then pick the faces it ends on.",
            Kind::Slot => "Pick the slot's two flat sides and its two round ends.",
            Kind::Width => "Pick the two opposed planar sides, assigning the first group before picking the second.",
            Kind::Chamfer | Kind::EdgeBreak => "Assign the bevel and the first support, then pick the other support.",
            Kind::BasicAngle => "Pick two angled planar groups, assigning the first before picking the second.",
            Kind::Tolerance => "Pick the face it applies to, or each face of a pattern.",
            Kind::PositionControl => {
                "Pick each cylindrical feature group. For a projected zone, also pick the entry faces in feature order."
            }
            Kind::Finish => "Pick the face the finish applies to.",
        }
    }
}

const ENDS: [(At, &str); 4] =
    [(At::Feature, "Face or axis"), (At::Middle, "Middle"), (At::Inner, "Near side"), (At::Outer, "Far side")];

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum TargetShape {
    #[default]
    Point,
    Line,
    Circular,
    Rectangular,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum FrameShape {
    #[default]
    Single,
    Composite,
    Projected,
}

#[derive(Default)]
struct SegmentDraft {
    value: String,
    material: MaterialCondition,
    datums: String,
}

impl SegmentDraft {
    fn of(segment: &PositionSegment) -> Self {
        Self {
            value: segment.value_mm.to_string(),
            material: segment.material,
            datums: segment
                .datums
                .iter()
                .map(|datum| {
                    if datum.material.is_regardless() {
                        datum.letter.clone()
                    } else {
                        format!("{}({})", datum.letter, datum.material.letter())
                    }
                })
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    fn typed(&self) -> Result<PositionSegment, String> {
        let datums = self
            .datums
            .split(|c: char| c == '|' || c == ',' || c.is_whitespace())
            .filter(|word| !word.is_empty())
            .map(|word| {
                let (letter, material) = if let Some(letter) = word.strip_suffix("(M)") {
                    (letter, MaterialCondition::Maximum)
                } else if let Some(letter) = word.strip_suffix("(L)") {
                    (letter, MaterialCondition::Least)
                } else {
                    (word, MaterialCondition::Regardless)
                };
                if !soliddraft_kernel_api::pmi::is_datum_letter(letter) {
                    return Err(format!("Invalid datum reference {word}; use A, A(M) or A(L)."));
                }
                Ok(DatumRef { letter: letter.into(), material })
            })
            .collect::<Result<_, String>>()?;
        Ok(PositionSegment {
            value_mm: decimal(&self.value, "zone diameter")?,
            diameter_zone: true,
            material: self.material,
            datums,
        })
    }

    fn show(&mut self, ui: &mut egui::Ui, title: &str) {
        ui.label(title);
        entry(ui, &mut self.value, "Zone diameter (mm)");
        entry(ui, &mut self.datums, "Datums (A B(M) C(L))");
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.material, MaterialCondition::Regardless, "RFS");
            ui.selectable_value(&mut self.material, MaterialCondition::Maximum, "MMC");
            ui.selectable_value(&mut self.material, MaterialCondition::Least, "LMC");
        });
    }
}

fn vector_text([x, y, z]: [f64; 3]) -> String {
    format!("{x}, {y}, {z}")
}

fn vector(text: &str, label: &str) -> Result<[f64; 3], String> {
    let mut values = text.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty());
    let [Some(x), Some(y), Some(z), None] = std::array::from_fn(|_| values.next()) else {
        return Err(format!("Enter three coordinates for {label}."));
    };
    let parse = |s: &str| {
        s.parse::<f64>().ok().filter(|n| n.is_finite()).ok_or_else(|| format!("Enter finite coordinates for {label}."))
    };
    Ok([parse(x)?, parse(y)?, parse(z)?])
}

fn decimal(text: &str, label: &str) -> Result<Decimal, String> {
    Decimal::parse(text).ok_or_else(|| format!("Enter the {label} as a decimal in mm."))
}

fn entry(ui: &mut egui::Ui, text: &mut String, label: &'static str) {
    ui.horizontal(|ui| {
        ui.label(format!("{label}:"));
        labelled(ui.add(egui::TextEdit::singleline(text)), label);
    });
}

/// The rows last listed, and what they were listed from.
struct Listed {
    geometry: soliddraft_document::ContentHash,
    pmi: PartPmi,
    rows: Vec<PmiRow>,
    /// The rows as a browser check reads them; see
    /// `web::shown_manufacturing_information`. Empty on the desktop.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    shown: String,
    /// The interview's open questions, worked out when the interview is first shown.
    questions: Option<Result<Vec<soliddraft_model::interview::Question>, String>>,
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
    /// The first group of a distance or planar dimension.
    from: Vec<FaceId>,
    bevel: Vec<FaceId>,
    min_mm: String,
    max_mm: String,
    /// The selection seen last frame: a new face selection is a pick.
    seen: Option<Selection>,
    pub kind: Kind,
    name: String,
    letter: String,
    tolerance: String,
    through: bool,
    /// A diameter's thread designation, such as `M6x1-6H`; empty for a plain one.
    thread: String,
    head_kind: Option<HoleHeadKind>,
    head_faces: Vec<FaceId>,
    drill_depth: String,
    thread_depth: String,
    /// Several picked faces are one feature each, such as four equal holes.
    pattern: bool,
    from_at: At,
    to_at: At,
    characteristic: GdtCharacteristic,
    value: String,
    diameter: bool,
    material: MaterialCondition,
    datums: String,
    target_shape: TargetShape,
    target_number: String,
    target_point: String,
    gdt_direction: String,
    gdt_length: String,
    gdt_width: String,
    frame_shape: FrameShape,
    upper: SegmentDraft,
    lower: SegmentDraft,
    position_groups: Vec<Feature<FaceId>>,
    entry_faces: Vec<FaceId>,
    roughness: String,
    process: SurfaceProcess,
    /// The item the form replaces, when it was filled from one.
    replaces: Option<String>,
    error: Option<String>,
    notes: Option<NotesDraft>,
    listed: Option<Listed>,
    /// The interview is shown above the items.
    pub interview: bool,
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
            thread: String::new(),
            head_kind: None,
            head_faces: Vec::new(),
            drill_depth: String::new(),
            thread_depth: String::new(),
            bevel: Vec::new(),
            min_mm: String::new(),
            max_mm: String::new(),
            pattern: true,
            from_at: At::Feature,
            to_at: At::Feature,
            characteristic: GdtCharacteristic::Position,
            value: String::new(),
            diameter: false,
            material: MaterialCondition::Regardless,
            datums: String::new(),
            target_shape: TargetShape::default(),
            target_number: String::new(),
            target_point: String::new(),
            gdt_direction: String::new(),
            gdt_length: String::new(),
            gdt_width: String::new(),
            frame_shape: FrameShape::default(),
            upper: SegmentDraft::default(),
            lower: SegmentDraft::default(),
            position_groups: Vec::new(),
            entry_faces: Vec::new(),
            roughness: String::new(),
            process: SurfaceProcess::Any,
            replaces: None,
            error: None,
            notes: None,
            listed: None,
            interview: false,
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
            PmiKind::DatumTarget(target) => {
                self.kind = Kind::DatumTarget;
                self.letter.clone_from(&target.datum);
                self.target_number = target.number.to_string();
                self.target_point = vector_text(target.point_mm);
                self.picks.push(target.face);
                self.target_shape = match &target.kind {
                    TargetKind::Point => TargetShape::Point,
                    TargetKind::Line { length_mm, direction } => {
                        self.gdt_length = length_mm.to_string();
                        self.gdt_direction = vector_text(*direction);
                        TargetShape::Line
                    }
                    TargetKind::Area { area: TargetArea::Circular { diameter_mm } } => {
                        self.gdt_length = diameter_mm.to_string();
                        TargetShape::Circular
                    }
                    TargetKind::Area { area: TargetArea::Rectangular { length_mm, width_mm, direction } } => {
                        self.gdt_length = length_mm.to_string();
                        self.gdt_width = width_mm.to_string();
                        self.gdt_direction = vector_text(*direction);
                        TargetShape::Rectangular
                    }
                };
            }
            PmiKind::PositionControl(control) => {
                self.kind = Kind::PositionControl;
                self.position_groups.clone_from(&control.features);
                match &control.frame {
                    PositionFrame::Single { segment, projected_zone } => {
                        self.upper = SegmentDraft::of(segment);
                        if let Some(zone) = projected_zone {
                            self.frame_shape = FrameShape::Projected;
                            self.gdt_length = zone.height_mm.to_string();
                            self.gdt_direction = vector_text(zone.direction);
                            self.entry_faces.clone_from(&zone.entry_faces);
                        }
                    }
                    PositionFrame::Composite { upper, lower } => {
                        self.frame_shape = FrameShape::Composite;
                        self.upper = SegmentDraft::of(upper);
                        self.lower = SegmentDraft::of(lower);
                    }
                }
            }
            PmiKind::Size { size, features, tolerance, through, thread, .. } => {
                self.kind = if *size == SizeKind::Diameter { Kind::Diameter } else { Kind::Radius };
                self.tolerance = tolerance.suffix().trim().to_owned();
                self.through = *through;
                self.thread = thread.clone().unwrap_or_default();
                self.pattern = features.len() > 1;
                self.picks = all(features);
            }
            PmiKind::Hole { pilot, head, tolerance, through, drill_depth, thread } => {
                self.kind = Kind::Hole;
                self.picks.clone_from(pilot);
                if let Some(head) = head {
                    self.head_kind = Some(head.kind);
                    self.head_faces.clone_from(&head.faces);
                }
                self.tolerance = tolerance.suffix().trim().to_owned();
                self.through = *through;
                if let Some(depth) = drill_depth {
                    self.drill_depth = depth.to_string();
                }
                if let Some(thread) = thread {
                    self.thread.clone_from(&thread.designation);
                    if let Some(depth) = &thread.depth {
                        self.thread_depth = depth.to_string();
                    }
                }
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
            PmiKind::Width { flats: [a, b], tolerance } => {
                self.kind = Kind::Width;
                self.from.clone_from(a);
                self.picks.clone_from(b);
                self.tolerance = tolerance.suffix().trim().to_owned();
            }
            PmiKind::BasicAngle { planes: [a, b] } => {
                self.kind = Kind::BasicAngle;
                self.from.clone_from(a);
                self.picks.clone_from(b);
            }
            PmiKind::Chamfer { bevel, sides: [a, b] } | PmiKind::EdgeBreak { bevel, sides: [a, b], .. } => {
                self.kind = Kind::Chamfer;
                self.bevel.clone_from(bevel);
                self.from.clone_from(a);
                self.picks.clone_from(b);
                if let PmiKind::EdgeBreak { min_mm, max_mm, .. } = &on_body.kind {
                    self.kind = Kind::EdgeBreak;
                    self.min_mm = min_mm.to_string();
                    self.max_mm = max_mm.to_string();
                }
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
    if form.picks.is_empty() && form.kind != Kind::PositionControl {
        return Err(form.kind.prompt().to_owned());
    }
    let planes =
        || -> Result<[Feature<FaceRef>; 2], String> { Ok([feature(doc, &form.from)?, feature(doc, &form.picks)?]) };
    let kind = match form.kind {
        Kind::Datum => PmiKind::Datum {
            letter: Some(form.letter.trim().to_ascii_uppercase())
                .filter(|letter| !letter.is_empty())
                .unwrap_or_else(|| pmi.next_datum_letter()),
            feature: feature(doc, &form.picks)?,
        },
        Kind::DatumTarget => {
            let [face] = form.picks.as_slice() else {
                return Err("Pick exactly one target face.".into());
            };
            let kind = match form.target_shape {
                TargetShape::Point => TargetKind::Point,
                TargetShape::Line => TargetKind::Line {
                    length_mm: decimal(&form.gdt_length, "line length")?,
                    direction: vector(&form.gdt_direction, "the target direction")?,
                },
                TargetShape::Circular => TargetKind::Area {
                    area: TargetArea::Circular { diameter_mm: decimal(&form.gdt_length, "target diameter")? },
                },
                TargetShape::Rectangular => TargetKind::Area {
                    area: TargetArea::Rectangular {
                        length_mm: decimal(&form.gdt_length, "target length")?,
                        width_mm: decimal(&form.gdt_width, "target width")?,
                        direction: vector(&form.gdt_direction, "the target direction")?,
                    },
                },
            };
            PmiKind::DatumTarget(DatumTarget {
                datum: form.letter.trim().to_ascii_uppercase(),
                number: form.target_number.trim().parse().map_err(|_| "Enter a positive target number.")?,
                face: doc.model().face_ref(*face).map_err(|error| error.to_string())?,
                point_mm: vector(&form.target_point, "the target point")?,
                kind,
            })
        }
        Kind::PositionControl => {
            let segment = form.upper.typed()?;
            let frame = match form.frame_shape {
                FrameShape::Single => PositionFrame::Single { segment, projected_zone: None },
                FrameShape::Composite => PositionFrame::Composite { upper: segment, lower: form.lower.typed()? },
                FrameShape::Projected => PositionFrame::Single {
                    segment,
                    projected_zone: Some(ProjectedZone {
                        height_mm: decimal(&form.gdt_length, "projection height")?,
                        direction: vector(&form.gdt_direction, "the outward direction")?,
                        entry_faces: form
                            .entry_faces
                            .iter()
                            .map(|face| doc.model().face_ref(*face).map_err(|error| error.to_string()))
                            .collect::<Result<_, _>>()?,
                    }),
                },
            };
            let features = form
                .position_groups
                .iter()
                .map(Vec::as_slice)
                .chain((!form.picks.is_empty()).then_some(form.picks.as_slice()))
                .map(|faces| feature(doc, faces))
                .collect::<Result<Vec<_>, _>>()?;
            if features.is_empty() {
                return Err(form.kind.prompt().into());
            }
            PmiKind::PositionControl(PositionControl { features, frame })
        }
        Kind::Diameter | Kind::Radius => PmiKind::Size {
            size: if form.kind == Kind::Diameter { SizeKind::Diameter } else { SizeKind::Radius },
            features: features(doc, &form.picks, form.pattern)?,
            tolerance: typed_tolerance(&form.tolerance)?,
            through: form.kind == Kind::Diameter && form.through,
            thread: Some(form.thread.trim().to_owned()).filter(|t| form.kind == Kind::Diameter && !t.is_empty()),
            nominal: None,
        },
        Kind::Hole => {
            let decimal = |input: &str, label: &str| {
                if input.trim().is_empty() {
                    Ok(None)
                } else {
                    Decimal::parse(input).map(Some).ok_or_else(|| format!("{label} must be a decimal in mm."))
                }
            };
            let head = match form.head_kind {
                Some(kind) => Some(HoleHead { kind, faces: feature(doc, &form.head_faces)? }),
                None => None,
            };
            let thread = match form.thread.trim() {
                "" if !form.thread_depth.trim().is_empty() => return Err("Enter a thread designation.".into()),
                "" => None,
                designation => Some(HoleThread {
                    designation: designation.to_owned(),
                    depth: decimal(&form.thread_depth, "Usable thread depth")?,
                }),
            };
            PmiKind::Hole {
                pilot: feature(doc, &form.picks)?,
                head,
                tolerance: typed_tolerance(&form.tolerance)?,
                through: form.through,
                drill_depth: decimal(&form.drill_depth, "Drill depth")?,
                thread,
            }
        }
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
        Kind::Width => PmiKind::Width { flats: planes()?, tolerance: typed_tolerance(&form.tolerance)? },
        Kind::BasicAngle => PmiKind::BasicAngle { planes: planes()? },
        Kind::Chamfer => PmiKind::Chamfer { bevel: feature(doc, &form.bevel)?, sides: planes()? },
        Kind::EdgeBreak => {
            let bound = |text: &str, label| Decimal::parse(text).ok_or_else(|| format!("Enter the {label} in mm."));
            PmiKind::EdgeBreak {
                bevel: feature(doc, &form.bevel)?,
                sides: planes()?,
                min_mm: bound(&form.min_mm, "minimum setback")?,
                max_mm: bound(&form.max_mm, "maximum setback")?,
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
            PmiKind::DatumTarget(target) => format!("Datum target {}{}", target.datum, target.number),
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
        let listed = Listed { geometry, pmi: pmi.clone(), rows, shown, questions: None };
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
    if form.kind == Kind::Hole {
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut form.head_kind, None, "No head");
            ui.selectable_value(&mut form.head_kind, Some(HoleHeadKind::Counterbore), "Counterbore");
            ui.selectable_value(&mut form.head_kind, Some(HoleHeadKind::Countersink), "Countersink");
        });
        if form.head_kind.is_some() {
            picked(ui, doc, "Head", &mut form.head_faces);
            if ui.button("Use picks as head").clicked() && !form.picks.is_empty() {
                form.head_faces = std::mem::take(&mut form.picks);
            }
        }
    }
    if matches!(form.kind, Kind::Chamfer | Kind::EdgeBreak) {
        picked(ui, doc, "Bevel", &mut form.bevel);
        if ui.button("Use picks as bevel").clicked() && !form.picks.is_empty() {
            form.bevel = std::mem::take(&mut form.picks);
        }
    }
    if matches!(form.kind, Kind::Width | Kind::Chamfer | Kind::EdgeBreak | Kind::BasicAngle) {
        picked(ui, doc, "First planar group", &mut form.from);
        if ui.button("Use picks as first group").clicked() && !form.picks.is_empty() {
            form.from = std::mem::take(&mut form.picks);
        }
    }
    let label = match form.kind {
        Kind::Distance => "To",
        Kind::Hole => "Pilot",
        Kind::Width | Kind::Chamfer | Kind::EdgeBreak | Kind::BasicAngle => "Second planar group",
        _ => "Picked",
    };
    picked(ui, doc, label, &mut form.picks);
    match form.kind {
        Kind::Datum => {
            let hint = doc.model().pmi().next_datum_letter();
            labelled(
                ui.add(egui::TextEdit::singleline(&mut form.letter).hint_text(format!("Letter, {hint} if empty"))),
                "Datum letter",
            );
        }
        Kind::DatumTarget => {
            entry(ui, &mut form.letter, "Declared datum");
            entry(ui, &mut form.target_number, "Target number");
            entry(ui, &mut form.target_point, "Target point X, Y, Z (mm)");
            ui.horizontal_wrapped(|ui| {
                for (kind, label) in [
                    (TargetShape::Point, "Point"),
                    (TargetShape::Line, "Line"),
                    (TargetShape::Circular, "Circular area"),
                    (TargetShape::Rectangular, "Rectangular area"),
                ] {
                    ui.selectable_value(&mut form.target_shape, kind, label);
                }
            });
            match form.target_shape {
                TargetShape::Point => {}
                TargetShape::Circular => entry(ui, &mut form.gdt_length, "Diameter (mm)"),
                TargetShape::Line | TargetShape::Rectangular => {
                    entry(ui, &mut form.gdt_length, "Length (mm)");
                    entry(ui, &mut form.gdt_direction, "Tangent direction X, Y, Z");
                    if form.target_shape == TargetShape::Rectangular {
                        entry(ui, &mut form.gdt_width, "Width (mm)");
                    }
                }
            }
        }
        Kind::PositionControl => {
            let mut empty = None;
            for (i, faces) in form.position_groups.iter_mut().enumerate() {
                picked(ui, doc, &format!("Controlled feature {}", i + 1), faces);
                if faces.is_empty() {
                    empty = Some(i);
                }
            }
            if let Some(i) = empty {
                form.position_groups.remove(i);
            }
            if ui.add_enabled(!form.picks.is_empty(), egui::Button::new("Use picks as feature")).clicked() {
                form.position_groups.push(std::mem::take(&mut form.picks));
            }
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut form.frame_shape, FrameShape::Single, "Single");
                ui.selectable_value(&mut form.frame_shape, FrameShape::Composite, "Composite");
                ui.selectable_value(&mut form.frame_shape, FrameShape::Projected, "Projected");
            });
            form.upper.show(ui, if form.frame_shape == FrameShape::Composite { "Upper segment" } else { "Segment" });
            if form.frame_shape == FrameShape::Composite {
                form.lower.show(ui, "Lower segment");
            }
            if form.frame_shape == FrameShape::Projected {
                picked(ui, doc, "Entry faces (feature order)", &mut form.entry_faces);
                if ui.add_enabled(!form.picks.is_empty(), egui::Button::new("Use picks as entries")).clicked() {
                    form.entry_faces = std::mem::take(&mut form.picks);
                }
                entry(ui, &mut form.gdt_length, "Projection height (mm)");
                entry(ui, &mut form.gdt_direction, "Outward direction X, Y, Z");
            }
        }
        Kind::Diameter | Kind::Radius | Kind::Hole | Kind::Distance | Kind::Slot | Kind::Width => {
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
            if matches!(form.kind, Kind::Diameter | Kind::Hole) {
                ui.checkbox(&mut form.through, "Through hole");
                labelled(
                    ui.add(
                        egui::TextEdit::singleline(&mut form.thread)
                            .hint_text("Thread: M6x1-6H, empty for a plain hole"),
                    ),
                    "Thread",
                );
            }
            if form.kind == Kind::Hole {
                if !form.through {
                    labelled(
                        ui.add(
                            egui::TextEdit::singleline(&mut form.drill_depth)
                                .hint_text("Drill depth (mm), blank to measure"),
                        ),
                        "Drill depth",
                    );
                } else {
                    form.drill_depth.clear();
                }
                labelled(
                    ui.add(
                        egui::TextEdit::singleline(&mut form.thread_depth)
                            .hint_text("Usable thread depth (mm), optional"),
                    ),
                    "Usable thread depth",
                );
            }
            if matches!(form.kind, Kind::Diameter | Kind::Radius) && form.picks.len() > 1 {
                ui.checkbox(&mut form.pattern, "Each picked face is one of a pattern");
            }
        }
        Kind::EdgeBreak => {
            labelled(
                ui.add(egui::TextEdit::singleline(&mut form.min_mm).hint_text("Minimum setback (mm)")),
                "Minimum setback",
            );
            labelled(
                ui.add(egui::TextEdit::singleline(&mut form.max_mm).hint_text("Maximum setback (mm)")),
                "Maximum setback",
            );
        }
        Kind::Chamfer | Kind::BasicAngle => {}
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
        if let Some(feature) = row.feature {
            // The hole states it: the thread is changed in the hole, not here.
            let hole =
                state.doc.feature(u64::from(feature.0)).map_or_else(|| "its hole".to_owned(), |f| f.name.clone());
            ui.label(egui::RichText::new(format!("From {hole}; change the thread in the hole")).color(theme::muted()));
            continue;
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
/// The interview: each decision the part has not taken, with the proposal the rules
/// make and why, and a button to take it.
fn interview(ui: &mut egui::Ui, state: &mut AppState) {
    let questions = match state.pmi.listed.as_mut() {
        Some(listed) => listed.questions.get_or_insert_with(|| state.doc.model().interview()).clone(),
        None => return,
    };
    let questions = match questions {
        Ok(questions) => questions,
        Err(error) => {
            ui.label(error);
            return;
        }
    };
    if questions.is_empty() {
        ui.label(egui::RichText::new("Nothing is missing.").color(theme::muted()));
        return;
    }
    let mut accept: Option<serde_json::Value> = None;
    if ui.button("Accept every proposal").clicked() {
        accept = Some(serde_json::json!("all"));
    }
    for question in &questions {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(&question.asks).family(theme::semibold()).color(theme::ink()));
        if question.proposes.is_empty() {
            ui.label("No rule can say. Type it in the notes above.");
        } else {
            ui.horizontal(|ui| {
                ui.label(format!("Proposed: {}", question.proposes));
                let button = ui.small_button("Accept");
                button.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Accept {}", question.id))
                });
                if button.clicked() {
                    accept = Some(serde_json::json!([question.id]));
                }
            });
        }
        ui.label(egui::RichText::new(&question.reason).color(theme::muted()));
    }
    if let Some(accept) = accept {
        state.pmi.error = super::tools::answer(&mut state.doc, &serde_json::json!({ "accept": accept })).err();
    }
}

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
    ui.checkbox(&mut state.pmi.interview, "Interview: ask for what is missing");
    let height = ui.available_height().max(120.0);
    egui::ScrollArea::vertical().id_salt("pmi_items").auto_shrink([false, true]).max_height(height).show(ui, |ui| {
        if state.pmi.interview {
            interview(ui, state);
            ui.separator();
        }
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
