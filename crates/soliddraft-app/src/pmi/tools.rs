//! Manufacturing information over MCP: add, change, remove and list a part's items.

use serde_json::{Value, json};
use soliddraft_kernel_api::pmi::{
    Anchor, At, DatumRef, Feature, GdtCharacteristic, Lay, MaterialCondition, PartNotes, PmiItem, PmiKind, SizeKind,
    SurfaceProcess, Tolerance,
};
use soliddraft_model::FaceRef;

use crate::command_tools::{face_by_name, name_of_face};
use crate::document::Document;
use crate::state::AppState;

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

/// The item a `pmi_add` call describes, named, on the part's faces.
fn item(doc: &Document, input: &Value) -> Result<PmiItem<FaceRef>, String> {
    let pmi = doc.model().pmi();
    let replaces = text(input, "replaces");
    let kind = match text(input, "kind").ok_or("kind is required")? {
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
            nominal: None,
        },
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
        other => {
            return Err(format!("kind `{other}` is not datum, diameter, radius, distance, slot, tolerance or finish"));
        }
    };
    let mut item = PmiItem { id: String::new(), kind };
    item.id = match (text(input, "name"), replaces) {
        (Some(name), _) => name.to_owned(),
        (None, Some(old)) => old.to_owned(),
        (None, None) => pmi.free_name(&match &item.kind {
            PmiKind::Datum { letter, .. } => format!("Datum {letter}"),
            _ => item.kind_label().to_owned(),
        }),
    };
    Ok(item)
}

/// The part's items as rows of JSON: name, kind, callout, faces and any problem.
pub(crate) fn rows(doc: &Document) -> Value {
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
                    .map(|feature| feature.iter().filter_map(|face| name_of_face(doc, *face)).collect())
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
            if !row.measured.is_empty() {
                value["measured_mm"] = json!(row.measured);
            }
            if !row.located.is_empty() {
                value["located"] = json!(row.located);
            }
            if let Some(problem) = &row.problem {
                value["problem"] = json!(problem);
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

/// Runs the manufacturing information tools. `None` hands the call on.
pub(crate) fn run(state: &mut AppState, name: &str, input: &Value) -> Option<Result<Value, String>> {
    let doc = &mut state.doc;
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
    use soliddraft_model::interview::Proposal;
    let questions = doc.model().interview()?;
    let material = text(input, "material").map(str::trim).filter(|text| !text.is_empty());
    let all = input.get("accept").and_then(Value::as_str) == Some("all");
    let named: Vec<String> = match input.get("accept") {
        Some(Value::Array(ids)) => ids
            .iter()
            .map(|id| id.as_str().map(str::to_owned).ok_or("accept lists question ids, or is \"all\""))
            .collect::<Result<_, _>>()?,
        Some(Value::String(word)) if word == "all" => {
            questions.iter().filter(|q| q.proposal.is_some()).map(|q| q.id.clone()).collect()
        }
        Some(Value::Null) | None => Vec::new(),
        Some(_) => return Err("accept lists question ids, or is \"all\"".into()),
    };
    if named.is_empty() && material.is_none() {
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
    for id in &named {
        let question = questions
            .iter()
            .find(|q| q.id == *id)
            .ok_or_else(|| format!("No open question `{id}`. pmi_interview lists the open ones."))?;
        match &question.proposal {
            Some(Proposal::Item { item }) => {
                commands.push(soliddraft_model::Command::PutPmiItem { item: item.clone(), replaces: None });
            }
            Some(Proposal::GeneralTolerance { text }) => {
                notes.general_tolerance = Some(text.clone());
                notes_changed = true;
            }
            None if all => continue,
            None => return Err(format!("`{id}` has no proposal. Answer it yourself: {}", question.asks)),
        }
        accepted.push(id.clone());
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
