//! Manufacturing information tools: add, change, remove and list a part's items, recognise
//! its features and run the interview. MCP clients and `soliddraft run` get every one;
//! the assistant gets the six its registry rows give it (see `assistant.rs`).

use serde_json::{Value, json};
use soliddraft_kernel_api::pmi::{
    Anchor, At, DatumRef, DatumTarget, Decimal, Feature, GdtCharacteristic, HoleHead, HoleHeadKind, HoleThread, Lay,
    MaterialCondition, PartNotes, PmiItem, PmiKind, PositionControl, PositionFrame, SizeKind, SurfaceProcess,
    Tolerance,
};
use soliddraft_model::FaceRef;

use crate::command_tools::{face_by_name, name_of_face};
use crate::document::Document;

fn text<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty())
}

/// A number given as a number or as text.
fn number(input: &Value, key: &str) -> Result<f64, String> {
    let read = match input.get(key) {
        Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
        Some(Value::Number(number)) => number.as_f64(),
        _ => return Err(format!("{key} is required")),
    };
    read.filter(|value| value.is_finite()).ok_or_else(|| format!("{key} is a number, such as 0.05"))
}

fn depth(input: &Value, key: &str) -> Result<Option<Decimal>, String> {
    let parsed = match input.get(key) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(value)) => Decimal::parse(value),
        Some(Value::Number(value)) => Decimal::parse(&value.to_string()),
        _ => None,
    };
    parsed.map(Some).ok_or_else(|| format!("{key} must be a decimal in mm"))
}

/// The faces of one feature, from their names.
fn feature(doc: &Document, value: &Value, what: &str) -> Result<Feature<FaceRef>, String> {
    let names =
        value.as_array().filter(|names| !names.is_empty()).ok_or_else(|| format!("{what} is a list of face names"))?;
    let mut faces = Vec::with_capacity(names.len());
    for name in names {
        let name = name.as_str().ok_or_else(|| format!("{what} is a list of face names"))?;
        faces.push(
            face_by_name(doc, name)
                .ok_or_else(|| format!("no face named `{name}`; `feature` lists each feature's faces by name"))?,
        );
    }
    doc.model().pmi_feature(&faces).map_err(|error| error.to_string())
}

/// One feature from `faces`, or the features of a pattern from `features`.
fn features(doc: &Document, input: &Value) -> Result<Vec<Feature<FaceRef>>, String> {
    let given = |key: &str| input.get(key).filter(|value| !value.is_null());
    match (given("faces"), given("features")) {
        (Some(faces), None) => Ok(vec![feature(doc, faces, "faces")?]),
        (None, Some(list)) => list
            .as_array()
            .filter(|list| !list.is_empty())
            .ok_or("features is a list of features, each a list of face names")?
            .iter()
            .map(|faces| feature(doc, faces, "a feature"))
            .collect(),
        _ => Err("give faces for one feature, or features for a pattern".into()),
    }
}

fn named_feature(doc: &Document, input: &Value, key: &str) -> Result<Feature<FaceRef>, String> {
    feature(doc, input.get(key).ok_or_else(|| format!("{key} is required"))?, key)
}

/// The two features `key` lists, such as a slot's flat sides.
fn pair(doc: &Document, input: &Value, key: &str) -> Result<[Feature<FaceRef>; 2], String> {
    let wrong = || format!("{key} is two features, each a list of face names");
    match input.get(key).and_then(Value::as_array).map(Vec::as_slice) {
        Some([first, second]) => Ok([feature(doc, first, key)?, feature(doc, second, key)?]),
        _ => Err(wrong()),
    }
}

fn tolerance(input: &Value) -> Result<Tolerance, String> {
    match text(input, "tolerance") {
        None => Ok(Tolerance::None),
        Some(written) => Tolerance::parse(written)
            .ok_or_else(|| format!("tolerance `{written}` is not H7, \u{b1}0.1, +0.1/0, +0.2/-0.1 or basic")),
    }
}

fn anchor(doc: &Document, input: &Value, end: &str) -> Result<Anchor<FaceRef>, String> {
    let feature = named_feature(doc, input, &format!("{end}_faces"))?;
    let at = match text(input, &format!("{end}_at")) {
        None | Some("feature") => At::Feature,
        Some("middle") => At::Middle,
        Some("inner") => At::Inner,
        Some("outer") => At::Outer,
        Some(other) => return Err(format!("{end}_at `{other}` is not feature, middle, inner or outer")),
    };
    Ok(Anchor { feature, at })
}

fn characteristic(name: &str) -> Result<GdtCharacteristic, String> {
    let key = name.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    GdtCharacteristic::from_key(&key).ok_or_else(|| {
        let keys: Vec<&str> = GdtCharacteristic::ALL.iter().map(|c| c.key()).collect();
        format!("no characteristic `{name}` (there are: {})", keys.join(", "))
    })
}

fn material(input: &Value, key: &str) -> Result<MaterialCondition, String> {
    match text(input, key) {
        None | Some("regardless") => Ok(MaterialCondition::Regardless),
        Some("maximum") => Ok(MaterialCondition::Maximum),
        Some("least") => Ok(MaterialCondition::Least),
        Some(other) => Err(format!("{key} `{other}` is not regardless, maximum or least")),
    }
}

fn lay(name: &str) -> Result<Lay, String> {
    Ok(match name {
        "parallel" => Lay::Parallel,
        "perpendicular" => Lay::Perpendicular,
        "crossed" => Lay::Crossed,
        "multidirectional" => Lay::Multidirectional,
        "circular" => Lay::Circular,
        "radial" => Lay::Radial,
        "particulate" => Lay::Particulate,
        other => {
            return Err(format!(
                "lay `{other}` is not parallel, perpendicular, crossed, multidirectional, circular, radial or \
                 particulate"
            ));
        }
    })
}

fn fields(input: &Value, allowed: &[&str]) -> Result<(), String> {
    let object = input.as_object().ok_or("PMI input must be an object.")?;
    if let Some(key) = object
        .keys()
        .find(|key| !allowed.contains(&key.as_str()) && !matches!(key.as_str(), "kind" | "name" | "replaces"))
    {
        return Err(format!("Unexpected field {key} for this PMI kind."));
    }
    Ok(())
}

fn typed_gdt(doc: &Document, input: &Value, target: bool) -> Result<PmiKind<FaceRef>, String> {
    let kind = if target {
        fields(input, &["target"])?;
        let target: DatumTarget<String> =
            serde_json::from_value(input["target"].clone()).map_err(|error| format!("Datum target: {error}"))?;
        PmiKind::DatumTarget(target)
    } else {
        fields(input, &["faces", "features", "frame"])?;
        for slot in ["segment", "upper", "lower"] {
            if let Some(datums) = input["frame"][slot]["datums"].as_array() {
                for datum in datums {
                    let datum = datum.as_object().ok_or("A datum reference needs a letter and optional material.")?;
                    if datum.keys().any(|key| !matches!(key.as_str(), "letter" | "material")) {
                        return Err("A datum reference accepts only letter and one material modifier.".into());
                    }
                }
            }
        }
        let frame: PositionFrame<String> =
            serde_json::from_value(input["frame"].clone()).map_err(|error| format!("Position frame: {error}"))?;
        PmiKind::PositionControl(PositionControl { features: Vec::new(), frame })
    };
    let mut mapped = PmiItem { id: String::new(), kind }
        .try_map(|name| {
            let face = face_by_name(doc, name).ok_or_else(|| format!("No face named `{name}`."))?;
            doc.model().face_ref(face).map_err(|error| error.to_string())
        })?
        .kind;
    if let PmiKind::PositionControl(control) = &mut mapped {
        control.features = features(doc, input)?;
    }
    Ok(mapped)
}

/// The item a `pmi_add` call describes, named, on the part's faces.
fn item(doc: &Document, input: &Value) -> Result<PmiItem<FaceRef>, String> {
    let pmi = doc.model().pmi();
    let replaces = text(input, "replaces");
    let requested = text(input, "kind").ok_or("kind is required")?;
    if input.get("projected_zone").is_some() || (requested != "position_control" && input.get("frame").is_some()) {
        return Err("Projection and composite segments require a position_control frame.".into());
    }
    if requested != "datum_target" && input.get("target").is_some() {
        return Err("A target payload requires the datum_target kind.".into());
    }
    let kind = match requested {
        "datum" => {
            let letter = match text(input, "letter") {
                Some(letter) => letter.to_owned(),
                // A replaced datum keeps its letter unless another is given.
                None => replaces
                    .and_then(|name| pmi.item(name))
                    .and_then(|old| match &old.kind {
                        PmiKind::Datum { letter, .. } => Some(letter.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| pmi.next_datum_letter()),
            };
            PmiKind::Datum { letter, feature: named_feature(doc, input, "faces")? }
        }
        size @ ("diameter" | "radius") => PmiKind::Size {
            size: if size == "diameter" { SizeKind::Diameter } else { SizeKind::Radius },
            features: features(doc, input)?,
            tolerance: tolerance(input)?,
            through: input["through"].as_bool().unwrap_or(false),
            thread: text(input, "thread").map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()),
            nominal: None,
        },
        "hole" => {
            let head = match text(input, "head_kind") {
                None if input.get("head_faces").is_some_and(|faces| !faces.is_null()) => {
                    return Err("head_kind is required when head_faces are given".into());
                }
                None => None,
                Some(kind) => Some(HoleHead {
                    kind: match kind {
                        "counterbore" => HoleHeadKind::Counterbore,
                        "countersink" => HoleHeadKind::Countersink,
                        _ => return Err("head_kind is counterbore or countersink".into()),
                    },
                    faces: named_feature(doc, input, "head_faces")?,
                }),
            };
            let thread_depth = depth(input, "thread_depth")?;
            let thread = match text(input, "thread") {
                Some(designation) => Some(HoleThread { designation: designation.to_owned(), depth: thread_depth }),
                None if thread_depth.is_some() => return Err("thread is required when thread_depth is given".into()),
                None => None,
            };
            PmiKind::Hole {
                pilot: named_feature(doc, input, "pilot_faces")?,
                head,
                through: input["through"].as_bool().unwrap_or(false),
                drill_depth: depth(input, "drill_depth")?,
                tolerance: tolerance(input)?,
                thread,
            }
        }
        "distance" => PmiKind::Location {
            from: anchor(doc, input, "from")?,
            to: anchor(doc, input, "to")?,
            tolerance: tolerance(input)?,
            nominal: None,
        },
        "slot" => PmiKind::Slot {
            flats: pair(doc, input, "flats")?,
            ends: pair(doc, input, "ends")?,
            tolerance: tolerance(input)?,
        },
        "width" => PmiKind::Width { flats: pair(doc, input, "flats")?, tolerance: tolerance(input)? },
        "chamfer" => PmiKind::Chamfer { bevel: named_feature(doc, input, "bevel")?, sides: pair(doc, input, "sides")? },
        "edge_break" => PmiKind::EdgeBreak {
            bevel: named_feature(doc, input, "bevel")?,
            sides: pair(doc, input, "sides")?,
            min_mm: depth(input, "min_mm")?.ok_or("min_mm is required")?,
            max_mm: depth(input, "max_mm")?.ok_or("max_mm is required")?,
        },
        "basic_angle" => PmiKind::BasicAngle { planes: pair(doc, input, "planes")? },
        "datum_target" => typed_gdt(doc, input, true)?,
        "position_control" => typed_gdt(doc, input, false)?,
        "tolerance" => PmiKind::Gdt {
            characteristic: characteristic(text(input, "characteristic").ok_or("characteristic is required")?)?,
            tolerance: number(input, "value")?,
            diameter: input["diameter_zone"].as_bool().unwrap_or(false),
            material: material(input, "material")?,
            datums: match input.get("datums").filter(|value| !value.is_null()) {
                None => Vec::new(),
                Some(list) => list
                    .as_array()
                    .ok_or("datums is a list of letters")?
                    .iter()
                    .map(|letter| letter.as_str().map(DatumRef::new).ok_or("datums is a list of letters"))
                    .collect::<Result<_, _>>()?,
            },
            features: features(doc, input)?,
        },
        "finish" => PmiKind::SurfaceFinish {
            text: text(input, "roughness").ok_or("roughness is required, such as Ra 1.6")?.to_owned(),
            process: match text(input, "process") {
                None | Some("any") => SurfaceProcess::Any,
                Some("removal") => SurfaceProcess::Removal,
                Some("no_removal") => SurfaceProcess::NoRemoval,
                Some(other) => return Err(format!("process `{other}` is not any, removal or no_removal")),
            },
            method: text(input, "method").unwrap_or_default().to_owned(),
            lay: text(input, "lay").map(lay).transpose()?,
            allowance: text(input, "allowance").unwrap_or_default().to_owned(),
            all_around: input["all_around"].as_bool().unwrap_or(false),
            feature: named_feature(doc, input, "faces")?,
        },
        "knurl" | "face_note" => {
            let Some([name]) = input["faces"].as_array().map(Vec::as_slice) else {
                return Err("faces must name exactly one face.".into());
            };
            let name = name.as_str().ok_or("faces must name exactly one face.")?;
            let id = face_by_name(doc, name).ok_or_else(|| format!("No face named `{name}`."))?;
            let face = doc.model().face_ref(id).map_err(|error| error.to_string())?;
            if requested == "knurl" {
                let designation = input["designation"].as_str().ok_or("designation is required")?.to_owned();
                PmiKind::Knurl { designation, face }
            } else {
                let text = input["text"].as_str().ok_or("text is required")?.to_owned();
                PmiKind::FaceNote { text, face }
            }
        }
        other => {
            return Err(format!(
                "unknown PMI kind `{other}`; use datum, datum_target, diameter, radius, hole, distance, slot, width, chamfer, edge_break, basic_angle, tolerance, position_control, finish, knurl or face_note"
            ));
        }
    };
    let mut item = PmiItem { id: String::new(), kind };
    item.id = match (text(input, "name"), replaces) {
        (Some(name), _) => name.to_owned(),
        (None, Some(old)) => old.to_owned(),
        (None, None) => pmi.free_name(&match &item.kind {
            PmiKind::Datum { letter, .. } => format!("Datum {letter}"),
            PmiKind::DatumTarget(target) => format!("Datum target {}{}", target.datum, target.number),
            _ => item.kind_label().to_owned(),
        }),
    };
    Ok(item)
}

/// The part's items as rows of JSON: name, kind, callout, faces and any problem.
pub(crate) fn rows(doc: &Document) -> Value {
    rows_named(doc, doc)
}

/// The rows of `doc`'s items, with faces named as `names` names them: a scratch copy's
/// items, named from the part it was copied from.
pub(crate) fn rows_named(doc: &Document, names: &Document) -> Value {
    let rows: Vec<Value> = doc
        .model()
        .pmi_rows()
        .iter()
        .map(|row| {
            // Faces by the names the part gives them now, or as they were called when
            // the item was made if they are gone.
            let faces: Vec<Vec<String>> = match &row.on_body {
                Some(on_body) => on_body
                    .features()
                    .into_iter()
                    .map(|feature| feature.iter().filter_map(|face| name_of_face(names, *face)).collect())
                    .collect(),
                None => row
                    .item
                    .features()
                    .into_iter()
                    .map(|feature| feature.iter().map(|face| face.label.clone()).collect())
                    .collect(),
            };
            let mut value = json!({
                "name": row.item.id,
                "kind": row.item.kind_label(),
                "callout": row.callout(),
                "faces": faces,
            });
            match &row.item.kind {
                PmiKind::Knurl { designation, .. } => value["designation"] = json!(designation),
                PmiKind::FaceNote { text, .. } => value["text"] = json!(text),
                PmiKind::DatumTarget(target) => {
                    value["target"] = json!({
                        "datum": target.datum, "number": target.number, "face": faces[0].first(),
                        "point_mm": target.point_mm, "kind": target.kind
                    });
                }
                PmiKind::PositionControl(control) => {
                    value["frame"] = match &control.frame {
                        PositionFrame::Single { segment, projected_zone } => {
                            let zone = projected_zone.as_ref().map(|zone| json!({
                                "height_mm": zone.height_mm, "direction": zone.direction,
                                "entry_faces": faces[control.features.len()..].iter().map(|group| group.first()).collect::<Vec<_>>()
                            }));
                            json!({"frame": "single", "segment": segment, "projected_zone": zone})
                        }
                        PositionFrame::Composite { upper, lower } => {
                            json!({"frame": "composite", "upper": upper, "lower": lower})
                        }
                    };
                }
                _ => {}
            }
            match (&row.item.kind, row.measured.as_slice()) {
                (PmiKind::BasicAngle { .. }, [angle]) => value["measured_deg"] = json!([angle]),
                (PmiKind::Chamfer { .. }, [setback, angle]) => {
                    value["measured_mm"] = json!([setback]);
                    value["measured_deg"] = json!([angle]);
                }
                (_, []) => {}
                (_, measured) => value["measured_mm"] = json!(measured),
            }
            if let PmiKind::EdgeBreak { min_mm, max_mm, .. } = &row.item.kind {
                value["bounds_mm"] = json!({ "min": min_mm.value(), "max": max_mm.value() });
            }
            if let PmiKind::Hole { head, tolerance, through, drill_depth, thread, .. } = &row.item.kind {
                let mut hole = json!({ "pilot_faces": faces[0], "through": through });
                if let Some(head) = head {
                    hole["head_kind"] = json!(head.kind);
                    hole["head_faces"] = json!(faces[1]);
                }
                if let Some(depth) = drill_depth {
                    hole["drill_depth"] = json!(depth);
                }
                if let Some(thread) = thread {
                    hole["thread"] = json!(thread.designation);
                    if let Some(depth) = &thread.depth {
                        hole["thread_depth"] = json!(depth);
                    }
                }
                if !tolerance.is_none() {
                    hole["tolerance"] = json!(tolerance.suffix().trim());
                }
                if let Some(measured) = &row.hole {
                    hole["measured"] = json!(measured);
                }
                value["hole"] = hole;
            }
            if !row.located.is_empty() {
                value["located"] = json!(row.located);
            }
            if let Some(problem) = &row.problem {
                value["problem"] = json!(problem);
            }
            if let Some(feature) = row.feature {
                // A threaded hole states the item; it changes with the hole.
                value["stated_by"] = json!(doc.feature(u64::from(feature.0)).map(|f| f.name.clone()));
            }
            value
        })
        .collect();
    let mut out = json!({ "items": rows, "notes": doc.model().part_notes() });
    if let Some(report) = &doc.pmi_import {
        out["read_from_file"] = json!(report);
    }
    out
}

fn notes(doc: &Document, input: &Value) -> Result<PartNotes, String> {
    let mut notes = doc.model().pmi().part.clone();
    // An empty text clears a field; a field left out keeps its value.
    let field = |key: &str, slot: &mut Option<String>| match input.get(key) {
        Some(Value::String(written)) => *slot = Some(written.trim().to_owned()).filter(|text| !text.is_empty()),
        Some(Value::Null) | None => {}
        Some(_) => {}
    };
    field("general_tolerance", &mut notes.general_tolerance);
    field("material", &mut notes.material);
    if let Some(list) = input.get("notes").filter(|value| !value.is_null()) {
        notes.notes = list
            .as_array()
            .ok_or("notes is a list of texts")?
            .iter()
            .map(|note| note.as_str().map(|text| text.trim().to_owned()).ok_or("notes is a list of texts"))
            .collect::<Result<_, _>>()?;
    }
    Ok(notes)
}

/// Runs the manufacturing information tools that work on the part alone. `None` hands
/// the call on.
pub(crate) fn run(doc: &mut Document, name: &str, input: &Value) -> Option<Result<Value, String>> {
    Some(match name {
        "pmi_add" => (|| {
            let item = item(doc, input)?;
            let name = item.id.clone();
            doc.put_pmi(item, text(input, "replaces").map(str::to_owned))?;
            let listed = rows(doc);
            let row = listed["items"].as_array().and_then(|rows| rows.iter().find(|row| row["name"] == name.as_str()));
            Ok(json!({ "added": row }))
        })(),
        "pmi_remove" => (|| {
            let name = text(input, "name").ok_or("name is required")?;
            doc.remove_pmi(name)?;
            Ok(json!({ "removed": name }))
        })(),
        "pmi_notes" => (|| {
            let notes = notes(doc, input)?;
            doc.set_part_notes(notes)?;
            Ok(json!({ "notes": doc.model().part_notes() }))
        })(),
        "pmi_list" => Ok(rows(doc)),
        "pmi_recognise" => doc.model().recognised().map(|features| {
            let names = |faces: &[soliddraft_kernel_api::FaceId]| -> Vec<String> {
                faces.iter().filter_map(|face| name_of_face(doc, *face)).collect()
            };
            let listed: Vec<Value> = features
                .iter()
                .map(|feature| {
                    let mut row = json!({
                        "kind": feature.kind,
                        "name": feature.name,
                        "faces": names(&feature.faces),
                        "sizes_mm": feature.sizes.iter().map(|(name, value)| (name.clone(), json!(value))).collect::<serde_json::Map<String, Value>>(),
                    });
                    if let Some(through) = feature.through {
                        row["through"] = json!(through);
                    }
                    if !feature.opens_onto.is_empty() {
                        row[if feature.kind == "boss" { "stands_on" } else { "opens_onto" }] = json!(names(&feature.opens_onto));
                    }
                    if !feature.closed_by.is_empty() {
                        row["closed_by"] = json!(names(&feature.closed_by));
                    }
                    row
                })
                .collect();
            json!({ "features": listed })
        }),
        "pmi_interview" => doc.model().interview().map(|questions| json!({ "questions": questions })),
        "pmi_answer" => answer(doc, input),
        _ => return None,
    })
}

/// Takes the interview's proposals for the questions named in `accept` (or all of them),
/// and the material when it is given, as one undo entry. Answers what was added and
/// the questions still open.
pub(super) fn answer(doc: &mut Document, input: &Value) -> Result<Value, String> {
    use soliddraft_model::interview::{Proposal, Question};
    let questions = doc.model().interview()?;
    let material = text(input, "material").map(str::trim).filter(|text| !text.is_empty());
    let wrong = || "accept lists question ids, or is \"all\"".to_owned();
    // "all" takes the questions themselves, never their ids looked up again.
    let chosen: Vec<&Question> = match input.get("accept") {
        Some(Value::Array(ids)) => ids
            .iter()
            .map(|id| {
                let id = id.as_str().ok_or_else(wrong)?;
                questions
                    .iter()
                    .find(|q| q.id == id)
                    .ok_or_else(|| format!("No open question `{id}`. pmi_interview lists the open ones."))
            })
            .collect::<Result<_, _>>()?,
        Some(Value::String(word)) if word == "all" => questions.iter().filter(|q| q.proposal.is_some()).collect(),
        Some(Value::Null) | None => Vec::new(),
        Some(_) => return Err(wrong()),
    };
    if chosen.is_empty() && material.is_none() {
        return Err("Nothing to answer: give accept, material or both.".into());
    }
    let mut notes = doc.model().pmi().part.clone();
    let mut notes_changed = false;
    if let Some(material) = material {
        notes.material = Some(material.to_owned());
        notes_changed = true;
    }
    let mut commands = Vec::new();
    let mut accepted = Vec::new();
    for question in chosen {
        match &question.proposal {
            Some(Proposal::Item { item }) => {
                commands.push(soliddraft_model::Command::PutPmiItem { item: item.clone(), replaces: None });
            }
            Some(Proposal::GeneralTolerance { text }) => {
                notes.general_tolerance = Some(text.clone());
                notes_changed = true;
            }
            None => return Err(format!("`{}` has no proposal. Answer it yourself: {}", question.id, question.asks)),
        }
        accepted.push(question.id.clone());
    }
    if notes_changed {
        commands.insert(0, soliddraft_model::Command::SetPartNotes { notes });
    }
    doc.run_pmi("Interview answers", commands)?;
    Ok(json!({
        "accepted": accepted,
        "material": material,
        "questions": doc.model().interview()?,
        "items": rows(doc)["items"],
    }))
}
